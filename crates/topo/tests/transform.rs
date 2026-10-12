//! Moving, mirroring, boxing and merging bodies (`Body::transformed`,
//! `Body::bbox`, `merge`) on every reference solid of the test kit: the
//! primitives, the trimmed solids, and the ones with Bézier and NURBS
//! faces. The extrusions are in `crates/kernel/tests/transform.rs`, the
//! check against the tessellator in `crates/tess/tests/transformed.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "common/transform_checks.rs"]
mod checks;

use checks::{check_image, check_mass, check_round_trip, isometries, tolerance};
use craft3d_geom::{Curve3, Line, Plane, Surface};
use craft3d_math::{Bbox3, Frame, Transform, Vec3};
use craft3d_testkit::solids::{self, Reference};
use craft3d_testkit::trimmed;
use craft3d_topo::{
    Body, BodyKind, Coedge, EdgeId, FaceId, LoopId, Offsets, TransformError, VertexId, check,
    mass_properties, merge,
};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

fn frames(scale: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap(),
    ]
}

/// Every solid `mass_properties` can confirm.
fn confirmed(frame: &Frame, scale: f64) -> Vec<Reference> {
    let mut all = solids::all(frame, scale).unwrap();
    all.extend(trimmed::all(frame, scale).unwrap());
    all
}

/// The two it cannot: a torus face with loops around the tube, and a
/// sphere face whose boundary passes through the pole.
fn unconfirmed(frame: &Frame, scale: f64) -> Vec<Reference> {
    vec![
        trimmed::pipe_elbow(frame, 3.0 * scale, 1.0 * scale, 1.9).unwrap(),
        trimmed::sphere_octant(frame, 2.0 * scale).unwrap(),
    ]
}

fn tilted() -> Frame {
    frames(1.0)[1]
}

#[test]
fn every_solid_survives_every_isometry() {
    for scale in [1e-3, 1.0, 1e6] {
        for (f, frame) in frames(scale).iter().enumerate() {
            for solid in confirmed(frame, scale) {
                for (name, t) in isometries(scale) {
                    let what = format!("{} (frame {f}, scale {scale}) under {name}", solid.name);
                    let image = check_image(&what, &solid.body, &t, scale);
                    check_mass(&what, &solid.body, &image, &t, scale);
                    check_round_trip(&what, &solid.body, &image, &t, scale);
                }
            }
            for solid in unconfirmed(frame, scale) {
                for (name, t) in isometries(scale) {
                    let what = format!("{} (frame {f}, scale {scale}) under {name}", solid.name);
                    let image = check_image(&what, &solid.body, &t, scale);
                    check_round_trip(&what, &solid.body, &image, &t, scale);
                }
            }
        }
    }
}

#[test]
fn the_fixtures_cover_every_kind_of_curve_and_surface() {
    // The test above means little for a kind no fixture has.
    let mut curves = std::collections::BTreeSet::new();
    let mut surfaces = std::collections::BTreeSet::new();
    let frame = tilted();
    for solid in confirmed(&frame, 1.0)
        .iter()
        .chain(&unconfirmed(&frame, 1.0))
    {
        curves.extend(solid.body.edges.iter().map(|e| e.curve.kind()));
        surfaces.extend(solid.body.faces.iter().map(|f| f.surface.kind()));
    }
    assert_eq!(
        curves.into_iter().collect::<Vec<_>>(),
        ["circle", "ellipse", "line", "nurbs curve"]
    );
    assert_eq!(
        surfaces.into_iter().collect::<Vec<_>>(),
        [
            "bezier patch",
            "cone",
            "cylinder",
            "nurbs surface",
            "plane",
            "sphere",
            "torus"
        ]
    );
}

#[test]
fn a_mirror_turns_loops_round_and_flips_the_flat_faces() {
    let mirror = Transform::mirror(Vec3::new(0.0, 5.0, 0.0), Vec3::new(1.0, -3.0, 2.0)).unwrap();
    let rigid = Transform::rotation_about(Vec3::X, Vec3::new(1.0, 2.0, 3.0), 1.0).unwrap();
    let frame = tilted();
    for solid in confirmed(&frame, 1.0)
        .iter()
        .chain(&unconfirmed(&frame, 1.0))
    {
        let body = &solid.body;
        // A rigid motion changes nothing but the geometry.
        let moved = body.transformed(&rigid).unwrap();
        assert_eq!(moved.loops, body.loops, "{}", solid.name);
        for (a, b) in body.faces.iter().zip(&moved.faces) {
            assert_eq!(a.reversed, b.reversed, "{}", solid.name);
        }

        let image = body.transformed(&mirror).unwrap();
        for (a, b) in body.loops.iter().zip(&image.loops) {
            // [c0, c1, .., cn] becomes [c0', cn', .., c1'].
            let mut expected: Vec<Coedge> = a
                .coedges
                .iter()
                .map(|c| Coedge {
                    edge: c.edge,
                    reversed: !c.reversed,
                })
                .collect();
            expected[1..].reverse();
            assert_eq!(b.coedges, expected, "{}", solid.name);
        }
        for (i, (a, b)) in body.faces.iter().zip(&image.faces).enumerate() {
            let flat = matches!(
                a.surface,
                Surface::Plane(_) | Surface::Bezier(_) | Surface::Nurbs(_)
            );
            assert_eq!(
                b.reversed,
                a.reversed != flat,
                "{}: face {i} ({})",
                solid.name,
                a.surface.kind()
            );
        }
        // Edges are untouched apart from their geometry.
        for (a, b) in body.edges.iter().zip(&image.edges) {
            assert_eq!((a.range, a.start, a.end), (b.range, b.start, b.end));
        }
    }
}

