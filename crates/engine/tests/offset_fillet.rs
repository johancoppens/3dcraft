//! Rolling-ball fillets along the rims of faces an offset or a shell
//! made, through the commands: the reproducers of the bike's saddle
//! shelled first and filleted after (`docs/design/face-operations.md`,
//! open item 4 of 2026-10-16), whose inner rims were refused. A small
//! loft through two closed splines (the saddle's middle sections), its
//! side moved out and in by 2 mm and its start rim filleted 3 mm, and
//! the loft shelled 2 mm with the rim of the hollow filleted 1 mm; each
//! built, checked deep, its fillet faces named, and its volume changed
//! the way the fillet goes (less at an outside rim, more at the inside
//! one) by the amount a ball of that radius rounding a corner of about a
//! right angle all along the rim gives. The closed forms are the kernel's
//! (`crates/kernel/tests/offset_fillet.rs`: the same on an exact twin of
//! a cylinder); the whole saddle is `#[ignore]`d (minutes in release).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::plain::{body, built, run, session};
use craft3d_engine::Session;
use serde_json::json;
use std::f64::consts::PI;

/// Sections on planes x = const: x, then the points a closed spline runs
/// through (y, z), mm.
type Section<'a> = (f64, &'a [[f64; 2]]);

/// A loft through closed splines, named `Raw`.
fn loft(s: &mut Session, sections: &[Section<'_>]) {
    run(s, "file.new", json!({"units": "mm"}));
    let mut names = Vec::new();
    for (i, (x, points)) in sections.iter().enumerate() {
        let name = format!("S{i}");
        built(
            s,
            "sketch.create",
            json!({"plane": "yz", "name": name, "offset": format!("{x} mm")}),
        );
        run(
            s,
            "sketch.spline",
            json!({"sketch": name, "through": points, "closed": true}),
        );
        names.push(name);
    }
    built(s, "feature.loft", json!({"sections": names, "name": "Raw"}));
}

/// The saddle's middle: from x = 20 to 85 mm.
const MIDDLE: [Section<'static>; 2] = [
    (
        20.0,
        &[
            [36.0, 10.0],
            [27.0, 32.0],
            [0.0, 40.0],
            [-27.0, 32.0],
            [-36.0, 10.0],
            [-21.6, 0.0],
            [0.0, -2.0],
            [21.6, 0.0],
        ],
    ),
    (
        85.0,
        &[
            [24.0, 11.0],
            [18.0, 30.8],
            [0.0, 38.0],
            [-18.0, 30.8],
            [-24.0, 11.0],
            [-14.4, 2.0],
            [0.0, 0.2],
            [14.4, 2.0],
        ],
    ),
];

/// The rim of the start cap of the body `source` (its faces named with
/// `prefix`) filleted with radius `r` (mm), as a chain: checked deep, its
/// fillet faces named, and what it changed in the volume against a ball
/// rounding a right-angled corner along `rim` mm (`sign` −1 at an
/// outside rim, +1 at an inside one).
fn filleted(s: &mut Session, source: &str, prefix: &str, r: f64, sign: f64, rim: f64) {
    let before = body(s, source)["volume"].as_f64().unwrap();
    built(
        s,
        "feature.fillet",
        json!({
            "edges": [[format!("{prefix}F3:startCap(e11)"), format!("{prefix}F3:side(e11,e11)")]],
            "radius": format!("{r} mm"),
            "chain": true,
            "name": "Rounded",
        }),
    );
    let check = run(s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    let rounded = body(s, "Rounded");
    let names: Vec<&str> = rounded["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    // The rim is two edges (the loft's side is two faces), one fillet
    // face each.
    let fillets = names.iter().filter(|n| n.contains(":fillet(0,")).count();
    assert_eq!(fillets, 2, "{names:?}");
    let change = rounded["volume"].as_f64().unwrap() - before;
    // A ball rounds off r² (cot(θ/2) − (π − θ)/2) of a corner of θ. The
    // side leans by up to about 10 degrees off square to the cap where
    // the loft is widest and little elsewhere (a corner of 80 degrees
    // loses half again what a right angle does), and the polygon through
    // the spline's points is a few per cent shorter than the spline: the
    // change is a fifth more than the right angle's along the polygon,
    // held to within 30 %. A fillet missing along part of the rim, or of
    // another radius, is outside that.
    let square = r * r * (1.0 - PI / 4.0) * rim;
    println!(
        "{source} rim {r} mm: volume changed by {change} (a right angle along {rim} mm: {})",
        sign * square
    );
    assert!(change * sign > 0.0, "{change}");
    assert!(
        (change.abs() - square).abs() <= 0.3 * square,
        "{change} against {square}"
    );
}

/// The perimeter of the closed polygon through `points`: a little less
/// than the closed spline through them. Used only for an estimate.
fn polygon(points: &[[f64; 2]]) -> f64 {
    (0..points.len())
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            (a[0] - b[0]).phypot(a[1] - b[1])
        })
        .sum()
}

/// The loft's side moved by `d` mm and its start rim filleted 3 mm.
fn moved_then_filleted(d: f64) {
    let outline = polygon(MIDDLE[0].1);
    let mut s = session();
    loft(&mut s, &MIDDLE);
    built(
        &mut s,
        "feature.offsetFace",
        json!({"faces": ["F3:side(e11,e11)"], "distance": format!("{d} mm"), "name": "Off"}),
    );
    // The outline moved by d (the side leans little): its length by
    // 2π d.
    filleted(
        &mut s,
        "Off",
        "F4:copy(0)/",
        3.0,
        -1.0,
        outline + 2.0 * PI * d,
    );
}

#[test]
fn the_rim_of_a_lofted_side_moved_out_is_filleted() {
    moved_then_filleted(2.0);
}

#[test]
fn the_rim_of_a_lofted_side_moved_in_is_filleted() {
    moved_then_filleted(-2.0);
}

#[test]
fn the_rim_of_the_hollow_of_a_shelled_loft_is_filleted() {
    let mut s = session();
    loft(&mut s, &MIDDLE);
    built(
        &mut s,
        "feature.shell",
        json!({"source": "Raw", "thickness": "2 mm", "name": "Shell"}),
    );
    let outline = polygon(MIDDLE[0].1) - 2.0 * PI * 2.0;
    filleted(&mut s, "Shell", "F4:copy(1)/", 1.0, 1.0, outline);
}

/// Sections across the bike's saddle (a showcase model): x, half width,
/// height, lift of the underside (mm).
const SADDLE: [(f64, f64, f64, f64); 6] = [
    (-125.0, 70.0, 40.0, 12.0),
    (-95.0, 76.0, 46.0, 4.0),
    (-40.0, 62.0, 44.0, 0.0),
    (20.0, 36.0, 40.0, 0.0),
    (85.0, 24.0, 36.0, 2.0),
    (140.0, 18.0, 30.0, 8.0),
];

/// The bike's saddle shelled 2 mm first and its four rims filleted after,
/// as its designer does it: outside 4 mm at the rear and 3 mm at the
/// front, inside 2 mm and 1 mm. Each fillet builds and the body checks
/// deep; what each changes in the volume is held to an independent
/// estimate made from a fine mesh of the shell before them (per piece of
/// rim, the corner a ball of the radius rounds off at the angle measured
/// there, times the piece's length, Pappus's rule for the rim's curvature
/// in the cap's plane; the side's curvature across the rim left out, a
/// few per cent at these radii): 680.5, 158.1, 170.7 and 16.2 mm³ against
/// the built 706.3, 159.3, 174.1 and 16.2 (2026-10-16,
/// `docs/design/face-operations.md`). Minutes in release: `cargo test
/// --release -p craft3d-engine --test offset_fillet -- --ignored`.
#[test]
#[ignore = "minutes: the bike's saddle shelled and its four rims filleted; run with --ignored"]
fn the_bike_saddle_shelled_first_takes_its_four_rim_fillets() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let mut names = Vec::new();
    for (i, (x, w, h, lift)) in SADDLE.iter().enumerate() {
        let name = format!("S{i}");
        built(
            &mut s,
            "sketch.create",
            json!({"plane": "yz", "name": name, "offset": format!("{x} mm")}),
        );
        let points: Vec<[f64; 2]> = [
            (*w, lift + 0.25 * h),
            (0.75 * w, lift + 0.8 * h),
            (0.0, lift + h),
            (-0.75 * w, lift + 0.8 * h),
            (-w, lift + 0.25 * h),
            (-0.6 * w, *lift),
            (0.0, lift - 0.05 * h),
            (0.6 * w, *lift),
        ]
        .iter()
        .map(|(a, b)| [*a, *b])
        .collect();
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": name, "through": points, "closed": true}),
        );
        names.push(name);
    }
    built(
        &mut s,
        "feature.loft",
        json!({"sections": names, "name": "SaddleRaw"}),
    );
    built(
        &mut s,
        "feature.shell",
        json!({"source": "SaddleRaw", "thickness": "2 mm", "name": "Shell"}),
    );
    let side = "F7:side(e11,e11,e11,e11,e11,e11)";
    let mut volume = body(&mut s, "Shell")["volume"].as_f64().unwrap();
    for (name, skin, cap, r, estimate) in [
        ("OuterRear", 0, "startCap", 4.0, -680.5),
        ("OuterFront", 0, "endCap", 3.0, -158.1),
        ("InnerRear", 1, "startCap", 2.0, 170.7),
        ("InnerFront", 1, "endCap", 1.0, 16.2),
    ] {
        built(
            &mut s,
            "feature.fillet",
            json!({
                "edges": [[format!("F8:copy({skin})/F7:{cap}(e11)"), format!("F8:copy({skin})/{side}")]],
                "radius": format!("{r} mm"),
                "chain": true,
                "name": name,
            }),
        );
        let now = body(&mut s, name)["volume"].as_f64().unwrap();
        let change: f64 = now - volume;
        println!("{name}: {change} mm³, estimated {estimate}");
        assert!(
            (change - estimate).abs() <= 0.05 * estimate.abs(),
            "{name}: {change} against {estimate}"
        );
        volume = now;
    }
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}
