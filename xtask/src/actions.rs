//! `cargo xtask actions`: the action matrix (docs/testing.md, oracle 12).
//!
//! Every local operation (fillet, chamfer, offset, delete face, shell,
//! thicken, draft, move face, mirror, pattern) is tried on the faces and
//! edges of the finished bodies of the showcase models, each on the
//! finished document and undone again, in `craft3d-cli serve` processes
//! (at most three, each under a memory cap) with a time limit per call. A
//! process that dies or runs out of time is a crash or a timeout of the
//! action it was on, and is started again for the next one.
//!
//! The outcome of each action is ok (built, `document.check` deep finds
//! nothing, the volume keeps its identity), refused (with the reason),
//! wrong (built, but the check or the identity fails), crash or timeout.
//! The ratchet (`crates/engine/tests/action_matrix/ratchet.json`) holds,
//! per model and kind, the entities that succeeded; one of them that does
//! not succeed now is a regression and fails the run, and so does an entry
//! that was not tried (gone, not planned, its model skipped or a worker
//! lost). `--update` adds the ones that newly succeed in under a third of
//! the time limit (raising the floor), `--raise` does it later from the
//! last run's results when they are of the commit checked out; nothing is
//! ever taken out here. A ratchet entry that times out is run once more
//! alone with four times the limit before it counts.
//!
//! What it leaves in `<main checkout>/target/actions/`: `summary.json`
//! (read by `cargo xtask health`), `results/<model>.jsonl` (one line per
//! action), `<verdict>/<model>-<kind>-<hash>.steps.json` with a `.sh` that
//! replays it (for every wrong, crash and timeout), `models/` (the models
//! built from steps) and `logs/` (the servers' stderr).
//!
//! The plan, the run of one action and its verdict are in
//! `crates/engine/tests/action_matrix/plan.rs`, shared with the gated
//! test that runs a small subset in process.

#[path = "../../crates/engine/tests/action_matrix/plan.rs"]
mod plan;

use plan::{Action, CallError, KINDS, Kind, Outcome, Ratchet, Verdict};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

const RATCHET: &str = "crates/engine/tests/action_matrix/ratchet.json";
/// Entities per model and kind when nothing else is asked for.
const PER_KIND: usize = 40;
/// Actions a server runs before it is started afresh.
const FRESH_EVERY: usize = 250;

enum Source {
    /// Steps of the repository, built here.
    Steps(&'static str),
    /// Steps under `target/work`, built here.
    WorkSteps(&'static str),
    /// A document under `target/work`.
    WorkFile(&'static str),
}

struct Model {
    name: &'static str,
    source: Source,
    /// Small enough to try every entity.
    every: bool,
    /// In the default run.
    default: bool,
}

const MODELS: &[Model] = &[
    Model {
        name: "bracket",
        source: Source::Steps("crates/engine/tests/action_matrix/bracket.steps.json"),
        every: true,
        default: true,
    },
    Model {
        name: "rotor",
        source: Source::Steps("crates/engine/tests/bench_models/rotor.steps.json"),
        every: true,
        default: true,
    },
    Model {
        name: "impeller-blades",
        source: Source::Steps("crates/engine/tests/bench_models/impeller-blades.steps.json"),
        every: false,
        default: true,
    },
    Model {
        name: "engine-block",
        source: Source::Steps("crates/engine/tests/bench_models/engine-block.steps.json"),
        every: false,
        default: true,
    },
    Model {
        name: "turbofan",
        source: Source::Steps("crates/engine/tests/bench_models/turbofan.steps.json"),
        every: false,
        default: true,
    },
    Model {
        name: "laserbox",
        source: Source::WorkFile("laserbox/box.3dcraft"),
        every: false,
        default: true,
    },
    Model {
        name: "weldtable",
        source: Source::WorkFile("weldtable/weldtable.3dcraft"),
        every: false,
        default: true,
    },
    Model {
        name: "weldtable-w3",
        source: Source::WorkFile("weldtable/weldtable-w3-k0.3dcraft"),
        every: false,
        default: true,
    },
    Model {
        name: "bike",
        source: Source::WorkSteps("bike/all.steps.json"),
        every: false,
        default: true,
    },
    // Regenerating the lamp takes about ten minutes: only on request.
    Model {
        name: "lamp",
        source: Source::WorkFile("lamp/lamp.3dcraft"),
        every: false,
        default: false,
    },
];

struct Options {
    models: Vec<&'static Model>,
    per_kind: Option<usize>,
    jobs: usize,
    timeout: Duration,
    load_timeout: Duration,
    memory: String,
    update: bool,
    raise: bool,
    cli: Option<PathBuf>,
}

fn usage() -> String {
    let names: Vec<&str> = MODELS.iter().map(|m| m.name).collect();
    format!(
        "usage: cargo xtask actions [--models a,b] [--per-kind N | --all] [--jobs N (1-3)] \
         [--timeout SECONDS] [--memory 2G] [--update] [--cli PATH]\n       cargo xtask actions --raise [--models a,b]   (raise the ratchet from the last run's results)\nmodels: {}",
        names.join(", ")
    )
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        models: MODELS.iter().filter(|m| m.default).collect(),
        per_kind: Some(PER_KIND),
        jobs: 3,
        timeout: Duration::from_secs(60),
        load_timeout: Duration::from_secs(1800),
        memory: "2G".to_string(),
        update: false,
        raise: false,
        cli: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value\n{}", usage()))
        };
        match arg.as_str() {
            "--models" => {
                let wanted = value()?;
                let mut models = Vec::new();
                for name in wanted.split(',').filter(|n| !n.is_empty()) {
                    let model = MODELS
                        .iter()
                        .find(|m| m.name == name)
                        .ok_or_else(|| format!("no model `{name}`\n{}", usage()))?;
                    models.push(model);
                }
                o.models = models;
            }
            "--per-kind" => {
                o.per_kind = Some(value()?.parse().map_err(|e| format!("--per-kind: {e}"))?);
            }
            "--all" => o.per_kind = None,
            "--jobs" => {
                let jobs: usize = value()?.parse().map_err(|e| format!("--jobs: {e}"))?;
                o.jobs = jobs.clamp(1, 3);
            }
            "--timeout" => {
                let s: u64 = value()?.parse().map_err(|e| format!("--timeout: {e}"))?;
                o.timeout = Duration::from_secs(s.max(1));
            }
            "--memory" => o.memory = value()?,
            "--update" => o.update = true,
            "--raise" => o.raise = true,
            "--cli" => o.cli = Some(PathBuf::from(value()?)),
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("unknown option `{other}`\n{}", usage())),
        }
    }
    Ok(o)
}