#[test]
fn a_mirrored_cone_opens_the_right_way() {
    // Standing on z = 0 with its apex at z = 3; mirrored in z = 0 it hangs
    // below with its apex at z = -3.
    let cone = solids::cone(&Frame::WORLD, 2.0, 3.0).unwrap();
    let mirror = Transform::mirror(Vec3::ZERO, Vec3::Z).unwrap();
    let image = cone.body.transformed(&mirror).unwrap();
    let (Surface::Cone(before), Surface::Cone(after)) =
        (&cone.body.faces[0].surface, &image.faces[0].surface)
    else {
        panic!("face 0 of the cone is its side");
    };
    assert_eq!(after.half_angle(), -before.half_angle());
    // The image of the axis is -z, and the mirrored frame takes the
    // opposite of that: the frame still looks up, the cone now widens
    // upward from an apex below.
    assert!(after.frame().z().distance(Vec3::Z) < 1e-15);
    assert!(after.half_angle() > 0.0);
    let apex = after.frame().point(0.0, 0.0, after.apex_v());
    assert!(apex.distance(Vec3::new(0.0, 0.0, -3.0)) < 1e-14);
    // Halfway down the radius is half the base radius.
    let (u, v) = image.faces[0]
        .surface
        .project(Vec3::new(1.0, 0.0, -1.5))
        .unwrap();
    let p = image.faces[0].surface.eval(u, v);
    assert!(p.distance(Vec3::new(1.0, 0.0, -1.5)) < 1e-14);
    let mass = mass_properties(&image).unwrap();
    assert!((mass.volume - cone.volume).abs() < 1e-12 * cone.volume);
    assert!(mass.centroid.unwrap().distance(Vec3::new(0.0, 0.0, -0.75)) < 1e-13);
}

#[test]
fn group_laws_hold() {
    let scale = 1.0;
    let frame = tilted();
    let (point, axis) = (Vec3::new(2.0, -1.0, 0.5), Vec3::new(0.2, 1.0, -0.4));
    let (a, b) = (0.7, 1.9);
    let rot = |angle: f64| Transform::rotation_about(point, axis, angle).unwrap();
    let mirror = Transform::mirror(point, axis).unwrap();
    let same = |what: &str, x: &Body, y: &Body| {
        assert_eq!(x.loops, y.loops, "{what}");
        for (v, w) in x.vertices.iter().zip(&y.vertices) {
            assert!(v.point.distance(w.point) <= 1e-12 * 10.0, "{what}");
        }
        for (e, f) in x.edges.iter().zip(&y.edges) {
            assert_eq!(e.range, f.range, "{what}");
            for k in 0..=4 {
                let s = e.range.0 + (e.range.1 - e.range.0) * f64::from(k) / 4.0;
                assert!(e.curve.eval(s).distance(f.curve.eval(s)) <= 1e-11, "{what}");
            }
        }
        for (f, g) in x.faces.iter().zip(&y.faces) {
            assert_eq!(f.reversed, g.reversed, "{what}");
        }
        let (m, n) = (mass_properties(x), mass_properties(y));
        if let (Ok(m), Ok(n)) = (m, n) {
            assert!((m.volume - n.volume).abs() <= 1e-12 * m.volume, "{what}");
            assert!((m.area - n.area).abs() <= 1e-12 * m.area, "{what}");
            assert!(
                m.centroid.unwrap().distance(n.centroid.unwrap()) <= 1e-11,
                "{what}"
            );
        }
    };
    for solid in confirmed(&frame, scale)
        .iter()
        .chain(&unconfirmed(&frame, scale))
    {
        let body = &solid.body;
        // Mirroring twice is the identity.
        let twice = body
            .transformed(&mirror)
            .unwrap()
            .transformed(&mirror)
            .unwrap();
        same(&format!("{}: mirrored twice", solid.name), body, &twice);
        // Turning by a and then by b about one axis is turning by a + b.
        let stepwise = body
            .transformed(&rot(a))
            .unwrap()
            .transformed(&rot(b))
            .unwrap();
        let at_once = body.transformed(&rot(a + b)).unwrap();
        same(&format!("{}: a then b", solid.name), &stepwise, &at_once);
        // A full turn in seven steps comes home.
        let mut turned = body.clone();
        for _ in 0..7 {
            turned = turned.transformed(&rot(TAU / 7.0)).unwrap();
        }
        same(&format!("{}: seven sevenths", solid.name), body, &turned);
        // One after the other is the composition, mirrors included.
        let t = rot(a).then(&mirror);
        let stepwise = body
            .transformed(&rot(a))
            .unwrap()
            .transformed(&mirror)
            .unwrap();
        same(
            &format!("{}: composed", solid.name),
            &stepwise,
            &body.transformed(&t).unwrap(),
        );
    }
}

