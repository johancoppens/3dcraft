//! `cargo xtask fuzz`: random multi-step designs (docs/testing.md ›
//! Random multi-step designs), many seeds, each in its own process with a
//! timeout and a memory cap; every crash, hang and wrong result shrunk to
//! the fewest groups and the roundest numbers that still fail the same
//! way; a summary for the morning health report.
//!
//! The designs are made and judged by `craft3d-fuzz` (apps/craft3d-fuzz),
//! which this task builds and runs; like the boolean corpus, the task
//! itself depends on no 3DCraft crate.
//!
//! What it leaves under the output directory (default `target/fuzz`):
//! `summary.json`, `seeds.jsonl` (one report per seed), and per finding
//! `<verdict>/<seed>.json` (the minimal design: `craft3d-fuzz check` runs
//! it), `<seed>.report.json` (its verdict) and `<seed>.script.json` (its
//! commands with ids, for `craft3d-cli run --script`).

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: cargo xtask fuzz [--seeds N] [--from S] [--jobs J] [--timeout SECONDS] \
[--memory SIZE] [--minutes M] [--shrink-runs N] [--shrink-minutes M] [--out DIR] [--bin PATH]";

/// Most processes at once: the machine has run out of memory before
/// (AGENTS.md › Parallel agents).
const MAX_JOBS: usize = 4;

struct Config {
    seeds: u64,
    from: u64,
    jobs: usize,
    timeout: u64,
    memory: String,
    minutes: Option<u64>,
    shrink_runs: usize,
    /// Wall time per finding for its shrinking: a hang costs the whole
    /// timeout per try.
    shrink_minutes: u64,
    out: PathBuf,
    bin: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Config, String> {
    let root = workspace_root();
    let mut c = Config {
        seeds: 100,
        from: 0,
        jobs: MAX_JOBS,
        timeout: 60,
        memory: "2G".to_string(),
        minutes: None,
        shrink_runs: 150,
        shrink_minutes: 10,
        // Where `cargo xtask health` reads it: the main checkout, also when
        // run from a worktree.
        out: crate::health::main_checkout(&root).join("target/fuzz"),
        bin: None,
    };
    let mut seeds_given = false;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it
            .next()
            .ok_or_else(|| format!("{flag} needs a value; {USAGE}"))?;
        let number = || -> Result<u64, String> {
            value
                .parse()
                .map_err(|_| format!("{flag}: `{value}` is not a whole number"))
        };
        match flag.as_str() {
            "--seeds" => {
                c.seeds = number()?;
                seeds_given = true;
            }
            "--from" => c.from = number()?,
            "--jobs" => {
                c.jobs = usize::try_from(number()?).unwrap_or(1).clamp(1, MAX_JOBS);
            }
            "--timeout" => c.timeout = number()?.max(1),
            "--memory" => {
                // A cap that does not read is an error, never no cap.
                if bytes(value).is_none() {
                    return Err(format!(
                        "--memory: `{value}` is not a size (such as 2G or 512M)"
                    ));
                }
                c.memory = value.clone();
            }
            "--minutes" => c.minutes = Some(number()?),
            "--shrink-runs" => c.shrink_runs = usize::try_from(number()?).unwrap_or(0),
            "--shrink-minutes" => c.shrink_minutes = number()?,
            "--out" => c.out = PathBuf::from(value),
            "--bin" => c.bin = Some(PathBuf::from(value)),
            other => return Err(format!("unknown option {other}; {USAGE}")),
        }
    }
    // A run for a time without a number of seeds runs until the time is up,
    // not until the default hundred seeds are done.
    if c.minutes.is_some() && !seeds_given {
        c.seeds = u64::MAX;
    }
    Ok(c)
}

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Builds `craft3d-fuzz` in release and returns where it is.
pub(crate) fn build() -> Result<PathBuf, String> {
    crate::cargo(&["build", "--release", "-p", "craft3d-fuzz"])?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace_root().join("target"), PathBuf::from);
    let bin = target.join("release/craft3d-fuzz");
    if bin.is_file() {
        Ok(bin)
    } else {
        Err(format!("built, but there is no {}", bin.display()))
    }
}

