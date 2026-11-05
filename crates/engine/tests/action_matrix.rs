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

use craft3d_math::Portable;
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
    let mut edges_of = std::collections::BTreeMap::new();
    // As the xtask: solids only.
    for body in plan::bodies(&inspect).into_iter().filter(|b| b.solid) {
        let edges = session
            .execute("document.edges", &json!({"feature": body.feature}))
            .unwrap();
        assert_eq!(edges["truncated"], false);
        let edges = plan::edges(&edges);
        actions.extend(plan::plan(&body, &edges, &unit, KINDS));
        edges_of.insert(body.feature.clone(), edges);
    }
    // A few of every kind, and every entry of the ratchet.
    let mut chosen = plan::choose(actions, Some(PER_KIND), &in_floor);
    // The straight edges between flat faces get their exact change: the
    // block's twelve edges and the boss's foot are not all of them.
    let oriented = plan::orient_all(&mut call(&mut session), &mut chosen, &edges_of).unwrap();
    plan::bound_all(&mut call(&mut session), &mut chosen);
    assert!(chosen.iter().all(|a| a.volume_bound.is_some()));
    println!("{oriented} blends with their exact change");
    assert!(
        oriented >= 20,
        "only {oriented} blends have an exact change"
    );
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
        if matches!(
            outcome.verdict,
            Verdict::Crash | Verdict::Bug | Verdict::Wrong
        ) {
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
        pair: None,
        volume_bound: None,
        area_bound: None,
    }
}

/// The edges of the block 0..10 cubed, its faces named by side.
fn cube_edges() -> Vec<plan::Edge> {
    let c = |x: f64, y: f64, z: f64| [x, y, z];
    let e = |a: &str, b: &str, s: [f64; 3], t: [f64; 3]| plan::Edge {
        faces: [a.to_string(), b.to_string()],
        curve: "line".into(),
        start: s,
        end: t,
        closed: false,
        length: Some(10.0),
    };
    vec![
        e("bottom", "y0", c(0., 0., 0.), c(10., 0., 0.)),
        e("bottom", "x1", c(10., 0., 0.), c(10., 10., 0.)),
        e("bottom", "y1", c(10., 10., 0.), c(0., 10., 0.)),
        e("bottom", "x0", c(0., 10., 0.), c(0., 0., 0.)),
        e("top", "y0", c(0., 0., 10.), c(10., 0., 10.)),
        e("top", "x1", c(10., 0., 10.), c(10., 10., 10.)),
        e("top", "y1", c(10., 10., 10.), c(0., 10., 10.)),
        e("top", "x0", c(0., 10., 10.), c(0., 0., 10.)),
        e("x0", "y0", c(0., 0., 0.), c(0., 0., 10.)),
        e("y0", "x1", c(10., 0., 0.), c(10., 0., 10.)),
        e("x1", "y1", c(10., 10., 0.), c(10., 10., 10.)),
        e("y1", "x0", c(0., 10., 0.), c(0., 10., 10.)),
    ]
}

fn outward() -> std::collections::BTreeMap<String, [f64; 3]> {
    [
        ("bottom", [0., 0., -1.]),
        ("top", [0., 0., 1.]),
        ("x0", [-1., 0., 0.]),
        ("x1", [1., 0., 0.]),
        ("y0", [0., -1., 0.]),
        ("y1", [0., 1., 0.]),
    ]
    .into_iter()
    .map(|(f, n)| (f.to_string(), n))
    .collect()
}

/// A blend of the block's edge between its top and the side x = 10.
fn cube_blend(size: f64, chamfer: bool) -> plan::Action {
    let mut a = action(plan::blend(size, 10.0, 0, 2, chamfer), 1000.0, 600.0);
    a.kind = if chamfer {
        plan::Kind::Chamfer
    } else {
        plan::Kind::FilletSmall
    };
    a.pair = Some(plan::Pair {
        faces: ["top".into(), "x1".into()],
        line: Some(([10., 0., 10.], [10., 10., 10.])),
    });
    plan::orient(&mut a, &cube_edges(), &outward());
    a
}

