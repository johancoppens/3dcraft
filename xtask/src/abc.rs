//! `cargo xtask abc`: 3DCraft against real-world CAD, the STEP files of
//! the ABC dataset (A Big CAD Model Dataset, NYU 2019: about a million
//! Onshape models in chunks of 10 000; docs/testing.md › Real-world
//! models). Downloads one chunk (cached outside the repository, reused
//! across runs), then runs `craft3d-fuzz step` once per model, each in its
//! own process with a time limit and a memory cap (the launcher of
//! `cargo xtask fuzz`):
//!
//! 1. **import**: `file.import`, `document.check`, a mesh, a measure:
//!    ok, refused (with the reason), invalid (accepted, but the check finds
//!    fault), crash (a panic or a signal), timeout, oom;
//! 2. **booleans** (with `--booleans`): on the models stage 1 read as valid
//!    solids, three tools sized to the part (a drill, a slab, a corner
//!    block), each united, subtracted and intersected, held to
//!    `V(A ∪ B) + V(A ∩ B) = V(A) + V(B)`, `V(A − B) + V(A ∩ B) = V(A)` and
//!    `document.check`.
//!
//! With `--freecad N` (and `CRAFT3D_FREECAD`), FreeCAD reads N of the
//! models stage 1 accepted and the volumes are compared.
//!
//! **Licence.** The models' copyright is their creators' (Onshape's terms
//! of use): no model file, or part of one, is ever written into the
//! repository. What this task leaves under `--out` (default `target/abc`)
//! holds model ids, file hashes and results only: `import.jsonl`,
//! `booleans.jsonl`, `freecad.jsonl`, `summary.json` and `scorecard.md`.

use crate::fuzz::{Cap, Launcher, cap_for, commit, now};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: cargo xtask abc [--chunk N] [--cache DIR] [--files DIR] [--limit N] \
[--jobs J] [--timeout SECONDS] [--memory SIZE] [--minutes M] [--booleans] [--boolean-limit N] \
[--boolean-minutes M] [--boolean-timeout SECONDS] [--freecad N] [--out DIR] [--bin PATH]";

/// The list of the STEP chunks: one `URL file-name` per line.
const LIST_URL: &str = "https://deep-geometry.github.io/abc-dataset/data/step_v00.txt";
/// Their MD5 sums: `file-name: sum` per line.
const MD5_URL: &str = "https://deep-geometry.github.io/abc-dataset/data/md5.yml";

/// Most processes at once (the Strix is shared).
const MAX_JOBS: usize = 8;

/// Refusal reasons listed in the scorecard.
const TOP: usize = 20;

struct Config {
    chunk: u32,
    cache: PathBuf,
    files: Option<PathBuf>,
    limit: Option<usize>,
    jobs: usize,
    timeout: u64,
    memory: String,
    minutes: Option<u64>,
    booleans: bool,
    boolean_limit: Option<usize>,
    boolean_minutes: Option<u64>,
    boolean_timeout: u64,
    freecad: usize,
    out: PathBuf,
    bin: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Config, String> {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    let mut c = Config {
        chunk: 0,
        cache: home.join("3dcraft-abc"),
        files: None,
        limit: None,
        jobs: 4,
        timeout: 60,
        memory: "3G".to_string(),
        minutes: None,
        booleans: false,
        boolean_limit: None,
        boolean_minutes: None,
        boolean_timeout: 180,
        freecad: 0,
        out: crate::fuzz::workspace_root().join("target/abc"),
        bin: None,
    };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        if flag == "--booleans" {
            c.booleans = true;
            continue;
        }
        let value = it
            .next()
            .ok_or_else(|| format!("{flag} needs a value; {USAGE}"))?;
        let number = || -> Result<u64, String> {
            value
                .parse()
                .map_err(|_| format!("{flag}: `{value}` is not a whole number"))
        };
        let count =
            || -> Result<usize, String> { Ok(usize::try_from(number()?).unwrap_or(usize::MAX)) };
        match flag.as_str() {
            "--chunk" => {
                c.chunk = u32::try_from(number()?)
                    .ok()
                    .filter(|n| *n < 10_000)
                    .ok_or_else(|| format!("--chunk: `{value}` is not a chunk number"))?;
            }
            "--cache" => c.cache = PathBuf::from(value),
            "--files" => c.files = Some(PathBuf::from(value)),
            "--limit" => c.limit = Some(count()?),
            "--jobs" => c.jobs = count()?.clamp(1, MAX_JOBS),
            "--timeout" => c.timeout = number()?.max(1),
            "--memory" => {
                if crate::fuzz::bytes(value).is_none() {
                    return Err(format!("--memory: `{value}` is not a size (such as 3G)"));
                }
                c.memory = value.clone();
            }
            "--minutes" => c.minutes = Some(number()?),
            "--boolean-limit" => c.boolean_limit = Some(count()?),
            "--boolean-minutes" => c.boolean_minutes = Some(number()?),
            "--boolean-timeout" => c.boolean_timeout = number()?.max(1),
            "--freecad" => c.freecad = count()?,
            "--out" => c.out = PathBuf::from(value),
            "--bin" => c.bin = Some(PathBuf::from(value)),
            other => return Err(format!("unknown option {other}; {USAGE}")),
        }
    }
    Ok(c)
}

