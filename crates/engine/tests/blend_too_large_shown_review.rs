//! The review of *Too large only where it is shown* (2026-11-25;
//! `docs/design/blends.md`): a *"too large … the largest radius (distance)
//! that fits there is below X"* is true only where no size at or above X
//! builds a right body. The search for what fits builds sizes up to the
//! one asked and never above it, so where the faces leave room again above
//! the size asked, X is said below sizes that build.
//!
//! The case: a block 10 × 10 × 10 with a shallow pocket in its top near its
//! front top edge (x 4 to 6, y 1 to 1.5, 0.05 deep), and that edge (y = 0,
//! z = 10) rounded or chamfered. The blend's border on the top reaches the
//! pocket's near rim at 1 (an event of the faces), and from there the blend
//! face runs into the pocket (*"reaches a face …"*): the faces leave no
//! room. But a blend large enough takes the whole pocket away:
//!
//! - a fillet of r leaves the material outside the circle about (y, z) =
//!   (r, 10 - r); its surface at y = 1.5, 10 - r + sqrt(r² - (r - 1.5)²),
//!   lies below the pocket's floor (z = 9.95) from r = (3.1 + sqrt(0.6)) / 2
//!   = 1.93730 on, and the pocket is then wholly in what the fillet takes;
//!   the body is the block less 10 (1 - π/4) r² (the pocket's void inside
//!   it);
//! - a chamfer of d is the plane z = 10 - d + y, at y = 1.5 below the floor
//!   from d = 1.55 on; the body is the block less 5 d².
//!
//! So a fillet of 1.2 or 1.8, or a chamfer of 1.2 or 1.4, is refused (the
//! blend runs into the pocket), and any "below X" the search says there,
//! from the pocket's near rim at 1 with the sizes between it and the size
//! asked failing (as they do), is false: radii of 2 and 2.5 and distances
//! of 1.6 and 2 build the closed form's body and pass the deep check.
//! Found by the review on 2026-11-25; on `main` (22f0bb13) as on the
//! branch: the event rule as it was, which the new rule keeps.
//!
//! At scales 1e-3 to 1e6 (the document's tolerance 1e-6 times the scale).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The top and the front of the block, after the pocket's cut.
const EDGE: [&str; 2] = ["F3:copy(0)/F1:face(1)", "F3:copy(0)/F1:face(2)"];

/// The block with its pocket, at scale `k`.
fn pocketed(k: f64) -> Vec<(&'static str, Value)> {
    vec![
        ("file.new", json!({"units": "mm", "tolerance": 1e-6 * k})),
        (
            "feature.block",
            json!({"name": "Block", "size": [10.0 * k, 10.0 * k, 10.0 * k]}),
        ),
        (
            "feature.block",
            json!({"name": "Pocket", "position": [4.0 * k, 1.0 * k, 9.95 * k], "size": [2.0 * k, 0.5 * k, 1.0 * k]}),
        ),
        (
            "feature.boolean",
            json!({"name": "Cut", "op": "subtract", "target": "Block", "tool": "Pocket"}),
        ),
    ]
}

/// What the blend answered.
#[derive(Debug)]
enum Answer {
    /// Built: its volume at scale 1, and whether the deep check passed.
    Built {
        volume: f64,
        deep: bool,
    },
    Refused(String),
}

impl Answer {
    /// The largest size a too-large refusal says fits, at the scale of the
    /// document.
    fn below(&self) -> Option<f64> {
        let Answer::Refused(words) = self else {
            return None;
        };
        let at = words.find("that fits there is below ")?;
        let rest = words.get(at + "that fits there is below ".len()..)?;
        rest.split(' ').next()?.parse().ok()
    }
}

/// The fillet (radius `size`) or the chamfer (distance `size`) of the
/// edge, at scale `k`.
fn blend(k: f64, chamfer: bool, size: f64) -> Answer {
    let mut s = Session::new(Workspace::unrestricted());
    for (id, params) in pocketed(k) {
        let done = s.execute(id, &params).unwrap();
        assert_ne!(done["status"], "failed", "{id} {params}: {done}");
    }
    let (id, params) = if chamfer {
        (
            "feature.chamfer",
            json!({"edges": [EDGE], "name": "Blend", "distance": size * k}),
        )
    } else {
        (
            "feature.fillet",
            json!({"chain": false, "edges": [EDGE], "name": "Blend", "radius": size * k}),
        )
    };
    let done = s.execute(id, &params).unwrap();
    if done["status"] == "failed" {
        return Answer::Refused(done["error"].as_str().unwrap().to_string());
    }
    let deep = s.execute("document.check", &json!({"deep": true})).unwrap()["ok"] == true;
    let measured = s
        .execute("document.measure", &json!({"feature": "Blend"}))
        .unwrap();
    let volume = measured["volume"].as_f64().unwrap() / (k * k * k);
    Answer::Built { volume, deep }
}

/// The volume of the block blended by `size`, where the blend takes the
/// whole pocket.
fn swallowed(chamfer: bool, size: f64) -> f64 {
    if chamfer {
        1000.0 - 5.0 * size * size
    } else {
        1000.0 - 10.0 * (1.0 - PI / 4.0) * size * size
    }
}

/// Each size `asked` is refused or said too large below an X that no
/// size of `larger` (each at least X) builds the closed form's body of.
fn never_below_a_size_that_builds(chamfer: bool, asked: &[f64], larger: &[f64]) {
    let kind = if chamfer { "chamfer" } else { "fillet" };
    let mut wrong = Vec::new();
    for k in SCALES {
        let mut built = Vec::new();
        for &size in larger {
            let got = blend(k, chamfer, size);
            println!("{kind} k={k:e} at {size}: {got:?}");
            if let Answer::Built { volume, deep: true } = got {
                let expected = swallowed(chamfer, size);
                if (volume - expected).abs() <= 1e-9 * expected {
                    built.push(size);
                } else {
                    println!("  volume {volume}, not the closed form's {expected}");
                }
            }
        }
        for &size in asked {
            let got = blend(k, chamfer, size);
            println!("{kind} k={k:e} at {size}: {got:?}");
            let Some(x) = got.below().map(|x| x / k) else {
                continue;
            };
            for &above in built.iter().filter(|&&b| b >= x) {
                wrong.push(format!(
                    "{kind} k={k:e} at {size}: said below {x}, but {above} builds the closed form's body and passes the deep check"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_fillet_is_never_said_too_large_below_a_radius_that_takes_a_pocket_whole() {
    never_below_a_size_that_builds(false, &[1.2, 1.8], &[2.0, 2.5]);
}

#[test]
fn a_chamfer_is_never_said_too_large_below_a_distance_that_takes_a_pocket_whole() {
    never_below_a_size_that_builds(true, &[1.2, 1.4], &[1.6, 2.0]);
}
