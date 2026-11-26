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
//!
//! Two more seeds of the same kind, first run on 2026-11-22 and refused as
//! zero thickness since this repair (looked at on 2026-11-26): 130205, a
//! pattern whose instances' half discs part along a line of their level
//! flat faces, and 135611, a plain union of two cut cylinders whose level
//! tops meet along a line where one's wall is tangent to the other's cut
//! face. Each as found and reduced (at four scales, either way round,
//! turned), the place held to the line found without the kernel, and the
//! neighbours moved off the touch built to their volumes in closed form.

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
        "130205" => include_str!("level_touch_found/seed-130205.json"),
        "135611" => include_str!("level_touch_found/seed-135611.json"),
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

/// Points scaled by `k`.
fn scaled(points: &[[f64; 2]], k: f64) -> Vec<[f64; 2]> {
    points.iter().map(|p| [p[0] * k, p[1] * k]).collect()
}

/// The integral of √(r² − s²) ds from 0 to `x`: the area under a quarter
/// circle of radius `r` up to `x`.
fn under_circle(r: f64, x: f64) -> f64 {
    0.5 * (x * (r * r - x * x).sqrt() + r * r * (x / r).pasin())
}

/// Seed 130205 reduced, at scale `k`: `N1`, an L in the plane z = 10 turned
/// half a turn about the sketch's y axis (half discs below the plane z = 10
/// about the line x = 0: of radius 10 over y 0..2.5 and of radius 5 over
/// y 2.5..10), joined with an L-shaped prism z 0..10 that reaches into the
/// larger disc; and `M`, a copy of it moved `d` along x (the pattern's
/// step is 10).
fn half_discs(s: &mut Session, k: f64, d: f64) {
    run(
        s,
        "sketch.create",
        json!({"name": "S0", "plane": "xy", "offset": 10.0 * k}),
    );
    let profile = [
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 2.5],
        [5.0, 2.5],
        [5.0, 10.0],
        [0.0, 10.0],
    ];
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "S0", "closed": true, "points": scaled(&profile, k)}),
    );
    built(
        s,
        "feature.revolve",
        json!({"name": "N0", "sketch": "S0", "axis": "y", "angle": "180 deg"}),
    );
    run(
        s,
        "sketch.create",
        json!({"name": "S1", "plane": "xy", "offset": 0.0}),
    );
    let ell = [
        [-7.5, -7.5],
        [2.5, -7.5],
        [2.5, -2.5],
        [-2.5, -2.5],
        [-2.5, 2.5],
        [-7.5, 2.5],
    ];
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "S1", "closed": true, "points": scaled(&ell, k)}),
    );
    built(
        s,
        "feature.extrude",
        json!({"name": "N1", "sketch": "S1", "distance": 10.0 * k, "join": "N0"}),
    );
    built(
        s,
        "feature.transform",
        json!({"name": "M", "source": "N1", "keep": true, "translate": [d * k, 0, 0]}),
    );
}

/// Whether `at` lies on the line where neighbouring half discs of radius 5
/// meet (x = 5, z = 10, y from 2.5 to 10, times `k`). Found without the
/// kernel: the circles x² + (z − 10)² = 25 and (x − 10)² + (z − 10)² = 25
/// are tangent at (5, 10), each disc's flat face (z = 10) and wall meet
/// there at a right angle, and around the line the material of the two is
/// two quarters on either side of x = 5, below z = 10, with the air between
/// their walls closing up at the line: two parts that touch.
fn on_the_discs_line(at: Vec3, k: f64) -> bool {
    let near = 2e-6 * k;
    (at.x - 5.0 * k).abs() <= near
        && (at.z - 10.0 * k).abs() <= near
        && at.y >= 2.5 * k - near
        && at.y <= 10.0 * k + near
}

