//! Through the commands: the floor of a round pocket under a spline wall
//! raised past the wall's lowest point over it, where the pocket's rim
//! runs out through the wall. The feature fails, saying how far the floor
//! can go: offset face asks the spline wall whether the rim went through
//! it. Short of the wall the floor is built, and is the pocket the boolean
//! cuts to that height.
//!
//! Until 2026-11-15 the floor 0.02 mm past the wall's lowest point was
//! built with status ok and passed `document.check` with `deep` (the
//! second review of 2026-11-14; pinned here as `BUILT_THROUGH_KNOWN`,
//! taken off with the repair): no freeform face was asked, and at that
//! depth the mesh the regeneration looks at shows no crossing. The
//! kernel's side of this, with the other kinds of case and three scales:
//! `crates/kernel/tests/face_op_freeform_known.rs`.
//!
//! The floor of a rectangular slot under the same wall, raised past the
//! wall's lowest point over it, where the floor's straight edges leave
//! through the wall between their ends: fails, saying how far the floor
//! can go, from 0.0005 mm past (until the review of 2026-11-15 of the
//! freeform faces asked again, 0.0005 and 0.002 mm past were built with
//! status ok, and from 0.02 mm the refusal said a distance 0.016 mm too far).
//!
//! **Known wrong, owed** (the same review): a face moved a little way into
//! the inside of a freeform face, with no vertex of the body in its way and
//! no edge made again through a face, is built with status ok and passes
//! `document.check` with `deep` ([`KNOWN_INTO_FREEFORM`]): a block's bottom
//! moved past the pole of a turned bowl in its top, a spline face moved up
//! into a flat arm over it, and into a spline arm. Each is shown wrong by
//! its volume against the body the boolean makes of the same parts. The
//! day one is refused its test fails until it is taken off the list.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{body, built, new_document, run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// The height of the wall's spline over x 20.5, where it is lowest over
/// the pocket.
fn lowest() -> f64 {
    let cubic = |t: f64, p: [f64; 4]| {
        let u = 1.0 - t;
        u * u * u * p[0] + 3.0 * u * u * t * p[1] + 3.0 * u * t * t * p[2] + t * t * t * p[3]
    };
    // x falls from 25 to 5 along the spline.
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if cubic(mid, [25.0, 18.0, 12.0, 5.0]) > 20.5 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    cubic(0.5 * (lo + hi), [20.0, 14.0, 27.0, 20.0])
}

/// A block 30 x 20, 40 long, its top between x 5 and x 25 a spline wall,
/// with a round pocket of radius 3 from its bottom up to `floor`, the
/// pocket's seam turned `turn` degrees about its axis: the body
/// `Pocketed`.
fn pocketed(floor: f64, turn: f64) -> Session {
    let mut s = session();
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Outline"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Outline", "points": [[5.0, 20.0], [0.0, 20.0], [0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [25.0, 20.0]]}),
    );
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Outline", "control": [[25.0, 20.0], [18.0, 14.0], [12.0, 27.0], [5.0, 20.0]]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 40.0, "name": "Prism"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 3.0, "height": floor + 5.0, "position": [17.5, -5.0, 20.0], "axis": [0.0, 1.0, 0.0], "angle": format!("{turn} deg"), "name": "Tool"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Prism", "tool": "Tool", "name": "Pocketed"}),
    );
    s
}

/// The pocket's floor: the tool's end cap.
const FLOOR: &str = "F4:copy(1)/F3:face(2)";

