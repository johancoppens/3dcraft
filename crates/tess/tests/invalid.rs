//! Input that cannot be tessellated is an error that names the face and
//! the reason, never a panic and never a bad mesh; and awkward but valid
//! input (a boundary through a pole, a collapsed patch edge) is handled.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{BezierPatch, Curve3, Line, NurbsCurve, Plane, Surface};
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessError, TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, LoopId, VertexId};

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    }
}

/// A sheet with one planar face bounded by the polygon `corners`.
fn polygon_sheet(corners: &[(f64, f64)]) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let points: Vec<Vec3> = corners.iter().map(|&(x, y)| Vec3::new(x, y, 0.0)).collect();
    let vertices: Vec<VertexId> = points.iter().map(|&p| body.add_vertex(p)).collect();
    let n = corners.len();
    let coedges = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            let line = Curve3::Line(Line::through(points[i], points[j]).unwrap());
            let edge = body.add_edge(
                line,
                (0.0, points[i].distance(points[j])),
                vertices[i],
                vertices[j],
            );
            Coedge {
                edge,
                reversed: false,
            }
        })
        .collect();
    let loop_ = body.add_loop(coedges);
    let face = body.add_face(Surface::Plane(Plane::new(Frame::WORLD)), false, vec![loop_]);
    body.add_shell(vec![face]);
    body
}

fn face_error(body: &Body, face: usize, word: &str) {
    let error = tessellate_body(body, &params(1e-2)).unwrap_err();
    assert!(
        matches!(error, TessError::Face { face: f, .. } if f == face),
        "expected an error for face {face}, got: {error}"
    );
    let text = error.to_string();
    assert!(text.contains(&format!("face {face}")), "{text}");
    assert!(
        text.contains(word),
        "the message does not say '{word}': {text}"
    );
}

#[test]
fn faces_whose_loops_bound_no_region_are_errors_that_say_why() {
    // A polygon that crosses itself.
    let bow_tie = polygon_sheet(&[(0.0, 2.0), (4.0, 0.0), (4.0, 4.0), (0.0, 0.0)]);
    face_error(&bow_tie, 0, "crosses itself");

    // A polygon without area.
    let flat = polygon_sheet(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]);
    face_error(&flat, 0, "no area");

    // A polygon that runs clockwise around its normal: the face would be
    // the whole plane outside it.
    let clockwise = polygon_sheet(&[(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)]);
    face_error(&clockwise, 0, "wrong way");
    // The same polygon is fine on a reversed face.
    let mut reversed = clockwise.clone();
    reversed.faces[0].reversed = true;
    let mesh = tessellate_body(&reversed, &params(1e-2)).unwrap();
    assert_eq!(mesh.triangles.len(), 2);
    assert!(mesh.normals.iter().all(|n| n.distance(-Vec3::Z) < 1e-15));
    for t in 0..2 {
        let [a, b, c] = mesh.corners(t).unwrap();
        assert!((b - a).cross(c - a).z < 0.0);
    }

    // A loop that is not a closed chain of edges.
    let mut open = solids::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap().body;
    open.loops[2].coedges.pop();
    face_error(&open, 2, "not a closed chain");
    let mut empty = solids::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap().body;
    empty.loops[4].coedges.clear();
    face_error(&empty, 4, "not a closed chain");

    // A plane without loops is unbounded; so is a cylinder.
    let mut bare = solids::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap().body;
    bare.faces[1].loops.clear();
    face_error(&bare, 1, "not closed");
    let mut endless = solids::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap().body;
    endless.faces[0].loops.clear();
    face_error(&endless, 0, "not bounded");
    // A cylinder wall with one circle: the wall has no other end.
    let mut one_end = solids::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap().body;
    one_end.faces[0].loops.pop();
    face_error(&one_end, 0, "not bounded");
    // Two circles that wind the same way: no wall lies between them.
    let mut same_way = solids::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap().body;
    let top = same_way.faces[0].loops[1];
    same_way.loops[top.0 as usize].coedges[0].reversed = false;
    face_error(&same_way, 0, "same way");
    // The two circles the wrong way round: the wall would be everything
    // but the part between them.
    let mut inside_out = solids::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap().body;
    for loop_ in inside_out.faces[0].loops.clone() {
        let coedge = &mut inside_out.loops[loop_.0 as usize].coedges[0];
        coedge.reversed = !coedge.reversed;
    }
    face_error(&inside_out, 0, "outside both");
    // A hole that is not inside the face: the plate's hole wall loop
    // given to the floor of a block elsewhere.
    let mut stray = trimmed::plate_with_round_hole(&Frame::WORLD, 5.0, 4.0, 1.5, 1.2)
        .unwrap()
        .body;
    for vertex in &mut stray.vertices[0..4] {
        vertex.point += Vec3::new(40.0, 0.0, 0.0);
    }
    // (The moved corners no longer lie on their edges' lines; only the
    // floor, face 5, matters here, and its outline is now a slanted
    // quadrilateral that does not contain the hole.)
    face_error(&stray, 5, "outside the loop");
}

