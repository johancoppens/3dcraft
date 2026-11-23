//! A spar's flat bottom beside a corner of a rib's notch (the RC wing
//! Jig's eighth listed range point, found 2026-11-19, reduced and
//! repaired 2026-11-23; docs/design/booleans.md › *A spar's flat bottom
//! beside a corner of a notch*).
//!
//! The case, with plain bodies, in the Jig's millimetres at scale 1 (its
//! tolerance 0.01). A prism over a triangle, 1.5 thick along z: a corner at
//! the origin, one edge up the y axis and one up and to the left at 45
//! degrees, as a corner of a rib's notch has the notch's side and the
//! rib's skin. A long block, 6 by 3 by 800, turned 25 degrees about the y
//! axis as the spar of a swept wing is: its bottom is the plane `y = d`
//! tolerances, and its bottom face begins `beside` tolerances to the right
//! of the corner where it crosses the prism's face `z = 0`. That plane
//! meets the face `z = 0` in a line `d` tolerances over the corner, which
//! crosses the upright edge at W and the slanting one at H, `d` tolerances
//! from W.
//!
//! With `d` between one and two tolerances, H and W are two places closer
//! together than the tolerance can tell apart, and the corner is further
//! than the tolerance from the plane, so neither is taken to it. Until
//! 2026-11-23 every boolean of the two and their interference were refused
//! as "two points closer together than the tolerance can tell apart (at
//! the edge of tolerance)", however far beyond the corner the block's face
//! began (3 tolerances, 37 as the wing's spar does, 300; Check 45241997155):
//! `section.rs` › `clip` refused two cuts of a curve that close whether or
//! not the curve there is in both faces.
//! There it is in neither: H and W are outside the block's face, and the
//! two bodies are `beside` (times the cosine of 25 degrees) apart.
//!
//! In the Jig the slanting edge is the rib's skin, nearly level, a fitted
//! spline that ends within the tolerance of the spar's plane (0.997
//! tolerances) at a corner that is not (1.063): the skin's touch at its
//! end and the notch side's crossing were the two cuts, 0.997 tolerances
//! apart and 0.37 mm beyond the end of the spar's face.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::{mass, verified};
use common::polygon;
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, Interference, boolean, interference};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives::block;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, check};

/// The scales the case is held at; the tolerance scales along.
const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

/// How far the block's bottom passes over the corner, in tolerances:
/// H and W are that far apart.
const OVER: [f64; 3] = [1.06, 1.5, 1.9];

/// How far beyond the corner the block's face begins, in tolerances,
/// where the corner is clear of it: by a little (W is then 2.72
/// tolerances from the face's edge, past twice the tolerance), as the
/// wing's spar does (37), and by a lot.
const CLEAR: [f64; 3] = [3.0, 37.0, 300.0];

/// The tolerance at scale `k`: the Jig's 0.01 mm at 1.
fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(0.01 * k, 1e-9).unwrap()
}