/// How one process is started: under a memory cap when systemd can give
/// one, always under `timeout`.
pub(crate) struct Launcher {
    pub(crate) bin: PathBuf,
    pub(crate) timeout: u64,
    pub(crate) memory: String,
    pub(crate) capped: Cap,
}

/// How a process's memory is capped.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cap {
    /// A systemd scope with `MemoryMax` (no swap): the whole process.
    Systemd,
    /// `prlimit --as`, where there is no user systemd (a CI runner): the
    /// address space, at twice the memory asked for (reserved but unused
    /// address space, thread stacks and malloc arenas, counts too), with
    /// two malloc arenas.
    AddressSpace(u64),
    None,
}

/// `2G`, `512M`, `100K` or a number of bytes.
pub(crate) fn bytes(size: &str) -> Option<u64> {
    let size = size.trim();
    let (digits, unit) = match size.char_indices().last()? {
        (i, c) if c.is_ascii_alphabetic() => (size.get(..i)?, c.to_ascii_uppercase()),
        _ => (size, 'B'),
    };
    let n: u64 = digits.trim().parse().ok()?;
    let scale: u64 = match unit {
        'B' => 1,
        'K' => 1 << 10,
        'M' => 1 << 20,
        'G' => 1 << 30,
        'T' => 1 << 40,
        _ => return None,
    };
    n.checked_mul(scale)
}