/// The exact change of a blend between flat faces, held to seeded
/// mistakes: a radius three times too large, the wrong sign.
#[test]
fn a_blend_between_flat_faces_has_its_exact_change() {
    let fillet = cube_blend(1.0, false);
    let Expect::Blend { exact: Some(x), .. } = fillet.expect else {
        panic!("the cube's edge is not oriented: {:?}", fillet.expect);
    };
    // An outside edge at 90°: r² (1 − π/4) = 0.2146 along 10.
    assert!(!x.gains);
    assert!((x.section - (1.0 - std::f64::consts::FRAC_PI_4)).abs() < 1e-12);
    assert!((x.setback - 1.0).abs() < 1e-12);
    let taken = 10.0 * x.section;
    assert!(holds(&fillet, 1000.0 - taken, 598.0));
    assert!(
        !holds(&fillet, 1000.0 - 9.0 * taken, 598.0),
        "three times the radius"
    );
    assert!(
        !holds(&fillet, 1000.0 + taken, 598.0),
        "material added at an outside edge"
    );
    assert!(!holds(&fillet, 1000.0, 600.0), "nothing done");
    // A chamfer of 1: half a unit square along 10.
    let chamfer = cube_blend(1.0, true);
    let Expect::Blend { exact: Some(x), .. } = chamfer.expect else {
        panic!("not oriented");
    };
    assert!((x.section - 0.5).abs() < 1e-12);
    assert!(holds(&chamfer, 995.0, 596.0));
    assert!(
        !holds(&chamfer, 1000.0 - 16.0 * 5.0, 596.0),
        "four times the distance"
    );
    // An inside edge (the faces' normals pointing the other way round, as
    // in a pocket) gains material.
    let mut inside = outward();
    for n in inside.values_mut() {
        *n = [-n[0], -n[1], -n[2]];
    }
    let mut pocket = action(plan::blend(1.0, 10.0, 0, 2, false), 1000.0, 600.0);
    pocket.pair = Some(plan::Pair {
        faces: ["top".into(), "x1".into()],
        line: Some(([10., 0., 10.], [10., 10., 10.])),
    });
    plan::orient(&mut pocket, &cube_edges(), &inside);
    let Expect::Blend { exact: Some(x), .. } = pocket.expect else {
        panic!("not oriented");
    };
    assert!(x.gains);
    assert!(holds(&pocket, 1000.0 + 10.0 * x.section, 598.0));
    assert!(
        !holds(&pocket, 1000.0 - 10.0 * x.section, 598.0),
        "material taken at an inside edge"
    );
}

fn measured(volume: f64, area: f64) -> plan::Measured {
    plan::Measured {
        volume,
        area,
        centroid: [5.0, 5.0, 5.0],
        volume_bound: None,
        area_bound: None,
        applied: None,
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
    assert!(!holds(&draft, 1000.0, 600.0), "nothing done");
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
                volume_bound: None,
                area_bound: None,
                applied: None,
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
                centroid: [35.0, 5.0, 5.0],
                volume_bound: None,
                area_bound: None,
                applied: None,
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
                volume_bound: None,
                area_bound: None,
                applied: None,
            },
        )
        .is_ok()
    };
    assert!(two(2000.0, 1200.0, 20.0));
    assert!(!two(1999.0, 1200.0, 20.0));
    assert!(!two(2000.0, 1100.0, 20.0), "instances merged");
    assert!(!two(2000.0, 1200.0, 5.0), "both in one place");
    // Without the exact change: a fillet of radius 0.1 along one straight
    // edge 10 long takes at most 0.1143 · (10 + 2 · 5.67 · 1.143) = 2.62
    // (r times its setback at 10°, along the edge and on past both ends
    // by the setback times cot 10°); a chamfer of 1 at most
    // 0.5 · (10 + 2 · 5.67) = 10.7; along a closed curved edge (no ends)
    // 2π times the setback more than its length.
    let fillet = action(plan::blend(0.1, 10.0, 0, 2, false), 1000.0, 600.0);
    assert!(holds(&fillet, 999.98, 599.98));
    assert!(!holds(&fillet, 1000.0, 600.0), "nothing done");
    assert!(!holds(&fillet, 997.0, 598.0), "more than the bound");
    let curved = action(plan::blend(1.0, 10.0, 1, 0, false), 1000.0, 600.0);
    assert!(holds(&curved, 880.0, 598.0));
    assert!(!holds(&curved, 50.0, 598.0), "more than the bound");
    let chamfer = action(plan::blend(1.0, 10.0, 0, 2, true), 1000.0, 600.0);
    assert!(holds(&chamfer, 995.0, 596.0));
    assert!(holds(&chamfer, 990.0, 596.0), "run on past its ends");
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

