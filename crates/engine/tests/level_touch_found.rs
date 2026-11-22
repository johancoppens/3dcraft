//! A union whose level faces meet along a line where the two bodies part,
//! as the random designs found it (fuzz seeds 110965, 116424, 111609 and
//! 115646, 2026-11-21; docs/design/booleans.md › *A union whose level tops
//! meet along a line where the bodies part*): each was refused as
//! *"boolean produced an invalid body"*, the boolean's own check of what it
//! had made, where the result would touch itself along that line. Here the
//! four seeds as found (`level_touch_found/seed-<n>.json`, the commands as
//! the fuzzer made them) and their reduction through the commands: a block
//! cut back to its top edge beside a block level with it, at four scales,
//! either way round, cut back by a plane and by a tangent cylinder, turned
//! out of the axes by `feature.transform`; and the neighbour that builds.
//! The kernel's side is `crates/kernel/tests/boolean_level_touch.rs` and
//! the unit tests of `boolean/merge.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{assert_solid, built, failed, failure, new_document, run, session};
use craft3d_engine::Session;
use craft3d_math::{Portable, Vec3};
use serde_json::{Value, json};

/// The seeds' scales (1e-3, 1 and 1e3) and the far one.
const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

/// The words of the refusal, up to the place.
const ZERO_THICK: &str = "the result would have zero thickness at (";

/// The place a refusal as zero thickness names: `… at (x, y, z): …`.
fn place(error: &str, words: &str) -> Vec3 {
    let start = error
        .find(words)
        .unwrap_or_else(|| panic!("not refused as zero thickness: {error}"))
        + words.len();
    let rest = &error[start..];
    let end = rest.find(')').unwrap();
    let c: Vec<f64> = rest[..end]
        .split(", ")
        .map(|n| n.parse().unwrap())
        .collect();
    Vec3::new(c[0], c[1], c[2])
}

/// Whether `at` lies on the line where the blocks part (x = 5, z = 5,
/// y from 5 to 10, times `k`), within the tolerance and the rounding of
/// the printed place.
fn on_the_line(at: Vec3, k: f64) -> bool {
    let near = 2e-6 * k;
    (at.x - 5.0 * k).abs() <= near
        && (at.z - 5.0 * k).abs() <= near
        && at.y >= 5.0 * k - near
        && at.y <= 10.0 * k + near
}

