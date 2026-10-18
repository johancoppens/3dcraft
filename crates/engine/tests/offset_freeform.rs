//! Shell of freeform parts through the command, held to an independent
//! volume: the area of the surfaces parallel to the skin at the depth `s`
//! is a quadratic in `s` while their topology holds (Steiner), so the
//! volume between the skin and the hollow is Simpson's rule over the
//! areas at 0, `t / 2` and `t`, exactly; the area at `t / 2` is that of
//! the hollow of a second shell, `t / 2` thick. Every face of the hollow
//! is within a quarter of the tolerance of its exact offset, so the volume
//! is held to that over the areas.
//!
//! The gate's test is a smooth loft through three circles of different
//! sizes on offset, raised centres (a tube that tapers and bends, its side
//! one rational spline surface), ends flat, at a tolerance of 1e-4 mm.
//! The bike's saddle itself (plan/bike.md,
//! `target/work/bike/gen.py`: six sections, its rims rounded by
//! rolling-ball fillets, which "the offset of a freeform face is not
//! built" refused before) takes minutes and is `#[ignore]`d: run it with
//! `cargo test --release -p craft3d-engine --test offset_freeform --
//! --ignored`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{body, built, run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// Sections across a saddle's length on planes x = const: x, half width,
/// height, lift of the underside (mm).
type Section = (f64, f64, f64, f64);

/// A saddle-like part, its clamp at the origin: a closed spline on every
/// section, a smooth loft through them named `Saddle`, or `SaddleRaw` with
/// its first and last rims rounded by 4 and 3 mm into `Saddle` when
/// `rounded`.
fn saddle(s: &mut Session, sections: &[Section], rounded: bool, tolerance: f64) {
    run(
        s,
        "file.new",
        json!({"units": "mm", "tolerance": tolerance}),
    );
    let mut names = Vec::new();
    for (i, (x, w, h, lift)) in sections.iter().enumerate() {
        let name = format!("S{i}");
        built(
            s,
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
            s,
            "sketch.spline",
            json!({"sketch": name, "through": points, "closed": true}),
        );
        names.push(name);
    }
    let loft = if rounded { "SaddleRaw" } else { "Saddle" };
    built(s, "feature.loft", json!({"sections": names, "name": loft}));
    if rounded {
        let id = sections.len() + 1;
        let side = format!("F{id}:side({})", vec!["e9"; sections.len()].join(","));
        built(
            s,
            "feature.fillet",
            json!({"edges": [[format!("F{id}:startCap(e9)"), side.clone()]], "radius": "4 mm", "chain": true, "name": "SaddleRear"}),
        );
        built(
            s,
            "feature.fillet",
            json!({"edges": [[format!("F{id}:endCap(e9)"), side]], "radius": "3 mm", "chain": true, "name": "Saddle"}),
        );
    }
}

/// A tube lofted smooth through circles on planes x = const (x, height
/// of the centre, radius, mm), named `Saddle` as the saddle is.
fn tube(s: &mut Session, circles: &[(f64, f64, f64)], tolerance: f64) {
    run(
        s,
        "file.new",
        json!({"units": "mm", "tolerance": tolerance}),
    );
    let mut names = Vec::new();
    for (i, (x, z, r)) in circles.iter().enumerate() {
        let name = format!("S{i}");
        built(
            s,
            "sketch.create",
            json!({"plane": "yz", "name": name, "offset": format!("{x} mm")}),
        );
        run(
            s,
            "sketch.circle",
            json!({"sketch": name, "center": [0.0, z], "radius": r}),
        );
        names.push(name);
    }
    built(
        s,
        "feature.loft",
        json!({"sections": names, "name": "Saddle"}),
    );
}

/// The part `make` builds, made hollow by `thickness` (mm): the skin's
/// area, the hollow's, the volume, and the body; checked `deep` when
/// asked.
fn hollow(make: &dyn Fn(&mut Session), thickness: f64, deep: bool) -> (f64, f64, f64, Value) {
    let mut s = session();
    make(&mut s);
    let skin = body(&mut s, "Saddle")["area"].as_f64().unwrap();
    built(
        &mut s,
        "feature.shell",
        json!({"source": "Saddle", "thickness": format!("{thickness} mm"), "name": "Shell"}),
    );
    let shell = body(&mut s, "Shell");
    let check = run(&mut s, "document.check", json!({"deep": deep}));
    assert_eq!(check["ok"], true, "{check}");
    let total = shell["area"].as_f64().unwrap();
    let volume = shell["volume"].as_f64().unwrap();
    (skin, total - skin, volume, shell)
}

