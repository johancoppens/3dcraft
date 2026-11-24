//! Near approaches that no place of the boolean shows (2026-11-24;
//! docs/design/booleans.md › *Near approaches inside an edge or a face*).
//!
//! The developer's rules: parts that touch, no further apart than the
//! tolerance can tell, are zero thickness and refused (2026-10-15); a wedge
//! or a sliver beside a vertex is accepted (2026-10-18); faces or points of
//! exact bodies between half and twice the tolerance apart are refused
//! (2026-10-22). The boolean looked for such approaches only at places:
//! the vertices and cuts the section makes or registers, where faces
//! touch, and a point of each stretch. Inside an edge or a face, where no
//! surfaces meet nearby, it saw nothing: two edges crossing 1.9 tolerances
//! apart, an edge passing a flat or a curved face, two rods side by side
//! or across, a ball or a rod over a plane, a vertex 1.9 tolerances from an
//! edge. Their unions were built as two parts that near (a wrong body) and
//! the interference said "clear" (the workshop's `plan/near-miss/`,
//! measured on `main` abdcaed0e). `boolean::near` looks at the result's
//! vertices, edges and faces that share no vertex, one on a face of each
//! operand.
//!
//! - Every kind of the investigation, at 0.25, 0.75, 1.1, 1.5, 1.9 and 2.5
//!   tolerances, at 1e-3, 1 and 1e6, as built and turned: in the band (0.75
//!   to 1.9) the union is refused (zero thickness, or a refusal of the
//!   section's own as before) and the interference is not "clear"; at 2.5
//!   the union is two parts, checked, and the interference clear; at 0.25
//!   the union is refused or one shell (never two parts that touch). The
//!   subtractions are the target (the approach is not in the result: as
//!   before), or the section's own refusal; the intersection is empty, or
//!   that refusal.
//! - The second and third reviews' beak (an edge crossing an edge 1.8987
//!   tolerances from it) and rod (an edge passing a rod's round side 1.9
//!   tolerances from it): their unions refused.
//! - The three random designs the investigation found, reduced to the pair
//!   and the distance (`xtask fuzz` from 20 780 000): two balls 1.6863
//!   tolerances apart (seed 20786865), a rod through the top of a block
//!   whose foot runs 1.0806 tolerances inside the block's side (one shell:
//!   refused as too near to tell apart; seed 20784442), and a plane cut
//!   whose curve passes a vertex 1.1329 tolerances off (seed 20823920).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{arc, line, polygon};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, Interference, boolean, interference};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Portable, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, check, mass_properties};

/// The gaps of the investigation's kinds, in tolerances.
const GAPS: [f64; 6] = [0.25, 0.75, 1.1, 1.5, 1.9, 2.5];

/// The scales the kinds are built at.
const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The tolerance at scale `k`: the investigation's 0.01 mm at 1, against
/// bodies of 2 to 20 units.
fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(0.01 * k, 1e-9).unwrap()
}

