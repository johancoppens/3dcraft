//! The land of a cut thread: the wall of a shank, a cylinder, with a
//! helical groove cut into it from its lower end, many turns up. The face
//! is bounded below by one loop that winds once around the axis on its
//! way along the groove, up one of its edges and down the other, and above
//! by a circle.
//!
//! The model that asked for this (an M10 screw with a thread 16 turns
//! long) sat for over thirteen minutes in the app at the display
//! tolerance: none of the straight seams tried from vertices spread along
//! the lower loop got past the turns of the groove above them, the search
//! for a seam in pieces (which lays the loops out a turn either way) found
//! no way either, so the face took its chords for too coarse and asked
//! for all of them to be cut, round after round, and the triangulations
//! of ever more points cost the square of their number. The seam is now
//! also tried straight up from the highest vertex of the lower loop, and
//! a triangulation has a work budget.
//!
//! The oracle: the closed-form area and the mesh's own boundary (after
//! welding, the only edges used by one triangle are on the face's edges,
//! all of them).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Cylinder, Line, NurbsCurve, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::cost::WorkClock;
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, VertexId};
use std::f64::consts::TAU;

const RADIUS: f64 = 5.0;
const PITCH: f64 = 1.5;
/// The groove's width on the wall, along the axis.
const GROOVE: f64 = 0.75;
const TURNS: f64 = 40.0;
/// Where the lower loop starts, up the lower edge of the groove, in turns.
const SPLIT: f64 = 18.0;
/// The wall's height above the end of the groove.
const ABOVE: f64 = 4.0;

fn forward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: false,
    }
}

fn backward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: true,
    }
}

fn on_wall(angle: f64, z: f64) -> Vec3 {
    Vec3::new(RADIUS * angle.pcos(), RADIUS * angle.psin(), z)
}

/// A helix on the wall from `from` to `to` (angles), at height `lift`
/// at angle 0: cubic Bézier pieces of an eighth of a turn, each through
/// the helix at its ends and tangent to it there (the circle within
/// 2e-5 of the radius, the height exact).
fn helix(from: f64, to: f64, lift: f64) -> NurbsCurve {
    let pieces = ((to - from) / (TAU / 8.0)).ceil().max(1.0) as usize;
    let step = (to - from) / pieces as f64;
    let k = 4.0 / 3.0 * (step / 4.0).ptan();
    let height = |angle: f64| lift + PITCH * angle / TAU;
    let mut ctrl = Vec::new();
    let mut knots = vec![0.0; 4];
    for piece in 0..pieces {
        let (a, b) = (from + step * piece as f64, from + step * (piece + 1) as f64);
        let (za, zb) = (height(a), height(b));
        let (pa, pb) = (on_wall(a, za), on_wall(b, zb));
        let ta = Vec3::new(-a.psin(), a.pcos(), 0.0) * (k * RADIUS);
        let tb = Vec3::new(-b.psin(), b.pcos(), 0.0) * (k * RADIUS);
        let third = (zb - za) / 3.0;
        if piece == 0 {
            ctrl.push(pa);
        }
        ctrl.push(pa + ta + Vec3::new(0.0, 0.0, third));
        ctrl.push(pb - tb - Vec3::new(0.0, 0.0, third));
        ctrl.push(pb);
        let end = (piece + 1) as f64;
        knots.extend(if piece + 1 == pieces {
            [end; 4].to_vec()
        } else {
            [end; 3].to_vec()
        });
    }
    NurbsCurve::polynomial(3, knots, ctrl).unwrap()
}