/// The deep check's problems: exactly one, of the feature `name`, which
/// is not an invalid body; its words.
fn the_one_problem(s: &mut Session, name: &str) -> String {
    let check = run(s, "document.check", json!({"deep": true}));
    let problems = check["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1, "{check}");
    assert_eq!(problems[0]["name"], name, "{check}");
    let words = problems[0]["problem"].as_str().unwrap().to_string();
    assert!(!words.contains("invalid body"), "{words}");
    assert_eq!(check["marginal"], 0, "{check}");
    words
}

/// Block `A` (5 × 10 × 5, times `k`) cut back over y 5..10 to its top
/// edge, as `X`: by the seed's sketch on the zx plane at y = 5 extruded 6
/// (the plane x = 3 + 0.4 z), or by the cylinder of radius 8 about the
/// line x = -3, z = 5 (tangent to the side x = 5 along the top edge).
/// Then block `B` beside it, its top at `top` (times `k`).
fn cut_back(s: &mut Session, k: f64, round: bool, top: f64) {
    built(
        s,
        "feature.block",
        json!({"name": "A", "position": [0, 0, 0], "size": [5.0 * k, 10.0 * k, 5.0 * k]}),
    );
    if round {
        built(
            s,
            "feature.block",
            json!({"name": "Q", "position": [0.0, 5.0 * k, 0.0], "size": [5.0 * k, 5.0 * k, 5.0 * k]}),
        );
        built(
            s,
            "feature.cylinder",
            json!({"name": "C", "position": [-3.0 * k, 4.0 * k, 5.0 * k], "axis": [0, 1, 0],
                   "radius": 8.0 * k, "height": 7.0 * k}),
        );
        built(
            s,
            "feature.boolean",
            json!({"name": "T", "op": "subtract", "target": "Q", "tool": "C"}),
        );
    } else {
        run(
            s,
            "sketch.create",
            json!({"name": "S", "plane": "zx", "offset": 5.0 * k}),
        );
        let points: Vec<[f64; 2]> = [[6.0, 5.4], [-1.0, 2.6], [-1.0, 7.0], [6.0, 7.0]]
            .iter()
            .map(|p| [p[0] * k, p[1] * k])
            .collect();
        run(
            s,
            "sketch.polyline",
            json!({"sketch": "S", "closed": true, "points": points}),
        );
        built(
            s,
            "feature.extrude",
            json!({"sketch": "S", "distance": 6.0 * k, "name": "T"}),
        );
    }
    built(
        s,
        "feature.boolean",
        json!({"name": "X", "op": "subtract", "target": "A", "tool": "T"}),
    );
    built(
        s,
        "feature.block",
        json!({"name": "B", "position": [5.0 * k, 0, 0], "size": [5.0 * k, 10.0 * k, top * k]}),
    );
}

/// `v` turned by `degrees` about the direction `axis`, as
/// `feature.transform` turns a body (Rodrigues' formula).
fn turned(v: Vec3, axis: Vec3, degrees: f64) -> Vec3 {
    let axis = axis.normalized().unwrap();
    let (sin, cos) = degrees.to_radians().psin_cos();
    v * cos + axis.cross(v) * sin + axis * (axis.dot(v) * (1.0 - cos))
}

/// The reduction of the four seeds through the commands: refused as zero
/// thickness at the line, in the boolean's words with the faces' names,
/// either way round, cut back by a plane and by a tangent cylinder; the
/// deep check's one problem is that refusal. Before the repair: *"boolean
/// produced an invalid body: loop 1 of face 1 … runs over itself along
/// edges 11 and 12 (a stretch 5 long about (5, 7.5, 5))"*.
#[test]
fn a_block_cut_back_to_its_top_edge_beside_a_block_is_refused_at_the_line() {
    for k in SCALES {
        for round in [false, true] {
            for swapped in [false, true] {
                let mut s = session();
                new_document(&mut s, k);
                cut_back(&mut s, k, round, 5.0);
                let (target, tool) = if swapped { ("B", "X") } else { ("X", "B") };
                let error = failed(
                    &mut s,
                    "feature.boolean",
                    json!({"name": "Y", "op": "union", "target": target, "tool": tool}),
                );
                let what = format!("scale {k}, round {round}, swapped {swapped}");
                let at = place(&error, ZERO_THICK);
                assert!(on_the_line(at, k), "{what}: {error}");
                assert!(
                    error.contains("two of its parts touch there"),
                    "{what}: {error}"
                );
                assert_eq!(the_one_problem(&mut s, "Y"), error, "{what}");
            }
        }
    }
}

/// The same turned out of the axes by `feature.transform`, both blocks
/// about one pivot: the place is on the line turned with them.
#[test]
fn turned_out_of_the_axes_by_the_commands() {
    for k in SCALES {
        for (axis, degrees, pivot) in [
            ([0.3, -0.4, 1.0], 33.0, [1.0, -2.0, 0.5]),
            ([2.0, -1.0, 0.5], 143.0, [-3.0, 0.7, 11.0]),
        ] {
            for round in [false, true] {
                let mut s = session();
                new_document(&mut s, k);
                cut_back(&mut s, k, round, 5.0);
                let pivot = Vec3::new(pivot[0], pivot[1], pivot[2]) * k;
                for (source, name) in [("X", "Xt"), ("B", "Bt")] {
                    built(
                        &mut s,
                        "feature.transform",
                        json!({"source": source, "angle": format!("{degrees} deg"), "axis": axis,
                               "pivot": [pivot.x, pivot.y, pivot.z], "name": name}),
                    );
                }
                let error = failed(
                    &mut s,
                    "feature.boolean",
                    json!({"name": "Y", "op": "union", "target": "Xt", "tool": "Bt"}),
                );
                let axis = Vec3::new(axis[0], axis[1], axis[2]);
                let at = place(&error, ZERO_THICK);
                let back = pivot + turned(at - pivot, axis, -degrees);
                assert!(
                    on_the_line(back, k),
                    "scale {k}, {degrees} degrees, round {round}: {back:?}: {error}"
                );
                assert_eq!(the_one_problem(&mut s, "Y"), error);
            }
        }
    }
}

/// Beside it. B's top 0.05 higher (its side then stands above the line):
/// refused in the same words, as it was before the repair, the level case
/// and this one now answered alike. B moved 0.05 into A (they overlap
/// along the whole side): built, 225 + 250 − 1.25 − 0.015625, the last the
/// sliver of B above the plane x = 3 + 0.4 z over y 5..10.
#[test]
fn what_lies_beside_it_through_the_commands() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        cut_back(&mut s, k, false, 5.05);
        let error = failed(
            &mut s,
            "feature.boolean",
            json!({"name": "Y", "op": "union", "target": "X", "tool": "B"}),
        );
        assert!(on_the_line(place(&error, ZERO_THICK), k), "{error}");
        let mut s = session();
        new_document(&mut s, k);
        cut_back(&mut s, k, false, 5.0);
        built(
            &mut s,
            "feature.transform",
            json!({"source": "B", "translate": [-0.05 * k, 0, 0], "name": "Into"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"name": "Y", "op": "union", "target": "X", "tool": "Into"}),
        );
        assert_solid(&mut s, "Y", 473.734_375, None, k);
    }
}

