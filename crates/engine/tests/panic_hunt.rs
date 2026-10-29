//! Rule 13 of AGENTS.md: no command may panic, whatever its parameters and
//! whatever the document. Every command of the registry is run with
//! adversarial values for each of its parameters, alone and combined, with
//! and without a document. A panic inside a command is caught by
//! `Session::execute` and reported as "internal error", which fails here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, ManualClock, RecoveryOptions, Session, Workspace};
use craft3d_testkit::parallel::map_parallel;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// What each thread of the hunt is running now, by thread: read by
/// [`watch_memory`] when the process reaches a new peak.
static IN_FLIGHT: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

/// Commands whose valid cases draw pictures or rebuild the document many
/// times (a replay renders a frame per step: about 160 MB each). At most
/// [`HEAVY_AT_ONCE`] of them run at a time, whatever the number of cores:
/// on 32 threads the replays running together made the hunt's peak
/// 5.2 GB (2026-10-23); the other cases still run on every core, and
/// every case still runs.
const HEAVY: &[&str] = &[
    "view.replay",
    "view.render",
    "file.export",
    "document.sample",
];

/// How many [`HEAVY`] commands run at once.
const HEAVY_AT_ONCE: usize = 2;

/// How many [`HEAVY`] commands are running, and the wait for a turn.
static HEAVY_RUNNING: Mutex<usize> = Mutex::new(0);
static HEAVY_TURN: std::sync::Condvar = std::sync::Condvar::new();

/// A turn to run a [`HEAVY`] command, given back when dropped (also when
/// the case panics).
struct HeavyTurn;

impl HeavyTurn {
    fn take() -> Self {
        let mut running = HEAVY_RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *running >= HEAVY_AT_ONCE {
            running = HEAVY_TURN
                .wait(running)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *running += 1;
        HeavyTurn
    }
}

impl Drop for HeavyTurn {
    fn drop(&mut self) {
        let mut running = HEAVY_RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *running = running.saturating_sub(1);
        HEAVY_TURN.notify_one();
    }
}

/// `light` with `heavy` spread evenly among them, so that the threads
/// taking cases in order do not all wait for the [`HEAVY_AT_ONCE`] slots
/// at the same time.
fn interleave<T>(light: Vec<T>, heavy: Vec<T>) -> Vec<T> {
    let total = light.len() + heavy.len();
    let mut out = Vec::with_capacity(total);
    let mut light = light.into_iter();
    let mut heavy = heavy.into_iter().peekable();
    let every = total.checked_div(heavy.len().max(1)).unwrap_or(1).max(1);
    for i in 0..total {
        let next = if i % every == every - 1 && heavy.peek().is_some() {
            heavy.next()
        } else {
            light.next().or_else(|| heavy.next())
        };
        out.extend(next);
    }
    out
}

/// Sets its flag when dropped, also when the work it guards panics: the
/// memory watcher stops, and the scope that waits for it ends.
struct SetOnDrop<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for SetOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

/// [`map_parallel`] with [`watch_memory`] beside it: the results, and the
/// peak resident memory with what ran then. A panic in the work stops the
/// watcher and is raised again here, never a hang.
fn watched<T: Send, R: Send>(
    items: Vec<T>,
    work: impl Fn(T) -> R + Sync,
) -> (Vec<R>, (u64, Vec<String>)) {
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        let watcher = scope.spawn(|| watch_memory(&done));
        let stop = SetOnDrop(&done);
        let results = map_parallel(items, work);
        drop(stop);
        (results, watcher.join().unwrap())
    })
}

/// What one thread runs, taken off [`IN_FLIGHT`] when the case ends.
struct InFlight(String);