/// A frame turned in space, at scale `k` (the reviews' tests').
fn turned(k: f64) -> Frame {
    Frame::new(
        Vec3::new(13.0 * k, -7.0 * k, 3.0 * k),
        Vec3::new(0.3, -0.5, 0.81),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

/// The kinds of approach: one pure kind per pair, the least distance
/// between the two bodies at one place (`plan/near-miss/tools/pairs.py`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// A corner of a turned block over a corner of the block A.
    VertexVertex,
    /// A corner over the middle of A's top back edge.
    VertexEdge,
    /// A corner over the middle of A's top face.
    VertexFace,
    /// The bottom front edge of a block crossing A's top back edge at 40
    /// degrees.
    EdgeEdgeSkew,
    /// The same edge along A's edge, overlapping it.
    EdgeEdgeParallel,
    /// The base circle of a cylinder tilted 30 degrees over A's top face.
    EdgeFaceFlat,
    /// A rod lying across A's top back edge, square to the edge's normal
    /// plane: the edge passes the rod's round side, at its middle.
    EdgeFaceCurved,
    /// The same, 1.3 units from the middle of the edge.
    EdgeFaceCurvedOff,
    /// A ball over A's top face.
    BallOnPlane,
    /// Two rods square to each other, one over the other.
    CrossingRods,
    /// A rod over a longer rod, side by side.
    ParallelRods,
    /// A rod lying over A's top face.
    RodOnPlane,
    /// A block over A's top face.
    ParallelPlanes,
}

fn unit(v: Vec3) -> Vec3 {
    v.normalized().unwrap()
}

/// The block from `at` with sides `size`, times `k`.
fn block(k: f64, at: Vec3, size: [f64; 3]) -> Body {
    primitives::block(
        &Frame::WORLD.at(at).unwrap(),
        size[0] * k,
        size[1] * k,
        size[2] * k,
    )
    .unwrap()
}

/// A cylinder of radius `r` and length `h` (times `k`) from the centre of
/// its base `base` along `axis`, the vertex of its circles in the
/// direction `seam` (kept away from the place of the approach).
fn rod(k: f64, base: Vec3, axis: Vec3, seam: Vec3, r: f64, h: f64) -> Body {
    primitives::cylinder(&Frame::new(base, axis, seam).unwrap(), r * k, h * k).unwrap()
}

/// A direction square to `axis`, turned `degrees` from `from` about it.
fn turned_about(axis: Vec3, from: Vec3, degrees: f64) -> Vec3 {
    let (s, c) = degrees.to_radians().psin_cos();
    from * c + axis.cross(from) * s
}

/// A 6 by 6 by 6 block turned 37 degrees about (1, 2, 3), moved so that
/// its corner furthest along `-toward` is at `target`: all of it lies
/// beyond the plane through `target` across `toward`.
fn corner_at(k: f64, target: Vec3, toward: Vec3) -> Body {
    let turn = Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 37.0_f64.to_radians()).unwrap();
    let s = 6.0 * k;
    let d = unit(toward);
    let mut first = None;
    for i in [0.0, s] {
        for j in [0.0, s] {
            for l in [0.0, s] {
                let p = turn.point(Vec3::new(i, j, l));
                if first.is_none_or(|q: Vec3| p.dot(d) < q.dot(d)) {
                    first = Some(p);
                }
            }
        }
    }
    let shift = target - first.unwrap();
    primitives::block(&Frame::WORLD, s, s, s)
        .unwrap()
        .transformed(&turn.then(&Transform::translation(shift)))
        .unwrap()
}