/// The box of `points`.
fn box_of(points: impl IntoIterator<Item = Vec3>) -> Bbox3 {
    Bbox3::from_points(points)
}

/// True when `inner` is inside `outer` grown by `margin`.
fn within(inner: &Bbox3, outer: &Bbox3, margin: f64) -> bool {
    let (lo, hi) = inner.corners().unwrap();
    outer.contains(lo, margin) && outer.contains(hi, margin)
}

fn assert_same_box(what: &str, got: &Bbox3, want: &Bbox3, margin: f64) {
    assert!(
        within(got, want, margin) && within(want, got, margin),
        "{what}: got {got:?}, want {want:?}"
    );
}

/// The exact box of the disc of `radius` about the z axis of `frame`.
fn disc_box(frame: &Frame, radius: f64) -> Bbox3 {
    let (o, z) = (frame.origin(), frame.z());
    let reach = Vec3::new(
        radius * (1.0 - z.x * z.x).sqrt(),
        radius * (1.0 - z.y * z.y).sqrt(),
        radius * (1.0 - z.z * z.z).sqrt(),
    );
    box_of([o - reach, o + reach])
}

fn union(a: &Bbox3, b: &Bbox3) -> Bbox3 {
    let ((a0, a1), (b0, b1)) = (a.corners().unwrap(), b.corners().unwrap());
    box_of([a0, a1, b0, b1])
}

#[test]
fn boxes_of_the_primitives_are_the_closed_forms() {
    for scale in [1e-3, 1.0, 1e6] {
        for frame in frames(scale) {
            let margin = 1e-12 * scale;
            // A block: the box of its eight corners.
            let (dx, dy, dz) = (4.0 * scale, 2.0 * scale, 1.0 * scale);
            let block = solids::block(&frame, dx, dy, dz).unwrap().body;
            let corners = (0..8).map(|i| {
                frame.point(
                    f64::from(i & 1) * dx,
                    f64::from((i >> 1) & 1) * dy,
                    f64::from((i >> 2) & 1) * dz,
                )
            });
            assert_same_box("block", &block.bbox().unwrap(), &box_of(corners), margin);

            // A cylinder: its two discs.
            let (r, h) = (1.5 * scale, 4.0 * scale);
            let cylinder = solids::cylinder(&frame, r, h).unwrap().body;
            let top = frame.at(frame.point(0.0, 0.0, h)).unwrap();
            let want = union(&disc_box(&frame, r), &disc_box(&top, r));
            assert_same_box("cylinder", &cylinder.bbox().unwrap(), &want, margin);

            // A cone: its base disc and its apex, which no edge reaches.
            let (r, h) = (2.0 * scale, 30.0 * scale);
            let cone = solids::cone(&frame, r, h).unwrap().body;
            let base = disc_box(&frame, r);
            let (lo, hi) = base.corners().unwrap();
            let want = box_of([lo, hi, frame.point(0.0, 0.0, h)]);
            assert_same_box("cone", &cone.bbox().unwrap(), &want, margin);

            // A frustum far from its apex: the two discs, and *not* the
            // apex (which is 62.5 scale up the axis).
            let (r0, r1, h) = (2.0 * scale, 1.9 * scale, 2.5 * scale);
            let frustum = trimmed::cone_frustum(&frame, r0, r1, h).unwrap().body;
            let top = frame.at(frame.point(0.0, 0.0, h)).unwrap();
            let want = union(&disc_box(&frame, r0), &disc_box(&top, r1));
            assert_same_box("frustum", &frustum.bbox().unwrap(), &want, margin);

            // A sphere and a torus: the whole surface.
            let r = 2.5 * scale;
            let sphere = solids::sphere(&frame, r).unwrap().body;
            let o = frame.origin();
            let want = box_of([o - Vec3::new(r, r, r), o + Vec3::new(r, r, r)]);
            assert_same_box("sphere", &sphere.bbox().unwrap(), &want, margin);

            let (major, minor) = (3.0 * scale, 1.0 * scale);
            let torus = solids::torus(&frame, major, minor).unwrap().body;
            let (lo, hi) = disc_box(&frame, major).corners().unwrap();
            let m = Vec3::new(minor, minor, minor);
            let want = box_of([lo - m, hi + m]);
            assert_same_box("torus", &torus.bbox().unwrap(), &want, margin);
        }
    }
}