/// The main checkout: the parent of the common git dir.
fn main_checkout(repo: &Path) -> PathBuf {
    Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(repo)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let dir = String::from_utf8_lossy(&o.stdout).trim().to_string();
            Path::new(&dir).parent().map(Path::to_path_buf)
        })
        .unwrap_or_else(|| repo.to_path_buf())
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Builds `craft3d-cli` in release and returns its path.
fn build_cli(repo: &Path) -> Result<PathBuf, String> {
    crate::cargo(&["build", "--release", "--quiet", "-p", "craft3d-cli"])?;
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| repo.join("target"), PathBuf::from);
    let cli = target.join("release").join("craft3d-cli");
    if cli.is_file() {
        Ok(cli)
    } else {
        Err(format!(
            "built craft3d-cli, but there is none at {}",
            cli.display()
        ))
    }
}

/// `systemd-run` when it can cap memory here, so each server runs under
/// its own cap.
fn memory_cap(memory: &str) -> Option<Vec<String>> {
    let cap = vec![
        "--user".to_string(),
        "--scope".to_string(),
        "-q".to_string(),
        "-p".to_string(),
        format!("MemoryMax={memory}"),
        "-p".to_string(),
        "MemorySwapMax=0".to_string(),
    ];
    let works = Command::new("systemd-run")
        .args(&cap)
        .arg("true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    works.then_some(cap)
}

struct Launch {
    cli: PathBuf,
    read_root: PathBuf,
    cap: Option<Vec<String>>,
    logs: PathBuf,
}

/// One `craft3d-cli serve` process.
struct Worker {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next: u64,
}

impl Worker {
    fn start(launch: &Launch, log: &str) -> Result<Worker, String> {
        let mut command = match &launch.cap {
            Some(cap) => {
                let mut c = Command::new("systemd-run");
                c.args(cap).arg(&launch.cli);
                c
            }
            None => Command::new(&launch.cli),
        };
        let stderr = log_file(launch, log);
        let mut child = command
            .arg("serve")
            .arg("--automation-read-root")
            .arg(&launch.read_root)
            .args(["--recovery-dir", "none"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", launch.cli.display()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (send, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Worker {
            child,
            stdin,
            lines,
            next: 0,
        })
    }

    fn call(&mut self, command: &str, params: &Value, limit: Duration) -> Result<Value, CallError> {
        self.next += 1;
        let id = self.next;
        let request = json!({"id": id, "method": "engine.execute", "params": {"command": command, "params": params}});
        if writeln!(self.stdin, "{request}")
            .and_then(|()| self.stdin.flush())
            .is_err()
        {
            return Err(CallError::Dead(self.exit()));
        }
        let until = Instant::now() + limit;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if reply.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
                        return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
                    }
                    let error = reply.get("error").map_or_else(String::new, |e| {
                        e.as_str().map_or_else(|| e.to_string(), str::to_string)
                    });
                    return Err(CallError::Command(error));
                }
                Err(RecvTimeoutError::Timeout) => return Err(CallError::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(CallError::Dead(self.exit())),
            }
        }
    }

    /// How the process ended, once it has.
    fn exit(&mut self) -> String {
        for _ in 0..50 {
            if let Ok(Some(status)) = self.child.try_wait() {
                return describe(status);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        "the server stopped answering".to_string()
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn describe(status: std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(signal) = status.signal() {
            let hint = if signal == 9 {
                " (the memory cap, or killed)"
            } else {
                ""
            };
            return format!("the server was killed by signal {signal}{hint}");
        }
    }
    format!("the server exited ({status})")
}

/// A worker with the model open and regenerated.
fn open(
    launch: &Launch,
    log: &str,
    file: &Path,
    limit: Duration,
) -> Result<(Worker, Value), String> {
    let mut worker = Worker::start(launch, log)?;
    let opened = worker.call("file.open", &json!({"path": file.to_string_lossy()}), limit);
    let inspect = opened.and_then(|_| worker.call("document.inspect", &json!({}), limit));
    match inspect {
        Ok(inspect) => Ok((worker, inspect)),
        Err(e) => {
            worker.stop();
            Err(format!("cannot open {}: {}", file.display(), call_text(&e)))
        }
    }
}

fn call_text(e: &CallError) -> String {
    match e {
        CallError::Command(m) | CallError::Dead(m) => m.clone(),
        CallError::Timeout => "timed out".to_string(),
    }
}

/// The document of a model, built from its steps when it has them.
fn prepare(
    model: &Model,
    repo: &Path,
    main: &Path,
    out: &Path,
    launch: &Launch,
    limit: Duration,
) -> Result<(PathBuf, Option<PathBuf>), Skip> {
    let fail = |why: String| Skip {
        why,
        missing: false,
    };
    let steps = match model.source {
        Source::Steps(path) => repo.join(path),
        Source::WorkSteps(path) => main.join("target/work").join(path),
        Source::WorkFile(path) => {
            let file = main.join("target/work").join(path);
            return if file.is_file() {
                Ok((file, None))
            } else {
                // Not in the repository: missing here is a warning.
                Err(Skip {
                    why: format!("{} is not there", file.display()),
                    missing: true,
                })
            };
        }
    };
    if !steps.is_file() {
        return Err(Skip {
            why: format!("{} is not there", steps.display()),
            missing: matches!(model.source, Source::WorkSteps(_)),
        });
    }
    let file = out.join("models").join(format!("{}.3dcraft", model.name));
    let _ = std::fs::remove_file(&file);
    let mut command = match &launch.cap {
        Some(cap) => {
            let mut c = Command::new("systemd-run");
            c.args(cap).arg(&launch.cli);
            c
        }
        None => Command::new(&launch.cli),
    };
    let mut child = command
        .arg("run")
        .arg("--script")
        .arg(&steps)
        .arg("--out")
        .arg(&file)
        .stdout(Stdio::null())
        .stderr(log_file(launch, &format!("{}-prepare.log", model.name)))
        .spawn()
        .map_err(|e| fail(format!("cannot run {}: {e}", launch.cli.display())))?;
    let until = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(200)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(fail(format!(
                    "building {} from its steps took more than {} s",
                    model.name,
                    limit.as_secs()
                )));
            }
            Err(e) => return Err(fail(format!("building {}: {e}", model.name))),
        }
    };
    if status.success() && file.is_file() {
        Ok((file, Some(steps)))
    } else {
        Err(fail(format!(
            "building {} from its steps failed ({status}; logs/{}-prepare.log)",
            model.name, model.name
        )))
    }
}