/// The two bodies of a kind `gap` tolerances apart at scale `k`, in the
/// world's axes or turned.
fn pair(kind: Kind, gap: f64, k: f64, turn: bool) -> (Body, Body) {
    let t = tolerance(k).linear;
    let g = gap * t;
    let p = |x: f64, y: f64, z: f64| Vec3::new(x * k, y * k, z * k);
    let cube = || block(k, Vec3::ZERO, [10.0, 10.0, 10.0]);
    // Out of A's top back edge (along x at y = z = 10).
    let m = unit(Vec3::new(0.0, 1.0, 1.0));
    let along = |axis: Vec3, near: Vec3| turned_about(axis, near, 17.0);
    let (a, b) = match kind {
        Kind::VertexVertex => {
            let u = unit(Vec3::new(1.0, 1.0, 1.0));
            (cube(), corner_at(k, p(10.0, 10.0, 10.0) + u * g, u))
        }
        Kind::VertexEdge => (cube(), corner_at(k, p(5.0, 10.0, 10.0) + m * g, m)),
        Kind::VertexFace => (
            cube(),
            corner_at(k, p(5.0, 5.0, 10.0) + Vec3::Z * g, Vec3::Z),
        ),
        Kind::EdgeEdgeSkew | Kind::EdgeEdgeParallel => {
            let d = g / 2.0_f64.sqrt();
            let b = block(k, p(2.0, 10.0, 10.0) + Vec3::new(0.0, d, d), [6.0; 3]);
            let b = if kind == Kind::EdgeEdgeSkew {
                let q = p(5.0, 10.0, 10.0) + m * g;
                let spin = Transform::rotation_about(q, m, 40.0_f64.to_radians()).unwrap();
                b.transformed(&spin).unwrap()
            } else {
                b
            };
            (cube(), b)
        }
        Kind::EdgeFaceFlat => {
            let (s, c) = 30.0_f64.to_radians().psin_cos();
            let axis = Vec3::new(s, 0.0, c);
            let w = unit(Vec3::Z - axis * axis.dot(Vec3::Z));
            let base = p(5.0, 5.0, 10.0) + Vec3::Z * g + w * (2.0 * k);
            (cube(), rod(k, base, axis, w, 2.0, 4.0))
        }
        Kind::EdgeFaceCurved | Kind::EdgeFaceCurvedOff => {
            let n2 = unit(Vec3::new(0.0, 1.0, -1.0));
            let (s, c) = 40.0_f64.to_radians().psin_cos();
            let axis = unit(Vec3::X * c + n2 * s);
            let at = if kind == Kind::EdgeFaceCurvedOff {
                p(3.7, 10.0, 10.0)
            } else {
                p(5.0, 10.0, 10.0)
            };
            let centre = at + m * (g + 2.0 * k);
            (
                cube(),
                rod(k, centre - axis * (5.0 * k), axis, m, 2.0, 10.0),
            )
        }
        Kind::BallOnPlane => (
            cube(),
            primitives::sphere(
                &Frame::WORLD.at(p(5.0, 5.0, 12.0) + Vec3::Z * g).unwrap(),
                2.0 * k,
            )
            .unwrap(),
        ),
        Kind::CrossingRods => (
            rod(
                k,
                p(-10.0, 0.0, 0.0),
                Vec3::X,
                along(Vec3::X, Vec3::Z),
                2.0,
                20.0,
            ),
            rod(
                k,
                p(0.0, -10.0, 4.0) + Vec3::Z * g,
                Vec3::Y,
                along(Vec3::Y, -Vec3::Z),
                2.0,
                20.0,
            ),
        ),
        Kind::ParallelRods => (
            rod(
                k,
                p(-10.0, 0.0, 0.0),
                Vec3::X,
                along(Vec3::X, Vec3::Z),
                2.0,
                20.0,
            ),
            rod(
                k,
                p(-6.0, 0.0, 4.0) + Vec3::Z * g,
                Vec3::X,
                along(Vec3::X, -Vec3::Z),
                2.0,
                12.0,
            ),
        ),
        Kind::RodOnPlane => (
            cube(),
            rod(
                k,
                p(2.0, 5.0, 12.0) + Vec3::Z * g,
                Vec3::X,
                along(Vec3::X, -Vec3::Z),
                2.0,
                6.0,
            ),
        ),
        Kind::ParallelPlanes => (cube(), block(k, p(3.0, 3.0, 10.0) + Vec3::Z * g, [4.0; 3])),
    };
    if turn {
        let place = Transform::from_frame(&turned(k));
        (
            a.transformed(&place).unwrap(),
            b.transformed(&place).unwrap(),
        )
    } else {
        (a, b)
    }
}

fn volume(body: &Body) -> f64 {
    mass_properties(body).unwrap().volume
}

/// A refusal of the boolean's own: the section's (too near to tell apart,
/// lying within twice the tolerance) or zero thickness.
fn refused(e: &KernelError) -> bool {
    matches!(
        e,
        KernelError::Unsupported { .. } | KernelError::ZeroThickness { .. }
    )
}

/// What each boolean and the interference of a pair said, in a word.
fn said(a: &Body, b: &Body, tol: &Tolerance) -> String {
    let word = |r: Result<craft3d_kernel::boolean::BooleanResult, KernelError>| match r {
        Ok(r) => format!("built ({} shells)", r.body.shells.len()),
        Err(KernelError::EmptyResult { .. }) => "empty".to_string(),
        Err(KernelError::ZeroThickness { .. }) => "zero thickness".to_string(),
        Err(KernelError::Unsupported { reason, .. }) => format!("refused: {reason}"),
        Err(e) => format!("error: {e}"),
    };
    let contact = match interference(a, b, tol) {
        Ok(Interference::Clear) => "clear".to_string(),
        Ok(Interference::Touch { .. }) => "touch".to_string(),
        Ok(Interference::Overlap(_)) => "overlap".to_string(),
        Err(e) => format!("refused: {e}"),
    };
    format!(
        "union {}; A less B {}; B less A {}; intersect {}; interference {contact}",
        word(boolean(a, b, BooleanOp::Union, tol)),
        word(boolean(a, b, BooleanOp::Subtract, tol)),
        word(boolean(b, a, BooleanOp::Subtract, tol)),
        word(boolean(a, b, BooleanOp::Intersect, tol)),
    )
}

