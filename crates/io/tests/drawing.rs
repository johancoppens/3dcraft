//! Outlines for DXF and SVG from bodies and sketches made by hand: the
//! cases the commands cannot reach yet (a flat face with a spline in its
//! boundary), and the ones that must be refused. The command-level tests
//! are in `crates/engine/tests/drawing_export.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{Entity, EntityId, Geometry, Sketch, SketchPlane};
use craft3d_drawing::{Color, Drawing, Layer, Path, Segment, Unit};
use craft3d_geom::{Circle, Curve3, Cylinder, Line, NurbsCurve, Plane, Surface};
use craft3d_io::{DrawingFormat, encode_drawing, face_outline, sketch_outline};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, FaceId};
use std::f64::consts::TAU;

/// A plane that is none of the world's.
fn tilted() -> Frame {
    Frame::new(
        Vec3::new(3.0, -2.0, 7.0),
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

/// A cubic from `(0, 0)` to `(8, 0)` of the frame with a knot inside,
/// continued beyond both ends: the edge is the piece on `[1, 3]`.
fn long_curve(frame: &Frame) -> NurbsCurve {
    let ctrl = [
        [-3.0, -2.0],
        [-1.0, 1.0],
        [1.0, 4.0],
        [4.0, 5.0],
        [7.0, 4.0],
        [9.0, 1.0],
        [11.0, -2.0],
    ];
    NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 4.0, 4.0],
        ctrl.iter().map(|p| frame.point(p[0], p[1], 0.0)).collect(),
    )
    .unwrap()
}

/// A flat sheet: the piece of the long curve on `[1, 3]` and the chord
/// under it. `backwards` stores the loop's use of the spline edge against
/// the edge's direction; `reversed` turns the face over.
fn bump(backwards: bool, reversed: bool) -> (Body, FaceId, Frame, NurbsCurve) {
    let frame = tilted();
    let curve = long_curve(&frame);
    let (a, b) = (curve.eval(1.0), curve.eval(3.0));
    let mut body = Body::new(BodyKind::Sheet);
    let (va, vb) = (body.add_vertex(a), body.add_vertex(b));
    let spline = body.add_edge(Curve3::Nurbs(curve.clone()), (1.0, 3.0), va, vb);
    let chord = Curve3::Line(Line::through(a, b).unwrap());
    let length = a.distance(b);
    // The loop runs a → b along the chord and back along the spline, or
    // the other way round.
    let loop_ = if backwards {
        let chord = body.add_edge(chord, (0.0, length), va, vb);
        body.add_loop(vec![
            Coedge {
                edge: chord,
                reversed: false,
            },
            Coedge {
                edge: spline,
                reversed: true,
            },
        ])
    } else {
        let chord = body.add_edge(chord, (0.0, length), va, vb);
        body.add_loop(vec![
            Coedge {
                edge: spline,
                reversed: false,
            },
            Coedge {
                edge: chord,
                reversed: true,
            },
        ])
    };
    let face = body.add_face(Surface::Plane(Plane::new(frame)), reversed, vec![loop_]);
    (body, face, frame, curve)
}

fn drawing(paths: Vec<Path>) -> Drawing {
    Drawing {
        unit: Unit::Millimeter,
        layers: vec![Layer {
            name: "outline".into(),
            color: Color::Black,
            paths,
        }],
    }
}