/// The block `[-4, 0] x [0, 3] x [-3, 3]` times `s` with both edges of its
/// side `x = 0` chamfered by `0.5 s` (end faces at 45° to y) and a
/// scallop of radius `1.5 s` round the y axis cut out of that side: the
/// side and the scallop meet in two straight edges along y, `2 s` long,
/// that end on the 45° chamfers (the turbofan's flange scallop, made
/// small).
fn scallop(s: f64) -> Session {
    let mm = |x: f64| format!("{} mm", x * s);
    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    for (id, params) in [
        (
            "feature.block",
            json!({"size": [mm(4.0), mm(3.0), mm(6.0)], "position": [mm(-4.0), mm(0.0), mm(-3.0)], "name": "Box"}),
        ),
        (
            "feature.chamfer",
            json!({"distance": mm(0.5), "edges": [["F1:face(2)", "F1:face(5)"], ["F1:face(3)", "F1:face(5)"]], "name": "Ends"}),
        ),
        (
            "feature.cylinder",
            json!({"radius": mm(1.5), "height": mm(5.0), "position": [mm(0.0), mm(-1.0), mm(0.0)], "axis": [0, 1, 0], "name": "Tool"}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "Ends", "tool": "Tool", "name": "Scallop"}),
        ),
    ] {
        session.execute(id, &params).unwrap();
    }
    session
}

