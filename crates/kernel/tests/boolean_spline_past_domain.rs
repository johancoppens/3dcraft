//! A spline edge whose range runs past its curve's domain (2026-11-26).
//! `NurbsCurve::eval` carries the end spans on past the domain as the
//! polynomials (or rational functions) they are, `craft3d_topo`'s check
//! takes such a range, and there the curve leaves the hull of its control
//! points: until this date `craft3d_topo`'s box of the edge (and of every
//! face and body bounded by it) and the boolean's own boxes of edges and
//! faces were those control points, and the speed bound the boolean's
//! searches step by was their polygon's. A face that bulges past its box
//! is never paired with what crosses the bulge: the boolean and the
//! interference query answered as if the bodies were apart.
//!
//! Two bodies whose bulge lies past the box of the control points, each
//! met there by a block, at four scales (1e-3 to 1e6) and turned:
//!
//! - a flat lens bounded by a quadratic spline over [0, 1] used over
//!   [-1, 1] (`y = 0.75 - x - x^2`, which peaks at 1 for `x = -0.5`, while
//!   its control points and vertices reach 0.75) and the chord below it: a
//!   sheet, trimmed by a block over the bulge (closed form: the integral of
//!   the curve above the block's floor);
//! - a solid of a circle's segment of some 210 degrees whose arcs are a
//!   rational quarter circle over [0, 1] used over [0, 10] (the curve stays
//!   on the circle past its domain, and passes x = -1 at t = 3.41, while its
//!   control points and vertices reach -0.86), crossed by a block at
//!   x = -1 (closed form: the area of a disc between two chords).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::{Curve3, Cylinder, Line, NurbsCurve, Plane, Surface};
use craft3d_kernel::boolean::{BooleanOp, Interference, boolean, interference};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, check, mass_properties};

const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The world, and a turn about a slanted axis with a shift far off.
fn placements(k: f64) -> [Transform; 2] {
    let axis = Vec3::new(0.3, -0.5, 0.8);
    let turn = Transform::rotation(axis, 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(7.0, -3.0, 2.0) * k));
    [Transform::translation(Vec3::ZERO), turn]
}

fn line(
    body: &mut Body,
    a: craft3d_topo::VertexId,
    b: craft3d_topo::VertexId,
) -> craft3d_topo::EdgeId {
    let (pa, pb) = (
        body.vertices[a.0 as usize].point,
        body.vertices[b.0 as usize].point,
    );
    let curve = Line::new(pa, (pb - pa).normalized().unwrap()).unwrap();
    body.add_edge(Curve3::Line(curve), (0.0, pa.distance(pb)), a, b)
}

fn coedge(edge: craft3d_topo::EdgeId, reversed: bool) -> Coedge {
    Coedge { edge, reversed }
}