/// The land as a sheet with one face, and its area in closed form.
fn land() -> (Body, f64) {
    let mut body = Body::new(BodyKind::Sheet);
    let top = PITCH * TURNS + GROOVE + ABOVE;
    let end = TAU * TURNS;
    // The groove lies between the helix through angle 0 at height 0 and
    // the one a groove's width above it, which starts on the lower end
    // of the wall `GROOVE / PITCH` of a turn earlier.
    let start = -TAU * GROOVE / PITCH;
    let vertex =
        |body: &mut Body, angle: f64, z: f64| -> VertexId { body.add_vertex(on_wall(angle, z)) };
    let v0 = vertex(&mut body, 0.0, 0.0);
    let vb = vertex(&mut body, start, 0.0);
    let v1 = vertex(&mut body, end, PITCH * TURNS);
    let v2 = vertex(&mut body, end, PITCH * TURNS + GROOVE);
    let vt = vertex(&mut body, 0.0, top);
    let bottom = Curve3::Circle(Circle::new(Frame::WORLD, RADIUS).unwrap());
    let bottom = body.add_edge(bottom, (0.0, TAU + start), v0, vb);
    let upper_edge = Curve3::Nurbs(helix(start, end, GROOVE));
    let range = upper_edge.domain();
    let upper_edge = body.add_edge(upper_edge, range, vb, v2);
    let line = Line::new(
        on_wall(end, PITCH * TURNS + GROOVE),
        Vec3::new(0.0, 0.0, -1.0),
    )
    .unwrap();
    let across = body.add_edge(Curve3::Line(line), (0.0, GROOVE), v2, v1);
    // The lower edge of the groove in two, so that the loop starts part of
    // the way up: the vertices spread along it then miss the last turn.
    let split = TAU * SPLIT;
    let vs = vertex(&mut body, split, PITCH * SPLIT);
    let first = Curve3::Nurbs(helix(0.0, split, 0.0));
    let range = first.domain();
    let first = body.add_edge(first, range, v0, vs);
    let second = Curve3::Nurbs(helix(split, end, 0.0));
    let range = second.domain();
    let second = body.add_edge(second, range, vs, v1);
    let level = Frame::new(Vec3::new(0.0, 0.0, top), Vec3::Z, Vec3::X).unwrap();
    let circle = Curve3::Circle(Circle::new(level, RADIUS).unwrap());
    let circle = body.add_edge(circle, (0.0, TAU), vt, vt);
    let lower = body.add_loop(vec![
        backward(first),
        forward(bottom),
        forward(upper_edge),
        forward(across),
        backward(second),
    ]);
    let upper = body.add_loop(vec![backward(circle)]);
    let surface = Surface::Cylinder(Cylinder::new(Frame::WORLD, RADIUS).unwrap());
    let face = body.add_face(surface, false, vec![lower, upper]);
    body.add_shell(vec![face]);
    // The wall less the groove: a strip `GROOVE` high along the whole
    // thread, and the triangle where it starts below the lower end.
    let groove = GROOVE * end + 0.5 * GROOVE * (-start);
    let area = RADIUS * (TAU * top - groove);
    (body, area)
}

#[test]
fn the_land_of_a_long_thread_meshes_quickly_at_a_fine_tolerance() {
    let (body, area) = land();
    for tolerance in [0.05, 0.0015] {
        let params = TessParams {
            chordal_tolerance: tolerance,
            ..TessParams::default()
        };
        // One face, meshed on this thread: its work, which a busy machine
        // does not stretch.
        let start = WorkClock::start();
        let mesh =
            tessellate_body(&body, &params).unwrap_or_else(|e| panic!("at {tolerance}: {e}"));
        let seconds = start.elapsed().as_secs_f64();
        let what = format!(
            "at {tolerance}: {} triangles in {seconds:.2} s",
            mesh.triangles.len()
        );
        // Every chord is within the tolerance of the wall, inside it: the
        // mesh's area is short of the wall's by at most about the
        // tolerance over the radius (and the edges' own 2e-5).
        let short = (area - mesh.area()) / area;
        assert!(
            short > -1e-6 && short < 2.0 * tolerance / RADIUS + 1e-5,
            "{what}: area {} against {area}",
            mesh.area()
        );
        // Open only along the face's edges: one closed boundary below and
        // one above, nothing across the face.
        let welded = mesh.weld().unwrap();
        assert!(!welded.is_closed(), "{what}");
        // Before, this did not end; now it is a fraction of a second in
        // release. A generous bound for a debug build on a busy machine.
        assert!(seconds < 60.0, "{what}");
        // Not refined beyond the tolerance: before, the face asked for
        // every chord of its edges to be cut, sixteen times over. The
        // chords of the two helices are what the tolerance needs, and the
        // triangles grow with them, about as the inverse square root of
        // the tolerance on a cylinder.
        let along = TAU * TURNS * RADIUS * 2.0;
        let chords = along / (8.0 * tolerance * RADIUS).sqrt();
        assert!(
            (mesh.triangles.len() as f64) < 6.0 * chords + 2000.0,
            "{what}: {chords:.0} chords on the helices"
        );
        println!("{what}");
    }
}