/// A frame turned in space, at scale `k`.
fn turned(k: f64) -> Frame {
    Frame::new(
        Vec3::new(13.0 * k, -7.0 * k, 3.0 * k),
        Vec3::new(0.3, -0.5, 0.81),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

/// Which face is a spline surface on its plane.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Spline {
    Neither,
    /// The block's bottom.
    Bottom,
    /// The prism's face `z = 0`.
    Side,
}

/// The prism (the rib) and the block (the spar) at scale `k` in `frame`,
/// the block's bottom `d` tolerances over the corner and its face
/// beginning `beside` tolerances beyond it.
fn pair(k: f64, frame: &Frame, d: f64, beside: f64, spline: Spline) -> (Body, Body, Tolerance) {
    leaning_pair(k, frame, d, beside, (0.0, 1.0), spline)
}

/// [`pair`] with the triangle's two edges leaning `lean.0` and `lean.1` to
/// the left (x per unit of y), away from the block's face: the triangle
/// (0, 0), (-5 lean.0, 5), (-5 lean.1, 5); [`pair`]'s has one edge upright
/// and one at 45 degrees, (0, 1).
fn leaning_pair(
    k: f64,
    frame: &Frame,
    d: f64,
    beside: f64,
    lean: (f64, f64),
    spline: Spline,
) -> (Body, Body, Tolerance) {
    let tol = tolerance(k);
    let t = tol.linear;
    let h = 5.0 * k;
    // (0 - lean, not -lean: an upright edge at +0, as before.)
    let left = |lean: f64| (0.0 - lean) * h;
    let outline = polygon(1, 1, &[[0.0, 0.0], [left(lean.0), h], [left(lean.1), h]]);
    let profiles = detect_profiles(&outline, &tol).unwrap();
    let mut rib = extrude(&profiles, frame, 0.0, 1.5 * k, &tol).unwrap().body;
    let (s, c) = 25.0_f64.to_radians().psin_cos();
    let along = frame.vector(s, 0.0, c);
    let across = frame.vector(c, 0.0, -s);
    // The block's edge between its bottom and its left side runs through
    // (beside, d, 0), and the block reaches 400 along it either way.
    let start = frame.point(beside * t, d * t, 0.0) - along * (400.0 * k);
    let mut spar = block(
        &Frame::new(start, along, across).unwrap(),
        6.0 * k,
        3.0 * k,
        800.0 * k,
    )
    .unwrap();
    match spline {
        Spline::Neither => {}
        Spline::Bottom => spar = spline_on(spar, frame.point(0.0, d * t, 0.0), frame.y(), &tol),
        Spline::Side => rib = spline_on(rib, frame.origin(), frame.z(), &tol),
    }
    (rib, spar, tol)
}

/// `body` with the face that lies in the plane through `at` square to
/// `normal` given a spline surface on that plane: a bicubic net over the
/// face's extent, so the face, its edges and its vertices stay where they
/// were and the surface is freeform.
fn spline_on(mut body: Body, at: Vec3, normal: Vec3, tol: &Tolerance) -> Body {
    let index = body
        .faces
        .iter()
        .position(|face| match &face.surface {
            Surface::Plane(plane) => {
                let f = *plane.frame();
                f.z().cross(normal).length() <= 1e-9
                    && (at - f.origin()).dot(f.z()).abs() <= 1e-3 * tol.linear
            }
            _ => false,
        })
        .expect("the face on the plane");
    let Surface::Plane(plane) = &body.faces[index].surface else {
        panic!("a plane");
    };
    let f = *plane.frame();
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for vertex in &body.vertices {
        let (x, y, z) = f.local(vertex.point);
        if z.abs() <= tol.linear {
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
        }
    }
    let mut ctrl = Vec::new();
    for i in 0..4 {
        for j in 0..4 {
            ctrl.push(f.point(
                lo[0] + (hi[0] - lo[0]) * f64::from(i) / 3.0,
                lo[1] + (hi[1] - lo[1]) * f64::from(j) / 3.0,
                0.0,
            ));
        }
    }
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    body.faces[index].surface = Surface::Nurbs(
        NurbsSurface::new(3, 3, knots.clone(), knots, 4, 4, ctrl, vec![1.0; 16]).unwrap(),
    );
    assert_eq!(check(&body, tol), [], "the body with a spline face");
    body
}

#[track_caller]
fn assert_close(actual: f64, expected: f64, what: &str) {
    let slack = 1e-9 * expected.abs().max(actual.abs());
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} is not {expected}"
    );
}

/// The two bodies told apart: their union is both, two shells; each less
/// the other is itself; they have nothing in common, and nothing of one
/// touches the other.
fn told_apart(rib: &Body, spar: &Body, tol: &Tolerance, what: &str) {
    let (v_rib, v_spar) = (mass(rib).volume, mass(spar).volume);
    let both = boolean(spar, rib, BooleanOp::Union, tol)
        .unwrap_or_else(|e| panic!("{what}: the union: {e}"));
    assert_eq!(check(&both.body, tol), [], "{what}: the union");
    verified(spar, rib, BooleanOp::Union, &both.body, tol, what);
    assert_eq!(both.body.shells.len(), 2, "{what}: the union's shells");
    assert_close(mass(&both.body).volume, v_rib + v_spar, what);
    for (target, tool, volume, name) in [(rib, spar, v_rib, "rib"), (spar, rib, v_spar, "spar")] {
        let less = boolean(target, tool, BooleanOp::Subtract, tol)
            .unwrap_or_else(|e| panic!("{what}: the {name} less the other: {e}"));
        assert_eq!(
            check(&less.body, tol),
            [],
            "{what}: the {name} less the other"
        );
        verified(target, tool, BooleanOp::Subtract, &less.body, tol, what);
        assert_eq!(
            less.body.shells.len(),
            1,
            "{what}: the {name} less the other"
        );
        assert_eq!(less.body.faces.len(), target.faces.len(), "{what}: {name}");
        assert_close(mass(&less.body).volume, volume, what);
    }
    match boolean(spar, rib, BooleanOp::Intersect, tol) {
        Err(KernelError::EmptyResult { .. }) => {}
        other => panic!("{what}: the intersection is not empty: {other:?}"),
    }
    match interference(spar, rib, tol) {
        Ok(Interference::Clear) => {}
        other => panic!("{what}: the interference is not clear: {other:?}"),
    }
}