/// Why a model was not run.
struct Skip {
    why: String,
    /// Its input is not there and is not in the repository (`target/work`):
    /// a warning, not a failure.
    missing: bool,
}

/// A log file of `launch`, appended to; nowhere when it cannot be opened.
fn log_file(launch: &Launch, name: &str) -> Stdio {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(launch.logs.join(name))
        .map_or_else(|_| Stdio::null(), Stdio::from)
}

/// One action's line in the results.
struct Record {
    action: Action,
    outcome: Outcome,
    seconds: f64,
    /// The time limit of each call it made.
    limit: f64,
    /// Run again alone after a timeout ([`retry_timeouts`]).
    retried: bool,
}

impl Record {
    /// Whether it may go into the ratchet: ok, in under a third of its
    /// limit, so that a slower day does not make it a timeout.
    fn ratchets(&self) -> bool {
        self.outcome.verdict == Verdict::Ok && self.seconds < self.limit / 3.0
    }
}

#[derive(Default)]
struct ModelReport {
    skipped: Option<Skip>,
    /// What kept actions from being planned or run: each fails the run.
    problems: Vec<String>,
    /// Bodies that are not solids (sheets): no action is planned on them.
    sheets: usize,
    /// Bodies whose edges were not all listed (`document.edges` lists 2000
    /// at most).
    truncated: usize,
    /// Blends whose exact change was worked out ([`plan::orient_all`]).
    exact: usize,
    seconds: f64,
    load_seconds: f64,
    bodies: usize,
    faces: usize,
    planned: usize,
    records: Vec<Record>,
}

