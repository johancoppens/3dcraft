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

fn ratchet() -> plan::Ratchet {
    plan::ratchet_from(&serde_json::from_str(include_str!("action_matrix/ratchet.json")).unwrap())
        .unwrap()
}

#[test]
fn the_bracket_keeps_what_succeeded_before() {
    let ratchet = ratchet();
    let floor = ratchet.get("bracket").expect("the ratchet has the bracket");
    let in_floor = |kind: plan::Kind, entity: &str| {
        floor.get(kind.as_str()).is_some_and(|s| s.contains(entity))
    };
    let mut session = bracket();
    let inspect = session.execute("document.inspect", &json!({})).unwrap();
    let unit = plan::length_unit(&inspect);
    let mut actions = Vec::new();
    // As the xtask: solids only.
    for body in plan::bodies(&inspect).into_iter().filter(|b| b.solid) {
        let edges = session
            .execute("document.edges", &json!({"feature": body.feature}))
            .unwrap();
        assert_eq!(edges["truncated"], false);
        actions.extend(plan::plan(&body, &plan::edges(&edges), &unit, KINDS));
    }
    // A few of every kind, and every entry of the ratchet.
    let chosen = plan::choose(actions, Some(PER_KIND), &in_floor);
    let mut failures = Vec::new();
    // Every entry of the ratchet is planned: one that is not (a name that
    // went away) is a failure, not a silent pass.
    for (kind, names) in floor {
        for entity in names {
            if !chosen
                .iter()
                .any(|a| a.kind.as_str() == kind && &a.entity == entity)
            {
                failures.push(format!(
                    "{kind} on {entity} is in the ratchet but not planned"
                ));
            }
        }
    }
    let mut held = 0;
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
        // Never a crash and never a wrong result, whatever the action.
        if matches!(outcome.verdict, Verdict::Crash | Verdict::Wrong) {
            failures.push(format!(
                "{kind} on {}: {}: {}",
                action.entity,
                outcome.verdict.as_str(),
                outcome.reason
            ));
        }
        if in_floor(action.kind, &action.entity) {
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
            failures.push(format!(
                "{kind} on {}: the document was not as it was after the undo",
                action.entity
            ));
            session = bracket();
        }
    }
    println!(
        "{} actions, {held} of them held to the ratchet, in {:.1} s",
        chosen.len(),
        start.elapsed().as_secs_f64()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let entries: usize = floor.values().map(std::collections::BTreeSet::len).sum();
    assert_eq!(held, entries);
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
    // A feature that is not built is said to be so, not "ok, no bodies".
    session
        .execute("feature.suppress", &json!({"feature": "F6"}))
        .unwrap();
    let suppressed = session
        .execute("document.check", &json!({"feature": "F6"}))
        .unwrap();
    assert_eq!(suppressed["ok"], false);
    assert_eq!(suppressed["status"], "suppressed");
    session
        .execute("history.rollback", &json!({"feature": "Drill"}))
        .unwrap();
    let rolled = session
        .execute("document.check", &json!({"feature": "Bracket"}))
        .unwrap();
    assert_eq!(rolled["ok"], false, "{rolled}");
    assert_eq!(rolled["status"], "rolledBack");
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
        centroid: [5.0, 5.0, 5.0],
        diag: 300f64.sqrt(),
    }
}

fn measured(volume: f64, area: f64) -> plan::Measured {
    plan::Measured {
        volume,
        area,
        centroid: [5.0, 5.0, 5.0],
    }
}

fn holds(action: &plan::Action, volume: f64, area: f64) -> bool {
    plan::identity(action, &measured(volume, area)).is_ok()
}