impl InFlight {
    fn start(what: String) -> Self {
        let me = format!("{:?}", std::thread::current().id());
        IN_FLIGHT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(me.clone(), what);
        InFlight(me)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}

/// The process's resident memory in kB (Linux; `None` elsewhere).
fn resident_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Samples the resident memory every 10 ms until `done`, and returns the
/// peak with what the threads were running then: where the hunt's memory
/// goes, printed with `--nocapture`.
fn watch_memory(done: &std::sync::atomic::AtomicBool) -> (u64, Vec<String>) {
    let mut peak = 0;
    let mut running = Vec::new();
    while !done.load(std::sync::atomic::Ordering::Acquire) {
        if let Some(kb) = resident_kb()
            && kb > peak
        {
            peak = kb;
            running = IN_FLIGHT
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .cloned()
                .collect();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    (peak, running)
}

fn adversarial() -> Vec<Value> {
    let mut deep = json!(1);
    for _ in 0..500 {
        deep = json!([deep]);
    }
    vec![
        Value::Null,
        json!(true),
        json!(false),
        json!(0),
        json!(-1),
        json!(1),
        json!(0.5),
        json!(-0.0),
        json!(1e308),
        json!(-1e308),
        json!(5e-324),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(""),
        json!(" "),
        json!("a"),
        json!("Plate"),
        json!("F1"),
        json!("F0"),
        json!("Profile"),
        json!("e1"),
        json!("e7"),
        json!("e0"),
        json!("c1"),
        json!("c18446744073709551615"),
        json!("F7:side(e5)"),
        json!("F7:endCap(e5,e6,e7,e8)"),
        json!("F8:side(e10)"),
        json!("F1:side(e1)"),
        json!("F0:lateralEdge()"),
        json!("F8:side(e18446744073709551615)"),
        json!("F8:side(e10[1])"),
        json!("F8:endCap(e5,e10[*])"),
        json!("F8:side(e10[65534])"),
        json!("F8:between[F8:side(e10[1]),F8:side(e5)]"),
        json!("e5[99]"),
        json!("e5[-1]"),
        json!("e5[last"),
        json!("e5[1,2,3]"),
        json!("e5[last]"),
        json!("e1[1]"),
        json!("e2[4294967295]"),
        json!("[last]"),
        json!("p1"),
        json!("p0"),
        json!(["e1", "e2[last]"]),
        json!("F8:side(".to_string() + &"e1,".repeat(5_000) + "e1)"),
        json!("distance"),
        json!("tangent"),
        json!("zx"),
        json!(["e1", "e2"]),
        json!(["e7", "e8"]),
        json!(["e7"]),
        json!(["e1", "e1"]),
        json!([[0, 0], [1, 0], [1, 1]]),
        json!([[0, 0], [0, 0], [0, 0]]),
        json!([[1e308, -1e308], "e1", [0, 5e-324]]),
        json!("F18446744073709551615"),
        json!("F99999999999999999999999"),
        json!("width"),
        json!("width / 0"),
        json!("1 mm"),
        json!("0 mm"),
        json!("-1 mm"),
        json!("1e999"),
        json!("1e308 m * 1e308 m"),
        json!("floor(width, 0 mm)"),
        json!("mod(1e300, 1e-300)"),
        json!("round(1e400)"),
        json!("floor(4503599627370495.5)"),
        json!("mod(-0, -1e-300)"),
        json!("30 deg"),
        json!("nope + 1"),
        json!("sin"),
        json!("mm"),
        json!("💥"),
        json!("a\0b"),
        json!("a\nb"),
        json!("../../etc/passwd"),
        json!("/etc/passwd"),
        json!("x.stl"),
        json!("x.3dcraft"),
        json!("."),
        json!("stl"),
        json!("ids"),
        json!("(".repeat(10_000)),
        json!("9".repeat(10_000)),
        json!("ab ".repeat(5_000)),
        json!([]),
        json!([1]),
        json!([1, 2, 3]),
        json!([0, 0, 0]),
        json!([-1, -1, -1]),
        json!([1e308, 1e308, 1e308]),
        json!(["a", "b", "c"]),
        json!(["width", "width / 0", "1 +"]),
        json!([null, null, null]),
        json!([[], [], []]),
        json!([1, 2, 3, 4]),
        json!({}),
        json!({"a": 1}),
        deep,
    ]
}

/// The parameter names a command documents: the quoted words before a
/// colon in its params string.
fn keys(params: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = params;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else { break };
        let (word, tail) = (&after[..end], &after[end + 1..]);
        if tail.trim_start().starts_with(':') {
            keys.push(word.to_string());
        }
        rest = tail;
    }
    keys
}

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("panic-hunt-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A session in one of the states commands must survive.
fn workspace(dir: &std::path::Path) -> Workspace {
    Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    }
}

/// The id of the document in [`seed_recovery`]'s directory.
const SEED_ID: &str = "0123456789abcdef-1";

/// A recovery directory as a launch that died left it: one document with
/// its record, and no lock held. Built once; [`with_recovery`] copies it.
fn seed_recovery(dir: &std::path::Path) -> PathBuf {
    let seed = dir.join("recovery-seed");
    std::fs::create_dir_all(&seed).unwrap();
    let mut s = Session::new(Workspace::none());
    s.execute("file.new", &json!({})).unwrap();
    s.execute("feature.block", &json!({"size": [1, 2, 3]}))
        .unwrap();
    s.set_workspace(workspace(&seed));
    s.execute("file.save", &json!({"path": format!("{SEED_ID}.3dcraft")}))
        .unwrap();
    std::fs::write(
        seed.join(format!("{SEED_ID}.json")),
        br#"{"recovery_version": 1, "name": "seed.3dcraft", "path": "seed.3dcraft", "revision": 1, "time": 1800000000, "pid": 1}"#,
    )
    .unwrap();
    seed
}

/// Gives the session a recovery directory of its own beneath `dir`,
/// holding what `seed` holds when the command is one that looks there.
/// The clock stands still and there is no thread: nothing is autosaved
/// but what a command asks for.
fn with_recovery(
    mut s: Session,
    dir: &std::path::Path,
    seed: &std::path::Path,
    id: &str,
) -> Session {
    let recovery = dir.join("recovery");
    if id.starts_with("recovery.") {
        std::fs::create_dir_all(&recovery).unwrap();
        for entry in std::fs::read_dir(seed).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), recovery.join(entry.file_name())).unwrap();
        }
    }
    s.enable_recovery(RecoveryOptions::manual(
        recovery,
        std::sync::Arc::new(ManualClock::new(1_800_000_000)),
    ));
    s
}