/// Faces the method does not cover are refused; none of them comes out
/// as a mesh of something else.
#[test]
fn faces_beyond_the_method_are_refused() {
    use craft3d_geom::{Circle, NurbsSurface, Sphere};
    use std::f64::consts::{FRAC_PI_2, TAU};
    let sphere = || Surface::Sphere(Sphere::new(Frame::WORLD, 2.0).unwrap());
    let meridian = |u: f64| {
        let x = Vec3::new(u.cos(), u.sin(), 0.0);
        let side = Vec3::new(-u.sin(), u.cos(), 0.0);
        Curve3::Circle(Circle::new(Frame::new(Vec3::ZERO, -side, x).unwrap(), 2.0).unwrap())
    };
    let one_face = |surface: Surface, edges: Vec<(Curve3, (f64, f64))>, uses: &[(usize, bool)]| {
        let mut body = Body::new(BodyKind::Sheet);
        let ids: Vec<EdgeId> = edges
            .into_iter()
            .map(|(curve, range)| {
                let (a, b) = (curve.eval(range.0), curve.eval(range.1));
                let find = |body: &mut Body, p: Vec3| {
                    let known = body
                        .vertices
                        .iter()
                        .position(|v| v.point.distance(p) < 1e-9);
                    known.map_or_else(|| body.add_vertex(p), |i| VertexId(i as u32))
                };
                let (start, end) = (find(&mut body, a), find(&mut body, b));
                body.add_edge(curve, range, start, end)
            })
            .collect();
        let loop_ = body.add_loop(
            uses.iter()
                .map(|&(i, reversed)| Coedge {
                    edge: ids[i],
                    reversed,
                })
                .collect(),
        );
        let face = body.add_face(surface, false, vec![loop_]);
        body.add_shell(vec![face]);
        body
    };
    let refused = |body: &Body, what: &str| {
        for tolerance in [0.3, 1e-2] {
            let result = tessellate_body(body, &params(tolerance));
            assert!(
                matches!(
                    result,
                    Err(TessError::Face { face: 0, .. } | TessError::Unsupported { face: 0, .. })
                ),
                "{what} at {tolerance}: {:?}",
                result.map(|m| m.triangles.len())
            );
        }
    };

    // A cap on the side of the sphere, cut by the plane x = 1: its rim
    // does not go around the sphere's axis. Counter-clockwise it bounds
    // the cap (area 2π r h) ...
    let rim = || {
        let plane = Frame::new(Vec3::new(1.0, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
        (
            Curve3::Circle(Circle::new(plane, 3.0_f64.sqrt()).unwrap()),
            (0.0, TAU),
        )
    };
    let cap = one_face(sphere(), vec![rim()], &[(0, false)]);
    let mesh = tessellate_body(&cap, &params(1e-3)).unwrap();
    assert!((mesh.area() - TAU * 2.0 * 1.0).abs() < 1e-3 * TAU * 2.0);
    // ... and clockwise the rest of the sphere, with both poles and the
    // seam in it. (Until 2026-10-13 this was asserted to be refused, as a
    // limit of the method. A sphere is a closed surface: a loop on it has
    // two sides and its direction says which is the face, so this is a
    // valid face, of area 4π r² − 2π r h. See closed_surface_holes.rs.)
    let outside = one_face(sphere(), vec![rim()], &[(0, true)]);
    for tolerance in [0.3, 1e-2, 1e-3] {
        let mesh = tessellate_body(&outside, &params(tolerance)).unwrap();
        let expected = 2.0 * TAU * 4.0 - TAU * 2.0 * 1.0;
        // The mesh is inscribed: smaller, by the order of the tolerance
        // over the radius.
        let short = expected - mesh.area();
        assert!(
            short > 0.0 && short < tolerance * expected,
            "the outside of a cap at {tolerance}: {} vs {expected}",
            mesh.area()
        );
    }
    // (Through the poles there is no such doubt: two meridians walked the
    // other way round bound the larger lune, see tests/periodic.rs.)
    let quarter = (-FRAC_PI_2, FRAC_PI_2);
    let larger = one_face(
        sphere(),
        vec![(meridian(0.3), quarter), (meridian(1.4), quarter)],
        &[(0, false), (1, true)],
    );
    let mesh = tessellate_body(&larger, &params(1e-3)).unwrap();
    let expected = 2.0 * 4.0 * (TAU - 1.1);
    assert!((mesh.area() - expected).abs() < 1e-3 * expected);

    // A whole sphere given with a seam edge: one meridian, walked up and
    // down. In parameter space nothing says that the face goes around.
    let seamed = one_face(
        sphere(),
        vec![(meridian(0.0), quarter)],
        &[(0, false), (0, true)],
    );
    refused(&seamed, "a sphere with a seam edge");

    // A tube as a NURBS surface that closes on itself, with its seam as
    // an edge: `geom` reports no period for it.
    let circle = craft3d_geom::NurbsCurve::circle(&Frame::WORLD, 1.5).unwrap();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (p, w) in circle.control_points().iter().zip(circle.weights()) {
        for z in [0.0, 2.0] {
            ctrl.push(*p + Vec3::new(0.0, 0.0, z));
            weights.push(*w);
        }
    }
    let tube = Surface::Nurbs(
        NurbsSurface::new(
            2,
            1,
            circle.knots().to_vec(),
            vec![0.0, 0.0, 1.0, 1.0],
            9,
            2,
            ctrl,
            weights,
        )
        .unwrap(),
    );
    let ring = |z: f64| {
        let frame = Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap();
        (Curve3::Circle(Circle::new(frame, 1.5).unwrap()), (0.0, TAU))
    };
    let seam = (
        Curve3::Line(Line::through(Vec3::new(1.5, 0.0, 0.0), Vec3::new(1.5, 0.0, 2.0)).unwrap()),
        (0.0, 2.0),
    );
    let closed_tube = one_face(
        tube,
        vec![ring(0.0), seam, ring(2.0)],
        &[(0, false), (1, false), (2, true), (1, true)],
    );
    assert_eq!(
        craft3d_topo::check(&closed_tube, &craft3d_math::Tolerance::default()),
        vec![]
    );
    refused(&closed_tube, "a closed NURBS tube");
}

#[test]
fn broken_bodies_are_errors() {
    let block = solids::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap().body;
    let invalid = |body: &Body, word: &str| {
        let error = tessellate_body(body, &params(1e-2)).unwrap_err();
        assert!(matches!(error, TessError::InvalidBody(_)), "{error}");
        assert!(error.to_string().contains(word), "{error}");
    };
    let mut broken = block.clone();
    broken.edges[3].end = VertexId(1000);
    invalid(&broken, "edge 3");
    let mut broken = block.clone();
    broken.faces[2].loops.push(LoopId(77));
    invalid(&broken, "face 2");
    let mut broken = block.clone();
    broken.loops[5].coedges[1].edge = EdgeId(u32::MAX);
    invalid(&broken, "face 5");
    let mut broken = block.clone();
    broken.vertices[0].point = Vec3::new(f64::NAN, 0.0, 0.0);
    invalid(&broken, "not finite");
    let mut broken = block.clone();
    broken.edges[0].range = (1.0, 1.0);
    invalid(&broken, "edge 0");
    let mut broken = block;
    broken.edges[0].range = (0.0, f64::INFINITY);
    invalid(&broken, "edge 0");
    // An empty body is an empty mesh.
    let nothing = Body::new(BodyKind::Solid);
    assert_eq!(
        tessellate_body(&nothing, &params(1e-2)).unwrap(),
        craft3d_tess::Mesh::default()
    );
}

#[test]
fn a_tolerance_that_needs_too_many_points_is_an_error_not_a_coarser_mesh() {
    let few = |tolerance: f64, max_segments: u32| TessParams {
        chordal_tolerance: tolerance,
        max_segments,
        ..TessParams::default()
    };
    // Faces: a sphere has no edges, so it is the face that says no.
    let sphere = solids::sphere(&Frame::WORLD, 1.0).unwrap().body;
    let error = tessellate_body(&sphere, &few(1e-9, 16)).unwrap_err();
    assert!(
        matches!(error, TessError::TooFine { patch: 0, .. }),
        "{error}"
    );
    assert!(error.to_string().contains("larger tolerance"));
    // The same limit lets a reachable tolerance through.
    assert!(tessellate_body(&sphere, &few(1e-2, 16)).is_ok());
    // Freeform faces and edges.
    for nurbs in [false, true] {
        let dome = trimmed::domed_block(&Frame::WORLD, 3.0, 2.0, 1.5, nurbs)
            .unwrap()
            .body;
        let error = tessellate_body(&dome, &few(1e-12, 4)).unwrap_err();
        assert!(matches!(error, TessError::TooFine { .. }), "{error}");
        // Edges are fine at this tolerance; the roof needs more than a
        // 4 × 4 grid allows.
        let error = tessellate_body(&dome, &few(1e-2, 4)).unwrap_err();
        assert!(
            matches!(error, TessError::TooFine { patch: 5, .. }),
            "{error}"
        );
    }
    let oblique = trimmed::oblique_cylinder(&Frame::WORLD, 1.5, 1.0, 3.0)
        .unwrap()
        .body;
    let error = tessellate_body(&oblique, &few(1e-12, 8)).unwrap_err();
    assert!(matches!(error, TessError::TooFine { .. }), "{error}");
    for tolerance in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            tessellate_body(&oblique, &params(tolerance)),
            Err(TessError::InvalidTolerance(_))
        ));
    }
}