/// Holds a pair `gap` tolerances apart to its answers (see the module);
/// every wrong answer is pushed to `faults`.
fn judge(a: &Body, b: &Body, gap: f64, tol: &Tolerance, what: &str, faults: &mut Vec<String>) {
    let band = gap > 0.5 && gap < 2.0;
    let mut fault = |why: String| faults.push(format!("{what}: {why}"));
    // The union.
    match boolean(a, b, BooleanOp::Union, tol) {
        Ok(r) if gap >= 2.0 => {
            if !check(&r.body, tol).is_empty() {
                fault("the union fails the check".to_string());
            }
            if r.body.shells.len() != 2 {
                fault(format!(
                    "the union has {} shells, not 2",
                    r.body.shells.len()
                ));
            }
            let (want, got) = (volume(a) + volume(b), volume(&r.body));
            if (got - want).abs() > 1e-6 * want {
                fault(format!("the union's volume {got}, not {want}"));
            }
        }
        Ok(r) if !band && r.body.shells.len() == 1 => {}
        Ok(r) => fault(format!(
            "a wrong body: the union built, {} shells, its parts {gap} tolerances apart",
            r.body.shells.len()
        )),
        Err(e) if gap < 2.0 && refused(&e) => {}
        Err(e) => fault(format!("the union: {e}")),
    }
    // The subtractions: the target, or the section's own refusal.
    for (x, y, name) in [(a, b, "A less B"), (b, a, "B less A")] {
        match boolean(x, y, BooleanOp::Subtract, tol) {
            Ok(r) => {
                let (want, got) = (volume(x), volume(&r.body));
                if !check(&r.body, tol).is_empty() || (got - want).abs() > 1e-6 * want {
                    fault(format!("{name}: not the target ({got}, not {want})"));
                }
            }
            Err(e) if gap < 2.0 && refused(&e) => {}
            Err(e) => fault(format!("{name}: {e}")),
        }
    }
    // The intersection: empty, or the section's own refusal.
    match boolean(a, b, BooleanOp::Intersect, tol) {
        Err(KernelError::EmptyResult { .. }) => {}
        Err(e) if gap < 2.0 && refused(&e) => {}
        Ok(r) => fault(format!(
            "the intersection built, {} faces",
            r.body.faces.len()
        )),
        Err(e) => fault(format!("the intersection: {e}")),
    }
    // The interference: clear only past twice the tolerance.
    match interference(a, b, tol) {
        Ok(Interference::Clear) if gap >= 2.0 => {}
        Ok(Interference::Touch { .. }) if gap < 2.0 => {}
        Err(e) if gap < 2.0 && refused(&e) => {}
        other => fault(format!("the interference: {other:?}")),
    }
}

