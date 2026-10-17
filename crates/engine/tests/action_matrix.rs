//! The gated part of the action matrix (docs/testing.md, oracle 12): a
//! small model, a few actions of every kind on it, held to the ratchet
//! `action_matrix/ratchet.json`. The whole matrix over the showcase
//! models is `cargo xtask actions`, which runs the same plan
//! (`action_matrix/plan.rs`) in server processes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[allow(dead_code)]
#[path = "action_matrix/plan.rs"]
mod plan;

use craft3d_engine::{Session, Workspace};
use plan::{CallError, Expect, KINDS, Verdict};
use serde_json::{Value, json};

/// Entities per kind the gated run takes (bottom-k by name, so the same
/// ones every time).
const PER_KIND: usize = 4;

fn bracket() -> Session {
    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    let text = include_str!("action_matrix/bracket.steps.json");
    let steps: Vec<(String, Value)> = serde_json::from_str(text).unwrap();
    for (id, params) in &steps {
        session.execute(id, params).unwrap();
    }
    session
}

fn call(session: &mut Session) -> impl FnMut(&str, &Value) -> Result<Value, CallError> + '_ {
    |id, params| {
        session
            .execute(id, params)
            .map_err(|e| CallError::Command(e.message().to_string()))
    }
}

#[test]
fn the_bracket_keeps_what_succeeded_before() {
    let ratchet = plan::ratchet_from(
        &serde_json::from_str(include_str!("action_matrix/ratchet.json")).unwrap(),
    );
    let floor = ratchet.get("bracket").expect("the ratchet has the bracket");
    let mut session = bracket();
    let inspect = session.execute("document.inspect", &json!({})).unwrap();
    let unit = plan::length_unit(&inspect);
    let mut actions = Vec::new();
    for body in plan::bodies(&inspect) {
        let edges = session
            .execute("document.edges", &json!({"feature": body.feature}))
            .unwrap();
        actions.extend(plan::plan(&body, &plan::edges(&edges), &unit, KINDS));
    }
    let chosen = plan::choose(actions, Some(PER_KIND), &|_, _| false);
    let mut held = 0;
    let mut failures = Vec::new();
    let start = std::time::Instant::now();
    for action in &chosen {
        let outcome = plan::run_action(&mut call(&mut session), action);
        let kind = action.kind.as_str();
        println!(
            "{kind:12} {:8} {} {}",
            outcome.verdict.as_str(),
            action.entity,
            outcome.reason
        );
        // Never a crash, whatever the action.
        if outcome.verdict == Verdict::Crash {
            failures.push(format!(
                "{kind} on {}: crash: {}",
                action.entity, outcome.reason
            ));
        }
        if floor.get(kind).is_some_and(|s| s.contains(&action.entity)) {
            held += 1;
            if outcome.verdict != Verdict::Ok {
                failures.push(format!(
                    "{kind} on {} succeeded before and is {} now: {}",
                    action.entity,
                    outcome.verdict.as_str(),
                    outcome.reason
                ));
            }
        }
        if outcome.restart {
            session = bracket();
        }
    }
    println!(
        "{} actions, {held} of them held to the ratchet, in {:.1} s",
        chosen.len(),
        start.elapsed().as_secs_f64()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // The run means something only when it holds enough of the ratchet.
    assert!(held >= 20, "only {held} actions are in the ratchet");
    // Undoing every action left the model as it was.
    let after = session.execute("document.inspect", &json!({})).unwrap();
    assert_eq!(after.get("bodies"), inspect.get("bodies"));
}

#[test]
fn check_takes_one_feature() {
    let mut session = bracket();
    let all = session
        .execute("document.check", &json!({"deep": true}))
        .unwrap();
    assert_eq!(all["booleansChecked"], 2);
    let one = session
        .execute(
            "document.check",
            &json!({"feature": "Bracket", "deep": true}),
        )
        .unwrap();
    assert_eq!(one["ok"], true);
    assert_eq!(one["bodiesChecked"], 1);
    assert_eq!(one["booleansChecked"], 1);
    // A feature whose body a later one took has no body of its own.
    let taken = session
        .execute("document.check", &json!({"feature": "Plate"}))
        .unwrap();
    assert_eq!(taken["bodiesChecked"], 0);
    // A failed feature elsewhere is not this feature's problem.
    session
        .execute(
            "feature.fillet",
            &json!({"edges": [["F5:copy(0)/F1:face(0)", "F5:copy(0)/F1:face(1)"]], "radius": "500 mm"}),
        )
        .unwrap();
    let all = session.execute("document.check", &json!({})).unwrap();
    assert_eq!(all["ok"], false);
    let one = session
        .execute("document.check", &json!({"feature": "Bracket"}))
        .unwrap();
    assert_eq!(one["ok"], true);
    let failed = session
        .execute("document.check", &json!({"feature": "F6"}))
        .unwrap();
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["bodiesChecked"], 0);
    for bad in [
        json!({"feature": "F99"}),
        json!({"feature": -1}),
        json!({"feature": [1]}),
        json!({"feature": "nothing"}),
    ] {
        assert!(session.execute("document.check", &bad).is_err(), "{bad}");
    }
}