/// Runs `actions` on `jobs` workers with the model `file` open.
fn run_model(
    o: &Options,
    launch: &Launch,
    model: &Model,
    file: &Path,
    first: Worker,
    actions: Vec<Action>,
    problems: &mut Vec<String>,
) -> Vec<Record> {
    let total = actions.len();
    let queue = Mutex::new(actions.into_iter().collect::<VecDeque<Action>>());
    let (send, receive) = channel::<Record>();
    let jobs = o.jobs.min(total).max(1);
    let mut records = Vec::with_capacity(total);
    let failures = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        let mut first = Some(first);
        for job in 0..jobs {
            let send = send.clone();
            let queue = &queue;
            let failures = &failures;
            let given = first.take();
            scope.spawn(move || {
                let log = format!("{}-{job}.log", model.name);
                let mut worker = given;
                let mut done = 0_usize;
                loop {
                    let next = queue
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .pop_front();
                    let Some(action) = next else { break };
                    if worker.is_none() || done >= FRESH_EVERY {
                        if let Some(w) = worker.take() {
                            w.stop();
                        }
                        done = 0;
                        match open(launch, &log, file, o.load_timeout) {
                            Ok((w, _)) => worker = Some(w),
                            Err(e) => {
                                eprintln!("actions: {}: worker {job}: {e}", model.name);
                                queue
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .push_front(action);
                                failures
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .push(format!(
                                        "worker {job} could not open the model again: {e}"
                                    ));
                                break;
                            }
                        }
                    }
                    let Some(w) = worker.as_mut() else { break };
                    let start = Instant::now();
                    let mut call = |id: &str, params: &Value| w.call(id, params, o.timeout);
                    let outcome = plan::run_action(&mut call, &action);
                    done += 1;
                    if outcome.restart
                        && let Some(w) = worker.take()
                    {
                        w.stop();
                    }
                    let record = Record {
                        action,
                        outcome,
                        seconds: start.elapsed().as_secs_f64(),
                        limit: o.timeout.as_secs_f64(),
                        retried: false,
                    };
                    if send.send(record).is_err() {
                        break;
                    }
                }
                if let Some(w) = worker.take() {
                    w.stop();
                }
            });
        }
        drop(send);
        for record in receive {
            records.push(record);
            if records.len() % 50 == 0 || records.len() == total {
                eprintln!("actions: {}: {}/{total}", model.name, records.len());
            }
        }
    });
    problems.extend(
        failures
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner),
    );
    let left = queue
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    if left > 0 {
        problems.push(format!(
            "{left} actions were not run: no worker could take them"
        ));
    }
    records
        .sort_by(|a, b| (a.action.kind, &a.action.entity).cmp(&(b.action.kind, &b.action.entity)));
    records
}

/// Runs the ratchet's entries that timed out once more, each alone in a
/// fresh worker with four times the limit, so that a slow machine is not
/// taken for a regression; the record keeps what that run says.
fn retry_timeouts(
    o: &Options,
    launch: &Launch,
    model: &Model,
    file: &Path,
    records: &mut [Record],
    keep: &dyn Fn(Kind, &str) -> bool,
) {
    for record in records.iter_mut() {
        if record.outcome.verdict != Verdict::Timeout
            || !keep(record.action.kind, &record.action.entity)
        {
            continue;
        }
        let log = format!("{}-retry.log", model.name);
        let Ok((mut worker, _)) = open(launch, &log, file, o.load_timeout) else {
            continue;
        };
        let limit = o.timeout * 4;
        let start = Instant::now();
        let mut call = |id: &str, params: &Value| worker.call(id, params, limit);
        let outcome = plan::run_action(&mut call, &record.action);
        eprintln!(
            "actions: {}: {} on {} timed out; alone with {} s: {}",
            model.name,
            record.action.kind.as_str(),
            record.action.entity,
            limit.as_secs(),
            outcome.verdict.as_str()
        );
        record.outcome = outcome;
        record.seconds = start.elapsed().as_secs_f64();
        record.limit = limit.as_secs_f64();
        record.retried = true;
        worker.stop();
    }
}