#[test]
fn a_pockets_floor_raised_out_through_a_spline_wall_fails() {
    let mut unexpected = Vec::new();
    for turn in [0.0, 90.0, 180.0, 270.0] {
        for out in [-0.3, -0.001, 0.02, 0.3, 1.0] {
            let to = lowest() + out;
            let mut s = pocketed(12.0, turn);
            let made = run(
                &mut s,
                "feature.offsetFace",
                json!({"faces": [FLOOR], "distance": -(to - 12.0), "name": "Moved"}),
            );
            let check = run(&mut s, "document.check", json!({"deep": true}));
            let what = format!("seam turned {turn}, the floor {out} over the wall's lowest");
            if out < 0.0 {
                // The same pocket cut to that height by the boolean.
                let direct = body(&mut pocketed(to, turn), "Pocketed")["volume"]
                    .as_f64()
                    .unwrap();
                let same = made["status"] == "ok" && check["ok"] == true && {
                    let volume = body(&mut s, "Moved")["volume"].as_f64().unwrap();
                    (volume - direct).abs() <= 1e-9 * direct
                };
                if same {
                    println!("{what}: built right");
                } else {
                    unexpected.push(format!(
                        "{what}: status {} ({}), deep check {}",
                        made["status"], made["error"], check["ok"]
                    ));
                }
                continue;
            }
            // Over the wall: fails, for the wall, with how far the floor
            // can go (to the wall's lowest point: 6.3838 mm).
            let error = made["error"].as_str().unwrap_or_default();
            if made["status"] == "failed"
                && error.contains("would pass through another part of the body")
                && error.contains("what fits is above -6.3838 mm")
            {
                println!("{what}: fails: {error}");
            } else {
                unexpected.push(format!(
                    "{what}: status {} ({error}), deep check {}",
                    made["status"], check["ok"]
                ));
            }
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// The walled block of [`pocketed`] with a slot from its bottom, x from 17
/// to 24 and z from 10 to 30, up to `floor`: the body `Pocketed`.
fn slotted(floor: f64) -> Session {
    let mut t = session();
    new_document(&mut t, 1.0);
    run(
        &mut t,
        "sketch.create",
        json!({"plane": "xy", "name": "Outline"}),
    );
    run(
        &mut t,
        "sketch.polyline",
        json!({"sketch": "Outline", "points": [[5.0, 20.0], [0.0, 20.0], [0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [25.0, 20.0]]}),
    );
    run(
        &mut t,
        "sketch.spline",
        json!({"sketch": "Outline", "control": [[25.0, 20.0], [18.0, 14.0], [12.0, 27.0], [5.0, 20.0]]}),
    );
    built(
        &mut t,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 40.0, "name": "Prism"}),
    );
    built(
        &mut t,
        "feature.block",
        json!({"size": [7.0, floor + 5.0, 20.0], "position": [17.0, -5.0, 10.0], "name": "Tool"}),
    );
    built(
        &mut t,
        "feature.boolean",
        json!({"op": "subtract", "target": "Prism", "tool": "Tool", "name": "Pocketed"}),
    );
    t
}

/// The slot's floor: the tool's top.
const SLOT_FLOOR: &str = "F4:copy(1)/F3:face(3)";

/// The lowest the wall's spline comes over the slot (x from 17 to 24), by
/// thirds: 18.3679.
fn lowest_over_slot() -> f64 {
    let cubic = |t: f64, p: [f64; 4]| {
        let u = 1.0 - t;
        u * u * u * p[0] + 3.0 * u * u * t * p[1] + 3.0 * u * t * t * p[2] + t * t * t * p[3]
    };
    let height = |x: f64| {
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if cubic(mid, [25.0, 18.0, 12.0, 5.0]) > x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        cubic(0.5 * (lo + hi), [20.0, 14.0, 27.0, 20.0])
    };
    let (mut lo, mut hi) = (17.0_f64, 24.0_f64);
    for _ in 0..200 {
        let (p, q) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if height(p) < height(q) {
            hi = q;
        } else {
            lo = p;
        }
    }
    height(0.5 * (lo + hi))
}

/// The distance in "what fits is above <d> mm".
fn what_fits(error: &str) -> Option<f64> {
    let rest = error.split("what fits is above ").nth(1)?;
    rest.split(" mm").next()?.trim().parse().ok()
}

#[test]
fn a_slots_floor_raised_out_through_a_spline_wall_fails_with_what_fits() {
    let low = lowest_over_slot();
    let limit = -(low - 12.0);
    let mut unexpected = Vec::new();
    for past in [-0.01, 0.0005, 0.002, 0.02, 0.3] {
        let to = low + past;
        let mut s = slotted(12.0);
        let made = run(
            &mut s,
            "feature.offsetFace",
            json!({"faces": [SLOT_FLOOR], "distance": -(to - 12.0), "name": "Moved"}),
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        let what = format!("the slot's floor {past} over the wall's lowest");
        if past < 0.0 {
            let direct = body(&mut slotted(to), "Pocketed")["volume"]
                .as_f64()
                .unwrap();
            let same = made["status"] == "ok" && check["ok"] == true && {
                let volume = body(&mut s, "Moved")["volume"].as_f64().unwrap();
                (volume - direct).abs() <= 1e-9 * direct
            };
            if same {
                println!("{what}: built right");
            } else {
                unexpected.push(format!(
                    "{what}: status {} ({}), deep check {}",
                    made["status"], made["error"], check["ok"]
                ));
            }
            continue;
        }
        // Past the wall: fails, saying how far the floor can go, which is
        // to the wall's lowest point over the slot (said as text, to six
        // figures: within 1e-5 of it, and not past it).
        let error = made["error"].as_str().unwrap_or_default();
        let said = what_fits(error);
        let right = said.is_some_and(|d| d >= limit - 1e-5 && d <= limit * (1.0 - 1e-5) + 1e-5);
        if made["status"] == "failed"
            && error.contains("would pass through another part of the body")
            && right
        {
            println!("{what}: fails: {error}");
        } else {
            unexpected.push(format!(
                "{what}: status {} ({error}), what fits {said:?} for {limit}, deep check {}",
                made["status"], check["ok"]
            ));
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// A body of a session by name, if there is one.
fn the_body(s: &mut Session, name: &str) -> Option<Value> {
    let document = run(s, "document.inspect", json!({}));
    document["bodies"]
        .as_array()?
        .iter()
        .find(|b| b["name"] == name)
        .cloned()
}

/// A block 30 x 30, its bottom at `bottom` and its top at 20, about the z
/// axis, with a bowl turned about it taken out from the top (a spline
/// from the pole at 10 up to the rim): the body `Bowled`.
fn bowled(bottom: f64) -> Session {
    let mut s = session();
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "feature.block",
        json!({"size": [30.0, 30.0, 20.0 - bottom], "position": [-15.0, -15.0, bottom], "name": "Block"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "yz", "name": "Bowl"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Bowl", "points": [[9.0, 40.0], [0.0, 40.0], [0.0, 10.0]]}),
    );
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Bowl", "control": [[0.0, 10.0], [5.0, 10.0], [9.0, 22.0], [9.0, 40.0]]}),
    );
    run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Bowl", "axis": "y", "name": "Plug"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Plug", "name": "Bowled"}),
    );
    s
}

/// A spline over x from 30 to 0 (clamped, uniform knots), the top of the
/// prism under the arm.
const BUMP: [[f64; 2]; 6] = [
    [30.0, 20.0],
    [24.0, 19.0],
    [18.0, 25.0],
    [12.0, 23.0],
    [6.0, 18.0],
    [0.0, 20.0],
];

/// The clamped uniform cubic B-spline of `ctrl` at `t` in [0, 1].
fn uniform_cubic(ctrl: &[[f64; 2]], t: f64) -> [f64; 2] {
    let n = ctrl.len();
    let spans = n - 3;
    let mut knots = vec![0.0; 4];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend([1.0; 4]);
    let mut span = 3;
    while span + 1 < n && knots[span + 1] <= t {
        span += 1;
    }
    let mut d: Vec<[f64; 2]> = (0..=3).map(|j| ctrl[j + span - 3]).collect();
    for r in 1..=3 {
        for j in (r..=3).rev() {
            let i = j + span - 3;
            let den = knots[i + 4 - r] - knots[i];
            let a = if den == 0.0 {
                0.0
            } else {
                (t - knots[i]) / den
            };
            d[j] = [
                (1.0 - a) * d[j - 1][0] + a * d[j][0],
                (1.0 - a) * d[j - 1][1] + a * d[j][1],
            ];
        }
    }
    d[3]
}

/// The bump's highest point.
fn bump_peak() -> [f64; 2] {
    (0..=20000)
        .map(|i| uniform_cubic(&BUMP, f64::from(i) / 20000.0))
        .max_by(|a, b| a[1].total_cmp(&b[1]))
        .unwrap()
}

/// The control points of a spline underside of the arm, 3 over the bump's
/// peak and dipping 1.5 over it.
fn ceiling() -> Vec<[f64; 2]> {
    let [xp, yp] = bump_peak();
    let top = yp + 3.0;
    vec![
        [35.0, top],
        [32.0, top],
        [xp + 3.0, top - 1.2],
        [xp, top - 1.5],
        [xp - 3.0, top - 1.2],
        [6.0, top],
        [2.0, top],
    ]
}

/// The height of the clamped uniform cubic of `ctrl` (x falling along it)
/// at `x`.
fn graph_at(ctrl: &[[f64; 2]], x: f64) -> f64 {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..200 {
        let m = 0.5 * (lo + hi);
        if uniform_cubic(ctrl, m)[0] > x {
            lo = m;
        } else {
            hi = m;
        }
    }
    uniform_cubic(ctrl, 0.5 * (lo + hi))[1]
}

/// A prism 30 long under the spline top `BUMP` lifted by `lift`, a tower
/// at x 30, and an arm from the tower over the bump, 2 over its peak: a
/// flat block, or (`spline_arm`) one whose underside is [`ceiling`]. The
/// body `Armed`, and the name of the bump's face.
fn under_arm(lift: f64, spline_arm: bool) -> (Session, String) {
    let arm = bump_peak()[1] + 2.0;
    let mut s = session();
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Bump"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Bump", "points": [[0.0, 20.0 + lift], [0.0, 0.0], [30.0, 0.0], [30.0, 20.0 + lift]]}),
    );
    let lifted: Vec<[f64; 2]> = BUMP.iter().map(|p| [p[0], p[1] + lift]).collect();
    let spline = run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Bump", "control": lifted}),
    );
    let entity = spline["entity"].as_str().unwrap_or("e?").to_string();
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Bump", "distance": 30.0, "name": "Prism"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [5.0, arm + 4.0, 20.0], "position": [30.0, 0.0, 5.0], "name": "Tower"}),
    );
    if spline_arm {
        let c = ceiling();
        let top = c[0][1];
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "offset": 5.0, "name": "Ceiling"}),
        );
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Ceiling", "points": [[2.0, top], [2.0, top + 3.0], [35.0, top + 3.0], [35.0, top]]}),
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Ceiling", "control": c}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Ceiling", "distance": 20.0, "name": "Arm"}),
        );
    } else {
        run(
            &mut s,
            "feature.block",
            json!({"size": [33.0, 3.0, 20.0], "position": [2.0, arm, 5.0], "name": "Arm"}),
        );
    }
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Prism", "tool": "Tower", "name": "Towered"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Towered", "tool": "Arm", "name": "Armed"}),
    );
    let names: Vec<String> = the_body(&mut s, "Armed")
        .and_then(|b| b["faceNames"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|n| n.as_str().map(str::to_string))
        .collect();
    let face = names
        .iter()
        .find(|n| n.ends_with(&format!("F2:side({entity})")))
        .cloned()
        .unwrap_or_else(|| format!("no face for {entity} in {names:?}"));
    (s, face)
}