/// A blend of straight edges between curved faces runs on past the edges'
/// ends onto slanted end faces, so the bound of the cross-section times
/// the length alone is too tight (the turbofan's chamfer of a flange
/// scallop broke it by 2.8 %); the bound gives each end the setback times
/// cot 10°, as the exact change does. The volume the chamfer takes is
/// worked out here independently, at three scales: in the plane square
/// to the edges the chamfer cuts the corner between the side `x = 0` and
/// the scallop's circle with the chord from `d` along the side to `d`
/// along the circle (arc length, as `distance` means on a curved face);
/// that cross-section `A` runs the edge's length and, past each end, on
/// over the 45° chamfer as far as its depth `−x` into the side. So each
/// edge takes `A·L + 2·∫_A (−x) dA`.
#[test]
fn a_blend_runs_on_past_slanted_ends() {
    for s in [0.1, 1.0, 100.0] {
        let mut session = scallop(s);
        let inspect = session.execute("document.inspect", &json!({})).unwrap();
        let unit = plan::length_unit(&inspect);
        let body = plan::bodies(&inspect)
            .into_iter()
            .find(|b| b.solid)
            .expect("the scallop's body");
        let edges = plan::edges(
            &session
                .execute("document.edges", &json!({"feature": body.feature}))
                .unwrap(),
        );
        let key = plan::pair_key("F4:copy(0)/F1:face(5)", "F4:copy(1)/F3:face(0)");
        let mut chamfer = plan::plan(&body, &edges, &unit, &[plan::Kind::Chamfer])
            .into_iter()
            .find(|a| a.entity == key)
            .expect("a chamfer of the side's edges with the scallop");
        let Expect::Blend {
            size: d,
            length,
            curved,
            ends,
            exact,
            ..
        } = chamfer.expect
        else {
            panic!("not a blend: {:?}", chamfer.expect);
        };
        // Two straight edges, four ends: the general bound.
        assert_eq!((curved, ends), (0, 4));
        assert!((length - 4.0 * s).abs() < 1e-9 * s, "{length}");
        plan::orient_all(
            &mut call(&mut session),
            std::slice::from_mut(&mut chamfer),
            &[(body.feature.clone(), edges.clone())]
                .into_iter()
                .collect(),
        )
        .unwrap();
        assert!(exact.is_none() && matches!(chamfer.expect, Expect::Blend { exact: None, .. }));
        // The faces meet square (the side and the scallop's wall, whose
        // axis runs along the edges), and each end is a 45° chamfer: cot
        // and 1/sin of 45° at each of the four ends.
        let Expect::Blend {
            wedge, tilt, slant, ..
        } = chamfer.expect
        else {
            panic!("not a blend");
        };
        let w = wedge.expect("the wedge from the normals");
        assert!((w - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "{w}");
        assert!((tilt - 4.0).abs() < 1e-9, "{tilt}");
        assert!(
            (slant - 4.0 * std::f64::consts::SQRT_2).abs() < 1e-9,
            "{slant}"
        );
        // The cross-section at the edge (0, ρ): the side runs on along +z,
        // the circle into x < 0. A polygon of the corner, the chord and
        // 4096 pieces of the arc: area and first moment exact for the
        // polygon, the arc's chord error (d/ρ)²/4096² of no account.
        let rho = 1.5 * s;
        let t = d / rho;
        let mut poly = vec![(0.0, rho), (0.0, rho + d)];
        let n = 4096;
        for i in (0..=n).rev() {
            let a = t * f64::from(i) / f64::from(n);
            poly.push((-rho * a.psin(), rho * a.pcos()));
        }
        let (mut area, mut moment) = (0.0, 0.0);
        for (i, &(x0, z0)) in poly.iter().enumerate() {
            let (x1, z1) = poly[(i + 1) % poly.len()];
            let cross = x0 * z1 - x1 * z0;
            area += cross / 2.0;
            moment += (x0 + x1) * cross / 6.0;
        }
        let (area, depth) = (area.abs(), moment.abs());
        let along = 2.0 * s;
        let taken = 2.0 * (area * along + 2.0 * depth);
        // The section alone along the length: what the old bound allowed
        // at most, d²/2 times the length, is less than what is taken.
        assert!(
            taken > 0.5 * d * d * length * 1.01,
            "{taken} against {}",
            0.5 * d * d * length
        );
        let outcome = plan::run_action(&mut call(&mut session), &chamfer);
        assert_eq!(outcome.verdict, plan::Verdict::Ok, "{}", outcome.reason);
        let (volume, _) = outcome.after.expect("measured after");
        let change = body.volume - volume;
        assert!(
            (change - taken).abs() <= 1e-6 * taken,
            "scale {s}: the chamfer took {change}, worked out {taken}"
        );
        // It is held: four times the distance is not let through.
        assert!(
            !holds(&chamfer, body.volume - 16.0 * taken, body.area),
            "four times the distance at scale {s}"
        );
    }
}

/// A blend the volume cannot tell from nothing: held to the noise of the
/// integrator's bounds and, below it, to other evidence and to its area.
/// Seeded: a blend that did nothing, one whose change the volume would
/// show, and one of the wrong size stay wrong.
#[test]
fn a_blend_below_the_noise_needs_other_evidence() {
    // Bounds of 1e-8 on the volume and 1e-9 on the area, before and after.
    let bounded = |mut a: plan::Action| {
        a.volume_bound = Some(1e-8);
        a.area_bound = Some(1e-9);
        a
    };
    let after = |volume: f64, area: f64, applied: Option<bool>| plan::Measured {
        volume,
        area,
        centroid: [5.0, 5.0, 5.0],
        volume_bound: Some(1e-8),
        area_bound: Some(1e-9),
        applied,
    };
    let holds_after = |a: &plan::Action, m: plan::Measured| plan::identity(a, &m).is_ok();
    // A fillet of radius 1e-5 along 10: at most 11.43e-10 · 10.0013 =
    // 1.14e-8 of volume, below the noise 2e-8; the faces lose strips
    // 1.14e-4 wide, at most 2.29e-3 of area.
    let tiny = bounded(action(plan::blend(1e-5, 10.0, 0, 2, false), 1000.0, 600.0));
    assert!((plan::noise(&tiny, &after(1000.0, 600.0, None)) - 2e-8).abs() < 1e-20);
    assert!(plan::needs_witness(
        &tiny,
        &after(1000.0 - 1e-8, 600.0, None)
    ));
    assert!(!plan::needs_witness(
        &tiny,
        &after(1000.0 - 1e-7, 600.0, None)
    ));
    let v = 1000.0 - 1e-9;
    assert!(
        holds_after(&tiny, after(v, 600.0 - 1e-3, Some(true))),
        "applied"
    );
    assert!(
        !holds_after(&tiny, after(v, 600.0 - 1e-3, Some(false))),
        "did nothing"
    );
    assert!(
        !holds_after(&tiny, after(v, 600.0 - 1e-3, None)),
        "not looked at"
    );
    assert!(
        !holds_after(&tiny, after(v, 600.0 + 1e-3, Some(true))),
        "area gained"
    );
    assert!(
        !holds_after(&tiny, after(v, 600.0 - 3e-3, Some(true))),
        "too wide"
    );
    // A fillet of radius 0.1 may change the volume by 2.6: unchanged, it is
    // wrong whatever else shows.
    let normal = bounded(action(plan::blend(0.1, 10.0, 0, 2, false), 1000.0, 600.0));
    assert!(
        !holds_after(&normal, after(1000.0, 599.9, Some(true))),
        "could tell"
    );
    // The exact fillet of radius 1 on the cube's edge, the volume as it
    // was: wrong, applied or not.
    let mut cube = cube_blend(1.0, false);
    cube = bounded(cube);
    assert!(
        !holds_after(&cube, after(1000.0, 598.0, Some(true))),
        "must take 2.1"
    );
    // An exact fillet of radius 1e-5 on it changes the volume by 2.1e-10,
    // below the noise, and the area by 10 · (π/2 − 2) · 1e-5 = −4.29e-5,
    // give or take the end faces (5e-10).
    let small = bounded(cube_blend(1e-5, false));
    let strip = 10.0 * (std::f64::consts::FRAC_PI_2 - 2.0) * 1e-5;
    let v = 1000.0 - 2e-10;
    assert!(
        holds_after(&small, after(v, 600.0 + strip, Some(true))),
        "exact"
    );
    for (k, why) in [
        (2.0, "twice the radius"),
        (1.0 / 30.0, "a thirtieth"),
        (4.0, "four times"),
    ] {
        assert!(
            !holds_after(&small, after(v, 600.0 + k * strip, Some(true))),
            "{why}"
        );
    }
    // Looser bounds than 1e-9 of the body leave the noise at that.
    let mut loose = tiny.clone();
    loose.volume_bound = Some(1.0);
    let mut far = after(1000.0, 600.0, None);
    far.volume_bound = Some(1.0);
    assert!((plan::noise(&loose, &far) - 1e-6).abs() < 1e-18);
    let plain = cube_blend(1.0, false);
    assert!((plan::noise(&plain, &measured(1000.0, 600.0)) - 1e-6).abs() < 1e-18);
}

/// A drum of radius `100 s` with a lug `s` cubed on its top: the fillet of
/// one of the lug's top edges with radius `0.003 s` takes
/// `0.2146 · (0.003 s)² · s = 1.9e-6 s³`, less than the integrator's own
/// bounds on the drum's volume (about `4e-6 s³` each): the weld table's
/// tiny fillets on a large body, made small. Held at three scales: the
/// fillet is ok by the evidence that it was applied and by its area; the
/// same fillet built at twice, a thirtieth and four times the radius is
/// not, nor are two features that did not round that edge (the body
/// moved by nothing; a fillet of another edge). The move by nothing also
/// pins how little the measure jitters: far below the bounds.
#[test]
fn a_tiny_blend_on_a_large_body_is_told_by_its_faces() {
    for s in [0.1, 1.0, 100.0] {
        let mm = |x: f64| format!("{} mm", x * s);
        let mut session = Session::new(Workspace::unrestricted());
        session
            .execute("file.new", &json!({"units": "mm"}))
            .unwrap();
        for (id, params) in [
            (
                "feature.cylinder",
                json!({"radius": mm(100.0), "height": mm(100.0), "name": "Drum"}),
            ),
            (
                "feature.block",
                json!({"size": [mm(1.0), mm(1.0), mm(1.0)], "position": [mm(-0.5), mm(-0.5), mm(99.5)], "name": "Lug"}),
            ),
            (
                "feature.boolean",
                json!({"op": "union", "target": "Drum", "tool": "Lug", "name": "Body"}),
            ),
        ] {
            session.execute(id, &params).unwrap();
        }
        let inspect = session.execute("document.inspect", &json!({})).unwrap();
        let unit = plan::length_unit(&inspect);
        let body = plan::bodies(&inspect)
            .into_iter()
            .find(|b| b.solid)
            .expect("the body");
        let edges = plan::edges(
            &session
                .execute("document.edges", &json!({"feature": body.feature}))
                .unwrap(),
        );
        let (a, b) = ("F3:copy(1)/F2:face(1)", "F3:copy(1)/F2:face(2)");
        let key = plan::pair_key(a, b);
        let mut fillet = plan::plan(&body, &edges, &unit, &[plan::Kind::FilletSmall])
            .into_iter()
            .find(|x| x.entity == key)
            .expect("a fillet of the lug's top edge");
        let r = 0.003 * s;
        let radius = |k: f64| json!({"edges": [[a, b]], "radius": format!("{} mm", k * r)});
        fillet.params = radius(1.0);
        if let Expect::Blend { size, .. } = &mut fillet.expect {
            *size = r;
        }
        let edges_of = [(body.feature.clone(), edges.clone())]
            .into_iter()
            .collect();
        let mut one = vec![fillet];
        plan::orient_all(&mut call(&mut session), &mut one, &edges_of).unwrap();
        plan::bound_all(&mut call(&mut session), &mut one);
        let fillet = one.pop().unwrap();
        // One straight edge between flat faces: the exact change; its end
        // faces (the lug's sides) are square to it.
        let Expect::Blend {
            exact: Some(x),
            tilt,
            slant,
            ..
        } = fillet.expect
        else {
            panic!("scale {s}: not exact: {:?}", fillet.expect);
        };
        assert!(
            tilt.abs() < 1e-9 && (slant - 2.0).abs() < 1e-9,
            "{tilt} {slant}"
        );
        let b0 = fillet.volume_bound.expect("the drum's bound");
        assert!(
            x.section * x.length < b0,
            "scale {s}: the fillet's change {} is not below the bound {b0}",
            x.section * x.length
        );
        let outcome = plan::run_action(&mut call(&mut session), &fillet);
        assert_eq!(
            outcome.verdict,
            plan::Verdict::Ok,
            "scale {s}: {}",
            outcome.reason
        );
        let (volume, _) = outcome.after.unwrap();
        let change = body.volume - volume;
        assert!(
            change.abs() <= 2.5 * b0,
            "scale {s}: {change} is not within the noise, the witness was not needed"
        );
        // The wrong size, held to the plan of radius r: never ok (a
        // thirtieth at the smallest scale is below the document's
        // tolerance and refused; built, it is wrong).
        for k in [2.0, 1.0 / 30.0, 4.0] {
            let mut wrong = fillet.clone();
            wrong.params = radius(k);
            let outcome = plan::run_action(&mut call(&mut session), &wrong);
            let built = outcome.verdict != plan::Verdict::Refused;
            assert!(
                outcome.verdict == plan::Verdict::Wrong || (!built && k < 1.0 && s < 1.0),
                "scale {s}: {k} times the radius: {:?} {}",
                outcome.verdict,
                outcome.reason
            );
        }
        // Did nothing: the body moved by nothing, and a fillet of another
        // edge of the lug, each with the volume it gives.
        let other = ("F3:copy(1)/F2:face(1)", "F3:copy(1)/F2:face(3)");
        for (command, params) in [
            (
                "feature.transform",
                json!({"source": "Body", "translate": ["0 mm", "0 mm", "0 mm"]}),
            ),
            (
                "feature.fillet",
                json!({"edges": [[other.0, other.1]], "radius": format!("{r} mm")}),
            ),
        ] {
            let made = session.execute(command, &params).unwrap();
            let id = made["id"].as_str().unwrap().to_string();
            assert_eq!(made["status"], "ok", "{made}");
            let measure = session
                .execute("document.measure", &json!({"feature": id, "bound": true}))
                .unwrap();
            let after = plan::Measured {
                volume: measure["volume"].as_f64().unwrap(),
                area: measure["area"].as_f64().unwrap(),
                centroid: [0.0; 3],
                volume_bound: measure["volumeBound"].as_f64(),
                area_bound: measure["areaBound"].as_f64(),
                applied: Some(plan::blend_applied(&mut call(&mut session), &fillet, &id).is_ok()),
            };
            if command == "feature.transform" {
                // The same body measured again: its change is jitter, far
                // below the bounds the noise is made of.
                let jitter = (after.volume - body.volume).abs();
                let bounds = b0 + after.volume_bound.unwrap();
                assert!(
                    jitter <= 1e-2 * bounds,
                    "scale {s}: a move by nothing changed the volume by {jitter}, bounds {bounds}"
                );
            }
            assert!(plan::needs_witness(&fillet, &after), "scale {s}: {command}");
            assert_eq!(after.applied, Some(false), "scale {s}: {command}");
            assert!(
                plan::identity(&fillet, &after).is_err(),
                "scale {s}: {command} passed for the fillet"
            );
            session.execute("edit.undo", &json!({})).unwrap();
        }
    }
}

#[test]
fn a_copied_name_merges_a_run_of_instance_zero() {
    assert_eq!(
        plan::copied("F5", "F4:copy(0)/F1:face(5)"),
        "F5:copy(0)/F1:face(5)"
    );
    assert_eq!(
        plan::copied("F5", "F4:copy(1)/F3:face(0)"),
        "F5:copy(0)/F4:copy(1)/F3:face(0)"
    );
    assert_eq!(plan::copied("F2", "F1:face(0)"), "F2:copy(0)/F1:face(0)");
}

/// A circle cut in two arcs has no ends; one arc has two, and turns
/// through the angle its chord and length say (never less).
#[test]
fn ends_are_vertices_of_one_edge_and_arcs_turn_as_drawn() {
    let arc = |from: f64, to: f64| {
        let r = 2.0;
        let point = |t: f64| [r * t.pcos(), r * t.psin(), 0.0];
        // The length document.edges gives: the polygon through 64 points.
        let length: f64 = (0..64)
            .map(|i| {
                let a = from + (to - from) * f64::from(i) / 64.0;
                let b = from + (to - from) * f64::from(i + 1) / 64.0;
                let (p, q) = (point(a), point(b));
                ((p[0] - q[0]).ppowi(2) + (p[1] - q[1]).ppowi(2)).sqrt()
            })
            .sum();
        plan::Edge {
            faces: ["a".into(), "b".into()],
            curve: "circle".into(),
            start: point(from),
            end: point(to),
            closed: false,
            length: Some(length),
        }
    };
    let pi = std::f64::consts::PI;
    let halves = [arc(0.0, pi), arc(pi, 2.0 * pi)];
    assert!(plan::loose_ends(&halves, 4.0).is_empty());
    assert_eq!(plan::loose_ends(&halves[..1], 4.0).len(), 2);
    for t in [0.01, 0.5, 2.0, 3.0] {
        let turned = plan::turning(&arc(0.0, t)).unwrap();
        assert!(
            turned >= t && turned <= t * (1.0 + 1e-3) + 2e-6,
            "{t}: {turned}"
        );
    }
}

/// A bug the code caught itself is the verdict bug, a caught panic a
/// crash, as for the fuzz runner; the panic's words are the engine's.
#[test]
fn a_bug_and_a_panic_are_told_apart() {
    assert_eq!(plan::PANICKED, craft3d_engine::PANICKED);
    let bug = format!("feature.fillet: {}: lost an edge", craft3d_engine::BUG);
    assert_eq!(plan::bug_verdict(&bug), Some(Verdict::Bug));
    let panic = format!(
        "feature.fillet: {}: {}: index out of range",
        craft3d_engine::BUG,
        craft3d_engine::PANICKED
    );
    assert_eq!(plan::bug_verdict(&panic), Some(Verdict::Crash));
    assert_eq!(plan::bug_verdict("the radius is too large"), None);
}