/// The identities refuse what is wrong: seeded mistakes, a no-op among
/// them.
#[test]
fn identities_catch_wrong_volumes() {
    // A 10 x 10 x 10 block, area 600, its top moved by 1.
    let up = action(Expect::Offset(1.0), 1000.0, 600.0);
    assert!(holds(&up, 1100.0, 640.0));
    assert!(!holds(&up, 900.0, 560.0), "the wrong way");
    assert!(!holds(&up, 1000.0, 600.0), "nothing done");
    assert!(!holds(&up, 2000.0, 640.0), "more than a layer");
    assert!(!holds(&up, -1.0, 600.0), "inside out");
    let down = action(Expect::Offset(-1.0), 1000.0, 600.0);
    assert!(holds(&down, 900.0, 560.0));
    assert!(!holds(&down, 1100.0, 640.0));
    let moved = action(Expect::Move(1.0), 1000.0, 600.0);
    assert!(holds(&moved, 900.0, 560.0) && holds(&moved, 1100.0, 640.0));
    assert!(!holds(&moved, 1000.0, 600.0), "nothing done");
    assert!(!holds(&moved, 1700.0, 640.0));
    // Tilted by 3° on a body of diagonal 17.3: at most 0.91 · 600.
    let draft = action(Expect::Draft(300f64.sqrt()), 1000.0, 600.0);
    assert!(holds(&draft, 1100.0, 600.0));
    assert!(!holds(&draft, 1600.0, 600.0));
    let shell = action(Expect::Hollow(1.0), 1000.0, 600.0);
    assert!(holds(&shell, 488.0, 1088.0));
    assert!(!holds(&shell, 1000.0, 600.0), "not hollow");
    assert!(!holds(&shell, 900.0, 600.0), "walls thicker than 1");
    let layer = action(Expect::Layer(1.0), 1000.0, 600.0);
    assert!(holds(&layer, 100.0, 240.0));
    assert!(!holds(&layer, 300.0, 240.0));
    // Mirrored in x = 20: the centroid goes to x = 35.
    let mirror = action(Expect::Same(20.0), 1000.0, 600.0);
    let image = |c: [f64; 3], area: f64| {
        plan::identity(
            &mirror,
            &plan::Measured {
                volume: 1000.0,
                area,
                centroid: c,
            },
        )
        .is_ok()
    };
    assert!(image([35.0, 5.0, 5.0], 600.0));
    assert!(!image([5.0, 5.0, 5.0], 600.0), "not mirrored");
    assert!(!image([35.0, 5.0, 5.0], 601.0), "area changed");
    assert!(
        !plan::identity(
            &mirror,
            &plan::Measured {
                volume: 1001.0,
                area: 600.0,
                centroid: [35.0, 5.0, 5.0]
            }
        )
        .is_ok()
    );
    // Two instances 30 apart: the centroid at x = 20.
    let pattern = action(Expect::Double(30.0), 1000.0, 600.0);
    let two = |v: f64, area: f64, x: f64| {
        plan::identity(
            &pattern,
            &plan::Measured {
                volume: v,
                area,
                centroid: [x, 5.0, 5.0],
            },
        )
        .is_ok()
    };
    assert!(two(2000.0, 1200.0, 20.0));
    assert!(!two(1999.0, 1200.0, 20.0));
    assert!(!two(2000.0, 1100.0, 20.0), "instances merged");
    assert!(!two(2000.0, 1200.0, 5.0), "both in one place");
    // A fillet of radius 1 along one edge 10 long: at most
    // (11.43)²/2 · (10 + 2π · 11.43) ≈ 5.6e3; a chamfer of 1: at most 8.6.
    let fillet = action(
        Expect::Blend {
            size: 1.0,
            length: 10.0,
            edges: 1,
            chamfer: false,
        },
        1000.0,
        600.0,
    );
    assert!(holds(&fillet, 997.85, 598.0));
    assert!(!holds(&fillet, 1000.0, 600.0), "nothing done");
    let chamfer = action(
        Expect::Blend {
            size: 1.0,
            length: 10.0,
            edges: 1,
            chamfer: true,
        },
        1000.0,
        600.0,
    );
    assert!(holds(&chamfer, 995.0, 596.0));
    assert!(!holds(&chamfer, 950.0, 596.0), "more than a chamfer takes");
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
    let ratchet = plan::ratchet_from(&value).unwrap();
    assert_eq!(
        plan::ratchet_from(&plan::ratchet_json(&ratchet)).unwrap(),
        ratchet
    );
    for kinds in ratchet.values() {
        for (kind, names) in kinds {
            assert!(
                KINDS.iter().any(|k| k.as_str() == kind),
                "unknown kind {kind}"
            );
            // Edge pairs are kept in order.
            for name in names {
                if let Some((a, b)) = name.split_once('|') {
                    assert_eq!(plan::pair_key(a, b), *name);
                }
            }
        }
    }
    // A ratchet read wrong would let every regression through.
    for bad in [
        json!({}),
        json!({"models": []}),
        json!({"models": {"bracket": {"fillet.tiny": []}}}),
        json!({"models": {"bracket": {"chamfer": "F1:face(0)"}}}),
        json!({"models": {"bracket": {"chamfer": [1]}}}),
    ] {
        assert!(plan::ratchet_from(&bad).is_err(), "{bad}");
    }
}
