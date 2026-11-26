//! The box of a spline edge whose range runs past its curve's domain
//! (2026-11-26): `NurbsCurve::eval` carries the end spans on there, the
//! check takes such a range, and the curve leaves the hull of its control
//! points, which until this date was the edge's box (and so, for a face on
//! a plane, a cylinder or a cone, the face's). Now the end pieces carried
//! on are boxed too (`NurbsCurve::continued_hull`), and an edge within its
//! domain is boxed as it was, bit for bit. Where the curve carried on
//! cannot be bounded, the edge has no box and the check refuses it.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::{Curve3, Line, NurbsCurve, Plane, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, Defect, EdgeId, check};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The world, and a turn about a slanted axis with a shift far off.
fn placements(k: f64) -> [Transform; 2] {
    let turn = Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
        .unwrap()
        .then(&Transform::translation(
            Vec3::new(7.0, -3.0, 2.0) * (1e3 * k),
        ));
    [Transform::translation(Vec3::ZERO), turn]
}

/// A body of the one edge `curve` over `range`, from and to its points
/// there.
fn edge_alone(curve: &NurbsCurve, range: (f64, f64)) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let a = body.add_vertex(curve.eval(range.0));
    let b = body.add_vertex(curve.eval(range.1));
    body.add_edge(Curve3::Nurbs(curve.clone()), range, a, b);
    body
}

/// The quadratic `(t, 0.75 - t - t^2)` over [0, 1], times `k`: over
/// [-1, 1] it peaks at y = 1 for t = -0.5, where its control points and
/// ends reach 0.75.
fn bulging(k: f64) -> NurbsCurve {
    NurbsCurve::polynomial(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![
            Vec3::new(0.0, 0.75, 0.0) * k,
            Vec3::new(0.5, 0.25, 0.0) * k,
            Vec3::new(1.0, -1.25, 0.0) * k,
        ],
    )
    .unwrap()
}

/// A rational quarter circle of radius `k` over [0, 1]: carried on, it
/// stays on its circle.
fn quarter(k: f64) -> NurbsCurve {
    NurbsCurve::new(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![
            Vec3::new(k, 0.0, 0.0),
            Vec3::new(k, k, 0.0),
            Vec3::new(0.0, k, 0.0),
        ],
        vec![1.0, 0.5_f64.sqrt(), 1.0],
    )
    .unwrap()
}

/// A cubic of four uneven spans, times `k`.
fn wavy(k: f64) -> NurbsCurve {
    NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 0.2, 0.5, 0.6, 1.0, 1.0, 1.0, 1.0],
        vec![
            Vec3::new(0.0, 0.0, 0.0) * k,
            Vec3::new(1.0, 2.0, -1.0) * k,
            Vec3::new(2.0, -1.0, 0.5) * k,
            Vec3::new(3.0, 0.5, 2.0) * k,
            Vec3::new(4.0, 1.0, -0.5) * k,
            Vec3::new(5.0, -2.0, 0.0) * k,
            Vec3::new(6.0, 0.0, 1.0) * k,
        ],
    )
    .unwrap()
}

/// Every point of the edge over `range`, at 4001 places, lies in the box
/// of `body`'s edge 0 (no margin: the box is padded for rounding).
#[track_caller]
fn boxed_whole(body: &Body, what: &str) {
    let edge = &body.edges[0];
    let held = body.edge_bbox(EdgeId(0)).unwrap();
    let (t0, t1) = edge.range;
    for i in 0..=4000 {
        let t = t0 + (t1 - t0) * f64::from(i) / 4000.0;
        let p = edge.curve.eval(t);
        assert!(
            held.contains(p, 0.0),
            "{what}: {p:?} at {t} outside {held:?}"
        );
    }
}

#[test]
fn a_spline_edge_past_its_domain_is_boxed_whole_at_every_scale_and_turned() {
    for k in SCALES {
        let cases: [(&str, NurbsCurve, (f64, f64)); 9] = [
            ("the bulge before the start", bulging(k), (-1.0, 1.0)),
            ("the bulge wholly before", bulging(k), (-1.0, -0.2)),
            ("the circle on past its end", quarter(k), (0.0, 10.0)),
            ("the circle back past its start", quarter(k), (-3.0, 1.0)),
            ("the circle past both ends", quarter(k), (-3.0, 10.0)),
            ("the circle wholly past", quarter(k), (4.0, 9.0)),
            ("the cubic past both ends", wavy(k), (-0.3, 1.4)),
            ("the cubic a billionth past", wavy(k), (-1e-9, 1.0 + 1e-9)),
            ("the cubic two spans on", wavy(k), (0.5, 1.8)),
        ];
        for (what, curve, range) in &cases {
            for (turn, place) in placements(k).iter().enumerate() {
                let body = edge_alone(curve, *range).transformed(place).unwrap();
                boxed_whole(&body, &format!("{what} at {k}, placement {turn}"));
                // Taken by the check (no defect of the edge's range).
                let defects = check(&body, &Tolerance::new(1e-6 * k, 1e-9).unwrap());
                assert!(
                    !defects.contains(&Defect::PastDomain { edge: EdgeId(0) }),
                    "{what}: {defects:?}"
                );
            }
        }
        // The box of the control points and the ends alone, which the box
        // was until 2026-11-26, does not hold the bulge.
        let curve = bulging(k);
        let mut old = Bbox3::from_points(curve.control_points().iter().copied());
        old.add(curve.eval(-1.0));
        old.add(curve.eval(1.0));
        assert!(!old.contains(curve.eval(-0.5), 0.2 * k), "{k}");
    }
}