/// The part `make` builds at `tolerance`, shelled `t` thick: checked deep,
/// every face of it twice (skin and cavity, the cavity's named
/// `copy(1)`), and its volume against Simpson's rule over the wall areas.
fn shelled_to_its_wall_areas(make: &dyn Fn(&mut Session), tolerance: f64, t: f64) {
    let (skin, inner, volume, shell) = hollow(make, t, true);
    let faces = shell["faces"].as_u64().unwrap();
    let names: Vec<&str> = shell["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert_eq!(
        2 * names.iter().filter(|n| n.contains(":copy(1)/")).count() as u64,
        faces,
        "{names:?}"
    );
    let (_, middle, _, _) = hollow(make, 0.5 * t, false);
    let simpson = t / 6.0 * (skin + 4.0 * middle + inner);
    let slack = 0.25 * tolerance * (skin + inner);
    assert!(
        (volume - simpson).abs() <= slack,
        "volume {volume} against {simpson} from the areas {skin}, {middle}, {inner} (off by {:e}, slack {slack:e})",
        volume - simpson
    );
}

#[test]
fn a_lofted_tube_made_hollow_has_the_volume_its_wall_areas_give() {
    let circles = [(-95.0, 25.0, 30.0), (20.0, 20.0, 20.0), (140.0, 22.0, 12.0)];
    shelled_to_its_wall_areas(&|s: &mut Session| tube(s, &circles, 1e-4), 1e-4, 2.0);
}

/// The bike's saddle, rims rounded, at the default tolerance, shelled
/// 2 mm: built, and checked deep. Its rolling-ball fillets are tangent to
/// the faces beside them only to what fitting left (to 6.6e-5 at 1e-4
/// mm), so offsetting each along its own normal parted them by more than
/// the tolerance and was refused (docs/decisions.md, 2026-10-16); the
/// fillets' faces of the hollow are now the fillets rebuilt between the
/// faces of the hollow with the radius less the thickness, tangent to
/// them by construction (docs/design/face-operations.md). The oracle is
/// the other order, the designer's: the loft shelled first and its four
/// rims filleted after, outside 4 and 3 mm, inside 2 and 1 mm, the same
/// solid, within half the tolerance times its area.
/// Minutes in release: `cargo test --release -p craft3d-engine --test
/// offset_freeform -- --ignored`.
#[test]
#[ignore = "minutes: the bike's saddle with its rolling-ball fillets; run with --ignored"]
fn the_bike_saddle_shelled_2_mm_is_the_saddle_shelled_first_and_filleted() {
    let sections = [
        (-125.0, 70.0, 40.0, 12.0),
        (-95.0, 76.0, 46.0, 4.0),
        (-40.0, 62.0, 44.0, 0.0),
        (20.0, 36.0, 40.0, 0.0),
        (85.0, 24.0, 36.0, 2.0),
        (140.0, 18.0, 30.0, 8.0),
    ];
    let mut s = session();
    saddle(&mut s, &sections, true, 1e-6);
    built(
        &mut s,
        "feature.shell",
        json!({"source": "Saddle", "thickness": "2 mm", "name": "Shell"}),
    );
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    let shell = body(&mut s, "Shell");
    let (volume, area) = (
        shell["volume"].as_f64().unwrap(),
        shell["area"].as_f64().unwrap(),
    );
    // The other order.
    let mut first = session();
    saddle(&mut first, &sections, false, 1e-6);
    built(
        &mut first,
        "feature.shell",
        json!({"source": "Saddle", "thickness": "2 mm", "name": "Shell"}),
    );
    let side = "F7:side(e9,e9,e9,e9,e9,e9)";
    for (name, skin, cap, r) in [
        ("OuterRear", 0, "startCap", 4.0),
        ("OuterFront", 0, "endCap", 3.0),
        ("InnerRear", 1, "startCap", 2.0),
        ("InnerFront", 1, "endCap", 1.0),
    ] {
        built(
            &mut first,
            "feature.fillet",
            json!({
                "edges": [[format!("F8:copy({skin})/F7:{cap}(e9)"), format!("F8:copy({skin})/{side}")]],
                "radius": format!("{r} mm"),
                "chain": true,
                "name": name,
            }),
        );
    }
    let other = body(&mut first, "InnerFront")["volume"].as_f64().unwrap();
    let slack = 0.5 * 1e-6 * area;
    println!(
        "the saddle shelled after its fillets: {volume} mm³; shelled first and filleted: {other} mm³ (off {:e}, slack {slack:e})",
        volume - other
    );
    assert!((volume - other).abs() <= slack);
}
