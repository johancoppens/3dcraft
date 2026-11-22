//! A block whose top passes within about a tolerance of a vertex an
//! earlier intersection made, at a convex corner of a prism (found
//! 2026-11-20 in a hipped roof of 24 planes whose normals were rounded to
//! six decimals; docs/design/booleans.md › *A plane that passes a vertex
//! by less than a tolerance*).
//!
//! The case, with plain booleans. A prism over a four-cornered outline,
//! 40 high; its corner at (50.4, 0) is the one of the roof's outline. A
//! first block, its top a plane through that corner at height 14 that
//! holds the outline's edge on one side of the corner, is intersected
//! with the prism: the corner's upright edge now ends in a vertex at
//! (50.4, 0, 14). A second block, its top a plane along the outline's
//! edge on the other side of the corner (to the six decimals its normal
//! was given with), passes `t` tolerances over that vertex and is
//! intersected with what the first left. Each block reaches far past the
//! prism on every other side.
//!
//! The plane either holds that vertex, V, or crosses the edge that
//! leaves V along the other wall at a place W. That edge leaves at a
//! shallow angle to the plane, so W is 3.23 `t` tolerances from V.
//!
//! Until 2026-11-22, where V and W came between one and two tolerances
//! apart (the plane about half a tolerance from V), the refusal said
//! "imprint lost: a curve in which two faces cross does not divide them;
//! a piece of it is missing", which names no place, no tolerance and
//! nothing to do. W was taken to V (a place within twice the tolerance of
//! an end of its edge that lies within the tolerance of the surface is
//! that end), and the curve in which the plane meets the roof's first
//! plane runs through W and, the two planes being 20 degrees apart, 1.45
//! tolerances from V: it got no cut there and lost its piece. Under V the
//! eaves' place and the upright edge's did the same. Now the state is
//! refused as the tolerance rule refuses it, "two points closer together
//! than the tolerance can tell apart (at the edge of tolerance)", with
//! the two faces. The bodies built with the plane 0.7 to 2 tolerances
//! from V are not inside that rule's band: their V and W are more than
//! two tolerances apart.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::tolerance;
use common::polygon;
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives::block;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, check};

/// The corner, and its neighbours along the outline of the roof.
const BEFORE: [f64; 2] = [44.727, -8.56];
const CORNER: [f64; 2] = [50.4, 0.0];
const AFTER: [f64; 2] = [50.464, 9.658];
/// The eaves' height.
const EAVES: f64 = 14.0;
/// The normals as the roof gave them, to six decimals: of the plane
/// through the edge from the corner on, and of the one through the edge
/// up to it.
const FIRST: [f64; 3] = [0.615_648, -0.004_08, 0.788_011];
const SECOND: [f64; 3] = [0.513_191, -0.340_109, 0.788_011];

/// How far the second plane passes over the vertex, in tolerances.
const OVER: [f64; 13] = [
    0.0, 0.3, 0.45, 0.49, 0.497, 0.51, 0.55, 0.7, 0.9, 1.0, 1.5, 1.9, 2.1,
];
/// And under it.
const UNDER: [f64; 8] = [-0.3, -0.45, -0.55, -0.7, -1.0, -1.5, -1.9, -2.1];

/// The tolerance rule's refusal of two places closer together than the
/// tolerance can tell apart.
const NEAR: &str =
    "two points closer together than the tolerance can tell apart (at the edge of tolerance)";

fn prism() -> Body {
    let outline = polygon(1, 1, &[CORNER, AFTER, [0.0, 20.0], BEFORE]);
    let profiles = detect_profiles(&outline, &tolerance(1.0)).unwrap();
    extrude(&profiles, &Frame::WORLD, 0.0, 40.0, &tolerance(1.0))
        .unwrap()
        .body
}

/// A block whose top lies in the plane through `at` square to `normal`
/// and which reaches 150 to every side of `at` in the plane and 150 down
/// from it: past the prism on every side but the top.
fn under(at: Vec3, normal: [f64; 3]) -> Body {
    let plane = Frame::from_z(at, Vec3::new(normal[0], normal[1], normal[2])).unwrap();
    let reach = 150.0;
    let corner = at - plane.x() * reach - plane.y() * reach - plane.z() * reach;
    block(&plane.at(corner).unwrap(), 2.0 * reach, 2.0 * reach, reach).unwrap()
}