/// An edge within its domain is boxed by its control points and its ends,
/// as before: the box of a part of a clamped spline's domain is the box of
/// the whole, bit for bit, and the box of a face bounded by such edges is
/// what it was.
#[test]
fn an_edge_within_its_domain_is_boxed_by_its_control_points_bit_for_bit() {
    for k in SCALES {
        for curve in [bulging(k), quarter(k), wavy(k)] {
            let whole = edge_alone(&curve, curve.domain())
                .edge_bbox(EdgeId(0))
                .unwrap();
            for range in [(0.0, 1.0), (0.1, 0.9), (0.3, 0.35), (0.0, 0.5)] {
                let part = edge_alone(&curve, range).edge_bbox(EdgeId(0)).unwrap();
                assert_eq!(
                    part.corners()
                        .map(|(lo, hi)| [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z].map(f64::to_bits)),
                    whole
                        .corners()
                        .map(|(lo, hi)| [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z].map(f64::to_bits)),
                    "{k} {range:?}"
                );
            }
        }
    }
}

/// A flat face bounded by the bulging curve over [-1, 1] and its chord: its
/// box, and the body's, hold the bulge (a face on a plane adds nothing to
/// its edges' boxes).
#[test]
fn a_flat_face_bounded_by_an_edge_past_its_domain_holds_its_bulge_in_its_box() {
    for k in SCALES {
        for (turn, place) in placements(k).iter().enumerate() {
            let curve = bulging(k);
            let mut body = Body::new(BodyKind::Sheet);
            let a = body.add_vertex(curve.eval(-1.0));
            let b = body.add_vertex(curve.eval(1.0));
            let arc = body.add_edge(Curve3::Nurbs(curve.clone()), (-1.0, 1.0), a, b);
            let (pa, pb) = (curve.eval(-1.0), curve.eval(1.0));
            let line = Line::new(pb, (pa - pb).normalized().unwrap()).unwrap();
            let chord = body.add_edge(Curve3::Line(line), (0.0, pa.distance(pb)), b, a);
            let lp = body.add_loop(vec![
                Coedge {
                    edge: chord,
                    reversed: true,
                },
                Coedge {
                    edge: arc,
                    reversed: true,
                },
            ]);
            let face = body.add_face(Surface::Plane(Plane::new(Frame::WORLD)), false, vec![lp]);
            body.add_shell(vec![face]);
            let body = body.transformed(place).unwrap();
            assert!(check(&body, &Tolerance::new(1e-6 * k, 1e-9).unwrap()).is_empty());
            let peak = body.edges[arc.0 as usize].curve.eval(-0.5);
            assert!(peak.distance(place.point(curve.eval(-0.5))) <= 1e-9 * k);
            for (which, held) in [
                ("face", body.face_bbox(face).unwrap()),
                ("held face", body.held_face_bbox(face).unwrap()),
                ("body", body.bbox().unwrap()),
                ("held body", body.held_bbox().unwrap()),
            ] {
                assert!(held.contains(peak, 0.0), "{which} at {k}, placement {turn}");
            }
        }
    }
}

/// Past the end of a domain that ends on a knot span of no width (knots
/// not clamped, the end knot repeated), `eval` carries that empty span's
/// polynomial on, which no piece of the curve is: nothing bounds the edge
/// there. It has no box, and the check refuses it; before its end, or
/// before the start (where `eval` takes the first span with a width), it
/// is boxed and taken.
#[test]
fn an_edge_past_a_domain_that_ends_on_an_empty_span_has_no_box_and_is_refused() {
    let curve = NurbsCurve::polynomial(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0],
        vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(2.0, -1.0, 1.0),
            Vec3::new(3.0, 0.0, 0.0),
        ],
    )
    .unwrap();
    assert_eq!(curve.domain(), (0.0, 1.0));
    let tol = Tolerance::new(1e-6, 1e-9).unwrap();
    let past = edge_alone(&curve, (0.2, 1.5));
    assert!(past.edge_bbox(EdgeId(0)).is_err());
    assert!(past.bbox().is_err());
    let defects = check(&past, &tol);
    assert!(
        defects.contains(&Defect::PastDomain { edge: EdgeId(0) }),
        "{defects:?}"
    );
    assert!(
        defects
            .iter()
            .any(|d| d.to_string().contains("past its spline's domain")),
        "{defects:?}"
    );
    for range in [(0.2, 1.0), (-0.5, 0.8)] {
        let body = edge_alone(&curve, range);
        boxed_whole(&body, &format!("{range:?}"));
        assert!(!check(&body, &tol).contains(&Defect::PastDomain { edge: EdgeId(0) }));
    }
}
