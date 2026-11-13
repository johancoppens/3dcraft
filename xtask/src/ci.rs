//! The gate: `cargo xtask ci` is fmt, clippy, the tests, the layer check
//! and the wasm check, in that order.
//!
//! The tests are built once (`cargo test --workspace --no-run`) and their
//! binaries are then run side by side, slowest first, so that one long
//! file does not hold up the rest: `cargo test --workspace` runs the
//! binaries one after the other, and the gate spent most of its 20
//! minutes with one busy core. The doctests still go through cargo. The
//! durations of the last run are kept in `target/ci-durations.txt` to
//! order the next one; a binary without a record goes first.
//!
//! The performance benchmark (`crates/engine/tests/benchmark.rs`, work
//! counted against a baseline, docs/testing.md › Performance tests) is one
//! of those binaries; it finds `CARGO_TARGET_TMPDIR` set to `target/tmp`
//! and leaves its counts there for `cargo xtask health`.
//!
//! `cargo xtask test -p <crate> [--test <file>]` runs some crates' test
//! binaries the same way, for a check of less than the whole workspace:
//! `cargo test -p craft3d-engine` runs its two hundred binaries one after
//! the other. It adds the times of the binaries it ran to the same file and
//! keeps every other entry, so the gate's order never shrinks to a subset.

use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

/// A test binary `cargo test --no-run` built.
struct TestBinary {
    /// `crates/engine: test panic_hunt`.
    label: String,
    executable: PathBuf,
    /// Cargo runs a test binary in its package's directory.
    manifest_dir: PathBuf,
}

pub fn run() -> Result<(), String> {
    super::cargo(&["fmt", "--all", "--check"])?;
    super::cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    let binaries = build_tests(&["--workspace"])?;
    run_side_by_side(binaries)?;
    super::cargo(&["test", "--workspace", "--doc"])?;
    super::layers::check()?;
    super::wasm()
}

const TEST_USAGE: &str = "usage: cargo xtask test -p <crate> [-p <crate> ...] [--test <file> ...]";

/// `cargo xtask test`: builds the tests of the crates named with `-p`
/// (only the test files named with `--test`, when there are any) and runs
/// their binaries side by side, slowest first, as `cargo xtask ci` runs the
/// workspace's: the same lines per binary, the same report, an error when
/// one failed. The doctests are not run (`cargo test -p <crate> --doc`; the
/// gate runs them all).
pub fn test(args: &[String]) -> Result<(), String> {
    let selection = test_selection(args)?;
    let selection: Vec<&str> = selection.iter().map(String::as_str).collect();
    let binaries = build_tests(&selection)?;
    let result = run_side_by_side(binaries);
    eprintln!("xtask test: the doctests were not run (`cargo test -p <crate> --doc`)");
    result
}

/// The arguments of `cargo xtask test` as cargo takes them: `-p` (or
/// `--package`) and `--test`, each with a name, at least one crate.
fn test_selection(args: &[String]) -> Result<Vec<String>, String> {
    let mut selection = Vec::new();
    let mut crates = 0usize;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let flag = match arg.as_str() {
            "-p" | "--package" => {
                crates += 1;
                "-p"
            }
            "--test" => "--test",
            other => return Err(format!("test: unknown argument `{other}`; {TEST_USAGE}")),
        };
        let name = args
            .next()
            .filter(|name| !name.is_empty() && !name.starts_with('-'))
            .ok_or_else(|| format!("test: `{arg}` needs a name; {TEST_USAGE}"))?;
        selection.push(flag.to_string());
        selection.push(name.clone());
    }
    if crates == 0 {
        return Err(format!(
            "test: name the crates with -p (the whole workspace is `cargo xtask ci`); {TEST_USAGE}"
        ));
    }
    Ok(selection)
}

fn field<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |v, key| v.get(key))
}