fn session(dir: &std::path::Path, state: usize) -> Session {
    let mut s = Session::new(workspace(dir));
    let mut run = |id: &str, params: Value| {
        s.execute(id, &params)
            .unwrap_or_else(|e| panic!("setup {id}: {e}"));
    };
    match state {
        // No document at all.
        0 => {}
        // An empty document.
        1 => run("file.new", json!({})),
        // A healthy part.
        2 => {
            run("file.new", json!({}));
            run("variable.set", json!({"name": "width", "expr": "40 mm"}));
            run(
                "feature.block",
                json!({"name": "Plate", "size": ["width", 20, 5]}),
            );
            run("feature.cylinder", json!({"radius": 2, "height": 10}));
            run("feature.sphere", json!({"radius": "width / 4"}));
            run(
                "feature.cone",
                json!({"radius": 3, "height": 9, "axis": [1, 1, 0]}),
            );
            run(
                "feature.torus",
                json!({"major": 10, "minor": 2, "angle": "30 deg"}),
            );
            // A constrained rectangle with a circle in it.
            run("sketch.create", json!({"name": "Profile"}));
            run(
                "sketch.polyline",
                json!({"sketch": "Profile", "closed": true, "points": [[0, 0], [9, 1], [10, 6], [1, 5]]}),
            );
            run(
                "sketch.circle",
                json!({"sketch": "Profile", "center": [5, 3], "radius": 1}),
            );
            for (kind, entities) in [
                ("fixed", json!(["e1"])),
                ("horizontal", json!(["e5"])),
                ("vertical", json!(["e6"])),
                ("parallel", json!(["e5", "e7"])),
            ] {
                run(
                    "sketch.constrain",
                    json!({"sketch": "Profile", "type": kind, "entities": entities}),
                );
            }
            run(
                "sketch.constrain",
                json!({"sketch": "Profile", "type": "distance", "entities": ["e5"], "value": "width / 4"}),
            );
            // A plate with a hole, made from it, and a boss on its side.
            run(
                "feature.extrude",
                json!({"sketch": "Profile", "distance": "width / 8", "name": "Body"}),
            );
            run(
                "sketch.create",
                json!({"name": "Seat", "face": "F7:side(e5)", "offset": 1}),
            );
            run(
                "sketch.circle",
                json!({"sketch": "Seat", "center": [3, 2], "radius": 1}),
            );
            run("feature.extrude", json!({"sketch": "Seat", "distance": 2}));
            // A sketch with a row of discs and a point tied to the last.
            run("sketch.create", json!({"name": "Rows"}));
            run(
                "sketch.circle",
                json!({"sketch": "Rows", "center": [0, 0], "radius": 1}),
            );
            run(
                "sketch.pattern",
                json!({"sketch": "Rows", "entities": ["e2"], "count": 3, "step": [3, 0]}),
            );
            run("sketch.point", json!({"sketch": "Rows", "at": [6, 0]}));
            run(
                "sketch.constrain",
                json!({"sketch": "Rows", "type": "coincident", "entities": ["e3", "e1[last]"]}),
            );
        }
        // An assembly (docs/design/assemblies.md): two instances of a
        // packed part, a picked body, a ground, a driven hinge, a mate on a
        // face that is not there, a coupling.
        4 => {
            run("file.new", json!({}));
            run("feature.block", json!({"name": "Bar", "size": [10, 2, 2]}));
            run(
                "feature.block",
                json!({"name": "Peg", "size": [1, 1, 1], "position": [0, 5, 0]}),
            );
            run("file.save", json!({"path": "hunt-part.3dcraft"}));
            run("assembly.new", json!({}));
            run(
                "instance.add",
                json!({"source": "hunt-part.3dcraft", "name": "A", "body": "Bar"}),
            );
            run(
                "instance.add",
                json!({"source": "hunt-part.3dcraft", "name": "B", "body": "Bar"}),
            );
            run(
                "instance.add",
                json!({"source": "hunt-part.3dcraft", "name": "C"}),
            );
            run(
                "mate.add",
                json!({"kind": "fixed", "a": "world/xy", "b": "A/F1:face(0)", "name": "Ground"}),
            );
            run(
                "mate.add",
                json!({"kind": "revolute", "a": "A/F1:face(1)", "b": "B/F1:face(0)", "name": "Hinge"}),
            );
            run(
                "mate.add",
                json!({"kind": "slider", "a": "B/F1:face(1)", "b": "C/F9:face(0)", "name": "Lost"}),
            );
            run("joint.set", json!({"mate": "Hinge", "value": "30 deg"}));
            run(
                "coupling.add",
                json!({"kind": "rack", "mates": ["Hinge", "Lost"], "value": 2}),
            );
            run("assembly.pack", json!({}));
        }
        // A document full of things that fail: bad formulas, a cycle, a
        // suppressed and a rolled-back feature, undo and redo stacks.
        _ => {
            run("file.new", json!({"units": "in", "tolerance": 1e-9}));
            run("variable.set", json!({"name": "a", "expr": "b + 1"}));
            run("variable.set", json!({"name": "b", "expr": "a * 2"}));
            run("variable.set", json!({"name": "broken", "expr": "1 +"}));
            run("variable.set", json!({"name": "zero", "expr": "1 mm / 0"}));
            run(
                "feature.block",
                json!({"name": "Plate", "size": ["a", "broken", "zero"]}),
            );
            run("feature.block", json!({"size": [1e-300, 1e300, 1]}));
            run("feature.sphere", json!({"radius": "30 deg"}));
            run("feature.cylinder", json!({"radius": 1, "height": 1}));
            // A sketch that cannot be solved: a dimension that does not
            // evaluate, two that contradict each other, a degenerate line.
            run(
                "sketch.create",
                json!({"name": "Profile", "offset": "zero"}),
            );
            run(
                "sketch.polyline",
                json!({"sketch": "Profile", "points": [[0, 0], [1, 0], [1, 0], [1e-300, 1e300]]}),
            );
            for value in ["broken", "1 in", "2 in"] {
                run(
                    "sketch.constrain",
                    json!({"sketch": "Profile", "type": "distance", "entities": ["e5"], "value": value}),
                );
            }
            run(
                "sketch.constrain",
                json!({"sketch": "Profile", "type": "angle", "entities": ["e6", "e7"], "value": "a"}),
            );
            // An extrusion of that sketch, and one whose sketch is gone.
            run(
                "feature.extrude",
                json!({"sketch": "Profile", "distance": "zero"}),
            );
            run(
                "sketch.create",
                json!({"name": "Nowhere", "face": "F99:endCap(e1)", "offset": "broken"}),
            );
            // A pattern whose count does not evaluate, and an image point
            // of an instance it does not have.
            run("sketch.create", json!({"name": "Holes"}));
            run(
                "sketch.circle",
                json!({"sketch": "Holes", "center": [0, 0], "radius": 1}),
            );
            run(
                "sketch.pattern",
                json!({"sketch": "Holes", "entities": ["e2"], "count": 4, "step": [3, 0]}),
            );
            run(
                "sketch.constrain",
                json!({"sketch": "Holes", "type": "fixed", "entities": ["e1[3]"]}),
            );
            run(
                "sketch.pattern",
                json!({"sketch": "Holes", "replace": "p1", "count": "broken"}),
            );
            run("sketch.create", json!({"name": "Gone"}));
            run("feature.extrude", json!({"sketch": "Gone", "distance": 1}));
            run("feature.delete", json!({"feature": "Gone"}));
            run("feature.suppress", json!({"feature": "Cylinder1"}));
            run("history.rollback", json!({"feature": "Plate"}));
            run("edit.undo", json!({}));
            run("edit.undo", json!({}));
        }
    }
    s
}

