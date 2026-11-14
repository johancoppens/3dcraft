//! Offsets of faces of copies, measured against closed forms: whichever
//! copy a face is on (an instance of a pattern turned about an axis, a
//! mirror image, which turns its orientation round), an offset by `d`
//! changes the volume by what `d` says and in its direction, inward and
//! outward, on flat and on round faces.
//!
//! And the finding that brought them (2026-11-14,
//! docs/design/face-operations.md › *An offset that measured smaller*):
//! in the action matrix the rotor's turned blade got 10.08 mm³ smaller
//! when its end cap moved out by 0.055 mm, where the cap adds 0.0013. The
//! body was right (its tip cut out by a block measures the closed form to
//! 1e-8 of the change); its measured volume was not: the new edge of the
//! cap lies up to 8.7e-8 mm past the border of the side face continued for
//! it, and the integrator summed rates along it that were not its feet's,
//! so the loop of feet did not close; and the blade's leading and trailing
//! edges, which do not move, were made again as fitted curves 1.9e-7 mm
//! off their faces where they had been exact. Seen from the middle of the
//! three blades, 4.65 m off the axis of a turned one, each was a strip
//! along the 20 m blade.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, failed, names_of, new_document, run};
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::PI;

/// The volume of the body of `feature`, and how far it may be off by the
/// integrator's own account.
fn volume(session: &mut Session, feature: &str) -> (f64, f64) {
    let m = run(
        session,
        "document.measure",
        json!({"feature": feature, "bound": true, "inertia": false}),
    );
    (
        m["volume"].as_f64().unwrap(),
        m["volumeBound"].as_f64().unwrap(),
    )
}

/// The volume after `feature.offsetFace` of `face` by `d`, which is then
/// undone; checked deep.
fn offset_volume(session: &mut Session, face: &str, d: f64) -> (f64, f64) {
    built(
        session,
        "feature.offsetFace",
        json!({"faces": [face], "distance": format!("{d} mm"), "name": "Off"}),
    );
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{face} by {d}: {check}");
    let measured = volume(session, "Off");
    run(session, "edit.undo", json!({}));
    measured
}

/// The rotor's last two stations (`bench_models/rotor.steps.json`): at 20 m
/// and 21.5 m along z, each a closed loop of two cubic Béziers (control
/// points in mm), the first spline's start tangent to the second's end
/// (the leading edge), a corner where they meet otherwise (the trailing
/// edge).
const STATION_6: [[[f64; 2]; 4]; 2] = [
    [
        [-0.239991, -0.002094],
        [-0.24049, 0.055103],
        [-0.000908, 0.103996],
        [0.559979, 0.004887],
    ],
    [
        [0.559979, 0.004887],
        [0.120495, -0.056151],
        [-0.239582, -0.048893],
        [-0.239991, -0.002094],
    ],
];
const STATION_7: [[[f64; 2]; 4]; 2] = [
    [[-0.165, 0.0], [-0.165, 0.0363], [0.0, 0.066], [0.385, 0.0]],
    [
        [0.385, 0.0],
        [0.0825, -0.0363],
        [-0.165, -0.0297],
        [-0.165, 0.0],
    ],
];
const Z_6: f64 = 20_000.0;
const Z_7: f64 = 21_500.0;

fn station(session: &mut Session, name: &str, z: f64, loops: &[[[f64; 2]; 4]; 2]) {
    run(
        session,
        "sketch.create",
        json!({"plane": "xy", "offset": format!("{z} mm"), "name": name}),
    );
    let [first, second] = loops;
    run(
        session,
        "sketch.spline",
        json!({"sketch": name, "control": first}),
    );
    run(
        session,
        "sketch.spline",
        json!({"sketch": name, "control": ["e4", second[1], second[2], "e1"]}),
    );
}