/// A bicubic patch whose v = 0 edge is one point (as on the teapot's lid),
/// used as a face with three spline edges: the collapsed edge is a whole
/// side of the parameter region but no edge of the body.
fn tent() -> (Body, BezierPatch) {
    let apex = Vec3::new(0.1, 0.2, 3.15);
    let mut net = [[apex; 4]; 4];
    for (i, row) in net.iter_mut().enumerate() {
        for (j, p) in row.iter_mut().enumerate().skip(1) {
            let r = j as f64 * 0.7;
            *p = apex + Vec3::new(r * (i as f64 - 1.5), r, -0.3 * r * r);
        }
    }
    let patch = BezierPatch::bicubic(net).unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    let corner = |i, j| patch.control_point(i, j).unwrap();
    let top = body.add_vertex(apex);
    let left = body.add_vertex(corner(0, 3));
    let right = body.add_vertex(corner(3, 3));
    let cubic = |points: Vec<Vec3>| {
        Curve3::Nurbs(
            NurbsCurve::polynomial(3, vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], points)
                .unwrap(),
        )
    };
    let along_v = |i: usize| cubic((0..4).map(|j| corner(i, j)).collect());
    let along_u = |j: usize| cubic((0..4).map(|i| corner(i, j)).collect());
    let down_left = body.add_edge(along_v(0), (0.0, 1.0), top, left);
    let down_right = body.add_edge(along_v(3), (0.0, 1.0), top, right);
    let base = body.add_edge(along_u(3), (0.0, 1.0), left, right);
    // Counter-clockwise in (u, v): up the u = 1 side, back along v = 1,
    // down the u = 0 side to the collapsed edge.
    let use_ = |edge, reversed| Coedge { edge, reversed };
    let loop_ = body.add_loop(vec![
        use_(down_right, false),
        use_(base, true),
        use_(down_left, true),
    ]);
    let face = body.add_face(Surface::Bezier(patch.clone()), false, vec![loop_]);
    body.add_shell(vec![face]);
    (body, patch)
}