/// What the three booleans and the interference of the pair say, in a
/// word each, for the table a run prints.
fn said(rib: &Body, spar: &Body, tol: &Tolerance) -> String {
    let word = |r: Result<usize, KernelError>| match r {
        Ok(shells) => format!("built ({shells} shells)"),
        Err(KernelError::EmptyResult { .. }) => "empty".to_string(),
        Err(KernelError::Unsupported { reason, faces, .. }) => {
            format!("refused: {reason} ({} faces)", faces.len())
        }
        Err(e) => format!("error: {e}"),
    };
    let op = |a: &Body, b: &Body, op| word(boolean(a, b, op, tol).map(|r| r.body.shells.len()));
    let contact = match interference(spar, rib, tol) {
        Ok(Interference::Clear) => "clear".to_string(),
        Ok(Interference::Touch { .. }) => "touch".to_string(),
        Ok(Interference::Overlap(_)) => "overlap".to_string(),
        Err(e) => word(Err(e)),
    };
    format!(
        "union {}; rib less spar {}; spar less rib {}; intersect {}; interference {contact}",
        op(spar, rib, BooleanOp::Union),
        op(rib, spar, BooleanOp::Subtract),
        op(spar, rib, BooleanOp::Subtract),
        op(spar, rib, BooleanOp::Intersect),
    )
}

/// The pairs of a test, each with what it is, after printing what each
/// pair says (so that a run shows the whole table, not only the first
/// pair that fails).
fn cases(
    scales: &[f64],
    frames: &[&str],
    over: &[f64],
    beside: &[f64],
    spline: Spline,
) -> Vec<(String, Body, Body, Tolerance)> {
    let mut all = Vec::new();
    for &k in scales {
        for &name in frames {
            let frame = if name == "turned" {
                turned(k)
            } else {
                Frame::WORLD
            };
            for &d in over {
                for &b in beside {
                    let what = format!(
                        "{d} tolerances over the corner, the face {b} beyond it, at {k}, {name}, {spline:?} a spline"
                    );
                    let (rib, spar, tol) = pair(k, &frame, d, b, spline);
                    println!("corner_beside: {what}: {}", said(&rib, &spar, &tol));
                    all.push((what, rib, spar, tol));
                }
            }
        }
    }
    all
}

/// The wing's state at four scales, as found and turned in space: the
/// block's bottom one to two tolerances over the corner, its face
/// beginning 3, 37 and 300 tolerances beyond it. The two bodies are told
/// apart. (Refused at every one of them until 2026-11-23.)
#[test]
fn a_spar_beside_a_corner_of_a_notch_is_told_apart() {
    let all = cases(
        &SCALES,
        &["as found", "turned"],
        &OVER,
        &CLEAR,
        Spline::Neither,
    );
    for (what, rib, spar, tol) in &all {
        told_apart(rib, spar, tol, what);
    }
}

/// The same with a freeform neighbour, at 1 and 1e3: the block's bottom a
/// spline surface on its plane (where the prism's edges meet it is then
/// found numerically, and the curve it meets the prism's face in is
/// traced), or the prism's face `z = 0` one.
#[test]
fn a_spar_beside_a_corner_of_a_notch_is_told_apart_beside_a_spline_surface() {
    for spline in [Spline::Bottom, Spline::Side] {
        let all = cases(
            &[1.0, 1e3],
            &["as found"],
            &[1.06, 1.5],
            &[3.0, 37.0],
            spline,
        );
        for (what, rib, spar, tol) in &all {
            told_apart(rib, spar, tol, what);
        }
    }
}

/// The rule itself, which the repair leaves alone: where the block's face
/// reaches over the corner (5 tolerances past it), begins right over it,
/// or begins a tolerance beyond it, the corner is within twice the
/// tolerance of the block's face and further than the tolerance from it,
/// and every boolean of the two is refused at the edge of tolerance,
/// naming a face of each body. As found and turned, at four scales.
#[test]
fn a_corner_within_twice_the_tolerance_of_the_face_is_refused() {
    let all = cases(
        &SCALES,
        &["as found", "turned"],
        &[1.06, 1.5],
        &[-5.0, 0.0, 1.0],
        Spline::Neither,
    );
    for (what, rib, spar, tol) in &all {
        for (a, b, op) in [
            (spar, rib, BooleanOp::Union),
            (rib, spar, BooleanOp::Subtract),
            (spar, rib, BooleanOp::Subtract),
            (spar, rib, BooleanOp::Intersect),
        ] {
            match boolean(a, b, op, tol) {
                Err(KernelError::Unsupported { reason, faces, .. })
                    if reason.ends_with("(at the edge of tolerance)") && faces.len() == 2 => {}
                Err(e) => panic!("{what}: {op:?}: {e}"),
                Ok(_) => panic!("{what}: {op:?} built"),
            }
        }
    }
}