/// The lens: the quadratic `(t, 0.75 - t - t^2)` over [-1, 1] (its domain
/// [0, 1]) above the chord between its ends, in the plane z = 0, times `k`.
fn lens(k: f64) -> Body {
    let spline = NurbsCurve::polynomial(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![
            Vec3::new(0.0, 0.75, 0.0) * k,
            Vec3::new(0.5, 0.25, 0.0) * k,
            Vec3::new(1.0, -1.25, 0.0) * k,
        ],
    )
    .unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    let a = body.add_vertex(Vec3::new(-1.0, 0.75, 0.0) * k);
    let b = body.add_vertex(Vec3::new(1.0, -1.25, 0.0) * k);
    let arc = body.add_edge(Curve3::Nurbs(spline), (-1.0, 1.0), a, b);
    let chord = line(&mut body, b, a);
    // Counter-clockwise about +z: along the chord from a to b, back
    // along the curve.
    let lp = body.add_loop(vec![coedge(chord, true), coedge(arc, true)]);
    let face = body.add_face(Surface::Plane(Plane::new(Frame::WORLD)), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// The lens's area inside the block over x in [-0.7, -0.3], y from 0.8:
/// the integral of `0.75 - x - x^2 - 0.8` there.
fn lens_in_block() -> f64 {
    let f = |x: f64| -x * x * x / 3.0 - x * x / 2.0 - 0.05 * x;
    f(-0.3) - f(-0.7)
}

/// A rational quarter circle of radius `k` over [0, 1], at height `z`.
fn quarter(k: f64, z: f64) -> NurbsCurve {
    NurbsCurve::new(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![
            Vec3::new(k, 0.0, z),
            Vec3::new(k, k, z),
            Vec3::new(0.0, k, z),
        ],
        vec![1.0, 0.5_f64.sqrt(), 1.0],
    )
    .unwrap()
}

/// How far the segment's arcs run: to t = 10, some 210 degrees round.
const ARC_END: f64 = 10.0;

/// The solid of the circle's segment from angle 0 to the angle of the
/// quarter circle at t = 10 (radius `k`, height `k`, standing on z = 0):
/// a face on the cylinder bounded by the two arcs (each the quarter
/// circle used over [0, 10]) and two rulings, the floor, the roof and the
/// flat face on the chord.
fn segment(k: f64) -> Body {
    let (low, high) = (quarter(k, 0.0), quarter(k, k));
    let mut body = Body::new(BodyKind::Solid);
    let b0 = body.add_vertex(low.eval(0.0));
    let b1 = body.add_vertex(low.eval(ARC_END));
    let t0 = body.add_vertex(high.eval(0.0));
    let t1 = body.add_vertex(high.eval(ARC_END));
    let floor_arc = body.add_edge(Curve3::Nurbs(low), (0.0, ARC_END), b0, b1);
    let roof_arc = body.add_edge(Curve3::Nurbs(high), (0.0, ARC_END), t0, t1);
    let up_start = line(&mut body, b0, t0);
    let up_end = line(&mut body, b1, t1);
    let floor_chord = line(&mut body, b1, b0);
    let roof_chord = line(&mut body, t1, t0);
    let wall = body.add_loop(vec![
        coedge(floor_arc, false),
        coedge(up_end, false),
        coedge(roof_arc, true),
        coedge(up_start, true),
    ]);
    let cylinder = Cylinder::new(Frame::WORLD, k).unwrap();
    let wall = body.add_face(Surface::Cylinder(cylinder), false, vec![wall]);
    let floor = body.add_loop(vec![coedge(floor_chord, true), coedge(floor_arc, true)]);
    let floor = body.add_face(Surface::Plane(Plane::new(Frame::WORLD)), true, vec![floor]);
    let roof = body.add_loop(vec![coedge(roof_arc, false), coedge(roof_chord, false)]);
    let up = Frame::WORLD.at(Vec3::new(0.0, 0.0, k)).unwrap();
    let roof = body.add_face(Surface::Plane(Plane::new(up)), false, vec![roof]);
    // The flat face on the chord, its normal away from the centre.
    let (p0, p1) = (
        body.vertices[b0.0 as usize].point,
        body.vertices[b1.0 as usize].point,
    );
    let along = (p0 - p1).normalized().unwrap();
    let out = Vec3::new(along.y, -along.x, 0.0);
    let frame = Frame::new(p1, out, along).unwrap();
    let flat = body.add_loop(vec![
        coedge(floor_chord, false),
        coedge(up_start, false),
        coedge(roof_chord, true),
        coedge(up_end, true),
    ]);
    let flat = body.add_face(Surface::Plane(Plane::new(frame)), false, vec![flat]);
    body.add_shell(vec![wall, floor, roof, flat]);
    body
}

/// The angle of the segment's arcs' far end, between a half and a whole
/// turn.
fn arc_angle() -> f64 {
    let p = quarter(1.0, 0.0).eval(ARC_END);
    p.y.patan2(p.x).rem_euclid(std::f64::consts::TAU)
}

/// The segment's volume at scale 1: its area times its height.
fn segment_volume() -> f64 {
    let phi = arc_angle();
    0.5 * (phi - phi.psin())
}

/// What the block over x in [-1.1, -0.9], y in [-0.1, 0.1], z in
/// [0.25, 0.75] shares with the segment at scale 1: half the area of the
/// unit disc between y = -0.1 and y = 0.1 left of x = -0.9.
fn segment_in_block() -> f64 {
    let half = 0.1 * 0.99_f64.sqrt() + 0.1_f64.pasin();
    0.5 * (half - 0.9 * 0.2)
}

fn block(k: f64, lo: [f64; 3], hi: [f64; 3]) -> Body {
    let frame = Frame::WORLD.at(Vec3::new(lo[0], lo[1], lo[2]) * k).unwrap();
    primitives::block(
        &frame,
        (hi[0] - lo[0]) * k,
        (hi[1] - lo[1]) * k,
        (hi[2] - lo[2]) * k,
    )
    .unwrap()
}

#[track_caller]
fn close(actual: f64, expected: f64, slack: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= slack * expected.abs(),
        "{what}: {actual} where {expected} is right"
    );
}

#[test]
fn a_sheet_that_bulges_past_its_spline_s_domain_is_trimmed_where_a_block_crosses_the_bulge() {
    for k in SCALES {
        let tol = tolerance(k);
        for (turn, place) in placements(k).iter().enumerate() {
            let sheet = lens(k).transformed(place).unwrap();
            assert!(check(&sheet, &tol).is_empty(), "the lens is a body");
            let whole = mass_properties(&sheet).unwrap().area;
            close(whole, 4.0 / 3.0 * k * k, 1e-7, "the lens's area");
            let cutter = block(k, [-0.7, 0.8, -1.0], [-0.3, 2.0, 1.0])
                .transformed(place)
                .unwrap();
            let what = |op: &str| format!("{op} at {k}, placement {turn}");
            let inside = boolean(&sheet, &cutter, BooleanOp::Intersect, &tol)
                .unwrap_or_else(|e| panic!("{}: {e}", what("the lens within the block")));
            let area = mass_properties(&inside.body).unwrap().area;
            close(
                area,
                lens_in_block() * k * k,
                1e-6,
                &what("the lens within the block"),
            );
            let outside = boolean(&sheet, &cutter, BooleanOp::Subtract, &tol)
                .unwrap_or_else(|e| panic!("{}: {e}", what("the lens less the block")));
            let area = mass_properties(&outside.body).unwrap().area;
            close(
                area,
                (4.0 / 3.0 - lens_in_block()) * k * k,
                1e-6,
                &what("the lens less the block"),
            );
        }
    }
}

#[test]
fn a_solid_whose_arcs_run_past_their_spline_s_domain_meets_a_block_at_the_far_side_of_its_circle() {
    for k in SCALES {
        let tol = tolerance(k);
        for (turn, place) in placements(k).iter().enumerate() {
            let solid = segment(k).transformed(place).unwrap();
            let defects = check(&solid, &tol);
            assert!(defects.is_empty(), "the segment is a body: {defects:?}");
            let volume = mass_properties(&solid).unwrap().volume;
            close(
                volume,
                segment_volume() * k * k * k,
                1e-7,
                "the segment's volume",
            );
            let cutter = block(k, [-1.1, -0.1, 0.25], [-0.9, 0.1, 0.75])
                .transformed(place)
                .unwrap();
            let what = |op: &str| format!("{op} at {k}, placement {turn}");
            let shared = segment_in_block() * k * k * k;
            match interference(&solid, &cutter, &tol)
                .unwrap_or_else(|e| panic!("{}: {e}", what("interference")))
            {
                Interference::Overlap(common) => {
                    let v = mass_properties(&common.body).unwrap().volume;
                    close(v, shared, 1e-6, &what("the interference's common volume"));
                }
                other => panic!("{}: {other:?}", what("interference")),
            }
            let common = boolean(&solid, &cutter, BooleanOp::Intersect, &tol)
                .unwrap_or_else(|e| panic!("{}: {e}", what("the intersection")));
            let v = mass_properties(&common.body).unwrap().volume;
            close(v, shared, 1e-6, &what("the intersection"));
            let left = boolean(&cutter, &solid, BooleanOp::Subtract, &tol)
                .unwrap_or_else(|e| panic!("{}: {e}", what("the block less the segment")));
            let v = mass_properties(&left.body).unwrap().volume;
            close(
                v,
                0.02 * k * k * k - shared,
                1e-6,
                &what("the block less the segment"),
            );
        }
    }
}
