//! What building a document command by command costs, as an agent or
//! `craft3d-cli run --script` builds it: per command, the time, what the
//! regeneration cache rebuilt and reused, and the entries of what features
//! are made from it looked at (`Session::last_regeneration_work`, which
//! grows with the length of the history when a feature is added, not with
//! its square). Run in release:
//!
//! ```sh
//! cargo run --release -p craft3d-engine --example regen_rows -- holes 1000
//! cargo run --release -p craft3d-engine --example regen_rows -- \
//!     steps all.steps.json --set mixFingers=14
//! ```
//!
//! `holes N` cuts N holes one after another in a plate (a sketch, a circle
//! and an extrusion that cuts per hole, each from the body the one before
//! left), then changes the radius of the last hole and the length of the
//! plate. `steps FILE` runs a steps file (`[["id", {params}], ...]`) in a
//! new document in millimetres; each `--set NAME=EXPR` afterwards sets a
//! variable and reports what was rebuilt.

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Default)]
struct Tally {
    commands: usize,
    seconds: f64,
    worst: f64,
}

struct Run {
    session: Session,
    by_command: BTreeMap<String, Tally>,
    started: Instant,
    most_work: usize,
}

impl Run {
    fn new() -> Self {
        Self {
            session: Session::new(Workspace::unrestricted()),
            by_command: BTreeMap::new(),
            started: Instant::now(),
            most_work: 0,
        }
    }

    fn step(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let t = Instant::now();
        let result = self
            .session
            .execute(id, params)
            .map_err(|e| format!("{id} {params}: {e}"))?;
        let spent = t.elapsed().as_secs_f64();
        let tally = self.by_command.entry(id.to_string()).or_default();
        tally.commands += 1;
        tally.seconds += spent;
        tally.worst = tally.worst.max(spent);
        self.most_work = self
            .most_work
            .max(self.session.last_regeneration_work().unwrap_or(0));
        Ok(result)
    }

    fn report(&self, label: &str) {
        println!(
            "{label}: {:.2} s, at most {} entries of what features are made from looked at in one regeneration",
            self.started.elapsed().as_secs_f64(),
            self.most_work
        );
        for (id, tally) in &self.by_command {
            println!(
                "  {id:<20} {:>6} x  {:>9.2} s  {:>8.2} ms each  {:>8.1} ms at most",
                tally.commands,
                tally.seconds,
                tally.seconds * 1e3 / tally.commands.max(1) as f64,
                tally.worst * 1e3
            );
        }
    }

    /// One more command, timed on its own, with what it rebuilt.
    fn single(&mut self, id: &str, params: &Value) -> Result<(), String> {
        let t = Instant::now();
        self.step(id, params)?;
        let (rebuilt, reused) = self.session.last_regeneration().unwrap_or_default();
        println!(
            "{id} {params}: {:.1} ms, {rebuilt} features rebuilt, {reused} reused, {} entries looked at",
            t.elapsed().as_secs_f64() * 1e3,
            self.session.last_regeneration_work().unwrap_or(0)
        );
        Ok(())
    }
}

fn holes(count: usize) -> Result<(), String> {
    let mut run = Run::new();
    run.step("file.new", &json!({"units": "mm"}))?;
    run.step("variable.set", &json!({"name": "r", "expr": "2 mm"}))?;
    let columns = count.div_ceil(2).max(5);
    let length = 10.0 * columns as f64;
    run.step(
        "feature.block",
        &json!({"size": [length, 40, 5], "name": "Plate"}),
    )?;
    let mut last = "Plate".to_string();
    for hole in 0..count {
        let sketch = format!("S{hole}");
        run.step("sketch.create", &json!({ "name": sketch }))?;
        let (x, y) = (
            5.0 + 10.0 * (hole / 2) as f64,
            10.0 + 20.0 * (hole % 2) as f64,
        );
        run.step(
            "sketch.circle",
            &json!({"sketch": sketch, "center": [x, y], "radius": 2}),
        )?;
        // The last hole's diameter is twice the variable.
        if hole + 1 == count {
            run.step(
                "sketch.constrain",
                &json!({"sketch": sketch, "type": "diameter", "entities": ["e2"], "value": "2 * r"}),
            )?;
        }
        let name = format!("H{hole}");
        run.step(
            "feature.extrude",
            &json!({"sketch": sketch, "distance": 15, "symmetric": true, "cut": last, "name": name}),
        )?;
        last = name;
    }
    run.report(&format!("{count} holes built command by command"));
    run.single("variable.set", &json!({"name": "r", "expr": "1.5 mm"}))?;
    run.single(
        "feature.edit",
        &json!({"feature": "Plate", "size": [length + 10.0, 40, 5]}),
    )?;
    Ok(())
}

fn steps(path: &str, sets: &[String]) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let steps: Vec<(String, Value)> =
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let mut run = Run::new();
    run.step("file.new", &json!({"units": "mm"}))?;
    for (id, params) in &steps {
        run.step(id, params)?;
    }
    run.report(&format!("{path}: {} steps", steps.len()));
    for set in sets {
        let (name, expr) = set
            .split_once('=')
            .ok_or_else(|| format!("--set {set}: expected NAME=EXPR"))?;
        run.single("variable.set", &json!({"name": name, "expr": expr}))?;
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: regen_rows holes N | regen_rows steps FILE [--set NAME=EXPR]...";
    match args.first().map(String::as_str) {
        Some("holes") => {
            let count = args
                .get(1)
                .and_then(|n| n.parse().ok())
                .ok_or(usage.to_string())?;
            holes(count)
        }
        Some("steps") => {
            let path = args.get(1).ok_or(usage.to_string())?;
            let mut sets = Vec::new();
            let mut rest = args.iter().skip(2);
            while let Some(flag) = rest.next() {
                match (flag.as_str(), rest.next()) {
                    ("--set", Some(set)) => sets.push(set.clone()),
                    _ => return Err(usage.to_string()),
                }
            }
            steps(path, &sets)
        }
        _ => Err(usage.to_string()),
    }
}