/// The prism under the first plane, and the vertex the plane made on the
/// corner's upright edge.
fn first_cut() -> (Body, Vec3) {
    let tol = tolerance(1.0);
    let at = Vec3::new(CORNER[0], CORNER[1], EAVES);
    let cut = boolean(&prism(), &under(at, FIRST), BooleanOp::Intersect, &tol).unwrap();
    assert_eq!(check(&cut.body, &tol), []);
    let vertex = cut
        .body
        .vertices
        .iter()
        .map(|v| v.point)
        .min_by(|a, b| a.distance(at).total_cmp(&b.distance(at)))
        .unwrap();
    assert!(vertex.distance(at) <= 1e-12, "{vertex:?}");
    (cut.body, vertex)
}

/// What the second block answers at `t` tolerances over the vertex: the
/// body, or the refusal.
fn second_cut(body: &Body, vertex: Vec3, t: f64) -> Result<Body, KernelError> {
    let tol = tolerance(1.0);
    let n = Vec3::new(SECOND[0], SECOND[1], SECOND[2])
        .normalized()
        .unwrap();
    let at = vertex + n * (t * tol.linear);
    boolean(body, &under(at, SECOND), BooleanOp::Intersect, &tol).map(|result| result.body)
}

/// How far apart the two nearest vertices of `body` are.
fn nearest_pair(body: &Body) -> f64 {
    let points: Vec<Vec3> = body.vertices.iter().map(|v| v.point).collect();
    let mut nearest = f64::INFINITY;
    for (i, a) in points.iter().enumerate() {
        for b in points.iter().skip(i + 1) {
            nearest = nearest.min(a.distance(*b));
        }
    }
    nearest
}

/// At every distance the second block either cuts (a body that passes
/// its check) or is refused as at the edge of tolerance, with the faces.
///
/// Until 2026-11-22 (Check runs 44767663080 and 44769475435 on
/// `check/cut-planes`, 44767711875 on `check/contact-border` f93760c7
/// with this file added: the same answers), by the plane's distance from
/// the vertex in tolerances, with how far apart the two nearest vertices
/// of a body built are:
///
/// - over it, 0 and 0.3: built, the vertex taken as lying in the plane
///   (11 vertices);
/// - 0.45, 0.49, 0.497, 0.51, 0.55: refused, "imprint lost: a curve in
///   which two faces cross does not divide them; a piece of it is missing
///   (face 1 of the second body)";
/// - 0.7, 0.9, 1.0, 1.5, 1.9, 2.1: built with two vertices there (12),
///   2.26, 2.91, 3.23, 4.84, 6.14 and 6.78 tolerances apart;
/// - under it, 0.3: built (11 vertices); 0.45 and 0.55: "imprint lost";
///   0.7: built (12 vertices, 2.08 apart); 1.0: "imprint lost"; 1.5:
///   refused as it should be, "two points closer together than the
///   tolerance can tell apart (at the edge of tolerance)", with its
///   faces; 1.9 and 2.1: built (6.14 and 6.78 apart).
///
/// Since, every "imprint lost" there is that refusal, with the two faces,
/// and every other answer is as it was.
#[test]
fn a_block_whose_top_passes_a_vertex_by_less_than_a_tolerance_cuts_or_says_where() {
    let tol = tolerance(1.0);
    let (body, vertex) = first_cut();
    let mut owed = Vec::new();
    for t in OVER.into_iter().chain(UNDER) {
        let said = match second_cut(&body, vertex, t) {
            Ok(cut) => {
                let problems = check(&cut, &tol);
                if problems.is_empty() {
                    format!(
                        "builds ({} faces, {} vertices, the two nearest {:.2} tolerances apart)",
                        cut.faces.len(),
                        cut.vertices.len(),
                        nearest_pair(&cut) / tol.linear
                    )
                } else {
                    owed.push(t);
                    format!("builds a body that fails its check: {problems:?}")
                }
            }
            Err(KernelError::Unsupported { reason, faces, .. })
                if reason.ends_with("(at the edge of tolerance)") && !faces.is_empty() =>
            {
                format!("refused at the edge of tolerance, with its faces: {reason}")
            }
            Err(error) => {
                owed.push(t);
                format!("refused without a place: {error}")
            }
        };
        println!("plane_by_vertex: {t:+.3} tolerances: {said}");
    }
    assert!(owed.is_empty(), "owed at {owed:?} tolerances");
}

