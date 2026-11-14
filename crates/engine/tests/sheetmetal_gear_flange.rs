//! A sheet-metal arm whose flange's outline carries a gear sector (A-019,
//! `sheetmetal_findings/gear-flange.json`): a web 163 × 38, 3 thick, one
//! flange bent up 90 degrees at radius 3 whose outline holds four teeth of
//! a ten-tooth gear (root, fillet and tip arcs and involutes drawn as
//! lines, one tooth patterned round the gear's centre as a chained ring,
//! so the root arcs of neighbouring teeth meet end to end on one circle),
//! relief notches at the web's ends and a round end.
//!
//! It was refused at values a billionth apart from values that built:
//! pitch radius 19 mm failed, 19 + 1e-9 built, 19 + 1e-6 failed; with the
//! two holes the plate's own check refused it ("loop 3 of face 2 does not
//! lie where the face's other loops leave it room"), without them the
//! self-intersection check could not mesh it ("the boundary crosses itself
//! in parameter space"). The cause was the plate's corner where two root
//! arcs meet on one circle: whether the boundary runs on smoothly there
//! was read from the cosine of the turn against the cosine of the angular
//! tolerance, which is 1 in floating point, so a dot of the two tangents
//! one unit in the last place above −1 made the corner a crease, solved on
//! the two rims (one cylinder twice), and it slid 1.4 to 26 mm round the
//! circle (docs/design/plates.md › *Corners*). Held here: the values that
//! failed build, a sweep of their neighbours builds, every body to the
//! closed form below, and a sector turned into the flange's bend or across
//! its own outline is refused for the reason that holds.
//!
//! The closed form (independent of the kernel): the flange's outline area
//! `A_f` by Green's theorem over its lines and arcs, the web's `163 × 38`,
//! the bend's `θ w T (R + T/2)` with `θ = π/2`, `w = 163`; the volume
//! `T (A_web + A_f − holes) + θ w T (R + T/2)`, the area both skins of the
//! flats, the bend's two cylinders `θ w (2R + T)`, the rims (the outlines'
//! perimeters but the bend's tangent lines, and the holes', times `T`) and
//! the bend's two ends `θ ((R + T)² − R²)`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{body, run};
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::PI;

const STEPS: &str = include_str!("sheetmetal_findings/gear-flange.json");

/// An arc of the outline: its start, end and centre, radius and signed
/// sweep (counter-clockwise positive).
type Arc = ([f64; 2], [f64; 2], [f64; 2], f64, f64);

/// The two holes of the flange (the gear's pivot, 11, and the trunnion
/// at the arm's end, 12), drawn before the outline's closing line as the
/// part's author drew them.
fn holes() -> [(String, Value); 2] {
    let circle = |x: &str, d: &str| {
        (
            "sketch.circle".to_string(),
            json!({"sketch": "Plus flange",
                   "center": [x, "(web_half + (sizes[\"all\"].pivot_height) - bend_tangent)"],
                   "radius": d}),
        )
    };
    [
        circle("0 mm", "pin_hole / 2"),
        circle("arm_length", "trunnion_hole / 2"),
    ]
}

/// A session with the arm at pitch radius `pitch` (mm) and the sector's
/// middle `middle` (degrees), with or without the holes; the feature's
/// result.
fn arm(pitch: &str, middle: &str, with_holes: bool) -> (Session, Value) {
    let mut steps: Vec<(String, Value)> = serde_json::from_str(STEPS).unwrap();
    for (id, params) in &mut steps {
        if id == "variable.set" && params["name"] == "pitch_radius" {
            params["expr"] = json!(format!("{pitch} mm"));
        }
        if id == "variable.set" && params["name"] == "mesh_middle" {
            params["expr"] = json!(format!("{middle} deg"));
        }
    }
    if with_holes {
        let at = steps
            .iter()
            .position(|(id, _)| id == "sketch.line")
            .unwrap();
        for (k, step) in holes().into_iter().enumerate() {
            steps.insert(at + k, step);
        }
    }
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let mut last = Value::Null;
    for (id, params) in &steps {
        last = run(&mut s, id, params.clone());
    }
    (s, last)
}