/// The volume of `N1` and its copy moved `d` (near 10, scale 1) united, in
/// closed form: twice `N1` (the half discs, 125π + 93.75π, and the prism,
/// 750, less where the prism reaches into the larger disc) less what the
/// two share: over y 0..2.5 the lens of the larger discs together with
/// where the copy's prism reaches into the first one's disc, over
/// y 2.5..10 the lens of the smaller discs (none from d = 10 on), and the
/// prisms' overlap over y −7.5..−2.5 (none from d = 10 on).
fn discs_united(d: f64) -> f64 {
    use std::f64::consts::PI;
    let one = 218.75 * PI + 750.0 - 2.5 * (under_circle(10.0, -2.5) - under_circle(10.0, -7.5));
    let large =
        2.0 * under_circle(10.0, 10.0) - under_circle(10.0, 7.5) - under_circle(10.0, d - 7.5);
    let small = if d >= 10.0 {
        0.0
    } else {
        0.5 * (50.0 * (d / 10.0).pacos() - 0.5 * d * (100.0 - d * d).sqrt())
    };
    let prisms = (10.0 - d).max(0.0) * 50.0;
    2.0 * one - (2.5 * large + 7.5 * small + prisms)
}

/// Seed 135611 reduced, at scale `k`: `N1`, a cylinder of radius 5 about the
/// line y = 2.5, z = 7.5 from x = −12.5 to −27.5, less the slab z 7.5..10 cut
/// from its far cap (a half below z = 7.5 and a segment above z = 10);
/// `N4` its mirror image in y = 0 (about y = −2.5); and `N5`, `N4` less the
/// L-shaped pocket sketched on its half's flat top, of which only the box
/// x −27.5..−25, y −2.5..2.5 below z = 7.5 lies in the body.
fn mirrored_halves(s: &mut Session, k: f64) {
    built(
        s,
        "feature.cylinder",
        json!({"name": "N0", "position": [-12.5 * k, 2.5 * k, 7.5 * k], "axis": [-1, 0, 0],
               "angle": "0 deg", "radius": 5.0 * k, "height": 15.0 * k}),
    );
    run(
        s,
        "sketch.create",
        json!({"name": "S1", "face": "F1:face(2)"}),
    );
    let slab = [[-5.0, -2.5], [5.0, -2.5], [5.0, 0.0], [-5.0, 0.0]];
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "S1", "closed": true, "points": scaled(&slab, k)}),
    );
    built(
        s,
        "feature.extrude",
        json!({"name": "N1", "sketch": "S1", "cut": "N0", "reversed": true, "throughAll": true}),
    );
    built(
        s,
        "feature.mirror",
        json!({"name": "N4", "source": "N1", "keep": true, "normal": [0, 1, 0],
               "point": [2.5 * k, 0, 0]}),
    );
    run(
        s,
        "sketch.create",
        json!({"name": "S5", "face": "F5:copy(0)/F4:copy(1)/F3:side(e7)"}),
    );
    let pocket = [
        [0.0, -2.5],
        [10.0, -2.5],
        [10.0, 2.5],
        [5.0, 2.5],
        [5.0, 7.5],
        [0.0, 7.5],
    ];
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "S5", "closed": true, "points": scaled(&pocket, k)}),
    );
    built(
        s,
        "feature.extrude",
        json!({"name": "N5", "sketch": "S5", "cut": "N4", "reversed": true, "throughAll": true}),
    );
}

/// Whether `at` lies on the line where the two cut cylinders touch
/// (y = −2.5, z = 7.5, x from −27.5 to −25, times `k`). Found without the
/// kernel: there the first one's wall, (y − 2.5)² + (z − 7.5)² = 25, is
/// tangent to the plane y = −2.5 and meets its flat top z = 7.5 at a right
/// angle, and the mirror image, its half cut back to y ≤ −2.5 by the
/// pocket, stands on the far side of that plane with its top level, z = 7.5:
/// around the line two quarters of material on either side of y = −2.5,
/// below z = 7.5, with the air between them closing up at the line. From
/// x = −25 on the mirror image's half is whole and holds the first one's
/// wall inside it.
fn on_the_halves_line(at: Vec3, k: f64) -> bool {
    let near = 2e-6 * k;
    (at.y + 2.5 * k).abs() <= near
        && (at.z - 7.5 * k).abs() <= near
        && at.x >= -27.5 * k - near
        && at.x <= -25.0 * k + near
}