/// Builds the tests `selection` names (`--workspace`, or `-p` and
/// `--test` arguments) and lists the binaries. Compiler diagnostics go to
/// the terminal as usual; the artifact messages on stdout say where each
/// binary is.
fn build_tests(selection: &[&str]) -> Result<Vec<TestBinary>, String> {
    let shown = format!("cargo test {} --no-run", selection.join(" "));
    eprintln!("$ {shown}");
    let output = Command::new(super::cargo_path())
        .arg("test")
        .args(selection)
        .args(["--no-run", "--message-format=json-render-diagnostics"])
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !output.status.success() {
        return Err(format!("`{shown}` failed ({})", output.status));
    }
    let root = std::env::current_dir().unwrap_or_default();
    let mut binaries = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let is_artifact =
            field(&message, &["reason"]).and_then(Value::as_str) == Some("compiler-artifact");
        let is_test = field(&message, &["profile", "test"]).and_then(Value::as_bool) == Some(true);
        if !is_artifact || !is_test {
            continue;
        }
        let Some(executable) = field(&message, &["executable"]).and_then(Value::as_str) else {
            continue;
        };
        let Some(manifest_dir) = field(&message, &["manifest_path"])
            .and_then(Value::as_str)
            .and_then(|p| Path::new(p).parent())
        else {
            continue;
        };
        let name = field(&message, &["target", "name"])
            .and_then(Value::as_str)
            .unwrap_or("?");
        let kind = field(&message, &["target", "kind", "0"])
            .or_else(|| field(&message, &["target", "kind"]).and_then(|k| k.get(0)))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let package = manifest_dir.strip_prefix(&root).unwrap_or(manifest_dir);
        binaries.push(TestBinary {
            label: format!("{}: {kind} {name}", package.display()),
            executable: PathBuf::from(executable),
            manifest_dir: manifest_dir.to_path_buf(),
        });
    }
    if binaries.is_empty() {
        return Err("cargo built no test binaries".to_string());
    }
    Ok(binaries)
}

/// The target directory, from where cargo put a test binary
/// (`<target>/debug/deps/<name>-<hash>`).
fn target_dir(binary: &TestBinary) -> Option<PathBuf> {
    Some(
        binary
            .executable
            .parent()?
            .parent()?
            .parent()?
            .to_path_buf(),
    )
}

fn read_durations(path: &Path) -> BTreeMap<String, f64> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let (secs, label) = line.split_once('\t')?;
            Some((label.to_string(), secs.parse().ok()?))
        })
        .collect()
}

/// Writes the durations whole or not at all: to a file of this process
/// beside it, renamed over it, so a run that stops halfway, or another run
/// writing at the same time, never leaves a torn file.
fn write_durations(path: &Path, durations: &BTreeMap<String, f64>) {
    let text: String = durations
        .iter()
        .map(|(label, secs)| format!("{secs:.2}\t{label}\n"))
        .collect();
    let mut part = path.as_os_str().to_owned();
    part.push(format!(".{}", std::process::id()));
    let part = PathBuf::from(part);
    if let Err(e) = std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, path)) {
        let _ = std::fs::remove_file(&part);
        eprintln!("warning: cannot write {}: {e}", path.display());
    }
}

/// How a test binary's run ended.
struct Outcome {
    label: String,
    secs: f64,
    passed: bool,
    /// `exit status: 101`, `signal: 9 (SIGKILL)` (killed: out of memory, a
    /// time limit), or `not started`.
    ended: String,
}

/// The durations known before a run, with those of the binaries it ran
/// put in: an entry of a binary it did not run is kept as it was.
fn merged(mut known: BTreeMap<String, f64>, ran: &[Outcome]) -> BTreeMap<String, f64> {
    for outcome in ran {
        known.insert(outcome.label.clone(), outcome.secs);
    }
    known
}

/// Binaries that must not run at the same time as each other: each GPU
/// test opens its own wgpu device, and on a software adapter several at
/// once run out of memory. The group runs on one worker, in order.
fn serial_group(binary: &TestBinary) -> Option<&'static str> {
    binary.label.starts_with("crates/gpu").then_some("gpu")
}

/// Runs the test binaries on a pool of workers, each in its package
/// directory as cargo would, and prints a line `-> <binary>` when one
/// starts (so a run cut off says which were still running) and its output
/// in one piece when it is done, after a line `== <binary> (<time>)`, or
/// `== <binary> (<time>, <how it ended>): FAILED` (`xtask health` reads
/// those). Every binary runs even after one fails, so one gate run reports
/// everything.
fn run_side_by_side(binaries: Vec<TestBinary>) -> Result<(), String> {
    run_side_by_side_into(binaries, &Mutex::new(std::io::stdout()))
}

