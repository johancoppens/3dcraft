//! A spar in the notch of a rib, its flat bottom all but in the skin the
//! notch was cut through: clear of the rib, whatever the angle at which
//! the bottom's plane crosses the skin's surface (the RC wing Jig's eight
//! range points, 2026-11-20; design: docs/design/booleans.md › *A spar's
//! face beside a rib's skin*).
//!
//! The bodies are made here, by the commands a Jig uses, in millimetres at
//! the Jig's tolerance of 0.01 mm: a wing blank extruded from an outline
//! with a spline lower side, or lofted from it to the same outline at 0.7
//! of its size 1000 mm further on; a notch cut along it through the lower
//! skin; a rib, the 4 mm slice of it at 200 mm; and a spar, a long block
//! in the notch, 0.03 mm clear of its sides. The rib's skin is a face on
//! the whole blank's skin, a ruled surface 1000 mm long, and the spar's
//! bottom a face 1000 mm long: the kernel meets the two surfaces over the
//! whole span, although the faces are beside each other only at the rib.
//!
//! The spar's bottom follows the skin's lowest line, a ruling, and rises
//! off it at a slope `d`, through it `z0` from the root: the plane crosses
//! the skin in a parabola about that line with its apex at `z0`, where it
//! turns with a radius of `d / 0.002` (0.002 per mm the outline's bend at
//! its lowest point) and the surfaces cross at the sine `d`. At
//! `d = 1e-4` that is a turn 0.05 mm across at a sine of 1e-4; the Jig's
//! own, at its first listed point, 0.0165 mm at 3.3e-5. A marched point
//! was brought within 1e-6 mm of the plane, which at such a sine is
//! anywhere across a band of 0.01 to 0.3 mm about the curve: at eight of
//! the settings below the march stalled at the turn, and
//! `document.interference` answered *unknown* (at the others it went
//! round the turn by luck). A second pair, the spar's bottom 0.02 mm
//! inside the lowest line and rising 0.003, was refused in other words
//! for the same reason: the curve could not be fitted, its points between
//! being as far off it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// `!(x <= y)` is deliberate: unlike `x > y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Portable;
use serde_json::{Value, json};

const TOLERANCE: f64 = 0.01;
const SPAN: f64 = 1000.0;
/// Where the rib is cut from the blank, and its thickness.
const AT: f64 = 200.0;
const SHEET: f64 = 4.0;
/// The spar's clearance to each side of the notch.
const FIT: f64 = 0.03;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// The wing's section at `k` of its size (about the middle of its chord,
/// on the chord line), `offset` along the span: a flat top and a spline
/// lower side whose lowest point is 3 below the chord line at its middle.
fn outline(s: &mut Session, name: &str, offset: f64, k: f64) {
    let at = |x: f64, y: f64| json!([50.0 + (x - 50.0) * k, y * k]);
    run(
        s,
        "sketch.create",
        json!({"name": name, "plane": "xy", "offset": offset}),
    );
    run(
        s,
        "sketch.polyline",
        json!({"sketch": name, "points": [at(0.0, 0.0), at(0.0, 10.0), at(100.0, 10.0), at(100.0, 0.0)]}),
    );
    run(
        s,
        "sketch.spline",
        json!({"sketch": name, "through": [
            "e4", at(75.0, -2.2), at(60.0, -2.9), at(50.0, -3.0), at(40.0, -2.9), at(25.0, -2.2), "e1",
        ]}),
    );
}