/// Runs a program, its stdout when it succeeds.
fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "`{program} {}` failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn fetch_text(url: &str) -> Result<String, String> {
    output("curl", &["-sSfL", "--retry", "3", "--max-time", "120", url])
}

/// The chunk's archive in the cache, downloaded when it is not there yet
/// (and checked against its MD5 sum).
fn download(c: &Config) -> Result<PathBuf, String> {
    let name = format!("abc_{:04}_step_v00.7z", c.chunk);
    std::fs::create_dir_all(&c.cache)
        .map_err(|e| format!("cannot make {}: {e}", c.cache.display()))?;
    let archive = c.cache.join(&name);
    let verified = c.cache.join(format!("{name}.ok"));
    if archive.is_file() && verified.is_file() {
        return Ok(archive);
    }
    let list = fetch_text(LIST_URL)?;
    let url = list
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let url = parts.next()?;
            (parts.next()? == name).then(|| url.to_string())
        })
        .ok_or_else(|| format!("{LIST_URL} does not list {name}"))?;
    let md5 = fetch_text(MD5_URL).ok().and_then(|text| {
        text.lines().find_map(|line| {
            let (file, sum) = line.split_once(':')?;
            (file.trim() == name).then(|| sum.trim().to_string())
        })
    });
    let partial = c.cache.join(format!("{name}.partial"));
    let path = partial.to_string_lossy().to_string();
    eprintln!("abc: downloading {name} from {url}");
    let started = Instant::now();
    // NYU's archive has been served with a certificate curl does not
    // accept (the dataset's own instructions say --insecure); the MD5 sum
    // checks what came.
    let fetched = output(
        "curl",
        &["-sSfL", "--retry", "3", "-C", "-", "-o", &path, &url],
    )
    .or_else(|e| {
        eprintln!("abc: {e}; again without checking the certificate");
        output(
            "curl",
            &[
                "-sSfL",
                "--insecure",
                "--retry",
                "3",
                "-C",
                "-",
                "-o",
                &path,
                &url,
            ],
        )
    });
    fetched?;
    eprintln!(
        "abc: downloaded in {:.0} s",
        started.elapsed().as_secs_f64()
    );
    match md5 {
        Some(want) => {
            let got = output("md5sum", &[&path])?;
            let got = got.split_whitespace().next().unwrap_or("");
            if got != want {
                let _ = std::fs::remove_file(&partial);
                return Err(format!("{name}: MD5 {got}, the dataset says {want}"));
            }
        }
        None => eprintln!("abc: no MD5 sum for {name}; not checked"),
    }
    std::fs::rename(&partial, &archive).map_err(|e| format!("cannot keep {name}: {e}"))?;
    let _ = std::fs::write(&verified, "");
    Ok(archive)
}