/// What holds today and must keep holding: well clear of the vertex, on
/// either side, the second block cuts, and so it does through the vertex
/// itself.
#[test]
fn a_block_whose_top_passes_a_vertex_well_clear_of_it_cuts() {
    let tol = tolerance(1.0);
    let (body, vertex) = first_cut();
    for t in [0.0, 5.0, -5.0, 1000.0, -1000.0] {
        match second_cut(&body, vertex, t) {
            Ok(cut) => assert_eq!(check(&cut, &tol), [], "{t} tolerances"),
            Err(error) => panic!("{t} tolerances: {error}"),
        }
    }
}

/// The case at scale `k` (the tolerance `1e-6 k`), its outline drawn in
/// the plane of `frame` and the prism raised along its z axis, either
/// block's top a spline surface on the same plane when asked.
struct Case {
    k: f64,
    tol: Tolerance,
    /// The prism under the first plane.
    first: Body,
    /// The vertex the first plane made on the corner's upright edge.
    vertex: Vec3,
    /// The second plane's normal.
    normal: Vec3,
}

/// Which top is a spline surface.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Spline {
    Neither,
    /// The second block's: the plane that passes V.
    Second,
    /// The first block's: the roof's face that holds V and the edge that
    /// leaves it along the wall, whose edges are then splines.
    First,
}

impl Case {
    fn new(k: f64, frame: Frame, spline: Spline) -> Case {
        let tol = tolerance(k);
        let scaled = |p: [f64; 2]| [p[0] * k, p[1] * k];
        let outline = polygon(
            1,
            1,
            &[
                scaled(CORNER),
                scaled(AFTER),
                [0.0, 20.0 * k],
                scaled(BEFORE),
            ],
        );
        let profiles = detect_profiles(&outline, &tol).unwrap();
        let prism = extrude(&profiles, &frame, 0.0, 40.0 * k, &tol)
            .unwrap()
            .body;
        let at = frame.point(CORNER[0] * k, CORNER[1] * k, EAVES * k);
        let normal = frame.vector(FIRST[0], FIRST[1], FIRST[2]);
        let mut roof = under_at(at, normal, k);
        if spline == Spline::First {
            roof = spline_top(roof, at, normal, &tol);
        }
        let first = boolean(&prism, &roof, BooleanOp::Intersect, &tol)
            .unwrap()
            .body;
        assert_eq!(check(&first, &tol), [], "the first cut at {k}");
        let vertex = first
            .vertices
            .iter()
            .map(|v| v.point)
            .min_by(|a, b| a.distance(at).total_cmp(&b.distance(at)))
            .unwrap();
        // Where an edge meets a spline surface is found to a fraction of
        // the tolerance.
        let exact = if spline == Spline::First { 0.05 } else { 1e-6 };
        assert!(
            vertex.distance(at) <= exact * tol.linear,
            "{vertex:?} at {k}"
        );
        Case {
            k,
            tol,
            first,
            vertex,
            normal: frame
                .vector(SECOND[0], SECOND[1], SECOND[2])
                .normalized()
                .unwrap(),
        }
    }

    /// What the second block answers at `t` tolerances over the vertex.
    fn second_cut(&self, t: f64, spline: Spline) -> Result<Body, KernelError> {
        let at = self.vertex + self.normal * (t * self.tol.linear);
        let mut second = under_at(at, self.normal, self.k);
        if spline == Spline::Second {
            second = spline_top(second, at, self.normal, &self.tol);
        }
        boolean(&self.first, &second, BooleanOp::Intersect, &self.tol).map(|result| result.body)
    }
}

/// `under` at scale `k`, square to a normal given as a vector.
fn under_at(at: Vec3, normal: Vec3, k: f64) -> Body {
    let plane = Frame::from_z(at, normal).unwrap();
    let reach = 150.0 * k;
    let corner = at - plane.x() * reach - plane.y() * reach - plane.z() * reach;
    block(&plane.at(corner).unwrap(), 2.0 * reach, 2.0 * reach, reach).unwrap()
}

/// `body` with the face that lies in the plane through `at` square to
/// `normal` given a spline surface on that plane: a bicubic net over the
/// face's extent, so the face, its edges and its vertices stay where they
/// were and the surface is freeform.
fn spline_top(mut body: Body, at: Vec3, normal: Vec3, tol: &Tolerance) -> Body {
    let index = body
        .faces
        .iter()
        .position(|face| match &face.surface {
            Surface::Plane(plane) => {
                let f = *plane.frame();
                f.z().cross(normal).length() <= 1e-9
                    && (at - f.origin()).dot(f.z()).abs() <= tol.linear
            }
            _ => false,
        })
        .expect("the block's top");
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
    assert_eq!(check(&body, tol), [], "the block with a spline top");
    body
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

/// What the second block answers: a body that passes its check, with this
/// many vertices, or the refusal of two places closer together than the
/// tolerance can tell apart, naming a face of each body.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Answer {
    Built(usize),
    Near,
}