/// The rib and the spar: the blank extruded (`taper` 1) or lofted to a
/// tip of `taper` times the root's size, and the spar's bottom `lift`
/// above the root's lowest point where the rib is, rising `slope` along
/// the span.
fn wing(lift: f64, slope: f64, taper: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": TOLERANCE}),
    );
    outline(&mut s, "Root", 0.0, 1.0);
    if taper == 1.0 {
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Root", "distance": SPAN, "name": "Blank"}),
        );
    } else {
        outline(&mut s, "Tip", SPAN, taper);
        run(
            &mut s,
            "feature.loft",
            json!({"sections": ["Root", "Tip"], "name": "Blank"}),
        );
    }
    run(
        &mut s,
        "feature.block",
        json!({"size": [14.0, 3.5, SPAN + 20.0], "position": [43.0, -3.45, -10.0], "name": "Notch"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Blank", "tool": "Notch", "name": "Slotted"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [300.0, 300.0, SHEET], "position": [-100.0, -100.0, AT], "name": "Slab"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "intersect", "target": "Slotted", "tool": "Slab", "name": "Rib"}),
    );
    let angle = slope.patan();
    let middle = AT + 0.5 * SHEET;
    run(
        &mut s,
        "feature.block",
        json!({
            "size": [14.0 - 2.0 * FIT, 3.0 - lift, SPAN],
            "position": [43.0 + FIT, -3.0 + lift - slope * middle, 0.0],
            "axis": [0.0, angle.psin(), angle.pcos()],
            "name": "Spar",
        }),
    );
    s
}

/// The spar is clear of the rib by its fit in the notch, by every way of
/// asking: the distance between them (which asks no boolean), the
/// interference check, their intersection (nothing) and their union (two
/// lumps, the volume of both).
fn clear_in_the_notch(s: &mut Session, what: &str) -> Result<(), String> {
    let out = run(s, "measure.clearance", json!({"from": "Spar", "to": "Rib"}));
    let gap = out["clearance"].as_f64();
    if out["least"]["status"] != "clear" || !gap.is_some_and(|g| (g - FIT).abs() <= 1e-4) {
        return Err(format!("{what}: the clearance: {out}"));
    }
    let out = run(
        s,
        "document.interference",
        json!({"pairs": [["Spar", "Rib"]]}),
    );
    if out["pairs"][0]["status"] != "clear" {
        return Err(format!(
            "{what}: interference: {} {}",
            out["pairs"][0]["status"], out["pairs"][0]["reason"]
        ));
    }
    let volume = |s: &mut Session, body: &str| {
        run(
            s,
            "document.measure",
            json!({"feature": body, "inertia": false}),
        )["volume"]
            .as_f64()
            .unwrap()
    };
    let (spar, rib) = (volume(s, "Spar"), volume(s, "Rib"));
    let common = s.execute(
        "feature.boolean",
        &json!({"op": "intersect", "target": "Spar", "tool": "Rib", "name": "Common"}),
    );
    let nothing = match &common {
        Ok(out) => out["error"]
            .as_str()
            .is_some_and(|e| e.contains("leaves nothing")),
        Err(e) => e.to_string().contains("leaves nothing"),
    };
    if !nothing {
        return Err(format!("{what}: the intersection: {common:?}"));
    }
    let both = s.execute(
        "feature.boolean",
        &json!({"op": "union", "target": "Spar", "tool": "Rib", "name": "Both"}),
    );
    match &both {
        Ok(out) if out["status"] == "ok" => {}
        _ => return Err(format!("{what}: the union: {both:?}")),
    }
    let all = volume(s, "Both");
    if !((all - spar - rib).abs() <= 1e-6 * (spar + rib)) {
        return Err(format!(
            "{what}: the union's volume {all}, not {spar} + {rib}"
        ));
    }
    Ok(())
}

fn all_clear(cases: Vec<(String, Session)>) {
    let failed: Vec<String> = cases
        .into_iter()
        .filter_map(|(what, mut s)| clear_in_the_notch(&mut s, &what).err())
        .collect();
    assert!(
        failed.is_empty(),
        "{} refused or wrong:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// The spar's bottom along the skin's lowest line, rising off it at `d`
/// through it `z0` from the root. The first eight settings were refused
/// before ("marching an intersection curve stalled"); the last two, the
/// Jig's own proportions, were answered then too.
#[test]
fn a_spar_whose_bottom_all_but_holds_a_ruling_of_the_skin_is_clear_of_the_rib() {
    let settings = [
        (0.7, 1e-4, 0.3),
        (0.7, 1e-4, 1.1),
        (0.7, 1e-4, 5.0),
        (0.7, 1e-4, 30.0),
        (0.7, 3.3e-5, 30.0),
        (0.7, 3e-6, 1.1),
        (1.0, 1e-4, 1.1),
        (1.0, 1e-4, 30.0),
        (0.7, 3.3e-5, 1.1),
        (1.0, 3.3e-5, 1.1),
    ];
    all_clear(
        settings
            .into_iter()
            .map(|(taper, d, z0)| {
                // The lowest line falls 3 (1 − taper) over the span.
                let slope = 3.0 * (1.0 - taper) / SPAN + d;
                let lift = slope * (AT + 0.5 * SHEET) - d * z0;
                (
                    format!("taper {taper}, slope {d:e} off the lowest line through it at {z0}"),
                    wing(lift, slope, taper),
                )
            })
            .collect(),
    );
}

/// The spar's bottom 0.02 mm inside the skin's lowest line at the rib and
/// rising 0.003 along the span, in a tapered blank: refused before in the
/// fit's words ("an intersection curve could not be fitted within the
/// tolerance").
#[test]
fn a_spar_a_little_inside_a_tapered_skin_is_clear_of_the_rib() {
    all_clear(
        [0.7, 0.9]
            .into_iter()
            .map(|taper| {
                (
                    format!("taper {taper}, 0.02 inside, rising 0.003"),
                    wing(0.02, 0.003, taper),
                )
            })
            .collect(),
    );
}