/// The point and the tangent of a cubic Bézier at `t`.
fn bezier(p: &[[f64; 2]; 4], t: f64) -> ([f64; 2], [f64; 2]) {
    let s = 1.0 - t;
    let at = |i: usize| {
        s * s * s * p[0][i]
            + 3.0 * s * s * t * p[1][i]
            + 3.0 * s * t * t * p[2][i]
            + t * t * t * p[3][i]
    };
    let rate = |i: usize| {
        3.0 * s * s * (p[1][i] - p[0][i])
            + 6.0 * s * t * (p[2][i] - p[1][i])
            + 3.0 * t * t * (p[3][i] - p[2][i])
    };
    ([at(0), at(1)], [rate(0), rate(1)])
}

/// The mixed area `¼ ∮ (a × db + b × da)` of two loops whose pieces are
/// matched in order (the area of a loop with itself), by Gauss–Legendre
/// with three nodes per piece: exact, the integrand is of degree five.
fn mixed(a: &[[[f64; 2]; 4]; 2], b: &[[[f64; 2]; 4]; 2]) -> f64 {
    // The three nodes on [0, 1]: ½ ± √15 / 10 and ½.
    let r = (0.15_f64).sqrt();
    let nodes = [
        (0.5 - r, 5.0 / 18.0),
        (0.5, 8.0 / 18.0),
        (0.5 + r, 5.0 / 18.0),
    ];
    let mut sum = 0.0;
    for (pa, pb) in a.iter().zip(b) {
        for (t, w) in nodes {
            let ((p, dp), (q, dq)) = (bezier(pa, t), bezier(pb, t));
            sum += 0.25 * w * (p[0] * dq[1] - p[1] * dq[0] + q[0] * dp[1] - q[1] * dp[0]);
        }
    }
    sum
}

/// What the end cap of the loft from station 6 to 7 moved out by `d`
/// adds to the volume (taken away for a negative `d`). The loft through
/// two sections is ruled, `(1 − λ) P₆ + λ P₇` at height `z₆ + λ L`, and
/// its continuation past station 7 is the same expression for `λ > 1`, so
/// its section has the area `(1 − λ)² A₆₆ + 2 λ (1 − λ) A₆₇ + λ² A₇₇`
/// (the loops' areas and their mixed area), a quadratic: Simpson's rule
/// over `[1, 1 + d / L]` is exact. The loops run clockwise: the areas are
/// taken with their sign turned.
fn tip_gain(d: f64) -> f64 {
    let (a66, a77, a67) = (
        -mixed(&STATION_6, &STATION_6),
        -mixed(&STATION_7, &STATION_7),
        -mixed(&STATION_6, &STATION_7),
    );
    let length = Z_7 - Z_6;
    let area = |l: f64| (1.0 - l) * (1.0 - l) * a66 + 2.0 * l * (1.0 - l) * a67 + l * l * a77;
    let (a, b) = (1.0, 1.0 + d / length);
    length * (b - a) / 6.0 * (area(a) + 4.0 * area(0.5 * (a + b)) + area(b))
}

/// The blade of the rotor's last two stations, `Blade` (F3), patterned
/// three times about y as the rotor's blades are: `Blades` (F4), three
/// lumps. The middle the integrator takes the moments about, the middle
/// of their box (0, 0, 5375), lies on the axis of the first and 4.65 m off
/// the axes of the two turned ones.
fn blades(session: &mut Session) {
    new_document(session, 1.0);
    station(session, "Station6", Z_6, &STATION_6);
    station(session, "Station7", Z_7, &STATION_7);
    built(
        session,
        "feature.loft",
        json!({"sections": ["Station6", "Station7"], "name": "Blade"}),
    );
    built(
        session,
        "feature.pattern",
        json!({"source": "Blade", "count": "3", "angle": "120 deg", "axis": [0, 1, 0], "name": "Blades"}),
    );
}