/// Many points of every edge of `body`.
fn edge_points(body: &Body) -> Vec<Vec3> {
    let mut out = Vec::new();
    for edge in &body.edges {
        for k in 0..=2048 {
            let s = edge.range.0 + (edge.range.1 - edge.range.0) * f64::from(k) / 2048.0;
            out.push(edge.curve.eval(s));
        }
    }
    out
}

#[test]
fn boxes_are_tight_where_promised_and_bounded_elsewhere() {
    for scale in [1e-3, 1.0, 1e6] {
        for frame in frames(scale) {
            for solid in confirmed(&frame, scale)
                .iter()
                .chain(&unconfirmed(&frame, scale))
            {
                for (name, t) in isometries(scale) {
                    let what = format!("{} at scale {scale} under {name}", solid.name);
                    let body = solid.body.transformed(&t).unwrap();
                    let bbox = body.bbox().unwrap();
                    // What the box may be made of at most: the edges, and
                    // the whole of every sphere, torus and control net. (A
                    // cone apex inside a face is added for the one fixture
                    // that has it.)
                    let mut allowed = edge_points(&body);
                    let mut exact = true;
                    for face in &body.faces {
                        match &face.surface {
                            Surface::Plane(_) | Surface::Cylinder(_) => {}
                            Surface::Cone(cone) => {
                                if solid.name == "cone" {
                                    allowed.push(cone.frame().point(0.0, 0.0, cone.apex_v()));
                                }
                            }
                            Surface::Sphere(_) | Surface::Torus(_) => {
                                exact &= face.loops.is_empty();
                                for a in 0..=256 {
                                    for b in 0..=256 {
                                        let u = TAU * f64::from(a) / 256.0;
                                        let v = if face.surface.period_v().is_some() {
                                            TAU * f64::from(b) / 256.0
                                        } else {
                                            -FRAC_PI_2 + PI * f64::from(b) / 256.0
                                        };
                                        allowed.push(face.surface.eval(u, v));
                                    }
                                }
                            }
                            Surface::Bezier(patch) => {
                                exact = false;
                                allowed.extend(patch.control_points());
                            }
                            Surface::Nurbs(surface) => {
                                exact = false;
                                allowed.extend(surface.control_points());
                            }
                        }
                    }
                    exact &= body
                        .edges
                        .iter()
                        .all(|e| !matches!(e.curve, Curve3::Nurbs(_)));
                    for edge in &body.edges {
                        if let Curve3::Nurbs(c) = &edge.curve {
                            allowed.extend(c.control_points());
                        }
                    }
                    let allowed = box_of(allowed);
                    // The samples miss the true extremes by the sag of an
                    // arc between two of them: r (1 − cos(π/256)) < 1e-4 r.
                    let size = allowed.diagonal();
                    assert!(
                        within(&bbox, &allowed, 1e-4 * size),
                        "{what}: the box {bbox:?} is looser than promised ({allowed:?})"
                    );
                    if exact {
                        assert!(
                            within(&allowed, &bbox, 0.0),
                            "{what}: the box {bbox:?} misses {allowed:?}"
                        );
                    }
                    // Padding is a few rounding errors, not a margin.
                    let (lo, hi) = bbox.corners().unwrap();
                    let largest = [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]
                        .iter()
                        .fold(0.0_f64, |m, c| m.max(c.abs()));
                    assert!(
                        within(&bbox, &allowed, 1e-4 * size) && largest.is_finite(),
                        "{what}"
                    );
                }
            }
        }
    }
}