fn action(expect: Expect, volume: f64, area: f64) -> plan::Action {
    plan::Action {
        kind: plan::Kind::OffsetOut,
        body: "F1".into(),
        entity: "F1:face(0)".into(),
        command: "feature.offsetFace".into(),
        params: json!({}),
        expect,
        volume,
        area,
    }
}

/// The identities refuse what is wrong: seeded mistakes.
#[test]
fn identities_catch_wrong_volumes() {
    // A 10 x 10 x 10 block, area 600, its top moved by 1.
    let up = action(Expect::Offset(1.0), 1000.0, 600.0);
    assert!(plan::identity(&up, 1100.0, 640.0).is_ok());
    assert!(plan::identity(&up, 900.0, 560.0).is_err(), "the wrong way");
    assert!(
        plan::identity(&up, 2000.0, 640.0).is_err(),
        "more than a layer"
    );
    assert!(plan::identity(&up, -1.0, 600.0).is_err(), "inside out");
    let down = action(Expect::Offset(-1.0), 1000.0, 600.0);
    assert!(plan::identity(&down, 900.0, 560.0).is_ok());
    assert!(plan::identity(&down, 1100.0, 640.0).is_err());
    let shell = action(Expect::Hollow, 1000.0, 600.0);
    assert!(plan::identity(&shell, 488.0, 1000.0).is_ok());
    assert!(plan::identity(&shell, 1000.0, 600.0).is_err());
    let layer = action(Expect::Layer(1.0), 1000.0, 600.0);
    assert!(plan::identity(&layer, 100.0, 240.0).is_ok());
    assert!(plan::identity(&layer, 300.0, 240.0).is_err());
    let mirror = action(Expect::Same, 1000.0, 600.0);
    assert!(plan::identity(&mirror, 1000.0 * (1.0 + 1e-9), 600.0).is_ok());
    assert!(plan::identity(&mirror, 1001.0, 600.0).is_err());
    let pattern = action(Expect::Double, 1000.0, 600.0);
    assert!(plan::identity(&pattern, 2000.0, 1200.0).is_ok());
    assert!(plan::identity(&pattern, 1999.0, 1200.0).is_err());
    let blend = action(Expect::Blend, 1000.0, 600.0);
    assert!(plan::identity(&blend, 990.0, 598.0).is_ok());
    assert!(plan::identity(&blend, 400.0, 598.0).is_err());
}

#[test]
fn a_reason_class_drops_names_and_numbers() {
    let a = plan::reason_class(
        "edges.0 (between F5:copy(0)/F1:face(0) and F5:copy(1)/F4:face(0)): with a radius of 1.2 mm the line along which the fillet touches the face F5:copy(0)/F1:face(0) leaves that face",
    );
    let b = plan::reason_class(
        "edges.0 (between F7:copy(0)/F2:side(e3) and F7:copy(0)/F2:endCap): with a radius of 30 mm the line along which the fillet touches the face F7:copy(0)/F2:endCap leaves that face",
    );
    assert_eq!(a, b);
    assert!(!a.contains("F5"), "{a}");
    assert!(a.starts_with("with a radius of <n> mm"), "{a}");
}

#[test]
fn sampling_is_stable() {
    let names: Vec<String> = (0..50).map(|i| format!("F1:face({i})")).collect();
    let few = plan::sample(plan::Kind::Shell, &names, Some(5));
    assert_eq!(few.len(), 5);
    // A name is chosen whatever else there is: more names take only those
    // that hash lower.
    let more: Vec<String> = (0..60).map(|i| format!("F1:face({i})")).collect();
    let again = plan::sample(plan::Kind::Shell, &more, Some(5));
    let kept = few.intersection(&again).count();
    assert!(kept >= 4, "{few:?} {again:?}");
    assert_eq!(plan::sample(plan::Kind::Shell, &names, None).len(), 50);
}

#[test]
fn the_ratchet_reads_back() {
    let value: Value = serde_json::from_str(include_str!("action_matrix/ratchet.json")).unwrap();
    let ratchet = plan::ratchet_from(&value);
    assert_eq!(plan::ratchet_from(&plan::ratchet_json(&ratchet)), ratchet);
    for kinds in ratchet.values() {
        for kind in kinds.keys() {
            assert!(
                KINDS.iter().any(|k| k.as_str() == kind),
                "unknown kind {kind}"
            );
        }
    }
}