/// The finding, reduced: each blade's end cap moved out and in by 0.055
/// mm changes the volume by the closed form of its ruled tip
/// (±0.0013379), within a hundredth of the change (measured within 0.41 %
/// on all six). Before, the first blade measured 0.0013669 and the turned
/// ones −0.1322 (the wrong way) and +0.1347 for +0.055, +0.1332 and −0.1360
/// for −0.055.
#[test]
fn a_turned_blades_end_cap_offset_changes_the_volume_by_its_closed_form() {
    let mut session = common::session();
    blades(&mut session);
    let (v0, b0) = volume(&mut session, "Blades");
    for copy in 0..3 {
        let face = format!("F4:copy({copy})/F3:endCap(e5,e8)");
        for d in [0.055, -0.055] {
            let (v1, b1) = offset_volume(&mut session, &face, d);
            let (change, want) = (v1 - v0, tip_gain(d));
            assert!(
                (change - want).abs() <= 1e-2 * want.abs(),
                "{face} by {d}: the volume changed by {change}, not {want} (bounds {b0}, {b1})"
            );
        }
    }
}

/// Move face shares the closing of the body with offset face
/// (`kernel::tweak`): each end cap moved 0.055 mm into its blade along
/// the blade's axis takes the closed form of the ruled tip (moved out, a
/// freeform side face is not continued by move face, and it is refused).
/// Before, the turned blades measured +0.1200 (the wrong way) and −0.1360,
/// where the closed form is −0.0013379.
#[test]
fn a_turned_blades_end_cap_moved_in_changes_the_volume_by_its_closed_form() {
    let mut session = common::session();
    blades(&mut session);
    let (v0, _) = volume(&mut session, "Blades");
    let d = -0.055;
    for copy in 0..3_u32 {
        let face = format!("F4:copy({copy})/F3:endCap(e5,e8)");
        // The blade's axis, z turned about y by 120° per copy.
        let (s, c) = (f64::from(copy) * 2.0 * PI / 3.0).psin_cos();
        let shift: Vec<String> = [s, 0.0, c]
            .iter()
            .map(|a| format!("{:.15} mm", d * a))
            .collect();
        built(
            &mut session,
            "feature.moveFace",
            json!({"faces": [face], "translate": shift, "name": "Moved"}),
        );
        let check = run(&mut session, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{face}: {check}");
        let (v1, _) = volume(&mut session, "Moved");
        run(&mut session, "edit.undo", json!({}));
        let (change, want) = (v1 - v0, tip_gain(d));
        assert!(
            (change - want).abs() <= 1e-2 * want.abs(),
            "{face} moved by {d}: the volume changed by {change}, not {want}"
        );
    }
}

/// The three blades mirrored (a mirror image turns the orientation of
/// every face round) across the plane x = 5 and across the plane y = 0,
/// in which their axes lie: each end cap moved out adds to the volume,
/// moved in takes from it, by the closed form (across y = 0 one blade's,
/// for the time the checked commands take). Before, the first mirror
/// measured 0.0013702 for +0.055 on its first blade.
#[test]
fn a_mirrored_blades_end_cap_offset_changes_the_volume_by_its_closed_form() {
    for (normal, name, take) in [
        ([1.0, 0.0, 0.0], "across x = 5", 3),
        ([0.0, 1.0, 0.0], "across y = 0", 1),
    ] {
        let mut session = common::session();
        blades(&mut session);
        built(
            &mut session,
            "feature.mirror",
            json!({"source": "Blades", "normal": normal, "point": [5.0, 0.0, 0.0], "name": "Mirrored"}),
        );
        let caps: Vec<String> = names_of(&body(&mut session, "Mirrored"))
            .into_iter()
            .filter(|n| n.contains("endCap"))
            .collect();
        assert_eq!(caps.len(), 3, "{name}: {caps:?}");
        let (v0, _) = volume(&mut session, "Mirrored");
        for cap in caps.iter().rev().take(take) {
            for d in [0.055, -0.055] {
                let (v1, _) = offset_volume(&mut session, cap, d);
                let (change, want) = (v1 - v0, tip_gain(d));
                assert!(
                    (change - want).abs() <= 1e-2 * want.abs(),
                    "{name}, {cap} by {d}: the volume changed by {change}, not {want}"
                );
            }
        }
    }
}

/// A rod of radius 2 and height 5 standing 10 from the z axis (F1),
/// patterned three times about z (F2) or mirrored across x = 0 (F2): its
/// wall offset by `d` changes the volume by `π h ((r + d)² − r²)`, its top
/// by `π r² d`, on every copy, outward and inward.
#[test]
fn offsets_of_the_faces_of_copies_change_the_volume_their_way() {
    let (r, h) = (2.0, 5.0);
    let copies: [(&str, Value, Vec<String>); 2] = [
        (
            "feature.pattern",
            json!({"source": "Rod", "count": "3", "angle": "120 deg", "axis": [0, 0, 1], "name": "Copies"}),
            (0..3).map(|k| format!("F2:copy({k})")).collect(),
        ),
        (
            "feature.mirror",
            json!({"source": "Rod", "normal": [1, 0, 0], "name": "Copies"}),
            vec!["F2:copy(0)".to_string()],
        ),
    ];
    for (command, params, instances) in copies {
        let mut session = common::session();
        new_document(&mut session, 1.0);
        built(
            &mut session,
            "feature.cylinder",
            json!({"radius": format!("{r} mm"), "height": format!("{h} mm"), "position": ["10 mm", "0 mm", "0 mm"], "name": "Rod"}),
        );
        built(&mut session, command, params.clone());
        let (v0, _) = volume(&mut session, "Copies");
        for instance in &instances {
            for d in [0.3, -0.3] {
                for (face, want) in [
                    ("face(0)", PI * h * ((r + d) * (r + d) - r * r)),
                    ("face(2)", PI * r * r * d),
                ] {
                    let name = format!("{instance}/F1:{face}");
                    let (v1, _) = offset_volume(&mut session, &name, d);
                    let change = v1 - v0;
                    assert!(
                        (change - want).abs() <= 1e-9 * v0,
                        "{command} {name} by {d}: the volume changed by {change}, not {want}"
                    );
                }
            }
        }
    }
}

/// What does not fit is refused with the reason, not built wrong: the
/// turned blade's end cap moved in by more than the blade is long, a
/// distance that is no length, and one that is not finite.
#[test]
fn an_offset_of_a_copys_face_that_cannot_be_built_is_refused() {
    let mut session = common::session();
    blades(&mut session);
    let face = "F4:copy(1)/F3:endCap(e5,e8)";
    let why = failed(
        &mut session,
        "feature.offsetFace",
        json!({"faces": [face], "distance": "-2000 mm", "name": "TooFar"}),
    );
    assert!(!why.is_empty() && !why.contains("internal"), "{why}");
    run(&mut session, "edit.undo", json!({}));
    let (v0, _) = volume(&mut session, "Blades");
    // Refused, or a feature that fails with the reason: never a body.
    for distance in [
        json!("a lot"),
        json!("1/0 mm"),
        json!([1, 2]),
        json!("0 mm"),
    ] {
        let params = json!({"faces": [face], "distance": distance, "name": "Bad"});
        match session.execute("feature.offsetFace", &params) {
            Err(e) => assert!(!e.message().contains("internal error"), "{distance}: {e}"),
            Ok(result) => {
                assert_eq!(result["status"], "failed", "{distance}: {result}");
                let why = result["error"].as_str().unwrap();
                assert!(
                    !why.is_empty() && !why.contains("internal"),
                    "{distance}: {why}"
                );
                run(&mut session, "edit.undo", json!({}));
            }
        }
    }
    assert_eq!(volume(&mut session, "Blades").0, v0);
}