#[test]
fn arcs_are_boxed_by_the_part_that_is_there() {
    // A quarter circle of radius 2 in the xy plane, from 30° to 120°: its
    // box reaches the top of the circle (90° is in range) and stops at the
    // end points otherwise.
    let mut body = Body::new(BodyKind::Sheet);
    let circle = Curve3::Circle(craft3d_geom::Circle::new(Frame::WORLD, 2.0).unwrap());
    let (t0, t1) = (PI / 6.0, 2.0 * PI / 3.0);
    let a = body.add_vertex(circle.eval(t0));
    let b = body.add_vertex(circle.eval(t1));
    body.add_edge(circle.clone(), (t0, t1), a, b);
    let (lo, hi) = body.bbox().unwrap().corners().unwrap();
    let close = |x: f64, y: f64| (x - y).abs() < 1e-12;
    assert!(
        close(lo.x, -1.0) && close(hi.x, 3.0_f64.sqrt()),
        "{lo:?} {hi:?}"
    );
    assert!(close(lo.y, 1.0) && close(hi.y, 2.0), "{lo:?} {hi:?}");
    assert!(close(lo.z, 0.0) && close(hi.z, 0.0));

    // The same arc given one turn further on is the same arc.
    let mut later = Body::new(BodyKind::Sheet);
    let a = later.add_vertex(circle.eval(t0));
    let b = later.add_vertex(circle.eval(t1));
    later.add_edge(circle.clone(), (t0 + 3.0 * TAU, t1 + 3.0 * TAU), a, b);
    let (lo2, hi2) = later.bbox().unwrap().corners().unwrap();
    assert!(lo2.distance(lo) < 1e-12 && hi2.distance(hi) < 1e-12);

    // An ellipse arc through its left end: 3 by 1, from 170° to 190°.
    let mut body = Body::new(BodyKind::Sheet);
    let ellipse = Curve3::Ellipse(craft3d_geom::Ellipse::new(Frame::WORLD, 3.0, 1.0).unwrap());
    let (t0, t1) = (170.0_f64.to_radians(), 190.0_f64.to_radians());
    let a = body.add_vertex(ellipse.eval(t0));
    let b = body.add_vertex(ellipse.eval(t1));
    body.add_edge(ellipse.clone(), (t0, t1), a, b);
    let (lo, hi) = body.bbox().unwrap().corners().unwrap();
    assert!(
        close(lo.x, -3.0) && close(hi.x, 3.0 * t0.cos()),
        "{lo:?} {hi:?}"
    );
    assert!(
        close(lo.y, t1.sin()) && close(hi.y, t0.sin()),
        "{lo:?} {hi:?}"
    );
}

fn block() -> Body {
    solids::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap().body
}

fn cylinder() -> Body {
    solids::cylinder(&Frame::WORLD, 1.5, 4.0).unwrap().body
}

#[test]
fn anything_but_an_isometry_is_refused() {
    let shear = Transform::rotation(Vec3::Z, 0.3)
        .unwrap()
        .then(&Transform::scaling(1.0 + 1e-6));
    let bad = [
        Transform::scaling(2.0),
        Transform::scaling(0.5),
        Transform::scaling(-2.0),
        Transform::scaling(0.0),
        Transform::scaling(f64::NAN),
        Transform::scaling(f64::INFINITY),
        Transform::translation(Vec3::new(f64::NAN, 0.0, 0.0)),
        Transform::translation(Vec3::new(0.0, f64::NEG_INFINITY, 0.0)),
        shear,
    ];
    for t in bad {
        for body in [block(), cylinder(), Body::new(BodyKind::Solid)] {
            assert_eq!(body.transformed(&t), Err(TransformError::NotIsometry));
        }
    }
    let message = TransformError::NotIsometry.to_string();
    assert!(message.contains("scaling"), "{message}");
}

#[test]
fn a_move_out_of_the_representable_range_is_an_error() {
    let far = Transform::translation(Vec3::new(f64::MAX, 0.0, 0.0));
    let at_the_edge = block()
        .transformed(&Transform::translation(Vec3::new(f64::MAX / 2.0, 0.0, 0.0)))
        .unwrap();
    assert!(matches!(
        at_the_edge.transformed(&far),
        Err(TransformError::Vertex { .. })
    ));
    // A body without vertices fails at its geometry instead.
    let sphere = solids::sphere(&Frame::WORLD, 1.0).unwrap().body;
    let sphere = sphere
        .transformed(&Transform::translation(Vec3::new(f64::MAX / 2.0, 0.0, 0.0)))
        .unwrap();
    assert!(matches!(
        sphere.transformed(&far),
        Err(TransformError::Face {
            face: FaceId(0),
            ..
        })
    ));
}

/// Bodies that fail `check`, each in its own way.
fn broken() -> Vec<Body> {
    let mut all = Vec::new();
    let mut b = block();
    b.edges[3].start = VertexId(99);
    all.push(b);
    let mut b = block();
    b.loops[2].coedges[1].edge = EdgeId(u32::MAX);
    all.push(b);
    let mut b = block();
    b.faces[1].loops.push(LoopId(77));
    all.push(b);
    let mut b = block();
    b.shells[0].faces.push(FaceId(1234));
    all.push(b);
    let mut b = block();
    b.loops[0].coedges.clear();
    all.push(b);
    let mut b = block();
    b.loops.clear();
    all.push(b);
    let mut b = cylinder();
    b.edges[0].range = (5.0, -5.0);
    all.push(b);
    let mut b = cylinder();
    b.faces[0].loops.clear();
    all.push(b);
    let mut b = cylinder();
    b.vertices.clear();
    all.push(b);
    let mut b = solids::cone(&Frame::WORLD, 2.0, 3.0).unwrap().body;
    b.loops[0].coedges[0].edge = EdgeId(9);
    all.push(b);
    let mut b = solids::cone(&Frame::WORLD, 2.0, 3.0).unwrap().body;
    b.faces[0].loops = vec![LoopId(9)];
    all.push(b);
    all
}