/// The archive unpacked beside it, once.
fn extract(c: &Config, archive: &Path) -> Result<PathBuf, String> {
    let dir = c.cache.join(format!("abc_{:04}_step", c.chunk));
    let done = dir.join(".extracted");
    if done.is_file() {
        return Ok(dir);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let a = archive.to_string_lossy().to_string();
    let d = dir.to_string_lossy().to_string();
    let to = format!("-o{d}");
    let script = "import sys, py7zr; py7zr.SevenZipFile(sys.argv[1]).extractall(sys.argv[2])";
    let tries: [(&str, Vec<&str>); 5] = [
        ("7z", vec!["x", "-y", &to, &a]),
        ("7za", vec!["x", "-y", &to, &a]),
        ("7zr", vec!["x", "-y", &to, &a]),
        ("bsdtar", vec!["-xf", &a, "-C", &d]),
        ("python3", vec!["-I", "-c", script, &a, &d]),
    ];
    let mut failures = Vec::new();
    for (program, args) in &tries {
        eprintln!("abc: unpacking with {program}");
        match output(program, args) {
            Ok(_) => {
                let _ = std::fs::write(&done, "");
                return Ok(dir);
            }
            Err(e) => failures.push(e),
        }
    }
    Err(format!(
        "cannot unpack {} (install 7-Zip, bsdtar or the Python module py7zr): {}",
        archive.display(),
        failures.join("; ")
    ))
}

/// Every `.step`/`.stp` file under `dir`, sorted, at most `depth` deep.
fn step_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            step_files(&path, depth + 1, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("step") || e.eq_ignore_ascii_case("stp"))
        {
            out.push(path);
        }
    }
}

/// A model's id: the ABC file `00000050_80d90bfdd2e74e709956122a_step_000.step`
/// is `00000050` (with `/1`, `/2` for a model's second and third file).
fn model_id(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut parts = stem.split('_');
    let first = parts.next().unwrap_or("").to_string();
    let last = stem.rsplit('_').next().unwrap_or("");
    match last.parse::<u32>() {
        Ok(n) if n > 0 && stem.contains("_step_") => format!("{first}/{n}"),
        _ => first,
    }
}

/// A reason with what differs per model taken out, so refusals of one kind
/// count together: the import's `` `file`, solid N (#id): `` prefix,
/// numbers, quoted and parenthesised details; the first words.
fn reason_key(reason: &str) -> String {
    let mut text = reason;
    if let Some(rest) = text.strip_prefix("file.import: ") {
        // `name.step`: ... or `name.step`, solid 0 (#123 at a/b): ...
        text = rest;
        if let Some(after) = rest.strip_prefix('`')
            && let Some(close) = after.find('`')
        {
            let tail = after.get(close + 1..).unwrap_or("");
            text = match tail.strip_prefix(", solid ") {
                Some(solid) => solid
                    .find("): ")
                    .map_or(solid, |i| solid.get(i + 3..).unwrap_or("")),
                None => tail.trim_start_matches(':').trim_start(),
            };
        }
    }
    let mut out = String::new();
    let mut depth = 0_u32;
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '`' => quoted = !quoted,
            _ if depth > 0 || quoted => {}
            c if c.is_ascii_digit() => {
                if !out.ends_with('#') {
                    out.push('#');
                }
            }
            c => out.push(c),
        }
    }
    let words: Vec<&str> = out.split_whitespace().take(12).collect();
    words.join(" ").chars().take(110).collect()
}

/// One model's line in a stage's `.jsonl`: its report from the process,
/// or how the process died, with the id and the file's name in place of
/// its path.
fn classify(launcher: &Launcher, args: &[String], file: &Path, stage: &str) -> Value {
    let ran = launcher.run(args);
    let mut line = Map::new();
    line.insert("id".into(), model_id(file).into());
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    line.insert("name".into(), name.into());
    match (ran.report, ran.died) {
        (Some(Value::Object(report)), _) => {
            for (k, v) in report {
                if k != "file" {
                    line.insert(k, v);
                }
            }
            if stage == "booleans" {
                // The stage's verdict is the booleans'; a model whose
                // import now fails says so.
                let verdict = line
                    .get("booleans")
                    .and_then(|b| b.get("verdict"))
                    .cloned()
                    .unwrap_or_else(|| json!("import-changed"));
                line.insert(
                    "importVerdict".into(),
                    line.get("verdict").cloned().unwrap_or(Value::Null),
                );
                line.insert("verdict".into(), verdict);
            }
        }
        (_, Some((kind, detail))) => {
            line.insert("verdict".into(), kind.into());
            line.insert(
                "reason".into(),
                detail.chars().take(600).collect::<String>().into(),
            );
        }
        _ => {
            line.insert("verdict".into(), "crash".into());
            line.insert("reason".into(), "the process printed no report".into());
        }
    }
    Value::Object(line)
}

