//! The offset of a fillet face is the fillet rebuilt between the offset
//! faces it rolls along (`docs/decisions.md`, 2026-10-16), through the
//! commands: two balls joined, or as their lens, the circle where they
//! meet filleted by the rolling ball, then every face moved out
//! (`feature.offsetFace`), the whole made hollow (`feature.shell`) and the
//! balls thickened (`feature.thicken`); each checked deep, its volume
//! against the closed form (Pappus's rule, piece by piece: the kernel's
//! `offset_blend.rs` has the same and more), its fillet named as the copy
//! of the fillet it was, and the radius the names say taken. The bike's
//! saddle shelled 2 mm with its fillets is `#[ignore]`d (minutes) in
//! `offset_freeform.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, deep, failed, new_document, run, session};
use craft3d_engine::Session;
use serde_json::json;
use std::f64::consts::PI;

/// `∫ sqrt(r² − z²) dz` from `z0` to `z1` (`|z| ≤ r`).
fn half_disc(r: f64, z0: f64, z1: f64) -> f64 {
    let f = |z: f64| 0.5 * (z * (r * r - z * z).max(0.0).sqrt() + r * r * (z / r).asin());
    f(z1) - f(z0)
}

/// Two balls of radius `big` about `(∓a, 0, 0)`, joined or as their lens,
/// their circle rounded by a ring of tube radius `rho`: the volume.
fn balls_volume(union: bool, big: f64, a: f64, rho: f64) -> f64 {
    let reach = if union { big + rho } else { big - rho };
    let centre = (reach * reach - a * a).sqrt();
    let touch = a * rho / reach;
    let cap = |from: f64| PI * (big * big * (big - from) - (big.powi(3) - from.powi(3)) / 3.0);
    let ring = |sign: f64| {
        let length = 2.0 * touch;
        PI * (centre * centre * length
            + 2.0 * sign * centre * half_disc(rho, -touch, touch)
            + rho * rho * length
            - 2.0 * touch.powi(3) / 3.0)
    };
    if union {
        2.0 * cap(touch - a) + ring(-1.0)
    } else {
        2.0 * cap(touch + a) + ring(1.0)
    }
}

const BIG: f64 = 2.0;
const A: f64 = 1.0;
const RHO: f64 = 0.5;

/// The two balls, joined (`union`) or as their lens, at scale `k`, the
/// circle where they meet filleted `RHO`, as `Round`.
fn rounded_balls(s: &mut Session, union: bool, k: f64) {
    new_document(s, k);
    built(
        s,
        "feature.sphere",
        json!({"name": "Left", "position": [-A * k, 0.0, 0.0], "radius": BIG * k}),
    );
    built(
        s,
        "feature.sphere",
        json!({"name": "Right", "position": [A * k, 0.0, 0.0], "radius": BIG * k}),
    );
    built(
        s,
        "feature.boolean",
        json!({"name": "Joined", "op": if union { "union" } else { "intersect" }, "target": "Left", "tool": "Right"}),
    );
    built(
        s,
        "feature.fillet",
        json!({"name": "Round", "edges": [["F3:copy(0)/F1:face(0)", "F3:copy(1)/F2:face(0)"]], "radius": format!("{} mm", RHO * k)}),
    );
}