#[test]
fn invalid_bodies_never_panic() {
    for (i, body) in broken().iter().enumerate() {
        assert!(
            !check(body, &tolerance(1.0)).is_empty(),
            "case {i} is valid"
        );
        for (name, t) in isometries(1.0) {
            // Entity by entity, without following references: it works,
            // and the result is as invalid as the original.
            let image = body
                .transformed(&t)
                .unwrap_or_else(|e| panic!("case {i} under {name}: {e}"));
            assert_eq!(
                check(&image, &tolerance(1.0)).len(),
                check(body, &tolerance(1.0)).len(),
                "case {i} under {name}"
            );
            let _ = image.bbox();
            let _ = merge(&[body, &image]);
        }
        let _ = body.bbox();
    }
}

#[test]
fn non_finite_bodies_are_errors() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut b = block();
        b.vertices[5].point.y = bad;
        let vertex = TransformError::Vertex {
            vertex: VertexId(5),
        };
        assert_eq!(
            b.transformed(&Transform::IDENTITY).err(),
            Some(vertex.clone())
        );
        assert_eq!(b.bbox().err(), Some(vertex));

        let mut b = cylinder();
        b.edges[1].range.1 = bad;
        // The range is copied as it is; the box needs it.
        assert!(b.transformed(&Transform::IDENTITY).is_ok());
        assert_eq!(b.bbox(), Err(TransformError::EdgeRange { edge: EdgeId(1) }));
        // On a cone the range is also walked to look for the apex.
        let mut b = solids::cone(&Frame::WORLD, 2.0, 3.0).unwrap().body;
        b.edges[0].range.0 = bad;
        assert_eq!(b.bbox(), Err(TransformError::EdgeRange { edge: EdgeId(0) }));
    }
}

#[test]
fn boxes_of_empty_and_unbounded_bodies() {
    for kind in [BodyKind::Solid, BodyKind::Sheet] {
        let empty = Body::new(kind);
        assert_eq!(empty.bbox(), Ok(Bbox3::EMPTY));
        let image = empty
            .transformed(&Transform::mirror(Vec3::ZERO, Vec3::X).unwrap())
            .unwrap();
        assert_eq!(image, empty);
    }
    // A plane, a cylinder and a cone without loops go on for ever.
    let frame = tilted();
    let unbounded = [
        Surface::Plane(Plane::new(frame)),
        Surface::Cylinder(craft3d_geom::Cylinder::new(frame, 1.0).unwrap()),
        Surface::Cone(craft3d_geom::Cone::new(frame, 1.0, 0.5).unwrap()),
    ];
    for surface in unbounded {
        let mut body = Body::new(BodyKind::Sheet);
        body.add_vertex(Vec3::ZERO);
        let face = body.add_face(surface, false, Vec::new());
        body.add_shell(vec![face]);
        assert_eq!(body.bbox(), Err(TransformError::UnboundedFace { face }));
        // Moving it is no problem.
        assert!(body.transformed(&Transform::translation(Vec3::X)).is_ok());
    }
    // A lone vertex is its own box, padded.
    let mut body = Body::new(BodyKind::Sheet);
    body.add_vertex(Vec3::new(1.0, 2.0, 3.0));
    let bbox = body.bbox().unwrap();
    assert!(bbox.contains(Vec3::new(1.0, 2.0, 3.0), 0.0));
    assert!(bbox.diagonal() < 1e-12);
}

/// A flat square sheet in the xy plane of `frame`: one face, four edges.
fn square_sheet(frame: &Frame, side: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let corners = [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side)];
    let vertices: Vec<VertexId> = corners
        .iter()
        .map(|&(x, y)| body.add_vertex(frame.point(x, y, 0.0)))
        .collect();
    let mut coedges = Vec::new();
    for i in 0..4 {
        let (a, b) = (vertices[i], vertices[(i + 1) % 4]);
        let (p, q) = (body.vertex(a).unwrap().point, body.vertex(b).unwrap().point);
        let line = Curve3::Line(Line::through(p, q).unwrap());
        let edge = body.add_edge(line, (0.0, p.distance(q)), a, b);
        coedges.push(Coedge {
            edge,
            reversed: false,
        });
    }
    let loop_ = body.add_loop(coedges);
    let face = body.add_face(Surface::Plane(Plane::new(*frame)), false, vec![loop_]);
    body.add_shell(vec![face]);
    body
}

#[test]
fn sheets_move_and_mirror_too() {
    for scale in [1e-3, 1.0, 1e6] {
        for frame in frames(scale) {
            let sheet = square_sheet(&frame, 3.0 * scale);
            for (name, t) in isometries(scale) {
                let what = format!("sheet at scale {scale} under {name}");
                let image = check_image(&what, &sheet, &t, scale);
                check_mass(&what, &sheet, &image, &t, scale);
                check_round_trip(&what, &sheet, &image, &t, scale);
                // The side the sheet faces is the image of that side.
                let n = checks::outward(&sheet.faces[0], frame.point(scale, scale, 0.0)).unwrap();
                let m = checks::outward(&image.faces[0], t.point(frame.point(scale, scale, 0.0)))
                    .unwrap();
                assert!(m.distance(t.vector(n)) < 1e-12, "{what}");
            }
        }
    }
}