/// Runs `files` through `args_of`, `jobs` at a time, until `deadline`;
/// the lines, in the order of `files` (those not reached left out).
fn run_stage(
    launcher: &Launcher,
    files: &[PathBuf],
    jobs: usize,
    deadline: Option<Instant>,
    stage: &str,
    args_of: &(dyn Fn(&Path) -> Vec<String> + Sync),
) -> Vec<Value> {
    let next = AtomicUsize::new(0);
    let lines: Mutex<Vec<(usize, Value)>> = Mutex::new(Vec::new());
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    if deadline.is_some_and(|d| Instant::now() >= d) {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(i) else {
                        return;
                    };
                    let line = classify(launcher, &args_of(file), file, stage);
                    let mut all = lines
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    all.push((i, line));
                    let n = all.len();
                    if n.is_multiple_of(250) {
                        eprintln!(
                            "abc: {stage}: {n} of {} in {:.0} s",
                            files.len(),
                            started.elapsed().as_secs_f64()
                        );
                    }
                }
            });
        }
    });
    let mut lines = lines
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    lines.sort_by_key(|(i, _)| *i);
    lines.into_iter().map(|(_, v)| v).collect()
}

fn verdict(line: &Value) -> &str {
    line.get("verdict").and_then(Value::as_str).unwrap_or("?")
}

fn id(line: &Value) -> &str {
    line.get("id").and_then(Value::as_str).unwrap_or("?")
}

