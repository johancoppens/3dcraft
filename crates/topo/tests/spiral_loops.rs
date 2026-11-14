//! A thin strip that spirals about a point of its surface's chart is a
//! right face however many turns one of its edges makes
//! (`docs/design/topology.md` › *A strip that spirals about a point of the
//! chart*): until 2026-11-14 a spline edge was followed in eight steps
//! whatever it did, and a side of 16 turns in one edge, two whole turns a
//! step, read as a straight line.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "common/spirals.rs"]
mod spirals;

use craft3d_topo::{Body, Defect, check, forget_passed, mass_properties};
use spirals::{Strip, grounds, spiral_strip, tolerance, turned};

/// What the check says of the loops of a body's faces.
fn loop_defects(body: &Body, k: f64) -> Vec<String> {
    forget_passed();
    check(body, &tolerance(k))
        .iter()
        .filter(|d| {
            matches!(
                d,
                Defect::LoopsBoundNoRegion { .. } | Defect::LoopNotSimple { .. }
            )
        })
        .map(ToString::to_string)
        .collect()
}

/// The smallest strip found refused: on a plane, the spiral from radius
/// 0.45 to 1.65 in 16 turns, the strip 0.35 of the pitch wide, its outer
/// side one edge and its inner side cut into quarter turns ("every loop
/// runs clockwise about the outward normal"), or the other way about ("it
/// winds 2 times about the point …"). Right by its measure: the area is the
/// closed form's.
#[test]
fn the_smallest_spiral_strip_is_a_face() {
    let (_, plane, at, area) = grounds(1.0).swap_remove(0);
    for cut in [(0, 4), (4, 0), (0, 0), (4, 4)] {
        let strip = Strip {
            cut,
            ..Strip::of(16.0)
        };
        let body = spiral_strip(&plane, at.as_ref(), &strip, 1e-6).unwrap();
        let want = strip.chart_area() * area.unwrap();
        let got = mass_properties(&body).unwrap().area;
        assert!(
            (got / want - 1.0).abs() < 1e-8,
            "cut {cut:?}: area {got} against {want}"
        );
        assert_eq!(
            check(&body, &tolerance(1.0)),
            Vec::new(),
            "cut {cut:?}: a right strip"
        );
    }
}