/// Three copies of every confirmed solid, far enough apart not to touch.
fn spread(solid: &Reference, scale: f64) -> Vec<Body> {
    let step = Vec3::new(40.0, 0.0, 0.0) * scale;
    vec![
        solid.body.clone(),
        solid
            .body
            .transformed(&Transform::translation(step))
            .unwrap(),
        solid
            .body
            .transformed(
                &Transform::mirror(Vec3::ZERO, Vec3::Y)
                    .unwrap()
                    .then(&Transform::translation(step * -1.0)),
            )
            .unwrap(),
    ]
}

/// Checks that entity `i` of `part` is entity `offset + i` of `merged`.
fn assert_embedded(what: &str, merged: &Body, part: &Body, at: &Offsets) {
    for (i, v) in part.vertices.iter().enumerate() {
        assert_eq!(&merged.vertices[at.vertices as usize + i], v, "{what}");
    }
    for (i, e) in part.edges.iter().enumerate() {
        let m = &merged.edges[at.edges as usize + i];
        assert_eq!((&m.curve, m.range), (&e.curve, e.range), "{what}");
        assert_eq!(
            (m.start, m.end),
            (
                VertexId(e.start.0 + at.vertices),
                VertexId(e.end.0 + at.vertices)
            ),
            "{what}: edge {i}"
        );
    }
    for (i, l) in part.loops.iter().enumerate() {
        let expected: Vec<Coedge> = l
            .coedges
            .iter()
            .map(|c| Coedge {
                edge: EdgeId(c.edge.0 + at.edges),
                reversed: c.reversed,
            })
            .collect();
        assert_eq!(
            merged.loops[at.loops as usize + i].coedges,
            expected,
            "{what}: loop {i}"
        );
    }
    for (i, f) in part.faces.iter().enumerate() {
        let m = &merged.faces[at.faces as usize + i];
        assert_eq!((&m.surface, m.reversed), (&f.surface, f.reversed), "{what}");
        let expected: Vec<LoopId> = f.loops.iter().map(|l| LoopId(l.0 + at.loops)).collect();
        assert_eq!(m.loops, expected, "{what}: face {i}");
    }
    for (i, s) in part.shells.iter().enumerate() {
        let expected: Vec<FaceId> = s.faces.iter().map(|f| FaceId(f.0 + at.faces)).collect();
        assert_eq!(
            merged.shells[at.shells as usize + i].faces,
            expected,
            "{what}: shell {i}"
        );
    }
}

#[test]
fn merged_bodies_keep_every_entity_at_its_offset() {
    for scale in [1e-3, 1.0, 1e6] {
        let frame = frames(scale)[1];
        // Different solids in one body, so that every offset differs.
        let all = confirmed(&frame, scale);
        for window in all.windows(3) {
            let step = Vec3::new(40.0, 0.0, 0.0) * scale;
            let parts = [
                window[0].body.clone(),
                window[1]
                    .body
                    .transformed(&Transform::translation(step))
                    .unwrap(),
                window[2]
                    .body
                    .transformed(&Transform::translation(step * 2.0))
                    .unwrap(),
            ];
            let what = format!(
                "{} + {} + {} at scale {scale}",
                window[0].name, window[1].name, window[2].name
            );
            let refs: Vec<&Body> = parts.iter().collect();
            let (merged, offsets) = merge(&refs).unwrap();
            assert_eq!(offsets.len(), 3, "{what}");
            assert_eq!(offsets[0], Offsets::default(), "{what}");

            // Counts add up, and each offset is the sum of what came before.
            let mut expected = Offsets::default();
            for (part, at) in parts.iter().zip(&offsets) {
                assert_eq!(*at, expected, "{what}");
                assert_embedded(&what, &merged, part, at);
                expected.vertices += part.vertices.len() as u32;
                expected.edges += part.edges.len() as u32;
                expected.loops += part.loops.len() as u32;
                expected.faces += part.faces.len() as u32;
                expected.shells += part.shells.len() as u32;
            }
            assert_eq!(merged.vertices.len(), expected.vertices as usize, "{what}");
            assert_eq!(merged.edges.len(), expected.edges as usize, "{what}");
            assert_eq!(merged.loops.len(), expected.loops as usize, "{what}");
            assert_eq!(merged.faces.len(), expected.faces as usize, "{what}");
            assert_eq!(merged.shells.len(), expected.shells as usize, "{what}");
            assert_eq!(merged.kind, BodyKind::Solid, "{what}");

            // Disjoint solids make a valid solid whose volume and area are
            // the sums.
            assert_eq!(check(&merged, &tolerance(scale)), vec![], "{what}");
            let mass = mass_properties(&merged).unwrap();
            let masses: Vec<_> = parts.iter().map(|p| mass_properties(p).unwrap()).collect();
            let volume: f64 = masses.iter().map(|m| m.volume).sum();
            let area: f64 = masses.iter().map(|m| m.area).sum();
            assert!((mass.volume - volume).abs() <= 1e-12 * volume, "{what}");
            assert!((mass.area - area).abs() <= 1e-12 * area, "{what}");

            // The box of the whole is the box of the parts.
            let (lo, hi) = merged.bbox().unwrap().corners().unwrap();
            let corners = parts.iter().flat_map(|p| {
                let (a, b) = p.bbox().unwrap().corners().unwrap();
                [a, b]
            });
            let (want_lo, want_hi) = box_of(corners).corners().unwrap();
            let slack = 1e-12 * want_lo.distance(want_hi);
            assert!(lo.distance(want_lo) <= slack && hi.distance(want_hi) <= slack);

            // Bit-identical on repeat.
            let (again, again_offsets) = merge(&refs).unwrap();
            assert_eq!(format!("{again:?}"), format!("{merged:?}"), "{what}");
            assert_eq!(again_offsets, offsets, "{what}");
        }
    }
}