/// Replays a seed as found, or with its last step's radius times
/// `radius`; the session after it.
fn replay_with(seed: &str, radius: Option<f64>) -> Session {
    let text = match seed {
        "110965" => include_str!("level_touch_found/seed-110965.json"),
        "116424" => include_str!("level_touch_found/seed-116424.json"),
        "111609" => include_str!("level_touch_found/seed-111609.json"),
        "115646" => include_str!("level_touch_found/seed-115646.json"),
        _ => panic!("no seed {seed}"),
    };
    let mut steps: Vec<Value> = serde_json::from_str(text).unwrap();
    if let Some(radius) = radius {
        let last = steps.last_mut().unwrap();
        let asked = last["params"]["radius"].as_f64().unwrap();
        last["params"]["radius"] = json!(asked * radius);
    }
    let mut s = session();
    for step in &steps {
        run(
            &mut s,
            step["command"].as_str().unwrap(),
            step["params"].clone(),
        );
    }
    s
}

fn replay(seed: &str) -> Session {
    replay_with(seed, None)
}

/// Seed 111609 as found (scale 1e-3, tolerance 1e-9): a quarter turn of an
/// L, cut by a plane, moved, and united with its mirror image, the two
/// tangent along their start faces' common edge. Refused as zero thickness
/// at that line, naming a face of each.
#[test]
fn seed_111609_as_found() {
    let mut s = replay("111609");
    let error = failure(&mut s, "N6");
    let at = place(&error, ZERO_THICK);
    assert!(
        (at.x - 0.0025).abs() < 1e-8 && (at.y - 0.005).abs() < 1e-8,
        "{error}"
    );
    assert_eq!(the_one_problem(&mut s, "N6"), error);
}

/// Seed 115646 as found (scale 1e3, tolerance 1e-3): a pattern of two whose
/// instances touch along the line where a ring's cut leaves a crescent
/// beside the next instance's block. The pattern's join is refused as zero
/// thickness at that line.
#[test]
fn seed_115646_as_found() {
    let mut s = replay("115646");
    let error = failure(&mut s, "N6");
    assert!(
        error.starts_with("instance 0 and instance 1 touch or overlap and cannot be joined"),
        "{error}"
    );
    let at = place(&error, "zero thickness at (");
    assert!((at.x + 7500.0).abs() < 1e-3 && at.y.abs() < 1e-3, "{error}");
    assert_eq!(the_one_problem(&mut s, "N6"), error);
}

/// Seeds 110965 (scale 1) and 116424 (scale 1e3) as found: two or three
/// edges of a body filleted together, a convex edge whose round is tangent
/// to a side along its top edge (the radius is the side's height) and a
/// concave edge beside it whose tool sits level with the top: the
/// reduction's touch, inside the fillet's steps. The fillet is refused no
/// longer as an invalid body but as it is refused at a smaller radius,
/// where the round does not reach the top edge and nothing touches: word
/// for word what it says at 0.8 of the radius (*"the fillet does not end
/// cleanly: at one end of the edge it would stop in a face of its own …"*,
/// 110965's second edge, 116424's first), which the base said as well at
/// 0.8 (and on the seeds' reductions at 0.4 and 0.8 of the radius, on
/// 110965's also at 0.998: measured on the base's program). Whether those
/// words are true of the corner where a
/// convex and a concave fillet meet is the blend's, not this finding's: a
/// regression test, not a proof.
#[test]
fn seeds_110965_and_116424_as_found() {
    for (seed, name, edge) in [("110965", "N5", "edges.1 ("), ("116424", "N2", "edges.0 (")] {
        let mut s = replay(seed);
        let error = failure(&mut s, name);
        assert!(error.starts_with(edge), "{seed}: {error}");
        assert!(
            error.contains("the fillet does not end cleanly"),
            "{seed}: {error}"
        );
        assert_eq!(the_one_problem(&mut s, name), error, "{seed}");
        let mut smaller = replay_with(seed, Some(0.8));
        assert_eq!(
            failure(&mut smaller, name),
            error,
            "{seed} at 0.8 of its radius"
        );
    }
}