/// The arm in closed form at pitch radius `p` (mm) and sector middle `m`
/// (degrees): its volume and area.
fn closed_form(p: f64, m: f64, with_holes: bool) -> (f64, f64) {
    let (pivot_height, end_radius, teeth) = (24.0, 13.0, 10.0);
    let pressure = 25.0_f64.to_radians();
    let (addendum_factor, root_fillet, relief_height, relief_fillet, backlash) =
        (0.8, 0.8, 4.0, 1.5, 0.4);
    let (web_start, web_end_back, arm, inner, t, r) = (3.0, 24.0, 190.0, 44.0, 3.0, 3.0);
    let module = 2.0 * p / teeth;
    let tooth = 2.0 * PI / teeth;
    let web_half = inner / 2.0 - r;
    let centre = [0.0, web_half + pivot_height - (t + r)];
    let base = p * pressure.pcos();
    let tip = p + addendum_factor * module;
    let root = p - module;
    let fc = root + root_fillet;
    let turn = (root_fillet / fc).pasin();
    let foot = (fc * fc - root_fillet * root_fillet).sqrt();
    let half_base = tooth / 4.0 - backlash / (4.0 * p) + (pressure.ptan() - pressure);
    let roll = ((tip / base).ppowi(2) - 1.0).sqrt();
    let half_tip = half_base - (roll - roll.patan());
    let first = m.to_radians() - 1.5 * tooth;
    let polar = |c: [f64; 2], rad: f64, a: f64| [c[0] + rad * a.pcos(), c[1] + rad * a.psin()];
    let at = |rad: f64, a: f64| polar(centre, rad, a);
    // Twice the area by Green's theorem and the perimeter, segment by
    // segment; an arc by its centre, radius and signed sweep.
    let (mut twice, mut perimeter) = (0.0_f64, 0.0_f64);
    let mut line = |a: [f64; 2], b: [f64; 2]| {
        twice += a[0] * b[1] - b[0] * a[1];
        perimeter += ((b[0] - a[0]).ppowi(2) + (b[1] - a[1]).ppowi(2)).sqrt();
    };
    let mut lines: Vec<([f64; 2], [f64; 2])> = Vec::new();
    let mut arcs: Vec<Arc> = Vec::new();
    let involute = |k: f64, side: f64, f: f64| {
        let q = roll * k / 4.0;
        at(
            base * (1.0 + q * q).sqrt(),
            f + side * (half_base - (q - q.patan())),
        )
    };
    for i in 0..4 {
        let f = first + f64::from(i) * tooth;
        let r0 = at(root, f - tooth / 2.0);
        let r1 = at(root, f - half_base - turn);
        arcs.push((r0, r1, centre, root, tooth / 2.0 - half_base - turn));
        let f0 = at(foot, f - half_base);
        // The fillet turns clockwise from the root circle to the flank:
        // through the right angle less the turn.
        arcs.push((
            r1,
            f0,
            at(fc, f - half_base - turn),
            root_fillet,
            -(PI / 2.0 - turn),
        ));
        let b0 = at(base, f - half_base);
        lines.push((f0, b0));
        let mut prev = b0;
        for k in [1.0, 2.0, 3.0] {
            let q = involute(k, -1.0, f);
            lines.push((prev, q));
            prev = q;
        }
        let t0 = at(tip, f - half_tip);
        lines.push((prev, t0));
        let t1 = at(tip, f + half_tip);
        arcs.push((t0, t1, centre, tip, 2.0 * half_tip));
        prev = t1;
        for k in [3.0, 2.0, 1.0] {
            let q = involute(k, 1.0, f);
            lines.push((prev, q));
            prev = q;
        }
        let b1 = at(base, f + half_base);
        lines.push((prev, b1));
        let f1 = at(foot, f + half_base);
        lines.push((b1, f1));
        let r2 = at(root, f + half_base + turn);
        arcs.push((
            f1,
            r2,
            at(fc, f + half_base + turn),
            root_fillet,
            -(PI / 2.0 - turn),
        ));
        let r3 = at(root, f + tooth / 2.0);
        arcs.push((r2, r3, centre, root, tooth / 2.0 - half_base - turn));
    }
    let start = at(root, first - tooth / 2.0);
    let end = at(root, first + 3.0 * tooth + tooth / 2.0);
    let q0 = [web_start - relief_fillet, web_half + relief_height];
    let q1 = [web_start, web_half + relief_height - relief_fillet];
    let q2 = [web_start, web_half];
    let q3 = [arm - web_end_back, web_half];
    let q4 = [arm - web_end_back, web_half + relief_height - relief_fillet];
    let q5 = [arm - web_end_back + relief_fillet, web_half + relief_height];
    let q6 = [arm, centre[1] - end_radius];
    let q7 = [arm, centre[1] + end_radius];
    lines.push((end, q0));
    arcs.push((q0, q1, [q0[0], q1[1]], relief_fillet, -PI / 2.0));
    lines.push((q1, q2));
    lines.push((q2, q3));
    lines.push((q3, q4));
    arcs.push((q4, q5, [q5[0], q4[1]], relief_fillet, -PI / 2.0));
    lines.push((q5, q6));
    arcs.push((q6, q7, [arm, centre[1]], end_radius, PI));
    lines.push((q7, start));
    for (a, b) in lines {
        line(a, b);
    }
    for (a, b, c, rad, sweep) in arcs {
        // The arc's ends lie on its circle, and its start turned by its
        // sweep is its end: the closed form is checked against itself.
        for e in [a, b] {
            let d = ((e[0] - c[0]).ppowi(2) + (e[1] - c[1]).ppowi(2)).sqrt();
            assert!(
                (d - rad).abs() <= 1e-9,
                "an arc's end off its circle by {}",
                d - rad
            );
        }
        let from = (a[1] - c[1]).patan2(a[0] - c[0]);
        let reached = polar(c, rad, from + sweep);
        assert!(
            (reached[0] - b[0]).abs() <= 1e-9 && (reached[1] - b[1]).abs() <= 1e-9,
            "an arc's sweep does not reach its end"
        );
        twice += rad * rad * sweep + c[0] * (b[1] - a[1]) - c[1] * (b[0] - a[0]);
        perimeter += rad * sweep.abs();
    }
    let flange = 0.5 * twice;
    let w = q3[0] - q2[0];
    let web = w * 2.0 * web_half;
    let web_perimeter = 2.0 * w + 4.0 * web_half;
    let (hole_area, hole_perimeter) = if with_holes {
        let d = [11.0_f64, 12.0];
        (
            d.iter().map(|d| PI * d * d / 4.0).sum::<f64>(),
            d.iter().map(|d| PI * d).sum::<f64>(),
        )
    } else {
        (0.0, 0.0)
    };
    let theta = PI / 2.0;
    let flats = web + flange - hole_area;
    let volume = t * flats + theta * w * t * (r + t / 2.0);
    let area = 2.0 * flats
        + theta * w * (2.0 * r + t)
        + t * (web_perimeter - w + perimeter - w + hole_perimeter)
        + theta * ((r + t).ppowi(2) - r * r);
    (volume, area)
}