fn names(s: &mut Session, name: &str) -> Vec<String> {
    body(s, name)["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

#[track_caller]
fn volume_is(s: &mut Session, name: &str, expected: f64, k: f64, tolerance: f64) {
    let found = body(s, name);
    assert_eq!(found["valid"], true, "{found}");
    let (volume, area) = (
        found["volume"].as_f64().unwrap(),
        found["area"].as_f64().unwrap(),
    );
    let off = volume - expected * k.powi(3);
    println!(
        "{name} at {k}: volume {volume}, closed form {}, off {off:e}",
        expected * k.powi(3)
    );
    assert!(
        off.abs() <= 0.5 * tolerance * area,
        "{name} at {k}: off by {off:e}"
    );
}

#[test]
fn rounded_balls_offset_shelled_and_thickened_through_the_commands() {
    for k in [1e-3, 1.0, 1e6] {
        let tolerance = 1e-6 * k;
        for union in [true, false] {
            let mut s = session();
            rounded_balls(&mut s, union, k);
            let faces = names(&mut s, "Round");
            assert_eq!(faces.len(), 3, "{faces:?}");
            assert!(faces.iter().any(|f| f == "F4:fillet(0)"), "{faces:?}");
            // Every face out by 0.2.
            built(
                &mut s,
                "feature.offsetFace",
                json!({"name": "Out", "faces": faces, "distance": format!("{} mm", 0.2 * k)}),
            );
            let tube = if union { RHO - 0.2 } else { RHO + 0.2 };
            volume_is(
                &mut s,
                "Out",
                balls_volume(union, BIG + 0.2, A, tube),
                k,
                tolerance,
            );
            // The rebuilt fillet keeps its name.
            assert!(
                names(&mut s, "Out")
                    .iter()
                    .any(|f| f == "F5:copy(0)/F4:fillet(0)"),
                "{:?}",
                names(&mut s, "Out")
            );
            deep(&mut s);
            // The whole made hollow 0.2 thick, a cavity (each operation on
            // a document of its own: a feature takes the body it is made
            // from, so the next would be of the offset body).
            let mut s = session();
            rounded_balls(&mut s, union, k);
            built(
                &mut s,
                "feature.shell",
                json!({"name": "Hollow", "source": "Round", "thickness": format!("{} mm", 0.2 * k)}),
            );
            let hollow_tube = if union { RHO + 0.2 } else { RHO - 0.2 };
            volume_is(
                &mut s,
                "Hollow",
                balls_volume(union, BIG, A, RHO) - balls_volume(union, BIG - 0.2, A, hollow_tube),
                k,
                tolerance,
            );
            deep(&mut s);
            // Every face thickened 0.1 outward: the layer between.
            let mut s = session();
            rounded_balls(&mut s, union, k);
            built(
                &mut s,
                "feature.thicken",
                json!({"name": "Layer", "faces": faces, "thickness": format!("{} mm", 0.1 * k)}),
            );
            let layer_tube = if union { RHO - 0.1 } else { RHO + 0.1 };
            volume_is(
                &mut s,
                "Layer",
                balls_volume(union, BIG + 0.1, A, layer_tube) - balls_volume(union, BIG, A, RHO),
                k,
                tolerance,
            );
            deep(&mut s);
        }
    }
}

/// Moved alone, without the balls it rolls along, the fillet is refused at
/// once with the faces and why; moved in so far that its radius reaches
/// zero, too far with the radius as what fits.
#[test]
fn a_fillet_moved_alone_or_too_far_is_refused_in_words() {
    let mut s = session();
    rounded_balls(&mut s, false, 1.0);
    let reason = failed(
        &mut s,
        "feature.offsetFace",
        json!({"name": "Alone", "faces": ["F4:fillet(0)"], "distance": "0.1 mm"}),
    );
    assert!(reason.contains("does not move with it"), "{reason}");
    assert!(reason.contains("F4:fillet(0)"), "{reason}");
    run(&mut s, "feature.delete", json!({"feature": "Alone"}));
    let faces = names(&mut s, "Round");
    let reason = failed(
        &mut s,
        "feature.offsetFace",
        json!({"name": "In", "faces": faces, "distance": "-0.6 mm"}),
    );
    assert!(reason.contains("radius would reach zero"), "{reason}");
    assert!(reason.contains("what fits is above -0.49999"), "{reason}");
}

/// An edit of the fillet's radius: the offset follows it (the radius the
/// names say is the one rebuilt from), and so do undo and redo.
#[test]
fn the_offset_follows_an_edit_of_the_fillet() {
    let mut s = session();
    rounded_balls(&mut s, true, 1.0);
    let faces = names(&mut s, "Round");
    built(
        &mut s,
        "feature.offsetFace",
        json!({"name": "Out", "faces": faces, "distance": "0.2 mm"}),
    );
    volume_is(
        &mut s,
        "Out",
        balls_volume(true, BIG + 0.2, A, RHO - 0.2),
        1.0,
        1e-6,
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "radius": "0.7 mm"}),
    );
    volume_is(
        &mut s,
        "Out",
        balls_volume(true, BIG + 0.2, A, 0.5),
        1.0,
        1e-6,
    );
    deep(&mut s);
    run(&mut s, "edit.undo", json!({}));
    volume_is(
        &mut s,
        "Out",
        balls_volume(true, BIG + 0.2, A, RHO - 0.2),
        1.0,
        1e-6,
    );
}
