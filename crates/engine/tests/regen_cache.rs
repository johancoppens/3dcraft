//! Regeneration rebuilds only what an edit touches, and what it holds is
//! always what regenerating from nothing gives (the definition of done of
//! M6: incremental = full regeneration).
//!
//! The other suites of this crate check the second half after every
//! command they run; here are the counts, and a seeded random walk.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    let result = session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

/// Runs a command and returns how many features regeneration then rebuilt.
fn rebuilt(session: &mut Session, id: &str, params: Value) -> usize {
    run(session, id, params);
    // Any reader regenerates; this one is cheap.
    run(session, "document.check", json!({}));
    session.last_regeneration().unwrap().0
}

/// Two plates, each from its own sketch and with its own height; a seat
/// and a boss on the first; a row of the second.
///
/// F1 SketchA  F2 PlateA  F3 SketchB  F4 PlateB  F5 Seat  F6 Boss  F7 Row
fn model() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    for (name, expr) in [
        ("ha", "5 mm"),
        ("hb", "3 mm"),
        ("n", "3"),
        ("spare", "1 mm"),
    ] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }
    for (sketch, plate, x, height) in [
        ("SketchA", "PlateA", 0, "ha"),
        ("SketchB", "PlateB", 100, "hb"),
    ] {
        run(&mut s, "sketch.create", json!({"name": sketch}));
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": sketch, "closed": true,
                   "points": [[x, 0], [x + 40, 0], [x + 40, 20], [x, 20]]}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": sketch, "distance": height, "name": plate}),
        );
    }
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Seat", "face": "F2:endCap()"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Seat", "center": [10, 10], "radius": 3}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Seat", "distance": 4, "name": "Boss"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "PlateB", "count": "n", "step": [0, 30, 0], "name": "Row"}),
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    s
}

#[test]
fn an_edit_rebuilds_what_depends_on_it_and_nothing_else() {
    let mut s = model();
    let set = |name: &str, expr: &str| json!({"name": name, "expr": expr});
    // A variable nothing uses: nothing is rebuilt.
    assert_eq!(rebuilt(&mut s, "variable.set", set("spare", "2 mm")), 0);
    // The height of plate B: the plate and the row made from it. Plate A,
    // its seat and its boss are left alone, and so are both sketches.
    assert_eq!(rebuilt(&mut s, "variable.set", set("hb", "4 mm")), 2);
    // The height of plate A: the plate, the seat on its top (it moves up)
    // and the boss on the seat.
    assert_eq!(rebuilt(&mut s, "variable.set", set("ha", "6 mm")), 3);
    // The count of the row: the row alone.
    assert_eq!(rebuilt(&mut s, "variable.set", set("n", "4")), 1);
    // The same value again, written differently: every feature that uses
    // it evaluates to what it had, so nothing is rebuilt.
    assert_eq!(rebuilt(&mut s, "variable.set", set("n", "2 + 2")), 0);
    // A point of sketch B moves: the sketch, plate B and the row.
    assert_eq!(
        rebuilt(
            &mut s,
            "sketch.edit",
            json!({"sketch": "SketchB", "entity": "e2", "at": [150, 0]})
        ),
        3
    );
    // A rename shows in the messages of what is made from the feature, so
    // those are looked at again; they come out the same, and what is made
    // from them (the boss, through the seat) is not rebuilt for it.
    assert_eq!(
        rebuilt(
            &mut s,
            "feature.rename",
            json!({"feature": "PlateA", "name": "Base"})
        ),
        3
    );
    // Undo and redo are edits like any other.
    assert_eq!(rebuilt(&mut s, "edit.undo", json!({})), 3);
    assert_eq!(rebuilt(&mut s, "edit.undo", json!({})), 3);
    assert_eq!(rebuilt(&mut s, "edit.redo", json!({})), 3);
    // Suppressing plate A: it, the seat and the boss (they fail).
    assert_eq!(
        rebuilt(&mut s, "feature.suppress", json!({"feature": "PlateA"})),
        3
    );
    // Rolling back to plate B: everything after it changes state.
    assert_eq!(
        rebuilt(&mut s, "history.rollback", json!({"feature": "PlateB"})),
        3
    );
}