#[test]
fn a_spline_edge_of_a_flat_face_is_a_spline_in_the_outline() {
    for backwards in [false, true] {
        for reversed in [false, true] {
            let (body, face, frame, curve) = bump(backwards, reversed);
            let outline = face_outline(&body, &[face], Tolerance::default()).unwrap();
            // Seen from the side the face looks to.
            let side = if reversed { -1.0 } else { 1.0 };
            assert_eq!(outline.frame.x(), frame.x());
            assert_eq!(outline.frame.z(), frame.z() * side);
            let flat = |p: Vec3| {
                let (x, y, _) = outline.frame.local(p);
                [x, y]
            };
            let [
                Path::Chain {
                    start,
                    segments,
                    closed: true,
                },
            ] = outline.paths.as_slice()
            else {
                panic!("one closed chain: {:?}", outline.paths)
            };
            let (a, b) = (flat(curve.eval(1.0)), flat(curve.eval(3.0)));
            let spline = match (backwards, segments.as_slice()) {
                (false, [Segment::Spline(spline), Segment::Line { to }]) => {
                    assert_eq!((*start, *to), (a, a));
                    assert_eq!((spline.start(), spline.end()), (a, b));
                    spline
                }
                (true, [Segment::Line { to }, Segment::Spline(spline)]) => {
                    assert_eq!((*start, *to), (a, b));
                    assert_eq!((spline.start(), spline.end()), (b, a));
                    spline
                }
                other => panic!("{other:?}"),
            };
            // The piece on [1, 3], and nothing of the rest of the curve:
            // its points are the curve's, in the face's axes.
            assert_eq!(spline.degree(), 3);
            assert!(!spline.is_rational());
            let (t0, t1) = spline.domain();
            for i in 0..=50 {
                let s = f64::from(i) / 50.0;
                let got = spline.eval(t0 + (t1 - t0) * s).unwrap();
                let u = if backwards {
                    3.0 - 2.0 * s
                } else {
                    1.0 + 2.0 * s
                };
                let expected = flat(curve.eval(u));
                assert!(
                    (got[0] - expected[0]).hypot(got[1] - expected[1]) < 1e-12,
                    "{backwards} {reversed} {s}: {got:?} {expected:?}"
                );
            }
            // Both files take it: a SPLINE in DXF, exact cubics in SVG.
            let drawing = drawing(outline.paths.clone());
            let (dxf, _) = encode_drawing(&drawing, DrawingFormat::Dxf, 1e-6).unwrap();
            let dxf = String::from_utf8(dxf).unwrap();
            assert_eq!(dxf.matches("  0\nSPLINE\n").count(), 1);
            assert_eq!(dxf.matches("  0\nLINE\n").count(), 1);
            let (svg, report) = encode_drawing(&drawing, DrawingFormat::Svg, 1e-6).unwrap();
            let svg = String::from_utf8(svg).unwrap();
            assert_eq!(report.unwrap().approximated, 0);
            // Two spans of the curve lie on [1, 3].
            assert_eq!(svg.matches(" C").count(), 2, "{svg}");
            assert!(svg.contains(" Z\""));
        }
    }
}

#[test]
fn a_rational_circle_edge_is_a_rational_spline_of_the_circle_s_length() {
    let frame = tilted();
    let curve = NurbsCurve::circle(&frame, 2.5).unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    let v = body.add_vertex(curve.eval(0.0));
    let edge = body.add_edge(Curve3::Nurbs(curve), (0.0, 1.0), v, v);
    let loop_ = body.add_loop(vec![Coedge {
        edge,
        reversed: false,
    }]);
    let face = body.add_face(Surface::Plane(Plane::new(frame)), false, vec![loop_]);
    let outline = face_outline(&body, &[face], Tolerance::default()).unwrap();
    let [
        Path::Chain {
            segments, closed, ..
        },
    ] = outline.paths.as_slice()
    else {
        panic!("{:?}", outline.paths)
    };
    let [Segment::Spline(spline)] = segments.as_slice() else {
        panic!("{segments:?}")
    };
    assert!(*closed && spline.is_rational());
    assert_eq!(spline.start(), spline.end());
    let drawing = drawing(outline.paths.clone());
    let length = drawing.length();
    println!("the circle as a spline: length {length}, 2πr {}", TAU * 2.5);
    assert!((length - TAU * 2.5).abs() < 1e-9 * TAU * 2.5);
    let (dxf, _) = encode_drawing(&drawing, DrawingFormat::Dxf, 1e-6).unwrap();
    let dxf = String::from_utf8(dxf).unwrap();
    // Rational and planar, with its nine weights.
    assert!(dxf.contains("  0\nSPLINE\n") && dxf.contains(" 70\n12\n"));
    let entities = &dxf[dxf.find("  2\nENTITIES\n").unwrap()..];
    assert_eq!(entities.matches("\n 41\n").count(), 9);
    // SVG has no rational curves: cubics within the tolerance, and the
    // report says so.
    for tolerance in [1e-3, 1e-6] {
        let (_, report) = encode_drawing(&drawing, DrawingFormat::Svg, tolerance).unwrap();
        let report = report.unwrap();
        assert_eq!(report.approximated, 1);
        assert!(report.deviation > 0.0 && report.deviation <= tolerance);
    }
    assert!(encode_drawing(&drawing, DrawingFormat::Svg, 0.0).is_err());
    assert!(encode_drawing(&drawing, DrawingFormat::Svg, f64::NAN).is_err());
}