#[test]
fn no_command_panics_on_adversarial_parameters() {
    let dir = workdir("hunt");
    let values = adversarial();
    let commands = Session::commands();
    let templates: Vec<Session> = (0..5).map(|state| session(&dir, state)).collect();
    let seed = seed_recovery(&dir);
    // One case per command, state and adversarial value: the value as the
    // params itself, under each parameter, under all of them at once, and
    // with one parameter left valid in turn. The cases are independent of
    // each other, so they run on every core, each in its own directory for
    // the files the commands write.
    let mut light = Vec::new();
    let mut heavy = Vec::new();
    for (c, spec) in commands.iter().enumerate() {
        for state in 0..5 {
            for v in 0..values.len() {
                if HEAVY.contains(&spec.id) {
                    heavy.push((c, state, v));
                } else {
                    light.push((c, state, v));
                }
            }
        }
    }
    let cases = interleave(light, heavy);
    let (timed, (peak, at_peak)) = watched(cases, |(c, state, v)| {
        let clock = std::time::Instant::now();
        let spec = &commands[c];
        let value = &values[v];
        let keys = keys(spec.params);
        let dir = dir.join(format!("{c}-{state}-{v}"));
        std::fs::create_dir_all(&dir).unwrap();
        // The first adversarial value runs in a session built from scratch
        // (a cold regeneration cache, as after `file.open`); the others in
        // a clone of a session built once per state, whose cache is warm,
        // as in a session that has been working for a while. Both must
        // hold, and the clone saves rebuilding the document a hundred
        // thousand times.
        // Every session autosaves into a recovery directory of its own.
        let fresh = |dir: &std::path::Path| -> Session {
            let s = if v == 0 {
                session(dir, state)
            } else {
                let mut s = templates[state].clone();
                s.set_workspace(workspace(dir));
                s
            };
            with_recovery(s, dir, &seed, spec.id)
        };
        let mut runs = 0_usize;
        let mut check = |params: &Value| {
            runs += 1;
            run_case(fresh(&dir), state, spec.id, params);
        };
        // The params value itself, in every shape.
        check(value);
        // Each parameter in turn, the others absent.
        for key in &keys {
            check(&json!({ key.as_str(): value }));
        }
        // Every parameter at once with the same value, so the checks
        // behind the first failing parameter are reached as well: also
        // with one parameter left valid in turn.
        let all: Map<String, Value> = keys.iter().map(|k| (k.clone(), value.clone())).collect();
        check(&Value::Object(all.clone()));
        for (key, valid) in [
            ("feature", json!("Plate")),
            ("sketch", json!("Profile")),
            ("distance", json!(5)),
            ("face", json!("F7:side(e5)")),
            ("type", json!("distance")),
            ("entities", json!(["e1", "e2"])),
            ("entity", json!("e1")),
            ("constraint", json!("c1")),
            ("name", json!("width")),
            ("path", json!("hunt.stl")),
            ("size", json!([1, 2, 3])),
            ("radius", json!(1)),
            ("major", json!(5)),
            ("axis", json!([0, 0, 1])),
            ("id", json!(SEED_ID)),
            ("all", json!(true)),
        ] {
            if all.contains_key(key) {
                let mut mixed = all.clone();
                mixed.insert(key.to_string(), valid);
                check(&Value::Object(mixed));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        (c, state, runs, clock.elapsed().as_secs_f64())
    });
    let runs: usize = timed.iter().map(|t| t.2).sum();
    // Where the time goes, per command and state: `--nocapture` shows it.
    let mut by_command: BTreeMap<(&str, usize), f64> = BTreeMap::new();
    for (c, state, _, secs) in &timed {
        *by_command.entry((commands[*c].id, *state)).or_default() += secs;
    }
    let mut slowest: Vec<_> = by_command.into_iter().collect();
    slowest.sort_by(|a, b| b.1.total_cmp(&a.1));
    let total: f64 = timed.iter().map(|t| t.3).sum();
    println!(
        "panic_hunt: {runs} command runs over {} commands, {total:.0} s of thread time; the slowest:",
        commands.len()
    );
    for ((id, state), secs) in slowest.iter().take(12) {
        println!("    {secs:7.1} s  {id} in state {state}");
    }
    println!(
        "panic_hunt: peak resident memory {} MB; running then:",
        peak / 1024
    );
    for case in &at_peak {
        println!("    {case}");
    }
    assert!(runs > 20_000);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs one command in a fresh session in the given state: it must not
/// have panicked, and the session must still answer afterwards.
fn run_case(mut s: Session, state: usize, id: &str, params: &Value) {
    let before = (s.document_revision(), s.document_status());
    let shown = || params.to_string().chars().take(200).collect::<String>();
    let _in_flight = InFlight::start(format!("{id} in state {state} on {}", shown()));
    // A heavy command waits for its turn.
    let _heavy = HEAVY.contains(&id).then(HeavyTurn::take);
    let outcome = s.execute(id, params);
    match &outcome {
        Err(e) => {
            assert!(
                !e.message().contains("internal error"),
                "{id} panicked in state {state} on {}: {e}",
                shown()
            );
            assert!(!e.message().is_empty(), "{id}: empty error message");
            // A command that fails leaves the documents as they were
            // (`Session::execute`): nothing changed, so there is nothing
            // to look at again. (Looking anyway, with document.inspect,
            // was 97% of this test's time.)
            assert_eq!(
                (s.document_revision(), s.document_status()),
                before,
                "{id} failed in state {state} on {} but changed the document: {e}",
                shown()
            );
        }
        Ok(_) => {
            // The command did something: the session still answers, and
            // the document still regenerates and reports.
            if s.has_document() {
                let inspected = s.execute("document.inspect", &Value::Null);
                assert!(
                    inspected.is_ok(),
                    "document.inspect broke after {id} in state {state} on {}: {inspected:?}",
                    shown()
                );
            }
        }
    }
}

/// A case that panics fails the hunt with its message instead of
/// hanging it: the memory watcher stops when the work unwinds.
#[test]
fn a_panicking_case_stops_the_watcher() {
    let outcome = std::panic::catch_unwind(|| {
        watched((0..64).collect(), |i: i32| {
            assert!(i != 40, "case {i} panicked");
            i
        })
    });
    let payload = outcome.unwrap_err();
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(message.contains("case 40 panicked"), "{message}");
    // And without a panic the results come back in order.
    let (results, _) = watched((0..64).collect(), |i: i32| i * 2);
    assert_eq!(results, (0..64).map(|i| i * 2).collect::<Vec<_>>());
}

#[test]
fn heavy_cases_are_spread_among_the_light_ones() {
    let spread = interleave(vec!['l'; 9], vec!['h'; 3]);
    assert_eq!(spread.iter().collect::<String>(), "lllhlllhlllh");
    assert_eq!(interleave(vec!['l'; 2], vec!['h'; 4]).len(), 6);
    assert_eq!(interleave(Vec::<char>::new(), vec!['h'; 2]), vec!['h', 'h']);
}

#[test]
fn the_parameter_names_are_read_from_the_registry() {
    assert_eq!(
        keys(r#"{"units": "um|mm" = "mm", "tolerance": number = 1e-6}"#),
        ["units", "tolerance"]
    );
    assert_eq!(keys("{}"), Vec::<String>::new());
    // Every documented parameter is one the command accepts: none is
    // answered with "unknown parameter".
    let dir = workdir("names");
    for spec in Session::commands() {
        for key in keys(spec.params) {
            let mut s = session(&dir, 2);
            if let Err(e) = s.execute(spec.id, &json!({ key.as_str(): Value::Null })) {
                assert!(
                    !e.message().contains("unknown parameter"),
                    "{}: `{key}` is documented but refused: {e}",
                    spec.id
                );
            }
        }
    }
}