/// The distance from `p` to a patch over the unit square. First what
/// `Surface::project` says; when that is more than `enough`, a search
/// that needs no derivatives: a 200 × 200 lattice, then 9 × 9 lattices
/// around the best point so far, finer whenever one finds nothing better. (Near the collapsed edge
/// `project` alone stops at its coarse first sample, see
/// `projection_near_a_collapsed_edge` below.)
fn distance_to_patch(surface: &Surface, p: Vec3, enough: f64) -> f64 {
    let (mut u, mut v) = surface.project(p).unwrap();
    let mut best = surface.eval(u, v).distance(p);
    if best <= enough {
        return best;
    }
    for i in 0..=200 {
        for j in 0..=200 {
            let (cu, cv) = (f64::from(i) / 200.0, f64::from(j) / 200.0);
            let d = surface.eval(cu, cv).distance(p);
            if d < best {
                (best, u, v) = (d, cu, cv);
            }
        }
    }
    // Near the tip the distance is a long narrow valley in (u, v): keep
    // the lattice's size for as long as it finds something better.
    let mut reach = 1.0 / 200.0;
    for _ in 0..5000 {
        if reach < 1e-12 {
            break;
        }
        let (centre_u, centre_v) = (u, v);
        let before = best;
        for i in -4..=4 {
            for j in -4..=4 {
                let cu = (centre_u + reach * f64::from(i) / 4.0).clamp(0.0, 1.0);
                let cv = (centre_v + reach * f64::from(j) / 4.0).clamp(0.0, 1.0);
                let d = surface.eval(cu, cv).distance(p);
                if d < best {
                    (best, u, v) = (d, cu, cv);
                }
            }
        }
        if best >= before {
            reach *= 0.5;
        }
    }
    best
}