/// Every gap, scale and frame of `kinds`, printed and judged.
fn kinds_hold(kinds: &[Kind]) {
    let mut faults = Vec::new();
    for &kind in kinds {
        for gap in GAPS {
            for k in SCALES {
                for turn in [false, true] {
                    let tol = tolerance(k);
                    let (a, b) = pair(kind, gap, k, turn);
                    let what = format!(
                        "{kind:?} {gap} tolerances apart at {k}{}",
                        if turn { ", turned" } else { "" }
                    );
                    println!("near_approach: {what}: {}", said(&a, &b, &tol));
                    judge(&a, &b, gap, &tol, &what, &mut faults);
                }
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} wrong answers:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}

/// Vertex to vertex, to an edge and to a face: the corner of a turned
/// block over a corner, over the middle of an edge and over the middle of
/// a face of the block A. The section sees the first (two vertices it
/// registers), and a corner over a face (`on_the_other`); a corner 1.9
/// tolerances from the middle of an edge it did not see.
#[test]
fn a_corner_near_a_corner_an_edge_or_a_face_is_not_united() {
    kinds_hold(&[Kind::VertexVertex, Kind::VertexEdge, Kind::VertexFace]);
}

/// Edge to edge: the bottom front edge of a block crossing A's top back
/// edge at 40 degrees, and along it, overlapping it (whose ends are a
/// vertex beside an edge).
#[test]
fn an_edge_near_an_edge_is_not_united() {
    kinds_hold(&[Kind::EdgeEdgeSkew, Kind::EdgeEdgeParallel]);
}

/// Edge to face: a circle over a plane, and a straight edge passing a
/// rod's round side at the middle of the edge and away from it.
#[test]
fn an_edge_near_a_face_is_not_united() {
    kinds_hold(&[
        Kind::EdgeFaceFlat,
        Kind::EdgeFaceCurved,
        Kind::EdgeFaceCurvedOff,
    ]);
}

/// Face to face: a ball, a rod and a block over a plane, two rods across
/// and side by side.
#[test]
fn a_face_near_a_face_is_not_united() {
    kinds_hold(&[
        Kind::BallOnPlane,
        Kind::CrossingRods,
        Kind::ParallelRods,
        Kind::RodOnPlane,
        Kind::ParallelPlanes,
    ]);
}

/// The scales and frames the reviews' pairs are built at (their tests').
fn settings() -> Vec<(f64, &'static str, Frame)> {
    let mut all = Vec::new();
    for k in [1.0, 1e3] {
        all.push((k, "as found", Frame::WORLD));
        all.push((k, "turned", turned(k)));
    }
    all
}

/// The union of a pair is not built (refused, in whatever words), and the
/// interference is not "clear".
fn not_united(a: &Body, b: &Body, tol: &Tolerance, what: &str, faults: &mut Vec<String>) {
    println!("near_approach: {what}: {}", said(a, b, tol));
    match boolean(a, b, BooleanOp::Union, tol) {
        Err(e) if refused(&e) => {}
        Err(e) => faults.push(format!("{what}: the union: {e}")),
        Ok(r) => faults.push(format!(
            "{what}: a wrong body: the union built, {} shells",
            r.body.shells.len()
        )),
    }
    match interference(a, b, tol) {
        Ok(Interference::Touch { .. }) => {}
        Err(e) if refused(&e) => {}
        other => faults.push(format!("{what}: the interference: {other:?}")),
    }
}

/// The second review's rib with a beak (`boolean_corner_beside_review2.rs`
/// on `check/review3-wing-eighth-point`), with or without the notch, and
/// moved `shift` tolerances to the right along its top (the third
/// review's far beak), 1.5 thick, at scale `k` in `frame`.
fn beaked_rib(k: f64, frame: &Frame, tol: &Tolerance, notch: bool, shift: f64) -> Body {
    let t = tol.linear;
    let (far, top) = ([-5.0 * k, 5.0 * k], [t, 5.0 * t]);
    let (shoulder, tip) = ([(1.0 + shift) * t, 5.0 * t], [(3.5 + shift) * t, 0.0]);
    let (low, back) = ([-k, -3.0 * k], [-5.0 * k, -3.0 * k]);
    let mut points = vec![far];
    if notch {
        points.push([0.0, 0.0]);
    }
    points.push(top);
    if shift > 0.0 {
        points.push(shoulder);
    }
    points.extend([tip, low, back]);
    let profiles = detect_profiles(&polygon(1, 1, &points), tol).unwrap();
    extrude(&profiles, frame, 0.0, 1.5 * k, tol).unwrap().body
}

/// The reviews' block, 6 by 3 by 800, turned 25 degrees about y as a swept
/// spar is: its bottom the plane `y = d` tolerances, its bottom face
/// beginning `beside` tolerances to the right of the origin at `z = 0`.
fn turned_block(k: f64, frame: &Frame, tol: &Tolerance, d: f64, beside: f64) -> Body {
    let t = tol.linear;
    let (s, c) = 25.0_f64.to_radians().psin_cos();
    let along = frame.vector(s, 0.0, c);
    let across = frame.vector(c, 0.0, -s);
    let start = frame.point(beside * t, d * t, 0.0) - along * (400.0 * k);
    primitives::block(
        &Frame::new(start, along, across).unwrap(),
        6.0 * k,
        3.0 * k,
        800.0 * k,
    )
    .unwrap()
}

/// The beak: the rib's beak edge passes the block's edge 1.8987
/// tolerances from it (the second review's pair, and the third's moved
/// 31.95 tolerances to the right with the block). Without the notch `main`
/// built the union, two parts that near, and called the pair clear; with
/// it `main` refused it at the notch's two places (too near to tell
/// apart), as it still does.
#[test]
fn the_reviews_beak_is_not_united() {
    let mut faults = Vec::new();
    for (k, name, frame) in settings() {
        let tol = tolerance(k);
        for shift in [0.0, 31.95] {
            for notch in [false, true] {
                let rib = beaked_rib(k, &frame, &tol, notch, shift);
                let spar = turned_block(k, &frame, &tol, 1.5, 5.05 + shift);
                let what = format!(
                    "the beak moved {shift} tolerances, {}, at {k}, {name}",
                    if notch {
                        "with the notch"
                    } else {
                        "without it"
                    }
                );
                not_united(&spar, &rib, &tol, &what, &mut faults);
            }
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

/// The second review's rib: a prism over the triangle (0, 0), (0, 5),
/// (-5, 5), 1.5 thick, at scale `k` in `frame`.
fn prism_rib(k: f64, frame: &Frame, tol: &Tolerance) -> Body {
    let h = 5.0 * k;
    let outline = polygon(1, 1, &[[0.0, 0.0], [0.0, h], [-h, h]]);
    let profiles = detect_profiles(&outline, tol).unwrap();
    extrude(&profiles, frame, 0.0, 1.5 * k, tol).unwrap().body
}

/// The second review's rod of radius 2, 20 long, its axis turned 25
/// degrees about y, with a flat along its underside in the plane `y = d`
/// tolerances, 20 tolerances under its axis; its round side passes the
/// rib's upright edge `gap` tolerances from it.
fn flat_rod(k: f64, frame: &Frame, tol: &Tolerance, d: f64, gap: f64) -> Body {
    let t = tol.linear;
    let (s, c) = 25.0_f64.to_radians().psin_cos();
    let along = frame.vector(s, 0.0, c);
    let across = frame.vector(c, 0.0, -s);
    let turned = Frame::new(frame.origin(), along, across).unwrap();
    let radius = 2.0 * k;
    let under = 20.0 * t;
    let half = (radius * radius - under * under).sqrt();
    let centre = [radius + gap * t, d * t + under];
    let left = [centre[0] - half, d * t];
    let right = [centre[0] + half, d * t];
    let profile = [line(11, left, right), arc(12, centre, right, left)];
    let profiles = detect_profiles(&profile, tol).unwrap();
    extrude(&profiles, &turned, -10.0 * k, 10.0 * k, tol)
        .unwrap()
        .body
}

/// The rod: the rib's upright edge passes the rod's round side 1.9
/// tolerances from it. With the flat 2.5 tolerances over the rib's corner
/// `main` built the union, two parts that near, and called the pair clear;
/// with it 1.5 over, `main` refused it at two places too near to tell
/// apart, as it still does.
#[test]
fn the_reviews_rod_is_not_united() {
    let mut faults = Vec::new();
    for (k, name, frame) in settings() {
        let tol = tolerance(k);
        for d in [1.5, 2.5] {
            let rib = prism_rib(k, &frame, &tol);
            let rod = flat_rod(k, &frame, &tol, d, 1.9);
            let what = format!("the rod, its flat {d} tolerances over the corner, at {k}, {name}");
            not_united(&rod, &rib, &tol, &what, &mut faults);
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

/// Seed 20786865, reduced: two balls whose faces pass 1.6863 tolerances
/// apart, united as two parts that near (`main`); refused as zero
/// thickness. At 2.5 tolerances, two parts. At three scales, turned.
#[test]
fn two_balls_a_little_apart_are_not_united() {
    let mut faults = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for turn in [false, true] {
            let frame = if turn { turned(k) } else { Frame::WORLD };
            for gap in [1.6863, 2.5] {
                let a = primitives::sphere(&frame.at(frame.point(0.0, 0.0, 0.0)).unwrap(), 3.0 * k)
                    .unwrap();
                let at = frame.point(5.0 * k + gap * tol.linear, 0.0, 0.0);
                let b = primitives::sphere(&frame.at(at).unwrap(), 2.0 * k).unwrap();
                let what = format!("two balls {gap} tolerances apart at {k}, turned {turn}");
                println!("near_approach: {what}: {}", said(&a, &b, &tol));
                match boolean(&a, &b, BooleanOp::Union, &tol) {
                    Err(KernelError::ZeroThickness { .. }) if gap < 2.0 => {}
                    Ok(r) if gap >= 2.0 && r.body.shells.len() == 2 => {}
                    other => faults.push(format!("{what}: the union: {other:?}")),
                }
            }
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

/// Seed 20784442, reduced: a ball sunk into the top of a block, the circle
/// in which it meets the top `gap` tolerances inside an edge of the top,
/// the ball bulging out past the block's side above the top (as the seed's
/// sphere does past the side of its turned block): one shell, the circle
/// that near the edge and the side with no surfaces crossing there. Where
/// the circle comes nearest the edge decides whether a place of the
/// boolean is there: by the right edge (`+x`) the middle of the circle the
/// section makes is, and `zero_thickness` refuses the union at it, the
/// shell touching itself, as `main` did; by the back and the front edges no
/// place is (the seed's case, which `main` built), and the look refuses it
/// as two points closer together than the tolerance can tell apart (the run
/// prints which). Refused in the band either way; one shell at 2.5. At
/// three scales, turned.
#[test]
fn a_ball_whose_foot_runs_just_inside_an_edge_is_refused() {
    let mut faults = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for turn in [false, true] {
            let frame = if turn { turned(k) } else { Frame::WORLD };
            // The edges the foot comes near: right, back, front (the
            // direction from the top's middle).
            for (side, out) in [
                ("right", [1.0, 0.0]),
                ("back", [0.0, 1.0]),
                ("front", [0.0, -1.0]),
            ] {
                for gap in [1.0806, 1.5, 2.5] {
                    let a = primitives::block(&frame, 10.0 * k, 10.0 * k, 10.0 * k).unwrap();
                    // A ball of radius 3 whose centre is 1 over the top meets
                    // it in a circle of radius √8.
                    let reach = 5.0 * k - gap * tol.linear - 8.0_f64.sqrt() * k;
                    let centre =
                        frame.point(5.0 * k + out[0] * reach, 5.0 * k + out[1] * reach, 11.0 * k);
                    let b = primitives::sphere(&frame.at(centre).unwrap(), 3.0 * k).unwrap();
                    let what = format!(
                        "a ball's foot {gap} tolerances inside the {side} edge at {k}, turned {turn}"
                    );
                    println!("near_approach: {what}: {}", said(&a, &b, &tol));
                    match boolean(&a, &b, BooleanOp::Union, &tol) {
                        Err(e) if gap < 2.0 && refused(&e) => {}
                        Ok(r)
                            if gap >= 2.0
                                && r.body.shells.len() == 1
                                && check(&r.body, &tol).is_empty() => {}
                        other => faults.push(format!(
                            "{what}: the union: {:?}",
                            other.map(|r| r.body.shells.len())
                        )),
                    }
                }
            }
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

/// Seed 20823920, reduced: a quarter of a ring (major radius 12.5, tube 7.5,
/// about the line x = 0, z = 10 along y, kept where x ≥ 0 and z ≤ 10) less a
/// cone standing on z = -1.25 at (2.5, 0) (radius 2.5, height 7.5), cut by
/// the plane x = 5 (kept below it). The cone's base circle meets the ring
/// in a vertex 1.15 tolerances short of the plane, where the circle is
/// all but tangent to it, and the curve in which the plane cuts the ring
/// passes that vertex 1.13 tolerances off: one shell with a sliver that
/// thin between them, which `main` built. Refused as two points closer
/// together than the tolerance can tell apart. At 1 and 1e3, the seed's
/// tolerance (a millionth of the unit).
#[test]
fn a_cut_whose_curve_passes_a_vertex_just_off_it_is_refused() {
    let mut faults = Vec::new();
    for k in [1.0, 1e3] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let p = |x: f64, y: f64, z: f64| Vec3::new(x * k, y * k, z * k);
        let ring = primitives::torus(
            &Frame::new(p(0.0, -7.5, 10.0), Vec3::Y, Vec3::X).unwrap(),
            12.5 * k,
            7.5 * k,
        )
        .unwrap();
        let quadrant = block(k, p(0.0, -20.0, -30.0), [40.0, 30.0, 40.0]);
        let quarter = boolean(&ring, &quadrant, BooleanOp::Intersect, &tol)
            .unwrap()
            .body;
        let cone = primitives::cone(
            &Frame::WORLD.at(p(2.5, 0.0, -1.25)).unwrap(),
            2.5 * k,
            7.5 * k,
        )
        .unwrap();
        let bitten = boolean(&quarter, &cone, BooleanOp::Subtract, &tol)
            .unwrap()
            .body;
        let below = block(k, p(-30.0, -30.0, -30.0), [35.0, 60.0, 60.0]);
        let what = format!("the cut of the bitten ring at x = 5, at {k}");
        println!("near_approach: {what}: {}", said(&bitten, &below, &tol));
        let cut = boolean(&bitten, &below, BooleanOp::Intersect, &tol);
        match cut {
            Err(KernelError::Unsupported { reason, .. })
                if reason.contains("closer together than the tolerance") => {}
            other => faults.push(format!("{what}: {:?}", other.map(|r| r.body.shells.len()))),
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}