/// How far the bump has to rise to touch the spline arm (1.6).
fn gap_to_spline_arm() -> f64 {
    let c = ceiling();
    (0..=20000)
        .map(|i| uniform_cubic(&BUMP, f64::from(i) / 20000.0))
        .filter(|p| p[0] > 3.0 && p[0] < 30.0)
        .map(|p| graph_at(&c, p[0]) - p[1])
        .fold(f64::MAX, f64::min)
}

/// **Known wrong, owed**: the cases built with status ok and passing the
/// deep check though the moved face has gone into a freeform face (or a
/// freeform face into a flat one) between its edges, with no vertex in
/// its way and no edge made again through a face: nothing says where to
/// ask (`docs/design/face-operations.md` › *What stays wrong*). Finding
/// them is finding where two faces of one body intersect.
const KNOWN_INTO_FREEFORM: [&str; 3] = [
    "the bottom 0.01 past the bowl's pole",
    "the bump 0.002 into the flat arm",
    "the bump 0.005 into the spline arm",
];

#[test]
fn a_face_moved_a_little_way_into_a_freeform_face_is_known_wrong() {
    let mut unexpected = Vec::new();
    let cases: Vec<(&str, Session, Value, Option<f64>)> = {
        let mut found = Vec::new();
        // A block's bottom moved up 10.01, 0.01 past the pole of the bowl
        // in its top: against the block from 10.01 up with the bowl taken
        // out.
        let mut s = bowled(0.0);
        let made = run(
            &mut s,
            "feature.moveFace",
            json!({"faces": ["F4:copy(0)/F1:face(0)"], "translate": [0.0, 0.0, 10.01], "name": "Moved"}),
        );
        let direct = the_body(&mut bowled(10.01), "Bowled").and_then(|b| b["volume"].as_f64());
        found.push((KNOWN_INTO_FREEFORM[0], s, made, direct));
        // The bump moved up 2.002, 0.002 into the flat arm 2 over its
        // peak: against the union of the bump that much higher and the arm.
        let (mut s, face) = under_arm(0.0, false);
        let made = run(
            &mut s,
            "feature.moveFace",
            json!({"faces": [face], "translate": [0.0, 2.002, 0.0], "name": "Moved"}),
        );
        let direct =
            the_body(&mut under_arm(2.002, false).0, "Armed").and_then(|b| b["volume"].as_f64());
        found.push((KNOWN_INTO_FREEFORM[1], s, made, direct));
        // The bump moved up 0.005 past where it touches the spline arm.
        let d = gap_to_spline_arm() + 0.005;
        let (mut s, face) = under_arm(0.0, true);
        let made = run(
            &mut s,
            "feature.moveFace",
            json!({"faces": [face], "translate": [0.0, d, 0.0], "name": "Moved"}),
        );
        let direct =
            the_body(&mut under_arm(d, true).0, "Armed").and_then(|b| b["volume"].as_f64());
        found.push((KNOWN_INTO_FREEFORM[2], s, made, direct));
        found
    };
    for (what, mut s, made, direct) in cases {
        let check = run(&mut s, "document.check", json!({"deep": true}));
        let volume = the_body(&mut s, "Moved").and_then(|b| b["volume"].as_f64());
        let known = KNOWN_INTO_FREEFORM.contains(&what);
        let wrong = match (volume, direct) {
            (Some(v), Some(d)) => (v - d).abs() > 1e-9 * d.abs(),
            _ => false,
        };
        if made["status"] == "ok" && check["ok"] == true && wrong && known {
            println!(
                "{what}: known wrong: status ok, deep check ok, volume {volume:?} for the boolean's {direct:?}"
            );
        } else if made["status"] == "failed" && known {
            unexpected.push(format!(
                "{what}: fails now ({}): take it off KNOWN_INTO_FREEFORM",
                made["error"]
            ));
        } else {
            unexpected.push(format!(
                "{what}: status {} ({}), deep check {}, volume {volume:?} for {direct:?}",
                made["status"], made["error"], check["ok"]
            ));
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}