/// What became of one process.
pub(crate) struct Ran {
    /// The report, when the process ran to a verdict.
    pub(crate) report: Option<Value>,
    /// `timeout`, `oom` or `crash` when it did not.
    pub(crate) died: Option<(&'static str, String)>,
}

impl Launcher {
    pub(crate) fn run(&self, args: &[String]) -> Ran {
        let timeout = self.timeout.to_string();
        let mut command = match self.capped {
            Cap::Systemd => {
                let mut c = Command::new("systemd-run");
                c.args(["--user", "--scope", "-q"])
                    .arg("-p")
                    .arg(format!("MemoryMax={}", self.memory))
                    .args(["-p", "MemorySwapMax=0", "--", "timeout"]);
                c
            }
            Cap::AddressSpace(limit) => {
                let mut c = Command::new("prlimit");
                c.arg(format!("--as={limit}")).args(["--", "timeout"]);
                // glibc reserves 64 MB of address space per malloc arena,
                // up to eight per core: on a machine of many cores that,
                // not the memory used, filled a 2 GB cap (thread stacks
                // and allocations of a few MB refused, 17 false findings
                // in 36296 seeds on 2026-10-30). Two arenas.
                c.env("MALLOC_ARENA_MAX", "2");
                c
            }
            Cap::None => Command::new("timeout"),
        };
        // TERM at the timeout, KILL ten seconds later if it is still there.
        command
            .args(["-k", "10"])
            .arg(&timeout)
            .arg(&self.bin)
            .args(args);
        let started = Instant::now();
        let out = match command.output() {
            Ok(out) => out,
            Err(e) => {
                return Ran {
                    report: None,
                    died: Some(("crash", format!("cannot start the process: {e}"))),
                };
            }
        };
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        let tail = tail.join(" | ");
        if out.status.success()
            && let Some(report) = stdout
                .lines()
                .last()
                .and_then(|l| serde_json::from_str::<Value>(l).ok())
        {
            return Ran {
                report: Some(report),
                died: None,
            };
        }
        let died = match out.status.code() {
            Some(124) => ("timeout", format!("no verdict within {} s", self.timeout)),
            // KILLed: by `timeout -k` after the timeout, or before it by the
            // memory cap.
            Some(137) if started.elapsed().as_secs() >= self.timeout => (
                "timeout",
                format!("no verdict within {} s, killed", self.timeout),
            ),
            Some(137) => (
                "oom",
                format!("killed (out of memory at {}): {tail}", self.memory),
            ),
            // Under `prlimit --as` an allocation past the cap aborts the
            // process with Rust's own words.
            Some(_) | None if stderr.contains("memory allocation of") => (
                "oom",
                format!(
                    "an allocation failed (address space capped at {}): {tail}",
                    self.memory
                ),
            ),
            Some(code) => ("crash", format!("the process exited with {code}: {tail}")),
            None => (
                "crash",
                format!("the process was killed by a signal: {tail}"),
            ),
        };
        Ran {
            report: None,
            died: Some(died),
        }
    }
}

/// The verdict of a run as the shrinker compares it: the kind, the
/// oracle and the command.
fn signature(ran: &Ran) -> (String, String, String) {
    if let Some((kind, _)) = &ran.died {
        return (kind.to_string(), String::new(), String::new());
    }
    let report = ran.report.as_ref();
    let verdict = report
        .and_then(|r| r.get("verdict"))
        .and_then(Value::as_str)
        .unwrap_or("ok");
    let finding = report.and_then(|r| r.get("finding"));
    let field = |k: &str| {
        finding
            .and_then(|f| f.get(k))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    (verdict.to_string(), field("oracle"), field("command"))
}

fn verdict_dir(sig: &(String, String, String)) -> String {
    match sig.0.as_str() {
        "wrong" => format!("wrong-{}", sig.1),
        other => other.to_string(),
    }
}

/// Reads a design file; `None` when it is missing or unreadable.
fn read_design(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn groups_of(design: &Value) -> Vec<Value> {
    design
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn with_groups(design: &Value, groups: Vec<Value>) -> Value {
    let mut d = design.clone();
    if let Some(map) = d.as_object_mut() {
        map.insert("groups".into(), Value::Array(groups));
    }
    d
}

/// Rounder versions of a value, to try in its place.
fn simpler(key: &str, value: &Value, base: f64) -> Vec<Value> {
    match value {
        Value::Bool(true) => vec![Value::Bool(false)],
        Value::String(s) if s.ends_with(" deg") && s != "0 deg" && s != "90 deg" => {
            vec![json!("0 deg"), json!("90 deg")]
        }
        Value::Number(n) => {
            let Some(x) = n.as_f64() else {
                return Vec::new();
            };
            let mut out = Vec::new();
            let half = base / 2.0;
            let round = (x / half).round() * half;
            for candidate in [round, 0.0, base] {
                // A size of 0 is never valid; a position or an offset may be.
                let positive_only = matches!(key, "radius" | "major" | "minor" | "thickness");
                if positive_only && candidate <= 0.0 {
                    continue;
                }
                if candidate != x
                    && candidate.is_finite()
                    && let Some(n) = serde_json::Number::from_f64(candidate)
                    && !out.contains(&Value::Number(n.clone()))
                {
                    out.push(Value::Number(n));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Every place in `value` where a simpler leaf can go, as paths of keys
/// and indices.
fn leaves(value: &Value, path: &mut Vec<Value>, out: &mut Vec<Vec<Value>>, depth: u32) {
    if depth > 16 {
        return;
    }
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                // Names and references stay as they are.
                if matches!(k.as_str(), "name" | "sketch" | "target" | "tool" | "source") {
                    continue;
                }
                path.push(json!(k));
                leaves(v, path, out, depth + 1);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                path.push(json!(i));
                leaves(v, path, out, depth + 1);
                path.pop();
            }
        }
        Value::Bool(_) | Value::Number(_) | Value::String(_) => out.push(path.clone()),
        Value::Null => {}
    }
}

fn at_path<'a>(value: &'a mut Value, path: &[Value]) -> Option<&'a mut Value> {
    let mut here = value;
    for step in path {
        here = match step {
            Value::String(k) => here.get_mut(k.as_str())?,
            Value::Number(i) => here.get_mut(usize::try_from(i.as_u64()?).ok()?)?,
            _ => return None,
        };
    }
    Some(here)
}

/// The key a leaf is under: the last object key on its path.
fn key_of(path: &[Value]) -> String {
    path.iter()
        .rev()
        .find_map(|p| p.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Shrinks a failing design: drops groups (halves, quarters, ... single
/// ones) and rounds its numbers, keeping each change that still fails
/// with the same signature, within `budget` runs.
fn shrink(
    launcher: &Launcher,
    design: &Value,
    want: &(String, String, String),
    budget: usize,
    minutes: u64,
    work: &Path,
) -> (Value, usize) {
    let mut runs = 0;
    let deadline = Instant::now() + Duration::from_secs(minutes.saturating_mul(60));
    let candidate = work.join("candidate.json");
    let fails = |d: &Value, runs: &mut usize| -> bool {
        if *runs >= budget || Instant::now() >= deadline {
            return false;
        }
        *runs += 1;
        if std::fs::write(&candidate, d.to_string()).is_err() {
            return false;
        }
        let ran = launcher.run(&[
            "check".to_string(),
            candidate.to_string_lossy().to_string(),
            "--scratch".to_string(),
            work.to_string_lossy().to_string(),
        ]);
        signature(&ran) == *want
    };
    let mut best = design.clone();
    // Groups.
    let mut chunk = groups_of(&best).len().div_ceil(2).max(1);
    loop {
        let mut start = 0;
        while start < groups_of(&best).len() {
            let groups = groups_of(&best);
            let end = (start + chunk).min(groups.len());
            let kept: Vec<Value> = groups
                .iter()
                .enumerate()
                .filter(|(i, _)| *i < start || *i >= end)
                .map(|(_, g)| g.clone())
                .collect();
            let trial = with_groups(&best, kept);
            if !groups_of(&trial).is_empty() && fails(&trial, &mut runs) {
                best = trial;
            } else {
                start = end;
            }
        }
        if chunk == 1 || runs >= budget {
            break;
        }
        chunk = chunk.div_ceil(2);
    }
    // Numbers, angles and flags.
    let base = 10.0 * best.get("scale").and_then(Value::as_f64).unwrap_or(1.0);
    let mut paths = Vec::new();
    leaves(
        best.get("groups").unwrap_or(&Value::Null),
        &mut vec![json!("groups")],
        &mut paths,
        0,
    );
    for path in paths {
        if runs >= budget {
            break;
        }
        let Some(current) = at_path(&mut best.clone(), &path).map(|v| v.clone()) else {
            continue;
        };
        for option in simpler(&key_of(&path), &current, base) {
            let mut trial = best.clone();
            if let Some(slot) = at_path(&mut trial, &path) {
                *slot = option;
            }
            if fails(&trial, &mut runs) {
                best = trial;
                break;
            }
        }
    }
    (best, runs)
}

/// A seed that failed: the seed, its design file, its signature and its
/// report (or how its process died).
type Found = (u64, PathBuf, (String, String, String), Value);

/// What the whole run counted.
#[derive(Default)]
struct Tally {
    seeds: u64,
    groups: u64,
    accepted: u64,
    refused: u64,
    booleans: u64,
    identities_checked: u64,
    identities_skipped: u64,
    verdicts: BTreeMap<String, u64>,
    wrong_by_oracle: BTreeMap<String, u64>,
    reasons: BTreeMap<String, u64>,
    /// (seed, the design file, the signature, the report or the death).
    findings: Vec<Found>,
    lines: Vec<String>,
}

fn number(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn one_seed(launcher: &Launcher, seed: u64, out: &Path, tally: &Mutex<Tally>, start: Instant) {
    let seeds = out.join("seeds");
    let design = seeds.join(format!("{seed}.json"));
    let ran = launcher.run(&[
        "seed".to_string(),
        seed.to_string(),
        "--steps-out".to_string(),
        design.to_string_lossy().to_string(),
        "--scratch".to_string(),
        seeds.to_string_lossy().to_string(),
    ]);
    let sig = signature(&ran);
    let report = match (&ran.report, &ran.died) {
        (Some(r), _) => r.clone(),
        (None, Some((kind, detail))) => json!({
            "seed": seed, "verdict": kind,
            "scale": read_design(&design).and_then(|d| d.get("scale").cloned()),
            "finding": {"kind": kind, "oracle": kind, "detail": detail},
        }),
        (None, None) => json!({ "seed": seed, "verdict": "crash" }),
    };
    let mut t = tally
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    t.seeds += 1;
    if t.seeds.is_multiple_of(100) {
        let findings = t.findings.len() + usize::from(sig.0 != "ok");
        eprintln!(
            "fuzz: {} seeds, {findings} findings, {:.0} s",
            t.seeds,
            start.elapsed().as_secs_f64()
        );
    }
    t.groups += number(&report, "groups");
    t.accepted += number(&report, "accepted");
    t.booleans += number(&report, "booleans");
    if let Some(ids) = report.get("identities") {
        t.identities_checked += number(ids, "checked");
        t.identities_skipped += number(ids, "skipped");
    }
    for r in report
        .get("refused")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        t.refused += 1;
        let key = r
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        *t.reasons.entry(key).or_default() += 1;
    }
    *t.verdicts.entry(sig.0.clone()).or_default() += 1;
    if sig.0 == "wrong" {
        *t.wrong_by_oracle.entry(sig.1.clone()).or_default() += 1;
    }
    t.lines.push(report.to_string());
    if sig.0 == "ok" {
        let _ = std::fs::remove_file(&design);
    } else {
        // At once, with its design, so a run cut short before the
        // shrinking still hands over what it found.
        println!("found: {report}");
        println!(
            "found design {seed}: {}",
            read_design(&design).unwrap_or(Value::Null)
        );
        t.findings.push((seed, design, sig, report));
    }
}

pub(crate) fn commit() -> String {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(
            || "unknown".to_string(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
}

pub(crate) fn now() -> String {
    Command::new("date")
        .arg("+%Y-%m-%d %H:%M")
        .output()
        .ok()
        .map_or_else(
            || "unknown".to_string(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
}

/// How processes capped at `memory` can be capped on this machine.
pub(crate) fn cap_for(memory: &str) -> Cap {
    let works = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if works("systemd-run", &["--user", "--scope", "-q", "true"]) {
        Cap::Systemd
    } else if let Some(limit) = bytes(memory).and_then(|b| b.checked_mul(2))
        && works("prlimit", &[&format!("--as={limit}"), "--", "true"])
    {
        eprintln!("xtask: no user systemd; each process's address space is capped by prlimit");
        Cap::AddressSpace(limit)
    } else {
        eprintln!("xtask: systemd-run is not available; the processes run without a memory cap");
        Cap::None
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let config = parse(args)?;
    let bin = match &config.bin {
        Some(b) => b.clone(),
        None => build()?,
    };
    let capped = cap_for(&config.memory);
    let launcher = Launcher {
        bin,
        timeout: config.timeout,
        memory: config.memory.clone(),
        capped,
    };
    let out = &config.out;
    std::fs::create_dir_all(out.join("seeds"))
        .map_err(|e| format!("cannot make {}: {e}", out.display()))?;
    eprintln!(
        "fuzz: seeds {}..{}, {} at once, {} s and {} each, into {}",
        config.from,
        config.from.saturating_add(config.seeds),
        config.jobs,
        config.timeout,
        if capped == Cap::None {
            "no cap"
        } else {
            config.memory.as_str()
        },
        out.display()
    );
    let start = Instant::now();
    let deadline = config.minutes.map(|m| start + Duration::from_secs(m * 60));
    let next = AtomicU64::new(config.from);
    let end = config.from.saturating_add(config.seeds);
    let tally = Mutex::new(Tally::default());
    std::thread::scope(|scope| {
        for _ in 0..config.jobs {
            scope.spawn(|| {
                loop {
                    if deadline.is_some_and(|d| Instant::now() >= d) {
                        return;
                    }
                    let seed = next.fetch_add(1, Ordering::Relaxed);
                    if seed >= end {
                        return;
                    }
                    one_seed(&launcher, seed, out, &tally, start);
                }
            });
        }
    });
    let run_seconds = start.elapsed().as_secs_f64();
    let mut tally = tally
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    tally.findings.sort_by_key(|f| f.0);
    eprintln!(
        "fuzz: {} seeds in {run_seconds:.0} s; {} findings to shrink",
        tally.seeds,
        tally.findings.len()
    );
    // Shrink the findings, a few at a time.
    let shrunk: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    let queue = Mutex::new(tally.findings.iter());
    std::thread::scope(|scope| {
        for worker in 0..config.jobs {
            let (launcher, queue, shrunk, out) = (&launcher, &queue, &shrunk, out);
            let budget = config.shrink_runs;
            let minutes = config.shrink_minutes;
            scope.spawn(move || {
                let work = out.join(format!("shrink-{worker}"));
                let _ = std::fs::create_dir_all(&work);
                loop {
                    let item = queue
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .next();
                    let Some((seed, path, sig, report)) = item else {
                        break;
                    };
                    let entry = shrink_one(
                        launcher,
                        *seed,
                        path,
                        sig,
                        report,
                        (budget, minutes),
                        &work,
                        out,
                    );
                    shrunk
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(entry);
                }
                let _ = std::fs::remove_dir_all(&work);
            });
        }
    });
    let mut findings = shrunk
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    findings.sort_by_key(|f| f.get("seed").and_then(Value::as_u64).unwrap_or(0));
    tally.lines.sort();
    let _ = std::fs::write(out.join("seeds.jsonl"), tally.lines.join("\n") + "\n");
    let mut reasons: Vec<(String, u64)> = tally.reasons.clone().into_iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top: Vec<Value> = reasons
        .iter()
        .take(10)
        .map(|(k, n)| json!({"reason": k, "count": n}))
        .collect();
    let wall = start.elapsed().as_secs_f64();
    let rate = |n: u64, of: u64| if of == 0 { 0.0 } else { n as f64 / of as f64 };
    let summary = json!({
        "date": now(),
        "commit": commit(),
        "seeds": tally.seeds,
        "from": config.from,
        "groups": tally.groups,
        "accepted": tally.accepted,
        "refused": tally.refused,
        "refusalRate": rate(tally.refused, tally.groups),
        "booleans": tally.booleans,
        "identitiesChecked": tally.identities_checked,
        "identitiesSkipped": tally.identities_skipped,
        "verdicts": tally.verdicts,
        "wrongByOracle": tally.wrong_by_oracle,
        "wrongRate": rate(tally.verdicts.get("wrong").copied().unwrap_or(0), tally.seeds),
        "crashes": tally.verdicts.get("crash").copied().unwrap_or(0),
        "bugs": tally.verdicts.get("bug").copied().unwrap_or(0),
        "timeouts": tally.verdicts.get("timeout").copied().unwrap_or(0),
        "outOfMemory": tally.verdicts.get("oom").copied().unwrap_or(0),
        "refusedTop": top,
        "findings": findings,
        "jobs": config.jobs,
        "timeoutSeconds": config.timeout,
        "memoryCap": if capped == Cap::None { "none".to_string() } else { config.memory.clone() },
        "runSeconds": run_seconds,
        "wallSeconds": wall,
        "seedsPerSecond": rate(tally.seeds, 1).max(0.0) / run_seconds.max(1e-9),
    });
    let text = serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?;
    std::fs::write(out.join("summary.json"), &text)
        .map_err(|e| format!("cannot write the summary: {e}"))?;
    let mut short = Map::new();
    for key in [
        "seeds",
        "groups",
        "accepted",
        "refused",
        "verdicts",
        "wrongByOracle",
        "runSeconds",
    ] {
        if let Some(v) = summary.get(key) {
            short.insert(key.into(), v.clone());
        }
    }
    println!("{}", Value::Object(short));
    // Every finding with its minimal design inline, so a run on a machine
    // whose files cannot be read (the Strix, through a Check run) still
    // hands over its reproducers in its log.
    for f in &findings {
        let design = f
            .get("reproducer")
            .and_then(Value::as_str)
            .and_then(|p| read_design(Path::new(p)))
            .unwrap_or(Value::Null);
        println!("finding: {f}");
        println!("design {}: {design}", number(f, "seed"));
    }
    println!("summary: {}", out.join("summary.json").display());
    Ok(())
}

/// Shrinks one finding and writes its reproducer; its summary entry.
#[allow(clippy::too_many_arguments)]
fn shrink_one(
    launcher: &Launcher,
    seed: u64,
    path: &Path,
    sig: &(String, String, String),
    report: &Value,
    (budget, minutes): (usize, u64),
    work: &Path,
    out: &Path,
) -> Value {
    let dir = out.join(verdict_dir(sig));
    let _ = std::fs::create_dir_all(&dir);
    let mut entry = json!({
        "seed": seed,
        "scale": report.get("scale"),
        "verdict": sig.0,
        "oracle": sig.1,
        "command": sig.2,
        "detail": report.pointer("/finding/detail"),
    });
    let Some(design) = read_design(path) else {
        if let Some(map) = entry.as_object_mut() {
            map.insert("reproducer".into(), Value::Null);
        }
        return entry;
    };
    // The groups after the one it was found at do not matter.
    let found_at = report
        .pointer("/finding/group")
        .and_then(Value::as_u64)
        .and_then(|g| usize::try_from(g).ok());
    let mut groups = groups_of(&design);
    if let Some(g) = found_at {
        groups.truncate(g.saturating_add(1));
    }
    let design = with_groups(&design, groups);
    let original = groups_of(&design).len();
    let (minimal, runs) = shrink(launcher, &design, sig, budget, minutes, work);
    let file = dir.join(format!("{seed}.json"));
    let _ = std::fs::write(
        &file,
        serde_json::to_string_pretty(&minimal).unwrap_or_default(),
    );
    // Its verdict now, and the script that replays it.
    let script = dir.join(format!("{seed}.script.json"));
    let check = launcher.run(&[
        "check".to_string(),
        file.to_string_lossy().to_string(),
        "--scratch".to_string(),
        work.to_string_lossy().to_string(),
        "--script-out".to_string(),
        script.to_string_lossy().to_string(),
    ]);
    let final_report = check.report.clone().unwrap_or_else(|| {
        let (kind, detail) = check.died.clone().unwrap_or(("crash", String::new()));
        json!({"verdict": kind, "finding": {"detail": detail}})
    });
    let _ = std::fs::write(
        dir.join(format!("{seed}.report.json")),
        serde_json::to_string_pretty(&final_report).unwrap_or_default(),
    );
    let _ = std::fs::remove_file(path);
    if let Some(map) = entry.as_object_mut() {
        map.insert("groups".into(), original.into());
        map.insert("shrunkTo".into(), groups_of(&minimal).len().into());
        map.insert("shrinkRuns".into(), runs.into());
        map.insert("stillFails".into(), (signature(&check) == *sig).into());
        map.insert(
            "minimalDetail".into(),
            final_report
                .pointer("/finding/detail")
                .cloned()
                .unwrap_or(Value::Null),
        );
        map.insert(
            "reproducer".into(),
            file.to_string_lossy().to_string().into(),
        );
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simpler_numbers_are_rounder() {
        assert_eq!(
            simpler("x", &json!(7.5), 10.0),
            vec![json!(10.0), json!(0.0)]
        );
        assert_eq!(
            simpler("radius", &json!(2.5), 10.0),
            vec![json!(5.0), json!(10.0)]
        );
        assert_eq!(simpler("keep", &json!(true), 10.0), vec![json!(false)]);
        assert!(simpler("keep", &json!(false), 10.0).is_empty());
        assert_eq!(simpler("angle", &json!("45 deg"), 10.0).len(), 2);
    }

    #[test]
    fn minutes_without_seeds_run_until_the_time_is_up() {
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let timed = parse(&args(&["--minutes", "60"])).expect("valid flags");
        assert_eq!(timed.seeds, u64::MAX);
        let both = parse(&args(&["--minutes", "60", "--seeds", "500"])).expect("valid flags");
        assert_eq!(both.seeds, 500);
        let plain = parse(&args(&[])).expect("valid flags");
        assert_eq!(plain.seeds, 100);
        assert!(parse(&args(&["--memory", "lots"])).is_err());
        assert!(parse(&args(&["--memory", "512M"])).is_ok());
    }

    #[test]
    fn sizes_read_as_bytes() {
        assert_eq!(bytes("2G"), Some(2 << 30));
        assert_eq!(bytes("512m"), Some(512 << 20));
        assert_eq!(bytes("4096"), Some(4096));
        assert_eq!(bytes("lots"), None);
        assert_eq!(bytes(""), None);
        assert_eq!(bytes("99999999999T"), None);
    }

    #[test]
    fn leaves_skip_names() {
        let v = json!({"name": "N1", "size": [1, 2], "target": "N0", "keep": true});
        let mut out = Vec::new();
        leaves(&v, &mut Vec::new(), &mut out, 0);
        assert_eq!(
            out,
            vec![
                vec![json!("keep")],
                vec![json!("size"), json!(0)],
                vec![json!("size"), json!(1)]
            ]
        );
    }
}