/// What a program prints, trimmed; "unknown" when it fails.
fn shell(dir: &Path, program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(
            || "unknown".to_string(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
}

fn short_hash(text: &str) -> String {
    format!("{:08x}", plan::fnv(text) & 0xffff_ffff)
}

/// Writes the reproducer of a record that is wrong, a crash or a
/// timeout; returns its path.
fn reproducer(
    out: &Path,
    model: &str,
    file: &Path,
    steps: Option<&Path>,
    record: &Record,
) -> Option<PathBuf> {
    let a = &record.action;
    let dir = out.join(record.outcome.verdict.as_str());
    std::fs::create_dir_all(&dir).ok()?;
    let stem = format!("{model}-{}-{}", a.kind.as_str(), short_hash(&a.entity));
    let action = json!([a.command, a.params]);
    let check = json!(["document.check", {"deep": true}]);
    let path = dir.join(format!("{stem}.steps.json"));
    let sh = dir.join(format!("{stem}.sh"));
    let (content, line) = match steps.and_then(|s| std::fs::read_to_string(s).ok()) {
        // A model built from steps: the whole of it, then the action.
        Some(text) => {
            let mut list: Vec<Value> = serde_json::from_str::<Value>(&text)
                .ok()?
                .as_array()?
                .clone();
            list.push(action);
            list.push(check);
            (
                Value::Array(list),
                format!("craft3d-cli run --script {}", path.display()),
            )
        }
        None => (
            Value::Array(vec![action, check]),
            format!(
                "craft3d-cli run {} --script {}",
                file.display(),
                path.display()
            ),
        ),
    };
    let text = serde_json::to_string_pretty(&content).ok()?;
    std::fs::write(&path, text).ok()?;
    let note = format!(
        "#!/bin/sh\n# {} {} on {} ({}): {}\n{line}\n",
        model,
        a.kind.as_str(),
        a.entity,
        record.outcome.verdict.as_str(),
        record.outcome.reason.replace('\n', " ")
    );
    std::fs::write(&sh, note).ok()?;
    Some(path)
}

fn read_ratchet(repo: &Path) -> Result<Ratchet, String> {
    let path = repo.join(RATCHET);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read the ratchet {}: {e}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| format!("the ratchet {} is not JSON: {e}", path.display()))?;
    plan::ratchet_from(&value).map_err(|e| format!("{}: {e}", path.display()))
}

/// Adds to the ratchet what the last run's results say succeeded (in
/// under a third of the limit), for the models asked for: the same as
/// `--update` at the end of that run. Refused when the results are not of
/// the commit checked out, or when that run had regressions or problems.
fn raise(o: &Options, repo: &Path, out: &Path) -> Result<(), String> {
    let summary_path = out.join("summary.json");
    let summary: Value = std::fs::read_to_string(&summary_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(|| format!("no summary of a run at {}", summary_path.display()))?;
    let head = shell(repo, "git", &["rev-parse", "--short", "HEAD"]);
    let of = summary
        .get("commit")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if of != head || summary.get("dirty").and_then(Value::as_bool) != Some(false) {
        return Err(format!(
            "the last run is of {of}{}, not of {head} as checked out: run again",
            if summary.get("dirty").and_then(Value::as_bool) == Some(false) {
                ""
            } else {
                " with changes not committed"
            }
        ));
    }
    if summary.get("cliGiven").and_then(Value::as_bool) != Some(false) {
        return Err(
            "the last run used a CLI given with --cli, which need not be built from this commit: run again without it"
                .to_string(),
        );
    }
    let count = |key: &str| {
        summary
            .get(key)
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    };
    if count("regressions") > 0 || count("problems") > 0 {
        return Err(format!(
            "the last run had {} regressions and {} problems: nothing is raised over them",
            count("regressions"),
            count("problems")
        ));
    }
    let mut ratchet = read_ratchet(repo)?;
    let mut added = 0_usize;
    for model in &o.models {
        match summary.get("models").and_then(|m| m.get(model.name)) {
            None => {
                println!("{}: not in the last run", model.name);
                continue;
            }
            Some(entry) if entry.get("skipped").is_some() => {
                println!("{}: skipped in the last run", model.name);
                continue;
            }
            Some(_) => {}
        }
        let path = out.join("results").join(format!("{}.jsonl", model.name));
        let Ok(text) = std::fs::read_to_string(&path) else {
            println!("{}: no results ({})", model.name, path.display());
            continue;
        };
        let mut here = 0_usize;
        for line in text.lines() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                return Err(format!("{}: a line that is not JSON", path.display()));
            };
            let field = |key: &str| record.get(key).and_then(Value::as_str);
            if field("commit") != Some(head.as_str()) {
                return Err(format!(
                    "{}: a result of {}, not of {head}: run again",
                    path.display(),
                    field("commit").unwrap_or("no commit")
                ));
            }
            let (Some("ok"), Some(kind), Some(entity)) =
                (field("verdict"), field("kind"), field("entity"))
            else {
                continue;
            };
            let seconds = record.get("seconds").and_then(Value::as_f64);
            let limit = record.get("limit").and_then(Value::as_f64);
            let (Some(seconds), Some(limit)) = (seconds, limit) else {
                continue;
            };
            if seconds >= limit / 3.0 {
                continue;
            }
            let set = ratchet
                .entry(model.name.to_string())
                .or_default()
                .entry(kind.to_string())
                .or_default();
            if set.insert(entity.to_string()) {
                here += 1;
            }
        }
        println!("{}: {here} added", model.name);
        added += here;
    }
    let text =
        serde_json::to_string_pretty(&plan::ratchet_json(&ratchet)).map_err(|e| e.to_string())?;
    std::fs::write(repo.join(RATCHET), format!("{text}\n"))
        .map_err(|e| format!("cannot write the ratchet: {e}"))?;
    println!("ratchet: raised by {added} (log it in the dev log)");
    Ok(())
}