/// The arm built at `pitch` and `middle`, to its closed form.
#[track_caller]
fn builds(pitch: &str, middle: &str, with_holes: bool) {
    let (mut s, result) = arm(pitch, middle, with_holes);
    assert_eq!(
        result["status"], "ok",
        "pitch {pitch}, middle {middle}, holes {with_holes}: {result}"
    );
    let found = body(&mut s, "Arm");
    assert_eq!(found["valid"], true, "{found}");
    let (v, a) = closed_form(pitch.parse().unwrap(), middle.parse().unwrap(), with_holes);
    let (got_v, got_a) = (
        found["volume"].as_f64().unwrap(),
        found["area"].as_f64().unwrap(),
    );
    assert!(
        (got_v - v).abs() <= 1e-9 * v && (got_a - a).abs() <= 1e-9 * a,
        "pitch {pitch}, middle {middle}, holes {with_holes}: volume {got_v} against {v} ({:e}), area {got_a} against {a} ({:e})",
        (got_v - v) / v,
        (got_a - a) / a
    );
}

#[test]
fn the_values_that_were_refused_build_to_the_closed_form() {
    // Each was refused before the fix (pitch radius 19 at 138 degrees,
    // 19 + 1e-6, and 138 + 1e-9 degrees at 19), with and without holes.
    for with_holes in [false, true] {
        for (pitch, middle) in [("19", "138"), ("19.000001", "138"), ("19", "138.000000001")] {
            builds(pitch, middle, with_holes);
        }
    }
    // The deep check of the document at the first of them.
    let (mut s, _) = arm("19", "138", true);
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

#[test]
fn the_neighbouring_values_build_to_the_closed_form() {
    // Steps of 1e-9 and 1e-6 about both values, written as decimals: before
    // the fix 14 of these 24 were refused (of 200 values each way, 95 and
    // 103 pitch radii and 168 and 159 sector middles), as a coin falls.
    for k in [-3_i32, -2, -1, 1, 2, 3] {
        for (step, digits) in [(1e-9, 9), (1e-6, 6)] {
            let pitch = format!("{:.*}", digits, 19.0 + f64::from(k) * step);
            builds(&pitch, "138", false);
            let middle = format!("{:.*}", digits, 138.0 + f64::from(k) * step);
            builds("19", &middle, false);
        }
    }
}

#[test]
fn a_sector_turned_into_the_flange_is_refused_for_that() {
    // Turned to 200 degrees the teeth reach below the bend line, so the
    // flat would lie across its own bend; turned to 230 they run across the
    // line to the relief notch, so the outline crosses itself. Each is
    // refused for that reason, nothing is built and nothing crashes.
    for (middle, reason) in [
        (
            "200",
            "the flange's outline reaches back across its edge's line",
        ),
        ("230", "sketch `Plus flange`"),
    ] {
        let (mut s, result) = arm("19", middle, false);
        assert_eq!(result["status"], "failed", "{result}");
        let error = result["error"].as_str().unwrap();
        assert!(error.contains(reason), "{middle}: {error}");
        assert!(!error.contains("internal error"), "{error}");
        if middle == "230" {
            assert!(error.contains("cross"), "{error}");
        }
        let inspect = run(&mut s, "document.inspect", json!({}));
        assert!(
            inspect["bodies"]
                .as_array()
                .unwrap()
                .iter()
                .all(|b| b["name"] != "Arm"),
            "{inspect}"
        );
    }
}