/// The strips the review found refused, and their neighbours: 15 to 33
/// turns, either side cut into quarter turns, on a plane, a cylinder, a
/// cone and a sphere, at three scales, in the world and turned a thousand
/// sizes off. None has a fault of its loops; on the plane and the cylinder
/// each measures its closed form (those of 16 turns also where they are
/// turned and far off).
#[test]
fn spiral_strips_with_a_side_cut_are_faces() {
    let mut refused = Vec::new();
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        for (name, surface, at, area) in grounds(k) {
            for turns in [15.0, 16.0, 16.5, 17.0, 32.0, 33.0] {
                for cut in [(4, 0), (0, 4)] {
                    let strip = Strip {
                        cut,
                        ..Strip::of(turns)
                    };
                    let body = spiral_strip(&surface, at.as_ref(), &strip, 1e-6 * k).unwrap();
                    if let Some(area) = area {
                        let want = strip.chart_area() * area;
                        let got = mass_properties(&body).unwrap().area;
                        assert!(
                            (got / want - 1.0).abs() < 1e-7,
                            "{name} at {k}, {turns} turns, cut {cut:?}: area {got} against {want}"
                        );
                    }
                    for (place, body) in [
                        ("world", body.clone()),
                        ("turned", body.transformed(&turned(k, 1e3)).unwrap()),
                    ] {
                        count += 1;
                        // The closed form far off and turned as well.
                        if let (Some(area), "turned", true) = (area, place, turns == 16.0) {
                            let want = strip.chart_area() * area;
                            let got = mass_properties(&body).unwrap().area;
                            assert!(
                                (got / want - 1.0).abs() < 1e-7,
                                "{name} at {k} turned, cut {cut:?}: area {got} against {want}"
                            );
                        }
                        let found = loop_defects(&body, k);
                        if !found.is_empty() {
                            refused.push(format!(
                                "{name} at {k} {place}, {turns} turns, cut {cut:?}: {}",
                                found.join("; ")
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        refused.is_empty(),
        "{} of {count} right strips refused:\n{}",
        refused.len(),
        refused.join("\n")
    );
}

/// A strip that is wrong is refused as it was, where its sides are in
/// edges short enough to be followed closely (both cut into quarter
/// turns): turned over (the face flipped, its loop kept: everything
/// outside the strip) by the region rule, and wider than its pitch, so
/// that it lies over itself, by the rule of simple loops. At three scales
/// on a plane, at one on a cylinder and a cone, in the world and turned.
#[test]
fn a_spiral_strip_turned_over_or_over_itself_is_refused() {
    for k in [1e-3, 1.0, 1e6] {
        for (name, surface, at, _) in grounds(k) {
            if name == "sphere" || (name != "plane" && k != 1.0) {
                continue;
            }
            for (strip, what) in [
                (
                    Strip {
                        cut: (4, 4),
                        flipped: true,
                        ..Strip::of(2.0)
                    },
                    "do not bound a region",
                ),
                (
                    Strip {
                        cut: (4, 4),
                        flipped: true,
                        ..Strip::of(5.0)
                    },
                    "do not bound a region",
                ),
                (
                    Strip {
                        cut: (4, 4),
                        share: 1.5,
                        ..Strip::of(3.0)
                    },
                    "is not a simple closed curve",
                ),
            ] {
                let body = spiral_strip(&surface, at.as_ref(), &strip, 1e-6 * k).unwrap();
                for body in [body.clone(), body.transformed(&turned(k, 1e3)).unwrap()] {
                    let found = loop_defects(&body, k);
                    assert!(
                        found.len() == 1 && found[0].contains(what),
                        "{name} at {k}, {strip:?}: {found:?}"
                    );
                }
            }
        }
    }
}

/// A side of `turns` turns as one cubic on control points along the
/// spiral, eight a turn, `off` from the middle spiral.
fn wound(turns: usize, off: f64) -> craft3d_geom::NurbsCurve {
    use craft3d_math::Portable;
    let count = 8 * turns + 1;
    let points: Vec<craft3d_math::Vec3> = (0..count)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / 8.0;
            let r = 1.0 + 0.01 * i as f64 / 8.0 + off;
            let (s, c) = t.psin_cos();
            craft3d_math::Vec3::new(r * c, r * s, 0.0)
        })
        .collect();
    let spans = count - 3;
    let mut knots = vec![0.0; 3];
    knots.extend((0..=spans).map(|j| j as f64 / spans as f64));
    knots.extend([1.0; 3]);
    craft3d_geom::NurbsCurve::polynomial(3, knots, points).unwrap()
}

/// A strip on the plane between two sides of `turns` turns, each one edge
/// ([`wound`]), `width` apart, with lines across its ends.
fn wound_strip(turns: usize, width: f64) -> Body {
    use craft3d_geom::{Curve3, Line, Plane, Surface};
    use craft3d_topo::{BodyKind, Coedge};
    let (outer, inner) = (wound(turns, 0.5 * width), wound(turns, -0.5 * width));
    let mut body = Body::new(BodyKind::Sheet);
    let ends = [
        outer.eval(0.0),
        outer.eval(1.0),
        inner.eval(1.0),
        inner.eval(0.0),
    ];
    let v: Vec<_> = ends.iter().map(|p| body.add_vertex(*p)).collect();
    let out = body.add_edge(Curve3::Nurbs(outer), (0.0, 1.0), v[0], v[1]);
    let back = body.add_edge(Curve3::Nurbs(inner), (0.0, 1.0), v[3], v[2]);
    let mut across = |a: usize, b: usize| {
        let line = Line::through(ends[a], ends[b]).unwrap();
        body.add_edge(
            Curve3::Line(line),
            (0.0, ends[a].distance(ends[b])),
            v[a],
            v[b],
        )
    };
    let (end, start) = (across(1, 2), across(3, 0));
    let forward = |edge| Coedge {
        edge,
        reversed: false,
    };
    let lp = body.add_loop(vec![
        forward(out),
        forward(end),
        Coedge {
            edge: back,
            reversed: true,
        },
        forward(start),
    ]);
    let plane = Surface::Plane(Plane::new(craft3d_math::Frame::WORLD));
    let face = body.add_face(plane, false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// Hostile strips get an answer, never a panic or a hang: a spiral of
/// three thousand turns between two single edges is not followed (it
/// passes, and is counted, for work that stays within the bound of a
/// face); a strip narrower than the tolerance, and one whose numbers are
/// not numbers, are given their defects.
#[test]
fn hostile_spirals_get_an_answer() {
    use craft3d_topo::{MOST_PROOF_WORK, step_proofs};
    let long = wound_strip(3000, 0.004);
    forget_passed();
    let (work, cut) = step_proofs();
    assert_eq!(loop_defects(&long, 1.0), Vec::<String>::new());
    let (work_now, cut_now) = step_proofs();
    assert_eq!(cut_now - cut, 1, "the face is counted as not followed");
    assert!(
        work_now - work <= 2 * MOST_PROOF_WORK as u64,
        "{}",
        work_now - work
    );
    // A strip a thousandth of the tolerance wide: an answer either way.
    let thin = wound_strip(3, 1e-9);
    let _ = check(&thin, &tolerance(1.0));
    // Numbers that are none: a vertex, an edge's range.
    for spoil in 0..3 {
        let mut body = wound_strip(3, 0.004);
        match spoil {
            0 => body.vertices[0].point.x = f64::NAN,
            1 => body.edges[0].range = (0.0, f64::NAN),
            _ => body.edges[1].range = (f64::NEG_INFINITY, f64::INFINITY),
        }
        assert!(!check(&body, &tolerance(1.0)).is_empty(), "spoilt {spoil}");
    }
}
