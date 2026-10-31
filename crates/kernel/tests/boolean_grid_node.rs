//! A plane through a node of the marcher's grid (2026-10-31, the
//! planetary gearbox's `sectionCentroid` of planet 1 at t = 0.78125).
//!
//! A narrow spline wall extruded along z from a hair below 0 to 10 (the
//! planet, turned and moved by its pose, stood on z = -7.9e-16) is cut by
//! the plane z = 5: the wall's chart is one column of cells, the plane
//! passes exactly through the nodes at v = 1/2, and the field (the
//! distance to the plane) is linear in v, so the reach of the border's
//! edge below those nodes is exactly the distance at its centre. Rounding
//! put that centre a hair further off, the edge was left out as clear of
//! zero, the edge above counted the node's zero as positive and showed no
//! change of sign, and the curve in which the plane cuts the wall was
//! lost: "imprint lost" on the plane's face. Held to the identities of
//! sets (the part below and the part above make the whole) and to closed
//! forms (the part below is the profile's area times its height, the
//! section is the profile), at three scales, for several such hairs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::line;
use craft3d_kernel::below::{CutPlane, below};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives::block;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, check, mass_properties};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// A clamped uniform knot vector for `count` control points.
fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

/// A tooth gap's root, as a gear's is: a cubic spline from (-w/2, 0) to
/// (w/2, 0) that dips below the axis, closed by three lines up to y = w;
/// extruded along z from `from` to `to` (all times `k`).
const W: f64 = 1.0;
const HEIGHTS: [f64; 6] = [0.0, -0.08, -0.12, -0.12, -0.08, 0.0];
const DEGREE: usize = 3;

/// The area of the root's profile, in closed form (times `k²`): the
/// control points lie at their Greville abscissae, so x runs linearly
/// with the parameter, x = -w/2 + w t, and the area under y = w less the
/// area under the spline is w² − w ∫ y dt, with ∫ N_i dt = (t_{i+p+1} −
/// t_i) / (p + 1) for each B-spline of degree p.
fn profile_area(k: f64) -> f64 {
    let knots = knots(DEGREE, HEIGHTS.len());
    let under: f64 = HEIGHTS
        .iter()
        .enumerate()
        .map(|(i, c)| c * (knots[i + DEGREE + 1] - knots[i]) / (DEGREE + 1) as f64)
        .sum();
    (W * W - W * under) * k * k
}

fn root_prism(k: f64, from: f64, to: f64) -> Body {
    let w = W;
    let heights = HEIGHTS;
    let degree = DEGREE;
    let knots = knots(degree, heights.len());
    let control: Vec<[f64; 2]> = heights
        .iter()
        .enumerate()
        .map(|(i, y)| {
            let greville: f64 = knots[i + 1..=i + degree].iter().sum::<f64>() / degree as f64;
            [(-0.5 * w + w * greville) * k, y * k]
        })
        .collect();
    let (first, last) = (control[0], control[control.len() - 1]);
    let curves = vec![
        SketchCurve {
            id: 1,
            curve: Curve2::Spline(Spline2 {
                degree,
                knots,
                control,
            }),
        },
        line(2, last, [0.5 * w * k, w * k]),
        line(3, [0.5 * w * k, w * k], [-0.5 * w * k, w * k]),
        line(4, [-0.5 * w * k, w * k], first),
    ];
    let tol = tolerance(k);
    let profiles = detect_profiles(&curves, &tol).unwrap();
    extrude(&profiles, &Frame::WORLD, from, to, &tol)
        .unwrap()
        .body
}

/// The block below the plane z = `at`, wider than `body` on every side.
fn below_box(body: &Body, at: f64) -> Body {
    let bbox = body.bbox().unwrap();
    let (lo, hi) = bbox.corners().unwrap();
    let r = lo.distance(hi);
    let corner = Vec3::new(lo.x - r, lo.y - r, lo.z - r);
    let frame = Frame::WORLD.at(corner).unwrap();
    block(
        &frame,
        hi.x - lo.x + 2.0 * r,
        hi.y - lo.y + 2.0 * r,
        at - corner.z,
    )
    .unwrap()
}