/// The volume of `N5` and `N1` moved `t` along y (near 0, scale 1) united,
/// in closed form: each cylinder of 15 · 25π less the slab, 15 times the
/// band of the circle 2.5 wide from its centre, `N5` less a quarter disc
/// 2.5 long for the pocket; less what the two share, 15 times the lower
/// half of the lens of the circles 5 + t apart and its part above z = 10,
/// less the lower half lens over the pocket's 2.5 but for the sliver of it
/// beyond y = −2.5 (only for t < 0).
fn halves_united(t: f64) -> f64 {
    use std::f64::consts::PI;
    let n1 = 15.0 * (25.0 * PI - 2.0 * under_circle(5.0, 2.5));
    let n5 = n1 - 2.5 * 25.0 * PI / 4.0;
    let d = 5.0 + t;
    let lower = 0.5 * (50.0 * (d / 10.0).pacos() - 0.5 * d * (100.0 - d * d).sqrt());
    let top = (25.0 - 0.25 * d * d).sqrt();
    let upper = 2.0 * (under_circle(5.0, top) - under_circle(5.0, 2.5)) - d * (top - 2.5);
    let sliver = if t < 0.0 {
        let a = (25.0 - d * d).sqrt();
        -d * a + under_circle(5.0, a)
    } else {
        0.0
    };
    n1 + n5 - (15.0 * (lower + upper) - 2.5 * (lower - sliver))
}

/// Seed 130205 as found (scale 1, tolerance 1e-6): `half_discs` above,
/// joined as a pattern of four along x by 10. Refused as zero thickness at
/// the line where the first two instances' smaller half discs meet; before
/// this finding's repair as *"loop 9 of face 8 is not a simple closed
/// curve: it runs over itself along edges 27 and 40"*.
#[test]
fn seed_130205_as_found() {
    let mut s = replay("130205");
    let error = failure(&mut s, "N2");
    assert!(
        error.starts_with("instance 0 and instance 1 touch or overlap and cannot be joined"),
        "{error}"
    );
    assert!(error.contains("two of its parts touch there"), "{error}");
    assert!(
        on_the_discs_line(place(&error, "zero thickness at ("), 1.0),
        "{error}"
    );
    assert_eq!(the_one_problem(&mut s, "N2"), error);
}

/// Seed 135611 as found (scale 1, tolerance 1e-6): `mirrored_halves`
/// above, with a turned copy of the first cylinder, a filleted block on it
/// and their union beside, and then `N5` united with `N1`. Refused as zero
/// thickness at the line where the two touch; before this finding's repair
/// as *"loop 7 of face 7 is not a simple closed curve: it runs over itself
/// along edges 21 and 22"*.
#[test]
fn seed_135611_as_found() {
    let mut s = replay("135611");
    let error = failure(&mut s, "N9");
    assert!(error.contains("two of its parts touch there"), "{error}");
    assert!(
        on_the_halves_line(place(&error, ZERO_THICK), 1.0),
        "{error}"
    );
    assert_eq!(the_one_problem(&mut s, "N9"), error);
}