/// xorshift64*: a seeded sequence, the same on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

#[test]
fn a_random_walk_of_edits_never_differs_from_a_full_regeneration() {
    const FEATURES: [&str; 7] = [
        "SketchA", "PlateA", "SketchB", "PlateB", "Seat", "Boss", "Row",
    ];
    let mut steps = 0;
    let mut refused = 0;
    for seed in 1..=40u64 {
        let mut s = model();
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        for _ in 0..250 {
            let feature = rng.pick(&FEATURES);
            let number = rng.below(60) as f64 / 4.0 - 3.0;
            let (id, params) = match rng.below(16) {
                0 => (
                    "variable.set",
                    json!({"name": rng.pick(&["ha", "hb", "spare"]), "expr": format!("{number} mm")}),
                ),
                1 => (
                    "variable.set",
                    json!({"name": "n", "expr": rng.below(6).to_string()}),
                ),
                2 => (
                    "variable.set",
                    json!({"name": rng.pick(&["ha", "n"]), "expr": rng.pick(&["nope", "ha + hb", "1 +", "hb * 2"])}),
                ),
                3 => (
                    "sketch.edit",
                    json!({"sketch": rng.pick(&["SketchA", "SketchB", "Seat"]),
                        "entity": format!("e{}", 1 + rng.below(4)), "at": [number * 9.0, number]}),
                ),
                4 => (
                    "feature.suppress",
                    json!({"feature": feature, "suppressed": rng.below(2) == 0}),
                ),
                5 => (
                    "feature.move",
                    json!({"feature": feature, "position": rng.below(7)}),
                ),
                6 => (
                    "history.rollback",
                    json!({"feature": if rng.below(3) == 0 { Value::Null } else { feature.into() }}),
                ),
                7 | 8 => ("edit.undo", json!({})),
                9 => ("edit.redo", json!({})),
                10 => (
                    "feature.edit",
                    json!({"feature": rng.pick(&["PlateA", "PlateB", "Boss"]),
                        "distance": format!("{number} mm"), "symmetric": rng.below(2) == 0}),
                ),
                11 => (
                    "feature.edit",
                    json!({"feature": "Row", "count": rng.pick(&["n", "2", "n + 1"]),
                        "step": [0, number * 10.0, 0]}),
                ),
                12 => (
                    "sketch.edit",
                    json!({"sketch": "Seat", "face": rng.pick(&[
                        "F2:endCap()", "F2:startCap()", "F2:side(e5)", "F7:copy(1)/F4:endCap()",
                        "F7:copy(0)/F4:side(e6)", "F4:endCap()", "F2:side(e99)"])}),
                ),
                13 => (
                    "feature.rename",
                    json!({"feature": feature, "name": format!("{feature}{}", rng.below(3))}),
                ),
                14 => (
                    "feature.transform",
                    json!({"source": rng.pick(&["PlateA", "Row", "Boss"]),
                        "translate": [number, 0, 0], "angle": format!("{} deg", number * 10.0),
                        "keep": rng.below(2) == 0}),
                ),
                _ => (
                    "feature.delete",
                    json!({"feature": if rng.below(4) == 0 { feature } else { "F8" }}),
                ),
            };
            steps += 1;
            // A refused command (a name that was changed, nothing to undo)
            // is fine; a difference from a full regeneration is not.
            if s.execute(id, &params).is_err() {
                refused += 1;
            }
            assert_ne!(
                s.regeneration_matches_a_full_one(),
                Some(false),
                "seed {seed}, after {id} {params}"
            );
        }
    }
    // The walk is not all refusals.
    assert!(
        refused * 2 < steps,
        "{refused} of {steps} steps were refused"
    );
}