#[test]
fn faces_that_have_no_flat_outline_are_refused() {
    let frame = tilted();
    let refused = |body: &Body, faces: &[FaceId], word: &str| {
        let error = face_outline(body, faces, Tolerance::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains(word), "`{error}` lacks `{word}`");
    };
    let (mut body, face, ..) = bump(false, false);
    refused(&body, &[], "no face");
    refused(&body, &[FaceId(7)], "no face 7");
    // A face that is not flat.
    let wall = body.add_face(
        Surface::Cylinder(Cylinder::new(frame, 2.0).unwrap()),
        false,
        vec![],
    );
    refused(&body, &[wall], "cylinder face");
    // A flat face without a boundary, and one whose loop is missing.
    let bare = body.add_face(Surface::Plane(Plane::new(frame)), false, vec![]);
    refused(&body, &[bare], "no boundary");
    let broken = body.add_face(
        Surface::Plane(Plane::new(frame)),
        false,
        vec![craft3d_topo::LoopId(99)],
    );
    refused(&body, &[broken], "does not exist");
    // Two faces of one name that are not in one plane.
    let lifted = frame.at(frame.point(0.0, 0.0, 1.0)).unwrap();
    let loops = body.faces[face.0 as usize].loops.clone();
    let above = body.add_face(Surface::Plane(Plane::new(lifted)), false, loops.clone());
    refused(&body, &[face, above], "one plane");
    let turned = body.add_face(Surface::Plane(Plane::new(frame)), true, loops.clone());
    refused(&body, &[face, turned], "one plane");
    // Two that are: both outlines.
    let twin = body.add_face(Surface::Plane(Plane::new(frame)), false, loops);
    let outline = face_outline(&body, &[face, twin], Tolerance::default()).unwrap();
    assert_eq!(outline.paths.len(), 2);
    // A circle that stands on the plane instead of lying in it.
    let upright = Frame::new(frame.origin(), frame.x(), frame.z()).unwrap();
    let circle = Circle::new(upright, 1.0).unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    let v = body.add_vertex(Curve3::Circle(circle).eval(0.0));
    let edge = body.add_edge(Curve3::Circle(circle), (0.0, TAU), v, v);
    let loop_ = body.add_loop(vec![Coedge {
        edge,
        reversed: false,
    }]);
    let face = body.add_face(Surface::Plane(Plane::new(frame)), false, vec![loop_]);
    refused(&body, &[face], "does not lie in the face's plane");
}

fn entity(id: u64, geometry: Geometry) -> Entity {
    Entity {
        id: EntityId(id),
        construction: false,
        geometry,
    }
}

#[test]
fn a_sketch_that_is_not_solved_or_not_whole_is_refused() {
    let point = |id, x, y| entity(id, Geometry::Point { x, y });
    let mut sketch = Sketch::new(SketchPlane::default());
    sketch.entities = vec![
        point(1, 0.0, 0.0),
        point(2, 5.0, 0.0),
        point(3, 0.0, 7.0),
        // The ends are at 5 and 7 from the centre.
        entity(
            4,
            Geometry::Arc {
                center: EntityId(1),
                start: EntityId(2),
                end: EntityId(3),
            },
        ),
    ];
    let stored = |_| None;
    let no_radius = |_| None;
    let error = sketch_outline(&sketch, &stored, &no_radius, false, 1e-6)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("arc e4") && error.contains("sketch.solve"),
        "{error}"
    );
    // Solved positions put them right: the arc is one open chain.
    let solved = |id: EntityId| (id == EntityId(3)).then_some((0.0, 5.0));
    let outline = sketch_outline(&sketch, &solved, &no_radius, false, 1e-6).unwrap();
    let [
        Path::Chain {
            start,
            segments,
            closed: false,
        },
    ] = outline.paths.as_slice()
    else {
        panic!("{:?}", outline.paths)
    };
    assert_eq!(*start, [5.0, 0.0]);
    assert_eq!(
        segments.as_slice(),
        [Segment::Arc {
            to: [0.0, 5.0],
            center: [0.0, 0.0],
            radius: 5.0,
            ccw: true
        }]
    );
    // A curve on a point that is not there, a point that is not a
    // number, a circle without a radius, and a tolerance that is none.
    sketch.entities.push(entity(
        5,
        Geometry::Line {
            start: EntityId(1),
            end: EntityId(99),
        },
    ));
    assert!(sketch_outline(&sketch, &solved, &no_radius, false, 1e-6).is_err());
    sketch.entities.pop();
    let nan = |id: EntityId| {
        Some(if id == EntityId(2) {
            (f64::NAN, 0.0)
        } else {
            (0.0, 5.0)
        })
    };
    assert!(sketch_outline(&sketch, &nan, &no_radius, false, 1e-6).is_err());
    sketch.entities.push(entity(
        6,
        Geometry::Circle {
            center: EntityId(1),
            radius: 0.0,
        },
    ));
    assert!(sketch_outline(&sketch, &solved, &no_radius, false, 1e-6).is_err());
    let radius = |_| Some(2.0);
    assert_eq!(
        sketch_outline(&sketch, &solved, &radius, false, 1e-6)
            .unwrap()
            .paths
            .len(),
        2
    );
    for tolerance in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(sketch_outline(&sketch, &solved, &radius, false, tolerance).is_err());
    }
}