#[test]
fn merging_copies_of_one_solid_multiplies_it() {
    let frame = tilted();
    for solid in confirmed(&frame, 1.0) {
        let parts = spread(&solid, 1.0);
        let refs: Vec<&Body> = parts.iter().collect();
        let (merged, offsets) = merge(&refs).unwrap();
        assert_eq!(check(&merged, &tolerance(1.0)), vec![], "{}", solid.name);
        for (part, at) in parts.iter().zip(&offsets) {
            assert_embedded(solid.name, &merged, part, at);
        }
        let mass = mass_properties(&merged).unwrap();
        assert!(
            (mass.volume - 3.0 * solid.volume).abs() <= 1e-9 * solid.volume,
            "{}",
            solid.name
        );
        assert!(
            (mass.area - 3.0 * solid.area).abs() <= 1e-9 * solid.area,
            "{}",
            solid.name
        );
        // One body merged alone is that body.
        let (alone, at) = merge(&[&solid.body]).unwrap();
        assert_eq!(alone, solid.body);
        assert_eq!(at, vec![Offsets::default()]);
    }
}

#[test]
fn merge_refuses_what_it_cannot_do() {
    assert_eq!(merge(&[]), Err(TransformError::NothingToMerge));
    let solid = block();
    let sheet = square_sheet(&Frame::WORLD, 1.0);
    assert_eq!(merge(&[&solid, &sheet]), Err(TransformError::MixedKinds));
    assert_eq!(
        merge(&[&sheet, &sheet, &solid]),
        Err(TransformError::MixedKinds)
    );
    // Sheets merge with sheets.
    let other = sheet
        .transformed(&Transform::translation(Vec3::new(5.0, 0.0, 0.0)))
        .unwrap();
    let (two, offsets) = merge(&[&sheet, &other]).unwrap();
    assert_eq!(two.kind, BodyKind::Sheet);
    assert_eq!(check(&two, &tolerance(1.0)), vec![]);
    assert_eq!(offsets[1].edges, 4);
    // Empty bodies are bodies.
    let empty = Body::new(BodyKind::Solid);
    let (merged, offsets) = merge(&[&empty, &solid, &empty]).unwrap();
    assert_eq!(merged, solid);
    assert_eq!(offsets[1], Offsets::default());
    assert_eq!(offsets[2].faces, 6);

    // A reference to nothing would point into the next body after the
    // offset: refused, and it says which body.
    for (i, body) in broken().iter().enumerate() {
        let dangling = [0, 1, 2, 3, 5, 8, 9, 10].contains(&i);
        for (k, pair) in [[&solid, body], [body, &solid]].iter().enumerate() {
            let result = merge(pair);
            if dangling {
                assert_eq!(
                    result.err(),
                    Some(TransformError::DanglingReference { body: 1 - k }),
                    "case {i}"
                );
            } else {
                assert!(result.is_ok(), "case {i}");
            }
        }
    }
}

#[test]
fn merge_says_plainly_that_it_does_not_unite() {
    // Two overlapping blocks: the result has both skins, and twice the
    // volume of the overlap counted double. That is the documented
    // behaviour, pinned here so that nobody takes merge for a union.
    let a = block();
    let b = a
        .transformed(&Transform::translation(Vec3::new(2.0, 0.0, 0.0)))
        .unwrap();
    let (merged, _) = merge(&[&a, &b]).unwrap();
    assert_eq!(merged.faces.len(), 12);
    let mass = mass_properties(&merged).unwrap();
    assert!((mass.volume - 16.0).abs() < 1e-12, "not the union's 12");
}