/// The two reduced, at four scales, either way round: refused as zero
/// thickness at their lines, the deep check's one problem.
#[test]
fn the_two_seeds_reduced_are_refused_at_their_lines() {
    for k in SCALES {
        for swapped in [false, true] {
            let what = format!("scale {k}, swapped {swapped}");
            let mut s = session();
            new_document(&mut s, k);
            half_discs(&mut s, k, 10.0);
            let (target, tool) = if swapped { ("M", "N1") } else { ("N1", "M") };
            let error = failed(
                &mut s,
                "feature.boolean",
                json!({"name": "U", "op": "union", "target": target, "tool": tool}),
            );
            assert!(
                error.contains("two of its parts touch there"),
                "{what}: {error}"
            );
            assert!(
                on_the_discs_line(place(&error, ZERO_THICK), k),
                "130205, {what}: {error}"
            );
            assert_eq!(the_one_problem(&mut s, "U"), error, "{what}");

            let mut s = session();
            new_document(&mut s, k);
            mirrored_halves(&mut s, k);
            let (target, tool) = if swapped { ("N1", "N5") } else { ("N5", "N1") };
            let error = failed(
                &mut s,
                "feature.boolean",
                json!({"name": "U", "op": "union", "target": target, "tool": tool}),
            );
            assert!(
                error.contains("two of its parts touch there"),
                "{what}: {error}"
            );
            assert!(
                on_the_halves_line(place(&error, ZERO_THICK), k),
                "135611, {what}: {error}"
            );
            assert_eq!(the_one_problem(&mut s, "U"), error, "{what}");
        }
    }
}

/// The same turned out of the axes by `feature.transform`, both operands
/// about one pivot: the place is on the line turned with them.
#[test]
fn the_two_seeds_reduced_and_turned_by_the_commands() {
    for k in SCALES {
        for (axis, degrees, pivot) in [
            ([0.3, -0.4, 1.0], 33.0, [1.0, -2.0, 0.5]),
            ([2.0, -1.0, 0.5], 143.0, [-3.0, 0.7, 11.0]),
        ] {
            let pivot = Vec3::new(pivot[0], pivot[1], pivot[2]) * k;
            let along = Vec3::new(axis[0], axis[1], axis[2]);
            for discs in [true, false] {
                let mut s = session();
                new_document(&mut s, k);
                let operands = if discs {
                    half_discs(&mut s, k, 10.0);
                    ["N1", "M"]
                } else {
                    mirrored_halves(&mut s, k);
                    ["N5", "N1"]
                };
                for source in operands {
                    built(
                        &mut s,
                        "feature.transform",
                        json!({"source": source, "angle": format!("{degrees} deg"), "axis": axis,
                               "pivot": [pivot.x, pivot.y, pivot.z], "name": format!("{source}t")}),
                    );
                }
                let error = failed(
                    &mut s,
                    "feature.boolean",
                    json!({"name": "U", "op": "union", "target": format!("{}t", operands[0]),
                           "tool": format!("{}t", operands[1])}),
                );
                let at = place(&error, ZERO_THICK);
                let back = pivot + turned(at - pivot, along, -degrees);
                let on = if discs {
                    on_the_discs_line(back, k)
                } else {
                    on_the_halves_line(back, k)
                };
                assert!(
                    on,
                    "scale {k}, {degrees} degrees, discs {discs}: {back:?}: {error}"
                );
                assert_eq!(the_one_problem(&mut s, "U"), error);
            }
        }
    }
}

/// Beside them, at four scales: the copy of 130205 moved 0.01 into the
/// first or away from it, and 135611's `N1` moved 0.01 along y into the
/// mirror image's cut half or away from it, are built, each to its volume
/// in closed form (at scale 1: 2498.505018525272 and 2499.50032001443;
/// 1337.8042666370895 and 1339.4411046731916).
#[test]
fn the_neighbours_off_the_touch_are_built() {
    for k in SCALES {
        for d in [9.99, 10.01] {
            let mut s = session();
            new_document(&mut s, k);
            half_discs(&mut s, k, d);
            built(
                &mut s,
                "feature.boolean",
                json!({"name": "U", "op": "union", "target": "N1", "tool": "M"}),
            );
            assert_solid(&mut s, "U", discs_united(d), None, k);
        }
        for t in [-0.01, 0.01] {
            let mut s = session();
            new_document(&mut s, k);
            mirrored_halves(&mut s, k);
            built(
                &mut s,
                "feature.transform",
                json!({"name": "M", "source": "N1", "keep": true, "translate": [0, t * k, 0]}),
            );
            built(
                &mut s,
                "feature.boolean",
                json!({"name": "U", "op": "union", "target": "N5", "tool": "M"}),
            );
            assert_solid(&mut s, "U", halves_united(t), None, k);
        }
    }
}
