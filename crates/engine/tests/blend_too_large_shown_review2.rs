//! The second review of *Too large only where it is shown* (2026-11-25;
//! `docs/design/blends.md`): a fillet's or a chamfer's *"too large"* says
//! which sizes do not fit, and each must be true.
//!
//! - *"the R mm asked does not fit …, nor the radii (distances) tried from
//!   X mm up to it"* (`Shown::Builds`, since this review's F4): R does not
//!   build (the sizes tried are not named, and a size between them may);
//! - *"the largest radius (distance) that fits there is below X mm"*
//!   (`Shown::Faces`): no size at or above X builds;
//! - *"the largest radius that fits there is X mm"* (`Shown::Itself`): no
//!   size above X builds;
//! - never *"below 0 mm"* (this review's F5: no face shows it).
//!
//! Each is held against sizes that build a right body: the deep check
//! passes and the volume is the closed form's to 1e-9. Two cases where a
//! message is false (on the branch 4342e899, and on `main` 22f0bb13 in the
//! words before):
//!
//! 1. **Seven sizes are not every size.** The search shows its range by a
//!    build just below X and builds at seven sizes evenly between X and the
//!    size asked, which fail. Where the faces leave a window of room
//!    between two of them, a size in it builds. A block 10 × 10 × 10 with
//!    two shallow pockets in its top, 0.05 deep, x 4 to 6, near its front
//!    top edge (y = 0, z = 10), that edge rounded or chamfered:
//!    - fillet: pockets at y 1 to 1.4 and 1.98 to 2.48. A fillet of r
//!      leaves the material outside the circle about (y, z) = (r, 10 - r);
//!      its surface at y = 1.4, 10 - r + sqrt(r² - (r - 1.4)²), lies below
//!      the first pocket's floor from r = (2.9 + sqrt(0.56)) / 2 = 1.82417
//!      on, so the first pocket lies wholly in what it takes, and its
//!      border on the top reaches the second pocket at 1.98: r from 1.8242
//!      to 1.98 builds, the body the block less the second pocket (0.05)
//!      less 10 (1 - π/4) r². Asked 3 (the second pocket is taken whole
//!      only from 3.028), the event is the first pocket's near rim at 1, a
//!      build just below it succeeds, and the seven sizes between, 1.25,
//!      1.5, …, 2.75, all fail; 1.9 and 1.95 build;
//!    - chamfer: pockets at y 1 to 1.4 and 1.5 to 2.2. A chamfer of d is
//!      the plane z = 10 - d + y, below the first pocket's floor at y = 1.4
//!      from d = 1.45, and its cut line on the top reaches the second
//!      pocket at 1.5: d from 1.45 to 1.5 builds, the body the block less
//!      the second pocket (0.07) less 5 d². Asked 2.1 (the second pocket
//!      taken whole only from 2.25), the seven sizes 1.1375, 1.275,
//!      1.4125, 1.55, … fail; 1.46 and 1.48 build.
//!
//!    Proposed: say what was tried, not every size between (*"… the R mm
//!    asked does not fit, nor the sizes tried from X mm up to it; the
//!    largest found to fit is just below X mm"*), or prove the range from
//!    the faces.
//! 2. **"Below 0" from the faces.** A rod of radius 10 halved through its
//!    axis (`section`, a straight edge with a cylinder beside it): the ball
//!    of a fillet of a cut edge lies r from the cut and 10 - r from the
//!    axis, which exists while r <= 5. From 5 on the two curves moved by the
//!    radius do not meet, and `section` says *"too large … the largest
//!    radius that fits there is below 0 mm"* (`Shown::Faces`, its largest
//!    0), while a fillet of 4 of both cut edges builds the closed form's
//!    body (`crates/kernel/tests/blend_cylinders.rs` › `half_disc_corner`).
//!    The faces give the limit, 5 (the line r from the cut and the circle
//!    10 - r about the axis meet while r <= 10 - r): the refusal could say
//!    it, or say no size.
//!
//! At scales 1e-3, 1 and 1e6 (the document's tolerance 1e-6 times the
//! scale). Each case prints what every size answered.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

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

/// The number before `" mm"` at the start of `text`.
fn number(text: &str) -> Option<f64> {
    text.split_once(" mm")?.0.parse().ok()
}