/// A gap in `craft3d_geom`, kept as a failing test for whoever closes it:
/// near a collapsed edge of a patch the tangent plane degenerates, the
/// Gauss–Newton iteration of `Surface::project` gives up at once, and the
/// answer is the nearest of its 17 × 17 samples: a point of the surface
/// itself comes back up to a few hundredths away.
#[test]
#[ignore = "known gap in craft3d-geom: Surface::project is only sample-accurate near a \
            collapsed patch edge"]
fn projection_near_a_collapsed_edge() {
    let (_, patch) = tent();
    let surface = Surface::Bezier(patch);
    let mut worst = 0.0_f64;
    for i in 0..=20 {
        for j in 1..=20 {
            let p = surface.eval(f64::from(i) / 20.0, f64::from(j) * 0.005);
            let (u, v) = surface.project(p).unwrap();
            worst = worst.max(surface.eval(u, v).distance(p));
        }
    }
    assert!(
        worst <= 1e-9,
        "a point of the surface projects {worst:e} away from itself"
    );
}

#[test]
fn a_patch_with_a_collapsed_edge_is_tessellated_up_to_its_tip() {
    let (body, patch) = tent();
    assert_eq!(
        craft3d_topo::check(&body, &craft3d_math::Tolerance::default()),
        vec![]
    );
    // The patch's area, by a fine midpoint sum over its parameters.
    let n = 600;
    let h = 1.0 / f64::from(n);
    let mut exact = 0.0;
    for i in 0..n {
        for j in 0..n {
            let (_, du, dv) = patch.derivatives((f64::from(i) + 0.5) * h, (f64::from(j) + 0.5) * h);
            exact += du.cross(dv).length() * h * h;
        }
    }
    let surface = Surface::Bezier(patch.clone());
    let mut previous = f64::INFINITY;
    for tolerance in [1e-1, 1e-2, 1e-3] {
        let mesh = tessellate_body(&body, &params(tolerance)).unwrap();
        // One point at the tip, shared by a fan of triangles. (The face
        // has a vertex there per direction it is reached from, each with
        // its own normal; welding makes them one.)
        let apex = body.vertices[0].point;
        let at_tip = mesh
            .weld()
            .unwrap()
            .positions
            .iter()
            .filter(|p| **p == [apex.x, apex.y, apex.z])
            .count();
        assert_eq!(at_tip, 1, "tolerance {tolerance}");
        let fan = (0..mesh.triangles.len())
            .filter(|&t| mesh.corners(t).unwrap().contains(&apex))
            .count();
        assert!(
            fan >= 3,
            "tolerance {tolerance}: {fan} triangles at the tip"
        );
        assert!(
            mesh.positions
                .iter()
                .all(|p| *p == apex || p.distance(apex) > 1e-9),
            "points a rounding error away from the tip"
        );
        for t in 0..mesh.triangles.len() {
            let [a, b, c] = mesh.corners(t).unwrap();
            let facing = (b - a).cross(c - a);
            assert!(facing.length() > 0.0);
            for p in [
                a.lerp(b, 0.5),
                b.lerp(c, 0.5),
                c.lerp(a, 0.5),
                (a + b + c) * (1.0 / 3.0),
            ] {
                let off = distance_to_patch(&surface, p, tolerance);
                assert!(
                    off <= tolerance * (1.0 + 1e-9),
                    "tolerance {tolerance}: {off:e}"
                );
            }
            // Facing along the patch's normal. Away from the tip, where
            // the projection can be trusted (see above).
            let centroid = (a + b + c) * (1.0 / 3.0);
            let (u, v) = surface.project(centroid).unwrap();
            if surface.eval(u, v).distance(centroid) <= tolerance {
                let normal = surface.normal(u, v).unwrap();
                assert!(facing.dot(normal) > 0.0, "triangle {t} faces the wrong way");
            } else {
                // At the tip: the patch opens downwards and outwards
                // around it, so its normal has a positive z everywhere.
                assert!(
                    facing.z > 0.0,
                    "triangle {t} at the tip faces the wrong way"
                );
            }
        }
        // A sheet has a boundary: every edge is used once or twice, never
        // more, and by two triangles in opposite directions.
        let welded = mesh.weld().unwrap();
        let mut edges = std::collections::BTreeMap::new();
        for &[a, b, c] in &welded.triangles {
            for e in [(a, b), (b, c), (c, a)] {
                *edges.entry(e).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&n| n == 1));
        let error = (mesh.area() - exact).abs() / exact;
        println!(
            "tent at {tolerance:e}: {} triangles, area error {error:e}",
            mesh.triangles.len()
        );
        assert!(error < previous);
        previous = error;
    }
    assert!(previous < 1e-3, "{previous:e}");
}