pub fn run(args: &[String]) -> Result<(), String> {
    let o = options(args)?;
    let started = Instant::now();
    let repo = repo_root();
    let main = main_checkout(&repo);
    let out = main.join("target/actions");
    if o.raise {
        return raise(&o, &repo, &out);
    }
    for dir in ["models", "results", "logs"] {
        std::fs::create_dir_all(out.join(dir))
            .map_err(|e| format!("cannot make {}: {e}", out.join(dir).display()))?;
    }
    for verdict in [Verdict::Wrong, Verdict::Crash, Verdict::Timeout] {
        let _ = std::fs::remove_dir_all(out.join(verdict.as_str()));
    }
    // What the run is of, read before anything is built: compared again at
    // the end, so that a commit or an edit meanwhile is not taken for it.
    let stamp = Stamp::now(&repo, o.cli.is_some());
    let ratchet = read_ratchet(&repo)?;
    for model in ratchet.keys() {
        if !MODELS.iter().any(|m| m.name == model) {
            return Err(format!(
                "the ratchet has a model `{model}` that is not one of the models\n{}",
                usage()
            ));
        }
    }
    let cli = match &o.cli {
        Some(cli) => cli.clone(),
        None => build_cli(&repo)?,
    };
    let cap = memory_cap(&o.memory);
    if cap.is_none() {
        eprintln!("actions: systemd-run cannot cap memory here; the servers run without a cap");
    }
    let launch = Launch {
        cli,
        read_root: main.clone(),
        cap,
        logs: out.join("logs"),
    };
    let mut reports: BTreeMap<&str, ModelReport> = BTreeMap::new();
    let mut sources: BTreeMap<&str, (PathBuf, Option<PathBuf>)> = BTreeMap::new();
    for model in &o.models {
        let model_start = Instant::now();
        let mut report = ModelReport::default();
        // Results of an earlier run must not outlive this one.
        let _ = std::fs::remove_file(out.join("results").join(format!("{}.jsonl", model.name)));
        eprintln!("actions: {}: preparing", model.name);
        let prepared = prepare(model, &repo, &main, &out, &launch, o.load_timeout);
        let (file, steps) = match prepared {
            Ok(p) => p,
            Err(skip) => {
                eprintln!("actions: {}: skipped: {}", model.name, skip.why);
                report.skipped = Some(skip);
                reports.insert(model.name, report);
                continue;
            }
        };
        let load = Instant::now();
        let opened = open(
            &launch,
            &format!("{}-0.log", model.name),
            &file,
            o.load_timeout,
        );
        let (mut worker, inspect) = match opened {
            Ok(w) => w,
            Err(e) => {
                eprintln!("actions: {}: skipped: {e}", model.name);
                report.skipped = Some(Skip {
                    why: e,
                    missing: false,
                });
                reports.insert(model.name, report);
                continue;
            }
        };
        let unit = plan::length_unit(&inspect);
        let bodies = plan::bodies(&inspect);
        let mut actions = Vec::new();
        if bodies.len()
            != inspect
                .get("bodies")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        {
            report
                .problems
                .push("a body of document.inspect could not be read".to_string());
        }
        report.sheets = bodies.iter().filter(|b| !b.solid).count();
        let mut edges_of: BTreeMap<String, Vec<plan::Edge>> = BTreeMap::new();
        for body in bodies.iter().filter(|b| b.solid) {
            let edges = match worker.call(
                "document.edges",
                &json!({"feature": body.feature}),
                o.load_timeout,
            ) {
                Ok(answer) => {
                    if answer.get("truncated").and_then(Value::as_bool) == Some(true) {
                        report.truncated += 1;
                    }
                    plan::edges(&answer)
                }
                Err(e) => {
                    report.problems.push(format!(
                        "no edges of {}: {}",
                        body.feature,
                        call_text(&e)
                    ));
                    Vec::new()
                }
            };
            report.faces += body.faces.len();
            actions.extend(plan::plan(body, &edges, &unit, KINDS));
            edges_of.insert(body.feature.clone(), edges);
        }
        report.load_seconds = load.elapsed().as_secs_f64();
        report.bodies = bodies.len();
        let kept = ratchet.get(model.name);
        let keep = |kind: Kind, entity: &str| {
            kept.and_then(|k| k.get(kind.as_str()))
                .is_some_and(|s| s.contains(entity))
        };
        let k = if model.every { None } else { o.per_kind };
        let mut chosen = plan::choose(actions, k, &keep);
        report.planned = chosen.len();
        let mut on_first = |id: &str, params: &Value| worker.call(id, params, o.timeout);
        match plan::orient_all(&mut on_first, &mut chosen, &edges_of) {
            Ok(n) => report.exact = n,
            Err(e) => report
                .problems
                .push(format!("cannot work the exact blends out: {e}")),
        }
        // How far each body's volume can be off, for the noise of the
        // identities (`plan::noise`).
        plan::bound_all(&mut on_first, &mut chosen);
        eprintln!(
            "actions: {}: {} bodies, {} faces, {} actions (loaded in {:.0} s)",
            model.name, report.bodies, report.faces, report.planned, report.load_seconds
        );
        let mut problems = std::mem::take(&mut report.problems);
        let mut records = run_model(&o, &launch, model, &file, worker, chosen, &mut problems);
        retry_timeouts(&o, &launch, model, &file, &mut records, &keep);
        report.records = records;
        report.problems = problems;
        for p in &report.problems {
            eprintln!("actions: {}: {p}", model.name);
        }
        report.seconds = model_start.elapsed().as_secs_f64();
        let mut lines = String::new();
        for r in &report.records {
            let line = json!({
                "kind": r.action.kind.as_str(), "body": r.action.body, "entity": r.action.entity,
                "verdict": r.outcome.verdict.as_str(), "reason": r.outcome.reason,
                "class": plan::reason_class(&r.outcome.reason), "seconds": r.seconds,
                "limit": r.limit, "retried": r.retried, "commit": stamp.commit,
                "dirty": stamp.dirty, "cliGiven": stamp.cli_given,
                "v0": r.action.volume, "a0": r.action.area,
                "v1": r.outcome.after.map(|a| a.0), "a1": r.outcome.after.map(|a| a.1),
                "command": r.action.command, "params": r.action.params,
            });
            let _ = writeln!(lines, "{line}");
        }
        let _ = std::fs::write(
            out.join("results").join(format!("{}.jsonl", model.name)),
            lines,
        );
        sources.insert(model.name, (file, steps));
        reports.insert(model.name, report);
    }
    finish(
        &o,
        &repo,
        &out,
        &ratchet,
        &reports,
        &sources,
        &stamp,
        started.elapsed().as_secs_f64(),
    )
}