/// [`run_side_by_side`], printing each binary's output to `out`.
fn run_side_by_side_into(
    mut binaries: Vec<TestBinary>,
    out: &Mutex<impl Write + Send>,
) -> Result<(), String> {
    let target = binaries
        .first()
        .and_then(target_dir)
        .unwrap_or_else(|| PathBuf::from("target"));
    let tmp = target.join("tmp");
    let _ = std::fs::create_dir_all(&tmp);
    let durations_file = target.join("ci-durations.txt");
    let known = read_durations(&durations_file);
    let weight = |b: &TestBinary| known.get(&b.label).copied().unwrap_or(f64::INFINITY);
    binaries.sort_by(|a, b| weight(b).total_cmp(&weight(a)));
    let count = binaries.len();
    // Work items: one binary each, except the serial groups, which stand
    // where their first member stood.
    let mut groups: Vec<Vec<TestBinary>> = Vec::new();
    let mut group_index: BTreeMap<&'static str, usize> = BTreeMap::new();
    for binary in binaries {
        match serial_group(&binary) {
            Some(name) => match group_index.get(name) {
                Some(&i) => groups.get_mut(i).map_or((), |g| g.push(binary)),
                None => {
                    group_index.insert(name, groups.len());
                    groups.push(vec![binary]);
                }
            },
            None => groups.push(vec![binary]),
        }
    }
    // A long binary uses every core itself; the short ones fill the gaps.
    // `CRAFT3D_CI_WORKERS` overrides the count, for a machine with less
    // memory or to measure.
    let workers = std::env::var("CRAFT3D_CI_WORKERS")
        .ok()
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(2, |n| n.get() / 2))
        .clamp(1, groups.len().max(1));
    eprintln!("running {count} test binaries, {workers} at a time");
    let clock = Instant::now();
    let queue = Mutex::new(VecDeque::from(groups));
    let outcomes: Mutex<Vec<Outcome>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let next = queue
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .pop_front();
                    let Some(group) = next else { break };
                    for binary in group {
                        {
                            let mut out = out.lock().unwrap_or_else(PoisonError::into_inner);
                            let _ = writeln!(out, "-> {}", binary.label);
                            let _ = out.flush();
                        }
                        let started = Instant::now();
                        let result = Command::new(&binary.executable)
                            .current_dir(&binary.manifest_dir)
                            .env("CARGO_MANIFEST_DIR", &binary.manifest_dir)
                            .env("CARGO_TARGET_TMPDIR", &tmp)
                            .output();
                        let secs = started.elapsed().as_secs_f64();
                        let (ok, ended, text) = match result {
                            Ok(output) => (
                                output.status.success(),
                                output.status.to_string(),
                                format!(
                                    "{}{}",
                                    String::from_utf8_lossy(&output.stdout),
                                    String::from_utf8_lossy(&output.stderr)
                                ),
                            ),
                            Err(e) => (
                                false,
                                "not started".to_string(),
                                format!("cannot run {}: {e}\n", binary.executable.display()),
                            ),
                        };
                        {
                            let mut out = out.lock().unwrap_or_else(PoisonError::into_inner);
                            let _ = if ok {
                                writeln!(out, "== {} ({secs:.1} s)", binary.label)
                            } else {
                                writeln!(out, "== {} ({secs:.1} s, {ended}): FAILED", binary.label)
                            };
                            let _ = out.write_all(text.as_bytes());
                            let _ = out.flush();
                        }
                        outcomes
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(Outcome {
                                label: binary.label.clone(),
                                secs,
                                passed: ok,
                                ended,
                            });
                    }
                }
            });
        }
    });
    let wall = clock.elapsed().as_secs_f64();
    let outcomes = outcomes
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    write_durations(&durations_file, &merged(known, &outcomes));
    let busy: f64 = outcomes.iter().map(|o| o.secs).sum();
    let failed: Vec<String> = outcomes
        .iter()
        .filter(|o| !o.passed)
        .map(|o| format!("{} ({})", o.label, o.ended))
        .collect();
    eprintln!(
        "tests: {} binaries, {} failed, {wall:.0} s wall, {busy:.0} s of binary time",
        outcomes.len(),
        failed.len()
    );
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} test binaries failed: {}",
            failed.len(),
            failed.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Outcome, TestBinary, merged, read_durations, run_side_by_side_into, test_selection,
    };
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn the_crates_and_test_files_go_to_cargo_as_named() {
        assert_eq!(
            test_selection(&args(&[
                "-p",
                "craft3d-engine",
                "--package",
                "craft3d-kernel",
                "--test",
                "guide_examples",
                "--test",
                "panic_hunt"
            ]))
            .unwrap(),
            args(&[
                "-p",
                "craft3d-engine",
                "-p",
                "craft3d-kernel",
                "--test",
                "guide_examples",
                "--test",
                "panic_hunt"
            ])
        );
        assert_eq!(
            test_selection(&args(&["-p", "xtask"])).unwrap(),
            args(&["-p", "xtask"])
        );
    }

    #[test]
    fn a_test_without_a_crate_or_a_name_or_with_another_argument_is_refused() {
        for (words, said) in [
            (&[][..], "name the crates with -p"),
            (&["--test", "guide_examples"][..], "name the crates with -p"),
            (&["-p"][..], "`-p` needs a name"),
            (&["-p", ""][..], "`-p` needs a name"),
            (&["-p", "--test", "x"][..], "`-p` needs a name"),
            (
                &["-p", "craft3d-engine", "--test"][..],
                "`--test` needs a name",
            ),
            (&["--workspace"][..], "unknown argument `--workspace`"),
            (
                &["-p", "craft3d-engine", "guide_examples"][..],
                "unknown argument `guide_examples`",
            ),
            (
                &["-p", "craft3d-engine", "--", "--nocapture"][..],
                "unknown argument `--`",
            ),
        ] {
            let refused = test_selection(&args(words)).unwrap_err();
            assert!(refused.contains(said), "{words:?}: {refused}");
            assert!(refused.contains("usage: cargo xtask test -p"), "{refused}");
        }
    }

    #[test]
    fn a_run_adds_its_durations_and_keeps_the_others() {
        let known = BTreeMap::from([
            ("crates/a: test long".to_string(), 600.0),
            ("crates/b: test short".to_string(), 1.0),
        ]);
        let outcome = |label: &str, secs: f64, passed: bool| Outcome {
            label: label.to_string(),
            secs,
            passed,
            ended: String::new(),
        };
        let ran = [
            outcome("crates/b: test short", 2.5, true),
            outcome("crates/c: test new", 7.0, false),
        ];
        assert_eq!(
            merged(known, &ran),
            BTreeMap::from([
                ("crates/a: test long".to_string(), 600.0),
                ("crates/b: test short".to_string(), 2.5),
                ("crates/c: test new".to_string(), 7.0),
            ])
        );
    }

    /// A binary that fails, and one that cannot be started, fail the run
    /// with their names, after every binary ran, and are printed as failed;
    /// the durations file then holds the binaries that ran beside the entry
    /// of one that did not.
    #[test]
    fn a_failing_binary_fails_the_run_and_the_durations_keep_every_entry() {
        let root = std::env::temp_dir().join(format!("xtask-side-by-side-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let deps = root.join("target/debug/deps");
        std::fs::create_dir_all(&deps).unwrap();
        let binary = |name: &str| TestBinary {
            label: format!("crates/x: test {name}"),
            executable: deps.join(name),
            manifest_dir: root.clone(),
        };
        let mut binaries = vec![binary("missing")];
        // The system's `true` and `false` as test binaries that pass and
        // fail, linked, not written: an executable written by a test can be
        // "busy" while another thread starts a process.
        #[cfg(unix)]
        {
            let system = |name: &str| {
                ["/usr/bin", "/bin"]
                    .iter()
                    .map(|dir| Path::new(dir).join(name))
                    .find(|p| p.is_file())
            };
            if let (Some(yes), Some(no)) = (system("true"), system("false")) {
                std::os::unix::fs::symlink(yes, deps.join("passes")).unwrap();
                std::os::unix::fs::symlink(no, deps.join("fails")).unwrap();
                binaries.push(binary("passes"));
                binaries.push(binary("fails"));
            }
        }
        let file: PathBuf = root.join("target/ci-durations.txt");
        std::fs::write(
            &file,
            "300.00\tcrates/y: test not run\n99.00\tcrates/x: test passes\n",
        )
        .unwrap();
        let ran = binaries.len();
        let printed = Mutex::new(Vec::new());
        let failed = run_side_by_side_into(binaries, &printed).unwrap_err();
        let printed = String::from_utf8(printed.into_inner().unwrap()).unwrap();
        assert!(
            failed.contains("crates/x: test missing (not started)"),
            "{failed}"
        );
        assert!(printed.contains("-> crates/x: test missing\n"), "{printed}");
        assert!(
            printed.contains(", not started): FAILED\ncannot run "),
            "{printed}"
        );
        let durations = read_durations(&file);
        assert_eq!(durations.get("crates/y: test not run"), Some(&300.0));
        let mut expected = vec![
            "crates/x: test missing",
            "crates/x: test passes",
            "crates/y: test not run",
        ];
        if ran == 3 {
            assert!(failed.starts_with("2 test binaries failed"), "{failed}");
            assert!(
                failed.contains("crates/x: test fails (exit status: 1)"),
                "{failed}"
            );
            assert!(!failed.contains("test passes"), "{failed}");
            assert!(printed.contains("-> crates/x: test fails\n"), "{printed}");
            assert!(printed.contains(", exit status: 1): FAILED\n"), "{printed}");
            let passes = printed
                .lines()
                .find(|l| l.starts_with("== crates/x: test passes ("))
                .unwrap();
            assert!(!passes.ends_with("FAILED"), "{printed}");
            assert!(durations["crates/x: test passes"] < 99.0, "{durations:?}");
            expected.push("crates/x: test fails");
        } else {
            // Not run here (no `true` and `false`): its entry is kept.
            assert!(failed.starts_with("1 test binaries failed"), "{failed}");
            assert_eq!(durations.get("crates/x: test passes"), Some(&99.0));
        }
        expected.sort_unstable();
        let keys: Vec<&str> = durations.keys().map(String::as_str).collect();
        assert_eq!(keys, expected);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A line of a workflow that asks for the workshop's own runners: it
    /// names a self-hosted runner or the Strix outside a comment (YAML's
    /// and the scripts' `#`; a comment may name the Strix).
    fn asks_for_the_workshops_runners(line: &str) -> bool {
        let code = line.split('#').next().unwrap_or_default().to_lowercase();
        code.contains("self-hosted") || code.contains("strix")
    }

    /// The workflows of the code tree run on GitHub's hosted runners only.
    /// The workshop's own runners (the Strix) take their workflows and
    /// scripts from a branch of the workshop that is not in this repository
    /// (docs/decisions.md, 2026-11-10): this tree is also the public
    /// repository's, where a job for a self-hosted runner is one a pull
    /// request from a fork could reach. Branches made before that day still
    /// carry the Strix's files; this catches one coming back with a merge.
    #[test]
    fn no_workflow_in_the_code_tree_asks_for_the_workshops_runners() {
        let root = crate::package::repo_root();
        let dir = root.join(".github/workflows");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        assert!(!files.is_empty(), "no workflow in {}", dir.display());
        let mut found = Vec::new();
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap();
            for (number, line) in text.lines().enumerate() {
                if asks_for_the_workshops_runners(line) {
                    found.push(format!(
                        "{}:{}: {}",
                        path.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
        assert!(
            found.is_empty(),
            "the Strix's workflows live on the workshop's own branch, not in the code tree:\n{}",
            found.join("\n")
        );
        for script in ["target-cap.sh", "sccache-env.sh"] {
            let path = root.join(".github/scripts").join(script);
            assert!(
                !path.exists(),
                "{} is the Strix's: it lives on the workshop's own branch",
                path.display()
            );
        }
    }

    /// The check above finds a job for the Strix's runners, written as the
    /// workshop's workflows write it, and lets the comments that name it be.
    #[test]
    fn a_job_for_the_strix_is_found_and_a_comment_is_not() {
        assert!(asks_for_the_workshops_runners(
            "    runs-on: [self-hosted, strix, 3dcraft]"
        ));
        assert!(asks_for_the_workshops_runners("    runs-on: Self-Hosted"));
        assert!(asks_for_the_workshops_runners(
            "      os: [strix] # the gate"
        ));
        assert!(!asks_for_the_workshops_runners(
            "    # Only in the workshop, where the Strix runners are attached."
        ));
        assert!(!asks_for_the_workshops_runners(
            "          # engine test files ran 1.8 times faster so on the Strix"
        ));
        assert!(!asks_for_the_workshops_runners(
            "    runs-on: ubuntu-latest"
        ));
    }
}