fn answer(case: &Case, t: f64, spline: Spline, what: &str) -> Answer {
    match case.second_cut(t, spline) {
        Ok(body) => {
            assert_eq!(check(&body, &case.tol), [], "{what}");
            Answer::Built(body.vertices.len())
        }
        Err(KernelError::Unsupported { reason, faces, .. }) if reason == NEAR => {
            assert_eq!(faces.len(), 2, "{what}: {faces:?}");
            Answer::Near
        }
        Err(error) => panic!("{what}: {error}"),
    }
}

/// The state at four scales, as found and turned in space, the plane over
/// V and under it. Over V by 0.2 tolerances the plane holds V (one place,
/// 11 vertices); by 0.5, W is 1.6 tolerances from V and V 1.45 from the
/// curve in which the two planes of the roof meet: two places too near;
/// by 0.9, W is 2.9 from V: two places (12 vertices). Under V by 0.2: one
/// place; by 0.5 the eaves' place is 1.5 from V and V 1.45 from that
/// curve: too near; by 0.72 the eaves' place is 2.14 from V and the
/// upright edge's 0.91, taken to V, which is within the tolerance of the
/// curves in which the plane meets both walls: two places; by 0.95 the
/// upright edge's place is 1.2 from V and V more than a tolerance from
/// those curves: too near; by 1.3 that place is 1.65 from V and not taken
/// to it (V is more than the tolerance from the plane): too near, as 1.5
/// was before; by 1.9: two places.
#[test]
fn the_same_state_at_four_scales_turned_in_space_over_and_under() {
    use Answer::{Built, Near};
    let answers = [
        (0.2, Built(11)),
        (0.5, Near),
        (0.9, Built(12)),
        (-0.2, Built(11)),
        (-0.5, Near),
        (-0.72, Built(12)),
        (-0.95, Near),
        (-1.3, Near),
        (-1.9, Built(12)),
    ];
    for k in [1e-3, 1.0, 1e3, 1e6] {
        for (name, frame) in [("as found", Frame::WORLD), ("turned", turned(k))] {
            let case = Case::new(k, frame, Spline::Neither);
            for (t, want) in answers {
                let what = format!("{t} tolerances from V, at {k}, {name}");
                assert_eq!(answer(&case, t, Spline::Neither, &what), want, "{what}");
            }
        }
    }
}

/// The same with a freeform neighbour: the second block's top a spline
/// surface on its plane (the place where an edge meets it is then found
/// numerically, and the curves it meets the roof's planes in are traced),
/// or the first block's (the roof's face and the edges that leave V along
/// the walls are then a spline surface and traced curves). At every
/// distance either a body that passes its check or a refusal at the edge
/// of tolerance with its faces; never "imprint lost".
#[test]
fn the_same_state_beside_a_spline_surface() {
    let mut owed = Vec::new();
    for spline in [Spline::Second, Spline::First] {
        for k in [1.0, 1e3] {
            let case = Case::new(k, Frame::WORLD, spline);
            for t in [0.0, 0.3, 0.5, 0.7, 1.0, -0.3, -0.5, -0.7, -0.95, -1.5, -1.9] {
                let said = match case.second_cut(t, spline) {
                    Ok(body) => {
                        let problems = check(&body, &case.tol);
                        if !problems.is_empty() {
                            owed.push((spline, k, t));
                        }
                        format!(
                            "builds ({} vertices, the two nearest {:.2} tolerances apart), {} problems",
                            body.vertices.len(),
                            nearest_pair(&body) / case.tol.linear,
                            problems.len()
                        )
                    }
                    Err(KernelError::Unsupported { reason, faces, .. })
                        if reason.ends_with("(at the edge of tolerance)") && !faces.is_empty() =>
                    {
                        format!("refused at the edge of tolerance: {reason} {faces:?}")
                    }
                    Err(error) => {
                        owed.push((spline, k, t));
                        format!("refused: {error}")
                    }
                };
                println!(
                    "plane_by_vertex, {spline:?} a spline, at {k}: {t:+.3} tolerances: {said}"
                );
            }
        }
    }
    assert!(owed.is_empty(), "owed at {owed:?}");
}