/// What a run is of: the commit, whether the tree had changes, and
/// whether the CLI was given (`--cli`, so possibly not built from it).
struct Stamp {
    commit: String,
    dirty: bool,
    cli_given: bool,
}

impl Stamp {
    fn now(repo: &Path, cli_given: bool) -> Stamp {
        Stamp {
            commit: shell(repo, "git", &["rev-parse", "--short", "HEAD"]),
            dirty: !shell(
                repo,
                "git",
                &["status", "--porcelain", "--untracked-files=no"],
            )
            .is_empty(),
            cli_given,
        }
    }
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn finish(
    o: &Options,
    repo: &Path,
    out: &Path,
    ratchet: &Ratchet,
    reports: &BTreeMap<&str, ModelReport>,
    sources: &BTreeMap<&str, (PathBuf, Option<PathBuf>)>,
    stamp: &Stamp,
    seconds: f64,
) -> Result<(), String> {
    let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
    let mut refusals: BTreeMap<String, usize> = BTreeMap::new();
    let mut bad: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    let mut regressions = Vec::new();
    let mut problems: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut not_tried_total = 0_usize;
    let mut newly: Ratchet = Ratchet::new();
    let mut models_json = serde_json::Map::new();
    let mut table = String::new();
    let _ = writeln!(
        table,
        "| model | action | ok | refused | wrong | crash | timeout |"
    );
    let _ = writeln!(table, "|---|---|---:|---:|---:|---:|---:|");
    for (name, report) in reports {
        if let Some(skip) = &report.skipped {
            let why = &skip.why;
            let floor: usize = ratchet
                .get(*name)
                .map_or(0, |k| k.values().map(BTreeSet::len).sum());
            models_json.insert(
                (*name).to_string(),
                json!({"skipped": why, "missing": skip.missing, "ratchetNotTried": floor}),
            );
            let _ = writeln!(table, "| {name} | skipped: {why} | | | | | |");
            skipped.push(json!({"model": name, "why": why, "missing": skip.missing}));
            // A model of the ratchet that could not be run hides whatever
            // regressed in it, unless its input is simply not here.
            if !skip.missing && floor > 0 {
                problems.push(json!({"model": name, "problem": format!("skipped with {floor} entries of the ratchet: {why}")}));
            }
            continue;
        }
        for p in &report.problems {
            problems.push(json!({"model": name, "problem": p}));
        }
        let mut kinds_json = serde_json::Map::new();
        let mut per_kind: BTreeMap<Kind, BTreeMap<Verdict, usize>> = BTreeMap::new();
        let mut ok_now: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut tried: BTreeMap<&str, BTreeMap<&str, &Record>> = BTreeMap::new();
        for r in &report.records {
            let kind = r.action.kind.as_str();
            *per_kind
                .entry(r.action.kind)
                .or_default()
                .entry(r.outcome.verdict)
                .or_default() += 1;
            *totals.entry(r.outcome.verdict.as_str()).or_default() += 1;
            tried.entry(kind).or_default().insert(&r.action.entity, r);
            match r.outcome.verdict {
                Verdict::Ok => {
                    if r.ratchets() {
                        ok_now.entry(kind).or_default().insert(&r.action.entity);
                    }
                }
                Verdict::Refused => {
                    *refusals
                        .entry(format!("{kind}: {}", plan::reason_class(&r.outcome.reason)))
                        .or_default() += 1;
                }
                verdict => {
                    let (file, steps) = sources
                        .get(name)
                        .map_or((Path::new(""), None), |(f, s)| (f.as_path(), s.as_deref()));
                    let repro = reproducer(out, name, file, steps, r);
                    let repro = repro.map(|p| p.display().to_string());
                    // A wrong result or a crash is a bug found: the run
                    // fails on it (a timeout is reported, not failed on).
                    if matches!(verdict, Verdict::Wrong | Verdict::Crash) {
                        problems.push(json!({
                            "model": name,
                            "problem": format!("{} {kind} on {}: {}", verdict.as_str(), r.action.entity, r.outcome.reason),
                            "reproducer": repro,
                        }));
                    }
                    bad.entry(verdict.as_str()).or_default().push(json!({
                        "model": name, "kind": kind, "entity": r.action.entity, "reason": r.outcome.reason,
                        "reproducer": repro,
                    }));
                }
            }
            if r.outcome.unrestored && matches!(r.outcome.verdict, Verdict::Ok | Verdict::Refused) {
                problems.push(json!({
                    "model": name,
                    "problem": format!("{kind} on {}: the undo did not give the document back as it was", r.action.entity),
                }));
            }
        }
        for kind in KINDS {
            let Some(counts) = per_kind.get(kind) else {
                continue;
            };
            let row: Vec<usize> = Verdict::ALL
                .iter()
                .map(|v| counts.get(v).copied().unwrap_or(0))
                .collect();
            kinds_json.insert(
                kind.as_str().to_string(),
                json!(
                    Verdict::ALL
                        .iter()
                        .map(|v| (v.as_str(), counts.get(v).copied().unwrap_or(0)))
                        .collect::<BTreeMap<_, _>>()
                ),
            );
            let cells: Vec<String> = row.iter().map(ToString::to_string).collect();
            let _ = writeln!(
                table,
                "| {name} | {} | {} |",
                kind.as_str(),
                cells.join(" | ")
            );
        }
        // The ratchet: what succeeded before must succeed now.
        let floor = ratchet.get(*name);
        let mut model_regressions = 0_usize;
        let mut not_tried = 0_usize;
        if let Some(floor) = floor {
            for (kind, names) in floor {
                for entity in names {
                    match tried
                        .get(kind.as_str())
                        .and_then(|t| t.get(entity.as_str()))
                    {
                        Some(r) if r.outcome.verdict == Verdict::Ok => {}
                        Some(r) => {
                            model_regressions += 1;
                            regressions.push(json!({
                                "model": name, "kind": kind, "entity": entity,
                                "verdict": r.outcome.verdict.as_str(), "reason": r.outcome.reason,
                            }));
                        }
                        None => not_tried += 1,
                    }
                }
            }
        }
        if not_tried > 0 {
            problems.push(json!({"model": name, "problem": format!("{not_tried} entries of the ratchet were not tried (no longer there, or not planned)")}));
        }
        not_tried_total += not_tried;
        let mut model_new = 0_usize;
        for (kind, names) in &ok_now {
            for entity in names {
                let known = floor
                    .and_then(|f| f.get(*kind))
                    .is_some_and(|s| s.contains(*entity));
                if !known {
                    model_new += 1;
                    newly
                        .entry((*name).to_string())
                        .or_default()
                        .entry((*kind).to_string())
                        .or_default()
                        .insert((*entity).to_string());
                }
            }
        }
        models_json.insert((*name).to_string(), json!({
            "seconds": report.seconds, "loadSeconds": report.load_seconds, "bodies": report.bodies,
            "faces": report.faces, "actions": report.records.len(), "planned": report.planned,
            "kinds": kinds_json, "regressions": model_regressions, "newlyOk": model_new,
            "ratchetNotTried": not_tried, "sheets": report.sheets,
            "truncatedEdges": report.truncated, "exactBlends": report.exact,
            "problems": report.problems,
        }));
    }
    let end = Stamp::now(repo, stamp.cli_given);
    if end.commit != stamp.commit || end.dirty != stamp.dirty {
        problems.push(json!({"problem": format!(
            "the checkout changed during the run: {}{} at the start, {}{} at the end",
            stamp.commit, if stamp.dirty { " with changes" } else { "" },
            end.commit, if end.dirty { " with changes" } else { "" },
        )}));
    }
    let mut top: Vec<(String, usize)> = refusals.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top.truncate(25);
    let newly_count: usize = newly
        .values()
        .flat_map(BTreeMap::values)
        .map(BTreeSet::len)
        .sum();
    let summary = json!({
        "generated": shell(repo, "date", &["+%Y-%m-%d %H:%M"]),
        "commit": stamp.commit,
        "dirty": stamp.dirty,
        "cliGiven": stamp.cli_given,
        "seconds": seconds,
        "limitSeconds": o.timeout.as_secs(),
        "perKind": o.per_kind,
        "totals": totals,
        "models": models_json,
        "topRefusals": top.iter().map(|(c, n)| json!([c, n])).collect::<Vec<_>>(),
        "wrong": bad.get("wrong").cloned().unwrap_or_default(),
        "crash": bad.get("crash").cloned().unwrap_or_default(),
        "timeout": bad.get("timeout").cloned().unwrap_or_default(),
        "regressions": regressions,
        "problems": problems,
        "skipped": skipped,
        "ratchetNotTried": not_tried_total,
        "newlyOk": newly_count,
    });
    let text = serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?;
    std::fs::write(out.join("summary.json"), text)
        .map_err(|e| format!("cannot write summary.json: {e}"))?;
    println!("{table}");
    println!("totals: {totals:?} in {:.0} s", seconds);
    println!("top refusals:");
    for (class, n) in top.iter().take(15) {
        println!("  {n:5}  {class}");
    }
    for verdict in ["wrong", "crash", "timeout"] {
        for item in bad.get(verdict).map(Vec::as_slice).unwrap_or_default() {
            println!("{verdict}: {item}");
        }
    }
    println!("summary: {}", out.join("summary.json").display());
    for p in &problems {
        println!("problem: {p}");
    }
    if o.update && newly_count > 0 && regressions.is_empty() && problems.is_empty() {
        let mut raised = ratchet.clone();
        for (model, kinds) in newly {
            for (kind, names) in kinds {
                raised
                    .entry(model.clone())
                    .or_default()
                    .entry(kind)
                    .or_default()
                    .extend(names);
            }
        }
        let text = serde_json::to_string_pretty(&plan::ratchet_json(&raised))
            .map_err(|e| e.to_string())?;
        std::fs::write(repo.join(RATCHET), format!("{text}\n"))
            .map_err(|e| format!("cannot write the ratchet: {e}"))?;
        println!("ratchet: raised by {newly_count} (log it in the dev log)");
    } else if newly_count > 0 {
        println!("ratchet: {newly_count} newly succeed; --update raises the floor");
    }
    for r in &regressions {
        println!("regression: {r}");
    }
    if regressions.is_empty() && problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} actions that succeeded before do not now, and {} problems kept actions from running (ratchet entries not tried: {not_tried_total})",
            regressions.len(),
            problems.len()
        ))
    }
}