impl Answer {
    /// The sizes a too-large refusal says do not fit, at the scale of the
    /// document: from the first up to the second (infinite where it says
    /// the largest that fits is below the first).
    fn claims(&self) -> Option<(f64, f64)> {
        let Answer::Refused(words) = self else {
            return None;
        };
        if !words.contains(": too large for edges") {
            return None;
        }
        const BELOW: &str = "that fits there is below ";
        if let Some(at) = words.find(BELOW) {
            let x = number(words.get(at + BELOW.len()..)?)?;
            return Some((x, f64::INFINITY));
        }
        const IS: &str = "that fits there is ";
        if let Some(at) = words.find(IS) {
            let x = number(words.get(at + IS.len()..)?)?;
            return Some((x * (1.0 + 1e-9), f64::INFINITY));
        }
        if let Some(at) = words.find(" mm asked does not fit") {
            let before = words.get(..at)?;
            let r: f64 = before
                .get(before.rfind("the ")? + "the ".len()..)?
                .parse()
                .ok()?;
            return Some((r, r));
        }
        None
    }
}

/// Runs `steps`, then blends the edge `pick` finds among the edges of
/// `feature` (by its two faces) with `size` times `k`.
fn blend(
    k: f64,
    steps: &[(&str, Value)],
    feature: &str,
    pick: fn(&Value, f64) -> bool,
    chamfer: bool,
    size: f64,
) -> Answer {
    let mut s = Session::new(Workspace::unrestricted());
    for (id, params) in steps {
        let done = s.execute(id, params).unwrap();
        assert_ne!(done["status"], "failed", "{id} {params}: {done}");
    }
    let listed = s
        .execute("document.edges", &json!({"feature": feature}))
        .unwrap();
    let edges = listed["edges"].as_array().unwrap();
    let found: Vec<&Value> = edges.iter().filter(|e| pick(e, k)).collect();
    assert!(!found.is_empty(), "no edge picked: {listed}");
    let faces = found[0]["faces"].clone();
    let (id, params) = if chamfer {
        (
            "feature.chamfer",
            json!({"edges": [faces], "name": "Blend", "distance": size * k}),
        )
    } else {
        (
            "feature.fillet",
            json!({"chain": false, "edges": [faces], "name": "Blend", "radius": size * k}),
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

/// Coordinate `i` of the edge's start and end.
fn ends(edge: &Value, i: usize) -> [f64; 2] {
    [
        edge["start"][i].as_f64().unwrap_or(f64::NAN),
        edge["end"][i].as_f64().unwrap_or(f64::NAN),
    ]
}

/// One case: the steps at a scale, the feature whose edges are listed,
/// the edge picked, the sizes refused and those that build with the
/// closed form's volume.
struct Case {
    label: &'static str,
    steps: fn(f64) -> Vec<(&'static str, Value)>,
    feature: &'static str,
    pick: fn(&Value, f64) -> bool,
    chamfer: bool,
    asked: &'static [f64],
    builds: &'static [f64],
    volume: fn(f64) -> f64,
}

/// Every size of `builds` builds the closed form's body and passes the
/// deep check, and no refusal of a size of `asked` says one of them does
/// not fit.
fn holds(case: &Case) {
    let mut wrong = Vec::new();
    for k in SCALES {
        let steps = (case.steps)(k);
        let mut built = Vec::new();
        for &size in case.builds {
            let got = blend(k, &steps, case.feature, case.pick, case.chamfer, size);
            println!("{} k={k:e} at {size}: {got:?}", case.label);
            let expected = (case.volume)(size);
            match got {
                Answer::Built { volume, deep: true }
                    if (volume - expected).abs() <= 1e-9 * expected =>
                {
                    built.push(size);
                }
                other => wrong.push(format!(
                    "{} k={k:e} at {size}: does not build the closed form's body ({expected}): {other:?}",
                    case.label
                )),
            }
        }
        for &size in case.asked {
            let got = blend(k, &steps, case.feature, case.pick, case.chamfer, size);
            println!("{} k={k:e} at {size}: {got:?}", case.label);
            if !matches!(got, Answer::Refused(_)) {
                wrong.push(format!(
                    "{} k={k:e} at {size}: not refused: {got:?}",
                    case.label
                ));
            }
            if matches!(&got, Answer::Refused(words) if words.contains("below 0 mm")) {
                wrong.push(format!(
                    "{} k={k:e} at {size}: says nothing fits: {got:?}",
                    case.label
                ));
            }
            let Some((from, to)) = got.claims().map(|(x, r)| (x / k, r / k)) else {
                continue;
            };
            for &b in built.iter().filter(|&&b| (from..=to).contains(&b)) {
                wrong.push(format!(
                    "{} k={k:e} at {size}: says no size from {from} up to {to} fits, but {b} builds the closed form's body and passes the deep check: {got:?}",
                    case.label
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A block 10 × 10 × 10 less two pockets 0.05 deep in its top, x 4 to 6,
/// at y `first` and `second` (each from, to), at scale `k`: the body
/// `Cut2`.
fn pocketed(k: f64, first: [f64; 2], second: [f64; 2]) -> Vec<(&'static str, Value)> {
    let pocket = |name: &str, [y0, y1]: [f64; 2]| {
        json!({"name": name, "position": [4.0 * k, y0 * k, 9.95 * k],
               "size": [2.0 * k, (y1 - y0) * k, 1.0 * k]})
    };
    vec![
        ("file.new", json!({"units": "mm", "tolerance": 1e-6 * k})),
        (
            "feature.block",
            json!({"name": "Block", "size": [10.0 * k, 10.0 * k, 10.0 * k]}),
        ),
        ("feature.block", pocket("Pocket1", first)),
        (
            "feature.boolean",
            json!({"name": "Cut1", "op": "subtract", "target": "Block", "tool": "Pocket1"}),
        ),
        ("feature.block", pocket("Pocket2", second)),
        (
            "feature.boolean",
            json!({"name": "Cut2", "op": "subtract", "target": "Cut1", "tool": "Pocket2"}),
        ),
    ]
}

/// The block's front top edge: along x at y = 0, z = 10.
fn front_top(edge: &Value, k: f64) -> bool {
    let near = |a: f64, b: f64| (a - b).abs() <= 1e-6 * k;
    let [y0, y1] = ends(edge, 1);
    let [z0, z1] = ends(edge, 2);
    edge["curve"] == "line"
        && near(y0, 0.0)
        && near(y1, 0.0)
        && near(z0, 10.0 * k)
        && near(z1, 10.0 * k)
}

#[test]
fn a_fillet_is_never_said_too_large_up_to_the_radius_asked_where_one_between_builds() {
    holds(&Case {
        label: "fillet",
        steps: |k| pocketed(k, [1.0, 1.4], [1.98, 2.48]),
        feature: "Cut2",
        pick: front_top,
        chamfer: false,
        asked: &[3.0],
        builds: &[1.9, 1.95],
        volume: |r| 1000.0 - 0.05 - 10.0 * (1.0 - PI / 4.0) * r * r,
    });
}

#[test]
fn a_chamfer_is_never_said_too_large_up_to_the_distance_asked_where_one_between_builds() {
    holds(&Case {
        label: "chamfer",
        steps: |k| pocketed(k, [1.0, 1.4], [1.5, 2.2]),
        feature: "Cut2",
        pick: front_top,
        chamfer: true,
        asked: &[2.1],
        builds: &[1.46, 1.48],
        volume: |d| 1000.0 - 0.07 - 5.0 * d * d,
    });
}

/// What a round of radius `r` takes out of the corner of a half disc of
/// radius `big` (`crates/kernel/tests/blend_cylinders.rs` ›
/// `half_disc_corner`): the ball r from the diameter and big - r from the
/// centre, the sector of the disc to the ray through the ball's centre
/// less the triangle and the ball's own sector.
fn half_disc_corner(big: f64, r: f64) -> f64 {
    let mx = ((big - r) * (big - r) - r * r).sqrt();
    let psi = r.patan2(mx);
    big * big * psi / 2.0 - mx * r / 2.0 - r * r * (PI / 2.0 + psi) / 2.0
}

/// A rod of radius 10 and height 20 about z less a block from x = 0 on:
/// the half x <= 0, cut through its axis, the body `Half`.
fn halved_rod(k: f64) -> Vec<(&'static str, Value)> {
    vec![
        ("file.new", json!({"units": "mm", "tolerance": 1e-6 * k})),
        (
            "feature.cylinder",
            json!({"name": "Rod", "radius": 10.0 * k, "height": 20.0 * k}),
        ),
        (
            "feature.block",
            json!({"name": "Cutter", "position": [0.0, -20.0 * k, -5.0 * k],
                   "size": [20.0 * k, 40.0 * k, 30.0 * k]}),
        ),
        (
            "feature.boolean",
            json!({"name": "Half", "op": "subtract", "target": "Rod", "tool": "Cutter"}),
        ),
    ]
}

/// A cut edge of the halved rod: a line along z in the cut (x = 0) at
/// y = -+10. Both lie between the cut and the wall, so the pair of faces
/// means both.
fn cut_edge(edge: &Value, k: f64) -> bool {
    let [x0, x1] = ends(edge, 0);
    let [y0, _] = ends(edge, 1);
    let [z0, z1] = ends(edge, 2);
    edge["curve"] == "line"
        && x0.abs() <= 1e-6 * k
        && x1.abs() <= 1e-6 * k
        && (y0.abs() - 10.0 * k).abs() <= 1e-6 * k
        && (z1 - z0).abs() >= 10.0 * k
}

#[test]
fn a_fillet_of_a_halved_rod_is_never_said_to_fit_below_0() {
    holds(&Case {
        label: "halved rod",
        steps: halved_rod,
        feature: "Half",
        pick: cut_edge,
        chamfer: false,
        asked: &[6.0],
        builds: &[4.0],
        volume: |r| 20.0 * (50.0 * PI - 2.0 * half_disc_corner(10.0, r)),
    });
}