fn reason(line: &Value) -> String {
    line.get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// For the booleans, the reason of the first tool that was not ok, with
/// its tool and operation.
fn boolean_reason(line: &Value) -> String {
    let tools = line
        .get("booleans")
        .and_then(|b| b.get("tools"))
        .and_then(Value::as_array);
    for tool in tools.into_iter().flatten() {
        if tool.get("verdict").and_then(Value::as_str) != Some("ok") {
            let field = |k: &str| tool.get(k).and_then(Value::as_str).unwrap_or("");
            return format!("{} {}: {}", field("tool"), field("at"), field("reason"));
        }
    }
    reason(line)
}

/// Counts per verdict, in a fixed order.
fn counts(lines: &[Value]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for line in lines {
        *out.entry(verdict(line).to_string()).or_insert(0) += 1;
    }
    out
}

fn percent(n: usize, of: usize) -> String {
    if of == 0 {
        "0.0".to_string()
    } else {
        format!("{:.1}", 100.0 * n as f64 / of as f64)
    }
}

/// The top reasons among the lines of `wanted` verdicts, with up to three
/// example ids each.
fn top_reasons(
    lines: &[Value],
    wanted: &[&str],
    why: &dyn Fn(&Value) -> String,
) -> Vec<(String, usize, Vec<String>)> {
    let mut by: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    for line in lines.iter().filter(|l| wanted.contains(&verdict(l))) {
        let key = reason_key(&why(line));
        let entry = by.entry(key).or_insert((0, Vec::new()));
        entry.0 += 1;
        if entry.1.len() < 3 {
            entry.1.push(id(line).to_string());
        }
    }
    let mut top: Vec<(String, usize, Vec<String>)> =
        by.into_iter().map(|(k, (n, ids))| (k, n, ids)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(TOP);
    top
}

/// FreeCAD reads up to `n` models stage 1 accepted (spread over them)
/// and gives their volumes; the lines of `freecad.jsonl`.
fn freecad(c: &Config, files: &[PathBuf], import: &[Value]) -> Result<Vec<Value>, String> {
    let program = std::env::var("CRAFT3D_FREECAD")
        .map_err(|_| "--freecad needs CRAFT3D_FREECAD".to_string())?;
    let accepted: Vec<(&PathBuf, &Value)> = files
        .iter()
        .zip(import)
        .filter(|(_, l)| verdict(l) == "ok")
        .collect();
    if accepted.is_empty() || c.freecad == 0 {
        return Ok(Vec::new());
    }
    let step = accepted.len().div_ceil(c.freecad).max(1);
    let sample: Vec<&(&PathBuf, &Value)> = accepted.iter().step_by(step).take(c.freecad).collect();
    let work = c.out.join("freecad-work");
    std::fs::create_dir_all(&work).map_err(|e| format!("cannot make {}: {e}", work.display()))?;
    let script = work.join("volumes.py");
    std::fs::write(&script, FREECAD_SCRIPT).map_err(|e| format!("cannot write the script: {e}"))?;
    let mut theirs: BTreeMap<String, Value> = BTreeMap::new();
    // In batches, so a FreeCAD that dies costs one batch.
    for (b, batch) in sample.chunks(20).enumerate() {
        let listing = work.join(format!("list-{b}.txt"));
        let report = work.join(format!("report-{b}.jsonl"));
        let text: String = batch
            .iter()
            .map(|(f, _)| format!("{}\n", f.display()))
            .collect();
        std::fs::write(&listing, text).map_err(|e| format!("cannot write the list: {e}"))?;
        let mut command = Command::new("timeout");
        command.args([
            "-k",
            "10",
            "900",
            "sh",
            "-c",
            "ulimit -v 4194304 && exec \"$@\"",
            "sh",
        ]);
        command.arg(&program);
        if program.ends_with(".AppImage") {
            command.arg("freecadcmd");
        }
        let _ = command
            .arg(&script)
            .env("CRAFT3D_FC_IN", &listing)
            .env("CRAFT3D_FC_OUT", &report)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let text = std::fs::read_to_string(&report).unwrap_or_default();
        for line in text.lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line)
                && let Some(file) = v.get("file").and_then(Value::as_str)
            {
                theirs.insert(file.to_string(), v);
            }
        }
    }
    let mut out = Vec::new();
    for (file, ours) in sample {
        let key = file.to_string_lossy().to_string();
        let mut line = Map::new();
        line.insert("id".into(), model_id(file).into());
        let our_volume = ours.get("volume").and_then(Value::as_f64);
        line.insert("ours".into(), our_volume.map_or(Value::Null, Value::from));
        match theirs.get(&key) {
            None => {
                line.insert("verdict".into(), "freecad-died".into());
            }
            Some(t) => {
                let their_volume = t.get("volume").and_then(Value::as_f64);
                line.insert(
                    "freecad".into(),
                    their_volume.map_or(Value::Null, Value::from),
                );
                for k in ["solids", "valid", "error"] {
                    if let Some(v) = t.get(k) {
                        line.insert(k.into(), v.clone());
                    }
                }
                let v = match (our_volume, their_volume) {
                    (Some(a), Some(b)) if b.abs() > 0.0 => {
                        let off = (a - b).abs() / b.abs();
                        line.insert("relative".into(), off.into());
                        if off <= 1e-3 { "agree" } else { "disagree" }
                    }
                    (Some(_), Some(_)) => "freecad-no-volume",
                    _ => "freecad-refused",
                };
                line.insert("verdict".into(), v.into());
            }
        }
        out.push(Value::Object(line));
    }
    let _ = std::fs::remove_dir_all(&work);
    Ok(out)
}

/// FreeCAD's side: the summed volume of the solids of each file.
const FREECAD_SCRIPT: &str = r#"import json, os
import Part
out = open(os.environ["CRAFT3D_FC_OUT"], "w")
for line in open(os.environ["CRAFT3D_FC_IN"]):
    path = line.strip()
    if not path:
        continue
    rec = {"file": path}
    try:
        shape = Part.Shape()
        shape.read(path)
        rec["solids"] = len(shape.Solids)
        rec["volume"] = sum(s.Volume for s in shape.Solids)
        rec["valid"] = shape.isValid()
    except Exception as e:
        rec["error"] = str(e)[:200]
    out.write(json.dumps(rec) + "\n")
    out.flush()
out.close()
"#;

