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
    let binaries = build_tests()?;
    run_side_by_side(binaries)?;
    super::cargo(&["test", "--workspace", "--doc"])?;
    super::layers::check()?;
    super::wasm()
}

fn field<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |v, key| v.get(key))
}

/// Builds every test of the workspace and lists the binaries. Compiler
/// diagnostics go to the terminal as usual; the artifact messages on
/// stdout say where each binary is.
fn build_tests() -> Result<Vec<TestBinary>, String> {
    let args = [
        "test",
        "--workspace",
        "--no-run",
        "--message-format=json-render-diagnostics",
    ];
    eprintln!("$ cargo test --workspace --no-run");
    let output = Command::new(super::cargo_path())
        .args(args)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo test --workspace --no-run` failed ({})",
            output.status
        ));
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

fn write_durations(path: &Path, durations: &BTreeMap<String, f64>) {
    let text: String = durations
        .iter()
        .map(|(label, secs)| format!("{secs:.2}\t{label}\n"))
        .collect();
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("warning: cannot write {}: {e}", path.display());
    }
}

/// Binaries that must not run at the same time as each other: each GPU
/// test opens its own wgpu device, and on a software adapter several at
/// once run out of memory. The group runs on one worker, in order.
fn serial_group(binary: &TestBinary) -> Option<&'static str> {
    binary.label.starts_with("crates/gpu").then_some("gpu")
}

/// Runs the test binaries on a pool of workers, each in its package
/// directory as cargo would, and prints each one's output in one piece
/// when it is done. Every binary runs even after one fails, so one gate
/// run reports everything.
fn run_side_by_side(mut binaries: Vec<TestBinary>) -> Result<(), String> {
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
    let outcomes: Mutex<Vec<(String, f64, bool)>> = Mutex::new(Vec::new());
    let stdout = std::io::stdout();
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
                        let started = Instant::now();
                        let result = Command::new(&binary.executable)
                            .current_dir(&binary.manifest_dir)
                            .env("CARGO_MANIFEST_DIR", &binary.manifest_dir)
                            .env("CARGO_TARGET_TMPDIR", &tmp)
                            .output();
                        let secs = started.elapsed().as_secs_f64();
                        let (ok, text) = match result {
                            Ok(output) => (
                                output.status.success(),
                                format!(
                                    "{}{}",
                                    String::from_utf8_lossy(&output.stdout),
                                    String::from_utf8_lossy(&output.stderr)
                                ),
                            ),
                            Err(e) => (
                                false,
                                format!("cannot run {}: {e}\n", binary.executable.display()),
                            ),
                        };
                        {
                            let mut out = stdout.lock();
                            let _ = writeln!(
                                out,
                                "== {} ({secs:.1} s){}",
                                binary.label,
                                if ok { "" } else { ": FAILED" }
                            );
                            let _ = out.write_all(text.as_bytes());
                            let _ = out.flush();
                        }
                        outcomes
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push((binary.label.clone(), secs, ok));
                    }
                }
            });
        }
    });
    let wall = clock.elapsed().as_secs_f64();
    let outcomes = outcomes
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let mut durations = known;
    for (label, secs, _) in &outcomes {
        durations.insert(label.clone(), *secs);
    }
    write_durations(&durations_file, &durations);
    let busy: f64 = outcomes.iter().map(|o| o.1).sum();
    let failed: Vec<&str> = outcomes
        .iter()
        .filter(|o| !o.2)
        .map(|o| o.0.as_str())
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