fn close(actual: f64, expected: f64, within: f64, what: &str) -> Result<(), String> {
    if (actual - expected).abs() <= within {
        Ok(())
    } else {
        Err(format!(
            "{what}: {actual} against {expected} (off by {:e}, allowed {within:e})",
            (actual - expected).abs()
        ))
    }
}

fn valid(body: &Body, tol: &Tolerance, what: &str) -> Result<(), String> {
    let defects = check(body, tol);
    if defects.is_empty() {
        Ok(())
    } else {
        Err(format!("{what}: invalid, {defects:?}"))
    }
}

/// The cut of the prism standing on z = `from` at z = 5 (times `k`), held
/// to its closed forms and to the identities of sets.
fn cut_at_the_middle(k: f64, from: f64) -> Result<(), String> {
    let tol = tolerance(k);
    let to = 10.0 * k;
    let prism = root_prism(k, from, to);
    valid(&prism, &tol, "the prism")?;
    let whole = mass_properties(&prism).unwrap();
    let area = profile_area(k);
    // The prism itself against its closed form, within the tolerance
    // times its walls' area.
    close(
        whole.volume,
        area * (to - from),
        tol.linear * 4.0 * k * (to - from),
        "the prism's volume",
    )?;
    let at = 5.0 * k;
    // The measure the gearbox asks for: the part below and its section.
    let side = CutPlane::new(Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let found = below(&[&prism], &side, at, &tol).map_err(|e| format!("below refused: {e}"))?;
    // The section is a curve within the tolerance of the wall: the part
    // below within the tolerance times the wall's area below the plane.
    let perimeter = 4.0 * k;
    let slack = tol.linear * perimeter * to;
    close(found.volume, area * (at - from), slack, "volume below")?;
    close(found.section.area, area, tol.linear * perimeter, "section")?;
    // The identities of sets with the half-space: A ∩ H and A − H make A,
    // and each is its closed form; A ∪ H is H and the part above.
    let half = below_box(&prism, at);
    let inside = boolean(&prism, &half, BooleanOp::Intersect, &tol)
        .map_err(|e| format!("intersect refused: {e}"))?;
    let outside = boolean(&prism, &half, BooleanOp::Subtract, &tol)
        .map_err(|e| format!("subtract refused: {e}"))?;
    let union = boolean(&prism, &half, BooleanOp::Union, &tol)
        .map_err(|e| format!("union refused: {e}"))?;
    valid(&inside.body, &tol, "A ∩ H")?;
    valid(&outside.body, &tol, "A − H")?;
    valid(&union.body, &tol, "A ∪ H")?;
    let volume = |b: &Body| mass_properties(b).unwrap().volume;
    let (vi, vo) = (volume(&inside.body), volume(&outside.body));
    close(vi + vo, whole.volume, slack, "A ∩ H + A − H")?;
    close(vi, area * (at - from), slack, "A ∩ H")?;
    close(vo, area * (to - at), slack, "A − H")?;
    let box_volume = volume(&half);
    close(
        volume(&union.body),
        box_volume + vo,
        1e-9 * box_volume,
        "A ∪ H",
    )
}

#[test]
fn a_plane_through_a_node_of_the_grid_cuts_a_narrow_spline_wall() {
    // The hairs the bottom lies below z = 0: the planet's -7.9e-16 mm,
    // and others of rounding's size, at each scale.
    // (Before the fix 8e-16 and the planet's own were refused at 1e-3
    // and 1, 9e-16 at 1e6, of 65 hairs from 1e-16 to 6.4e-15.)
    let hairs = [
        2e-16,
        4e-16,
        7.91569916301027e-16,
        8e-16,
        9e-16,
        1.2e-15,
        1.6e-15,
        3.3e-15,
    ];
    let mut failed = Vec::new();
    for k in SCALES {
        for hair in hairs {
            let from = -hair * k;
            if let Err(why) = cut_at_the_middle(k, from) {
                failed.push(format!("k {k}, bottom at {from:e}: {why}"));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {}:\n{}",
        failed.len(),
        3 * hairs.len(),
        failed.join("\n")
    );
}