fn write_lines(path: &Path, lines: &[Value]) -> Result<(), String> {
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The scorecard, in Markdown: numbers and ids only.
fn scorecard(
    head: &Map<String, Value>,
    total: usize,
    import: &[Value],
    booleans: &[Value],
    fc: &[Value],
) -> String {
    let mut s = String::new();
    let field = |k: &str| {
        head.get(k)
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    let _ = writeln!(s, "# ABC scorecard: chunk {}\n", field("chunk"));
    let _ = writeln!(
        s,
        "Commit {}, {}; {} of the chunk's {total} STEP files run, {} at once, {} s and {} per model (docs/testing.md › Real-world models).\n",
        field("commit"),
        field("date"),
        import.len(),
        field("jobs"),
        field("timeout"),
        field("memory"),
    );
    let _ = writeln!(s, "## Stage 1: import, check, mesh, measure\n");
    let _ = writeln!(s, "| Verdict | Models | % |\n|---|---:|---:|");
    let c = counts(import);
    for (v, n) in &c {
        let _ = writeln!(s, "| {v} | {n} | {} |", percent(*n, import.len()));
    }
    let ready = import
        .iter()
        .filter(|l| l.get("booleanReady").and_then(Value::as_bool) == Some(true))
        .count();
    let _ = writeln!(s, "\nRead as valid solids (ready for stage 2): {ready}.\n");
    let mut at: BTreeMap<String, usize> = BTreeMap::new();
    for l in import
        .iter()
        .filter(|l| matches!(verdict(l), "refused" | "invalid"))
    {
        let step = l.get("at").and_then(Value::as_str).unwrap_or("?");
        *at.entry(format!("{} at {step}", verdict(l))).or_insert(0) += 1;
    }
    if !at.is_empty() {
        let _ = writeln!(s, "| Where | Models |\n|---|---:|");
        for (k, n) in &at {
            let _ = writeln!(s, "| {k} | {n} |");
        }
        s.push('\n');
    }
    let table = |s: &mut String, title: &str, rows: Vec<(String, usize, Vec<String>)>| {
        if rows.is_empty() {
            return;
        }
        let _ = writeln!(
            s,
            "### {title}\n\n| Reason | Models | Examples |\n|---|---:|---|"
        );
        for (k, n, ids) in rows {
            let k = k.replace('|', "\\|");
            let _ = writeln!(s, "| {k} | {n} | {} |", ids.join(", "));
        }
        s.push('\n');
    };
    table(
        &mut s,
        "Top refusal reasons",
        top_reasons(import, &["refused"], &reason),
    );
    table(
        &mut s,
        "Top problems of invalid imports",
        top_reasons(import, &["invalid"], &reason),
    );
    let died: Vec<&Value> = import
        .iter()
        .filter(|l| matches!(verdict(l), "crash" | "timeout" | "oom"))
        .collect();
    if !died.is_empty() {
        let _ = writeln!(
            s,
            "### Crashes, timeouts, out of memory\n\n| Model | Verdict | Detail |\n|---|---|---|"
        );
        for l in died {
            let r: String = reason(l).replace('|', "\\|").chars().take(200).collect();
            let _ = writeln!(s, "| {} | {} | {r} |", id(l), verdict(l));
        }
        s.push('\n');
    }
    if !booleans.is_empty() {
        let _ = writeln!(
            s,
            "## Stage 2: booleans (drill, slab, corner block; union, subtract, intersect)\n"
        );
        let _ = writeln!(s, "| Verdict | Models | % |\n|---|---:|---:|");
        for (v, n) in &counts(booleans) {
            let _ = writeln!(s, "| {v} | {n} | {} |", percent(*n, booleans.len()));
        }
        let mut per_tool: BTreeMap<(String, String), usize> = BTreeMap::new();
        for l in booleans {
            let tools = l
                .get("booleans")
                .and_then(|b| b.get("tools"))
                .and_then(Value::as_array);
            for t in tools.into_iter().flatten() {
                let name = t
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                let v = t
                    .get("verdict")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                *per_tool.entry((name, v)).or_insert(0) += 1;
            }
        }
        let _ = writeln!(s, "\n| Tool | Verdict | Models |\n|---|---|---:|");
        for ((t, v), n) in &per_tool {
            let _ = writeln!(s, "| {t} | {v} | {n} |");
        }
        s.push('\n');
        table(
            &mut s,
            "Top boolean refusals",
            top_reasons(booleans, &["refused"], &boolean_reason),
        );
        table(
            &mut s,
            "Wrong booleans (identity or check)",
            top_reasons(booleans, &["invalid"], &boolean_reason),
        );
        let died: Vec<&Value> = booleans
            .iter()
            .filter(|l| matches!(verdict(l), "crash" | "timeout" | "oom"))
            .collect();
        if !died.is_empty() {
            let _ = writeln!(
                s,
                "### Boolean crashes, timeouts, out of memory\n\n| Model | Verdict | Detail |\n|---|---|---|"
            );
            for l in died {
                let r: String = boolean_reason(l)
                    .replace('|', "\\|")
                    .chars()
                    .take(200)
                    .collect();
                let _ = writeln!(s, "| {} | {} | {r} |", id(l), verdict(l));
            }
            s.push('\n');
        }
    }
    if !fc.is_empty() {
        let _ = writeln!(
            s,
            "## Volumes against FreeCAD (a sample of stage 1's accepted models)\n"
        );
        let _ = writeln!(s, "| Verdict | Models |\n|---|---:|");
        for (v, n) in &counts(fc) {
            let _ = writeln!(s, "| {v} | {n} |");
        }
        let off: Vec<&Value> = fc.iter().filter(|l| verdict(l) == "disagree").collect();
        if !off.is_empty() {
            let _ = writeln!(
                s,
                "\n| Model | Ours | FreeCAD | Relative |\n|---|---:|---:|---:|"
            );
            for l in off {
                let n = |k: &str| {
                    l.get(k)
                        .and_then(Value::as_f64)
                        .map_or("-".to_string(), |v| format!("{v:.6e}"))
                };
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} |",
                    id(l),
                    n("ours"),
                    n("freecad"),
                    n("relative")
                );
            }
        }
        s.push('\n');
    }
    s
}