/// The least distance, in tolerances, between the leaning edge (the
/// triangle's first, `lean` to the left per unit up) from the corner to
/// where the block's bottom plane crosses it, `d` tolerances up, and the
/// line of the block's edge on the corner's side, which begins `beside`
/// tolerances to the right of the corner and runs 25 degrees off z: of the
/// two bodies near the corner this edge's piece is the nearest to the
/// block's face (the slanting edge leans further away, the corner's upright
/// edge and the prism's faces meet the block's edge further off for larger
/// z, and above the plane the block's side recedes), so it is the bodies'
/// least distance. A point of the piece `u` up is `c (lean u + beside)`
/// from the line along x and z (`c` the cosine of 25 degrees) and `d - u`
/// along y.
fn least_gap(lean: f64, d: f64, beside: f64) -> f64 {
    let c = 25.0_f64.to_radians().psin_cos().1;
    let u = ((d - c * c * lean * beside) / (1.0 + c * c * lean * lean)).clamp(0.0, d);
    (c * (lean * u + beside)).phypot(d - u)
}

/// Where the block's face must begin, in tolerances beyond the corner, for
/// the least distance of [`least_gap`] to be `gap`.
fn beside_for_gap(lean: f64, d: f64, gap: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, 10.0);
    for _ in 0..100 {
        let middle = 0.5 * (lo + hi);
        if least_gap(lean, d, middle) < gap {
            lo = middle;
        } else {
            hi = middle;
        }
    }
    0.5 * (lo + hi)
}

/// A corner whose edges lean away from the block's face (0.5 and 1.5 to the
/// left per unit up), at the edges of the band: the block's bottom 1.06 or
/// 1.5 tolerances over the corner, its face beginning where the least
/// distance of the two bodies (the leaning edge's piece under the plane to
/// the block's edge) is 1.95 or 1.99 tolerances, or 2.01 or 2.05. Both
/// places where the plane crosses the triangle's edges are then clear of
/// the face (2.14 to 3.61 tolerances from its edge), and so, at 1.5 over
/// it, is the corner itself (2.09 to 2.17); what is within twice the
/// tolerance is a stretch of the leaning edge between them. Under twice the
/// tolerance every boolean is refused at the edge of tolerance, as on
/// `main`; past it the bodies are told apart. As found and turned, at four
/// scales (1.99 and 2.01 at 1, as found).
#[test]
fn a_corner_that_leans_away_from_the_face_is_told_apart_only_past_twice_the_tolerance() {
    let lean = (0.5, 1.5);
    let mut refused = Vec::new();
    let mut apart = Vec::new();
    for k in SCALES {
        for name in ["as found", "turned"] {
            let frame = if name == "turned" {
                turned(k)
            } else {
                Frame::WORLD
            };
            let fine = k == 1.0 && name == "as found";
            for d in [1.06, 1.5] {
                for gap in [1.95, 1.99, 2.01, 2.05] {
                    if !fine && (gap == 1.99 || gap == 2.01) {
                        continue;
                    }
                    let beside = beside_for_gap(lean.0, d, gap);
                    assert!((least_gap(lean.0, d, beside) - gap).abs() < 1e-9);
                    let what = format!(
                        "leaning, {d} tolerances over the corner, the bodies {gap} apart (the face {beside:.4} beyond the corner), at {k}, {name}"
                    );
                    let (rib, spar, tol) =
                        leaning_pair(k, &frame, d, beside, lean, Spline::Neither);
                    println!("corner_beside: {what}: {}", said(&rib, &spar, &tol));
                    if gap < 2.0 {
                        refused.push((what, rib, spar, tol));
                    } else {
                        apart.push((what, rib, spar, tol));
                    }
                }
            }
        }
    }
    for (what, rib, spar, tol) in &refused {
        for (a, b, op) in [
            (spar, rib, BooleanOp::Union),
            (rib, spar, BooleanOp::Subtract),
            (spar, rib, BooleanOp::Subtract),
            (spar, rib, BooleanOp::Intersect),
        ] {
            match boolean(a, b, op, tol) {
                Err(KernelError::Unsupported { reason, faces, .. })
                    if reason.ends_with("(at the edge of tolerance)") && faces.len() == 2 => {}
                Err(e) => panic!("{what}: {op:?}: {e}"),
                Ok(_) => panic!("{what}: {op:?} built"),
            }
        }
    }
    for (what, rib, spar, tol) in &apart {
        told_apart(rib, spar, tol, what);
    }
}