/// xorshift64*: deterministic pseudo-random numbers without a crate.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Damaged bodies: whatever is done to a valid solid, the tessellator
/// answers with a mesh or with an error. It does not panic, hang or
/// index out of range, on any number of threads.
#[test]
fn damaged_bodies_never_panic() {
    let frame = Frame::new(
        Vec3::new(1.0, -2.0, 0.5),
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap();
    let mut pristine: Vec<Body> = solids::all(&frame, 1.0)
        .unwrap()
        .into_iter()
        .chain(trimmed::all(&frame, 1.0).unwrap())
        .map(|s| s.body)
        .collect();
    pristine.push(trimmed::pipe_elbow(&frame, 3.0, 1.0, 1.9).unwrap().body);
    pristine.push(trimmed::sphere_octant(&frame, 2.0).unwrap().body);
    let mut random = Random(0x853C_49E6_748F_EA9B);
    let (mut meshes, mut errors) = (0, 0);
    for round in 0..400 {
        let mut body = pristine[random.below(pristine.len())].clone();
        for _ in 0..1 + random.below(3) {
            match random.below(12) {
                0 if !body.faces.is_empty() => {
                    let f = random.below(body.faces.len());
                    body.faces[f].reversed = !body.faces[f].reversed;
                }
                1 if !body.faces.is_empty() => {
                    let f = random.below(body.faces.len());
                    body.faces[f].loops.pop();
                }
                2 if !body.loops.is_empty() => {
                    let l = random.below(body.loops.len());
                    for coedge in &mut body.loops[l].coedges {
                        coedge.reversed = !coedge.reversed;
                    }
                }
                3 if !body.loops.is_empty() => {
                    let l = random.below(body.loops.len());
                    body.loops[l].coedges.reverse();
                }
                4 if !body.loops.is_empty() => {
                    let l = random.below(body.loops.len());
                    body.loops[l].coedges.pop();
                }
                5 if !body.vertices.is_empty() => {
                    let v = random.below(body.vertices.len());
                    let bad = [f64::NAN, f64::INFINITY, 0.0, 1e300, -3.0][random.below(5)];
                    body.vertices[v].point.x = bad;
                }
                6 if !body.edges.is_empty() => {
                    let e = random.below(body.edges.len());
                    let (a, b) = body.edges[e].range;
                    body.edges[e].range =
                        [(b, a), (a, a), (a, b * 50.0), (f64::NAN, b), (a - 3.0, b)]
                            [random.below(5)];
                }
                7 if body.faces.len() > 1 => {
                    // Surfaces swapped between faces: loops on the wrong
                    // surface.
                    let (f, g) = (
                        random.below(body.faces.len()),
                        random.below(body.faces.len()),
                    );
                    let surface = body.faces[f].surface.clone();
                    body.faces[f].surface = body.faces[g].surface.clone();
                    body.faces[g].surface = surface;
                }
                8 if body.faces.len() > 1 => {
                    let (f, g) = (
                        random.below(body.faces.len()),
                        random.below(body.faces.len()),
                    );
                    let loops = body.faces[f].loops.clone();
                    body.faces[g].loops.extend(loops);
                }
                9 if !body.edges.is_empty() => {
                    let e = random.below(body.edges.len());
                    body.edges[e].start = VertexId(random.below(body.vertices.len() + 2) as u32);
                }
                10 if !body.loops.is_empty() => {
                    let l = random.below(body.loops.len());
                    if let Some(coedge) = body.loops[l].coedges.first_mut() {
                        coedge.edge = EdgeId(random.below(body.edges.len() + 2) as u32);
                    }
                }
                _ => {
                    if !body.vertices.is_empty() {
                        let v = random.below(body.vertices.len());
                        body.vertices[v].point += Vec3::new(0.3, -0.2, 0.1);
                    }
                }
            }
        }
        let tolerance = [0.5, 0.05, 0.005][random.below(3)];
        let threads = [1, 3][random.below(2)];
        match tessellate_body_with_threads(&body, &params(tolerance), threads) {
            Ok(mesh) => {
                meshes += 1;
                assert_eq!(mesh.normals.len(), mesh.positions.len(), "round {round}");
                assert_eq!(mesh.face_ids.len(), mesh.triangles.len(), "round {round}");
                assert!(
                    mesh.positions.iter().all(|p| p.is_finite()),
                    "round {round}"
                );
                for (t, tri) in mesh.triangles.iter().enumerate() {
                    assert!(
                        tri.iter().all(|&k| (k as usize) < mesh.positions.len()),
                        "round {round}"
                    );
                    let [a, b, c] = mesh.corners(t).unwrap();
                    assert!(
                        (b - a).cross(c - a).length() > 0.0,
                        "round {round}: no area"
                    );
                }
            }
            Err(error) => {
                errors += 1;
                assert!(!error.to_string().is_empty());
                assert!(
                    !matches!(error, TessError::Internal(_)),
                    "round {round}: {error}"
                );
            }
        }
    }
    println!("{meshes} meshes, {errors} errors");
    assert!(
        meshes > 20 && errors > 20,
        "{meshes} meshes, {errors} errors"
    );
}