pub fn run(args: &[String]) -> Result<(), String> {
    let c = parse(args)?;
    let mut files = Vec::new();
    match &c.files {
        Some(dir) => step_files(dir, 0, &mut files),
        None => {
            let archive = download(&c)?;
            let dir = extract(&c, &archive)?;
            step_files(&dir, 0, &mut files);
        }
    }
    let total = files.len();
    if total == 0 {
        return Err("no STEP files found".to_string());
    }
    if let Some(limit) = c.limit {
        files.truncate(limit);
    }
    let bin = match &c.bin {
        Some(b) => b.clone(),
        None => crate::fuzz::build()?,
    };
    let capped = cap_for(&c.memory);
    let launcher = Launcher {
        bin: bin.clone(),
        timeout: c.timeout,
        memory: c.memory.clone(),
        capped,
    };
    std::fs::create_dir_all(&c.out).map_err(|e| format!("cannot make {}: {e}", c.out.display()))?;
    let scratch = c.out.join("scratch");
    let scratch_text = scratch.to_string_lossy().to_string();
    eprintln!(
        "abc: {} of {total} files, {} at once, {} s and {} each, into {}",
        files.len(),
        c.jobs,
        c.timeout,
        if capped == Cap::None {
            "no cap"
        } else {
            c.memory.as_str()
        },
        c.out.display()
    );
    let start = Instant::now();
    let deadline = c
        .minutes
        .map(|m| start + Duration::from_secs(m.saturating_mul(60)));
    let args_of = |f: &Path| {
        vec![
            "step".to_string(),
            f.to_string_lossy().to_string(),
            "--scratch".to_string(),
            scratch_text.clone(),
        ]
    };
    let import = run_stage(&launcher, &files, c.jobs, deadline, "import", &args_of);
    let import_seconds = start.elapsed().as_secs_f64();
    write_lines(&c.out.join("import.jsonl"), &import)?;
    eprintln!(
        "abc: import: {} models in {import_seconds:.0} s: {:?}",
        import.len(),
        counts(&import)
    );

    let mut booleans = Vec::new();
    let mut boolean_seconds = 0.0;
    if c.booleans {
        let mut ready: Vec<PathBuf> = files
            .iter()
            .zip(&import)
            .filter(|(_, l)| l.get("booleanReady").and_then(Value::as_bool) == Some(true))
            .map(|(f, _)| f.clone())
            .collect();
        if let Some(limit) = c.boolean_limit {
            ready.truncate(limit);
        }
        let launcher = Launcher {
            bin,
            timeout: c.boolean_timeout,
            memory: c.memory.clone(),
            capped,
        };
        let started = Instant::now();
        let deadline = c
            .boolean_minutes
            .map(|m| started + Duration::from_secs(m.saturating_mul(60)));
        let args_of = |f: &Path| {
            let mut a = args_of(f);
            a.push("--booleans".to_string());
            a
        };
        booleans = run_stage(&launcher, &ready, c.jobs, deadline, "booleans", &args_of);
        boolean_seconds = started.elapsed().as_secs_f64();
        write_lines(&c.out.join("booleans.jsonl"), &booleans)?;
        eprintln!(
            "abc: booleans: {} models in {boolean_seconds:.0} s: {:?}",
            booleans.len(),
            counts(&booleans)
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);

    let fc = if c.freecad > 0 {
        match freecad(&c, &files, &import) {
            Ok(lines) => lines,
            Err(e) => {
                eprintln!("abc: FreeCAD: {e}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    if !fc.is_empty() {
        write_lines(&c.out.join("freecad.jsonl"), &fc)?;
    }

    let mut head = Map::new();
    head.insert("date".into(), now().into());
    head.insert("commit".into(), commit().into());
    head.insert(
        "chunk".into(),
        if c.files.is_some() {
            json!("local files")
        } else {
            json!(format!("{:04}", c.chunk))
        },
    );
    head.insert("jobs".into(), c.jobs.into());
    head.insert("timeout".into(), c.timeout.into());
    head.insert(
        "memory".into(),
        if capped == Cap::None {
            "no cap".into()
        } else {
            c.memory.clone().into()
        },
    );
    let card = scorecard(&head, total, &import, &booleans, &fc);
    std::fs::write(c.out.join("scorecard.md"), &card)
        .map_err(|e| format!("cannot write the scorecard: {e}"))?;
    let mut summary = head;
    summary.insert("files".into(), total.into());
    summary.insert("run".into(), import.len().into());
    summary.insert("import".into(), json!(counts(&import)));
    summary.insert("importSeconds".into(), import_seconds.into());
    summary.insert("booleans".into(), json!(counts(&booleans)));
    summary.insert("booleanSeconds".into(), boolean_seconds.into());
    summary.insert("freecad".into(), json!(counts(&fc)));
    let text = serde_json::to_string_pretty(&Value::Object(summary)).map_err(|e| e.to_string())?;
    std::fs::write(c.out.join("summary.json"), text)
        .map_err(|e| format!("cannot write the summary: {e}"))?;
    println!("{card}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_come_from_the_file_names() {
        let p = Path::new("/x/00000050/00000050_80d90bfdd2e74e709956122a_step_000.step");
        assert_eq!(model_id(p), "00000050");
        let p = Path::new("/x/00000050/00000050_80d90bfdd2e74e709956122a_step_002.step");
        assert_eq!(model_id(p), "00000050/2");
        assert_eq!(model_id(Path::new("block.step")), "block");
    }

    #[test]
    fn reasons_of_one_kind_count_together() {
        let a = "file.import: `00000050_80d9_step_000.step`, solid 0 (#123): unsupported surface: OFFSET_SURFACE of a B-spline (#77)";
        let b = "file.import: `00000051_aaaa_step_000.step`, solid 3 (#9 at a/b): unsupported surface: OFFSET_SURFACE of a B-spline (#12)";
        assert_eq!(reason_key(a), reason_key(b));
        assert_eq!(
            reason_key(a),
            "unsupported surface: OFFSET_SURFACE of a B-spline"
        );
        assert_eq!(
            reason_key("edge 12 is used 3 times"),
            "edge # is used # times"
        );
    }

    #[test]
    fn options_read() {
        let args: Vec<String> = [
            "--chunk",
            "7",
            "--booleans",
            "--jobs",
            "99",
            "--limit",
            "10",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let c = parse(&args).unwrap();
        assert_eq!(c.chunk, 7);
        assert!(c.booleans);
        assert_eq!(c.jobs, MAX_JOBS);
        assert_eq!(c.limit, Some(10));
        assert!(parse(&["--chunk".to_string(), "10000".to_string()]).is_err());
        assert!(parse(&["--memory".to_string(), "lots".to_string()]).is_err());
    }
}
