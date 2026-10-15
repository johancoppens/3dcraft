//! Revolution of profiles that touch the axis, against the primitives and
//! Pappus; which surface each curve sweeps; origins worked out by hand;
//! determinism; and every way the input can be wrong.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod revolve_support;

use common::{P, Rng, arc, catalogue, circle, frames, line, lines_reversed, polygon, rectangle};
use craft3d_geom::Surface;
use craft3d_kernel::extrude::{OpResult, Origin, Role};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{SketchCurve, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, check, genus, mass_properties};
use revolve_support::{
    Axis, SWEEPS, assert_pappus, build, clear_axis, close, half_disc, lathe, lathe_polygon,
    tolerance,
};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// The frame a primitive needs to coincide with a revolution: at height
/// `h` on the axis, z along it.
fn about(plane: &Frame, axis: Axis, h: f64) -> Frame {
    let p = lathe(axis, 1.0, 0.0, h);
    // The direction as a direction: the difference of two points a unit
    // apart loses ten digits at a million units from the origin.
    let along = plane.vector(axis.direction[0], axis.direction[1], 0.0);
    Frame::from_z(plane.point(p[0], p[1], 0.0), along).unwrap()
}

fn counts(body: &Body) -> (usize, usize, usize) {
    (body.vertices.len(), body.edges.len(), body.faces.len())
}

fn kinds(body: &Body) -> Vec<&'static str> {
    let mut kinds: Vec<&str> = body.faces.iter().map(|f| f.surface.kind()).collect();
    kinds.sort_unstable();
    kinds
}

/// A full revolution of the profile a primitive is the revolution of gives
/// the primitive: the same entity counts, surfaces, genus, volume, area
/// and centroid.
#[test]
fn full_revolutions_equal_the_primitives() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = tolerance(k);
        for plane in frames(k) {
            let axis = Axis {
                point: [1.5 * k, -0.5 * k],
                direction: [0.3, 0.9],
            };
            for side in [1.0, -1.0] {
                let (r, h) = (1.5 * k, 2.5 * k);
                let cases: Vec<(&str, Vec<SketchCurve>, Body)> = vec![
                    (
                        "cylinder",
                        lathe_polygon(axis, side, &[(0.0, 0.0), (r, 0.0), (r, h), (0.0, h)]),
                        primitives::cylinder(&about(&plane, axis, 0.0), r, h).unwrap(),
                    ),
                    (
                        "cone",
                        lathe_polygon(axis, side, &[(0.0, 0.0), (r, 0.0), (0.0, h)]),
                        primitives::cone(&about(&plane, axis, 0.0), r, h).unwrap(),
                    ),
                    (
                        "sphere",
                        half_disc(axis, side, 0.7 * k, r),
                        primitives::sphere(&about(&plane, axis, 0.7 * k), r).unwrap(),
                    ),
                    (
                        "torus",
                        vec![circle(1, lathe(axis, side, 3.0 * k, 0.4 * k), k)],
                        primitives::torus(&about(&plane, axis, 0.4 * k), 3.0 * k, k).unwrap(),
                    ),
                ];
                for (name, curves, primitive) in cases {
                    let what = format!("{name} at scale {k}, side {side}");
                    let result = build(&curves, &plane, axis, (0.4, 0.4 + TAU), &tol)
                        .unwrap_or_else(|e| panic!("{what}: {e}"));
                    let body = &result.body;
                    assert_eq!(check(body, &tol), vec![], "{what}");
                    assert_eq!(counts(body), counts(&primitive), "{what}");
                    assert_eq!(kinds(body), kinds(&primitive), "{what}");
                    assert_eq!(genus(body), genus(&primitive), "{what}");
                    assert_eq!(body.shells.len(), 1, "{what}");
                    let (ours, theirs) = (
                        mass_properties(body).unwrap(),
                        mass_properties(&primitive).unwrap(),
                    );
                    close(
                        ours.volume,
                        theirs.volume,
                        1e-11,
                        &format!("{what}: volume"),
                    );
                    close(ours.area, theirs.area, 1e-11, &format!("{what}: area"));
                    let off = ours.centroid.unwrap().distance(theirs.centroid.unwrap());
                    assert!(off <= 1e-10 * k, "{what}: centroids {off:e} apart");
                }
            }
        }
    }
}

/// A profile on the axis: its name, curves, regions as (area, centroid),
/// and how many closed surfaces a full turn of it has.
type OnAxis = (&'static str, Vec<SketchCurve>, Vec<(f64, P)>, usize);

/// Profiles that touch the axis, through partial sweeps as well: along a
/// line (cylinder, cone, ball, cup), at a point (a triangle on its tip),
/// at two points (a chevron, which encloses a cavity).
#[test]
fn profiles_that_touch_the_axis_match_pappus() {
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        let tol = tolerance(k);
        for plane in frames(k) {
            let axis = Axis {
                point: [-2.0 * k, 0.5 * k],
                direction: [-1.0, 0.4],
            };
            for side in [1.0, -1.0] {
                let p = |rho: f64, h: f64| lathe(axis, side, rho * k, h * k);
                let poly = |points: &[(f64, f64)]| {
                    let scaled: Vec<(f64, f64)> =
                        points.iter().map(|&(r, h)| (r * k, h * k)).collect();
                    lathe_polygon(axis, side, &scaled)
                };
                let k2 = k * k;
                // Regions as (area, centroid), by hand:
                // - rectangle 1.5 × 2.5 on the axis: centre (0.75, 1.25)
                // - right triangle (0,0) (1.5,0) (0,2.5): the mean of its
                //   corners
                // - half disc of radius 1.5: 4r/3π = 2/π from the axis
                // - triangle on its tip (0,1) (2,0) (2,2): area 2
                // - chevron (0,0) (2,1) (0,2) (1,1): the triangle (0,0)
                //   (2,1) (0,2) (area 2, rho 2/3) less the triangle (0,0)
                //   (1,1) (0,2) (area 1, rho 1/3): area 1 at rho 1, h 1
                // - cup (0,0) (2,0) (2,2) (1.5,2) (1.5,0.5) (0,0.5): a
                //   2 × 0.5 foot on the axis and a 0.5 × 1.5 wall on it
                let cases: Vec<OnAxis> = vec![
                    (
                        "cylinder",
                        poly(&[(0.0, 0.0), (1.5, 0.0), (1.5, 2.5), (0.0, 2.5)]),
                        vec![(3.75 * k2, p(0.75, 1.25))],
                        1,
                    ),
                    (
                        "cone",
                        poly(&[(0.0, 0.0), (1.5, 0.0), (0.0, 2.5)]),
                        vec![(1.875 * k2, p(0.5, 2.5 / 3.0))],
                        1,
                    ),
                    (
                        "ball",
                        half_disc(axis, side, 0.7 * k, 1.5 * k),
                        vec![(PI * 1.125 * k2, p(2.0 / PI, 0.7))],
                        1,
                    ),
                    (
                        "triangle on its tip",
                        poly(&[(0.0, 1.0), (2.0, 0.0), (2.0, 2.0)]),
                        vec![(2.0 * k2, p(4.0 / 3.0, 1.0))],
                        1,
                    ),
                    (
                        "chevron",
                        poly(&[(0.0, 0.0), (2.0, 1.0), (0.0, 2.0), (1.0, 1.0)]),
                        vec![(k2, p(1.0, 1.0))],
                        2,
                    ),
                    (
                        "cup",
                        poly(&[
                            (0.0, 0.0),
                            (2.0, 0.0),
                            (2.0, 2.0),
                            (1.5, 2.0),
                            (1.5, 0.5),
                            (0.0, 0.5),
                        ]),
                        vec![(k2, p(1.0, 0.25)), (0.75 * k2, p(1.75, 1.25))],
                        1,
                    ),
                ];
                for (name, curves, regions, closed_surfaces) in cases {
                    for (from, to) in SWEEPS {
                        let what = format!("{name} at scale {k}, side {side}, {from}..{to}");
                        let result = build(&curves, &plane, axis, (from, to), &tol)
                            .unwrap_or_else(|e| panic!("{what}: {e}"));
                        assert_pappus(&result, &curves, &regions, axis, to - from, k, &what);
                        let full = to - from == TAU;
                        // One shell per profile, or per closed surface.
                        assert_eq!(
                            result.body.shells.len(),
                            if full { closed_surfaces } else { 1 },
                            "{what}"
                        );
                        assert_eq!(genus(&result.body), Some(0), "{what}");
                        count += 1;
                    }
                }
            }
        }
    }
    println!("{count} revolutions of profiles on the axis");
}

/// An arc counter-clockwise about `centre` from `a` to `b` as seen in the
/// axis's own terms (rho across, h along), all three given in those terms.
fn lathe_arc(
    id: u64,
    axis: Axis,
    side: f64,
    centre: (f64, f64),
    a: (f64, f64),
    b: (f64, f64),
) -> SketchCurve {
    let p = |q: (f64, f64)| lathe(axis, side, q.0, q.1);
    // (rho, h) is turned the sketch's way round to the right of the axis
    // and mirrored to its left.
    if side < 0.0 {
        arc(id, p(centre), p(a), p(b))
    } else {
        arc(id, p(centre), p(b), p(a))
    }
}

/// Arcs whose circle reaches the axis without being centred on it lie on a
/// spindle or horn torus; the arc itself is clear of the axis, or reaches
/// it at an end, and sweeps a rational surface of revolution (Pappus, with
/// the regions worked out by hand):
/// - a D: half a disc of radius 2 about (0.5, 0), its arc away from the
///   axis: area 2π at rho 0.5 + 8/3π
/// - a lens: the segment of the circle of radius 5 about (−3, 0) beyond
///   the axis, its ends on it at h = ±4: with sin α = 0.8 (cos α = 0.6),
///   area 25 (α − 0.48), its centroid 20 · 0.512 / 3 (2α − 0.96) from the
///   centre
/// - a horn: the square (0, 0) (2, 2) less the quarter disc of radius 2
///   about (2, 0), its arc ending on the axis where its circle touches it:
///   area 4 − π, from the square (4 at (1, 1)) less the quarter disc (π at
///   (2 − 8/3π, 8/3π))
#[test]
fn arcs_whose_circle_reaches_the_axis_match_pappus() {
    let mut count = 0;
    let alpha = 0.6_f64.acos();
    let lens_area = 25.0 * (alpha - 0.48);
    let lens_rho = 20.0 * 0.512 / (3.0 * (2.0 * alpha - 0.96)) - 3.0;
    let quarter = 8.0 / (3.0 * PI);
    let horn_area = 4.0 - PI;
    let horn_rho = (4.0 - PI * (2.0 - quarter)) / horn_area;
    let horn_h = (4.0 - PI * quarter) / horn_area;
    for k in [1e-3, 1.0, 1e6] {
        let tol = tolerance(k);
        for plane in frames(k) {
            let axis = Axis {
                point: [-2.0 * k, 0.5 * k],
                direction: [-1.0, 0.4],
            };
            for side in [1.0, -1.0] {
                let p = |rho: f64, h: f64| lathe(axis, side, rho * k, h * k);
                let q = |rho: f64, h: f64| (rho * k, h * k);
                let k2 = k * k;
                let cases: Vec<(&str, Vec<SketchCurve>, (f64, P))> = vec![
                    (
                        "D",
                        vec![
                            lathe_arc(1, axis, side, q(0.5, 0.0), q(0.5, -2.0), q(0.5, 2.0)),
                            line(2, p(0.5, 2.0), p(0.5, -2.0)),
                        ],
                        (2.0 * PI * k2, p(0.5 + quarter, 0.0)),
                    ),
                    (
                        "lens",
                        vec![
                            lathe_arc(1, axis, side, q(-3.0, 0.0), q(0.0, -4.0), q(0.0, 4.0)),
                            line(2, p(0.0, 4.0), p(0.0, -4.0)),
                        ],
                        (lens_area * k2, p(lens_rho, 0.0)),
                    ),
                    (
                        "horn",
                        vec![
                            lathe_arc(1, axis, side, q(2.0, 0.0), q(2.0, 2.0), q(0.0, 0.0)),
                            line(2, p(0.0, 0.0), p(0.0, 2.0)),
                            line(3, p(0.0, 2.0), p(2.0, 2.0)),
                        ],
                        (horn_area * k2, p(horn_rho, horn_h)),
                    ),
                ];
                for (name, curves, region) in cases {
                    for (from, to) in SWEEPS {
                        let what = format!("{name} at scale {k}, side {side}, {from}..{to}");
                        let result = build(&curves, &plane, axis, (from, to), &tol)
                            .unwrap_or_else(|e| panic!("{what}: {e}"));
                        assert_pappus(&result, &curves, &[region], axis, to - from, k, &what);
                        // The arc's face (two halves in a full turn) is a
                        // rational surface.
                        let rational = result
                            .origins
                            .faces
                            .iter()
                            .zip(&result.body.faces)
                            .filter(|(o, f)| {
                                o.role == Role::Side
                                    && o.sources == [1]
                                    && matches!(
                                        &f.surface,
                                        Surface::Nurbs(s) if s.weights().iter().any(|w| *w != 1.0)
                                    )
                            })
                            .count();
                        let full = to - from == TAU;
                        assert_eq!(rational, if full { 2 } else { 1 }, "{what}");
                        assert_eq!(result.body.shells.len(), 1, "{what}");
                        // A full turn of the D, clear of the axis, is a ring.
                        let ring = full && name == "D";
                        assert_eq!(genus(&result.body), Some(u32::from(ring)), "{what}");
                        count += 1;
                    }
                }
            }
        }
    }
    println!("{count} revolutions of arcs on a spindle or horn torus");
}

/// Area and centroid (rho, h) of the region left of a closed chain of
/// arcs, each counter-clockwise about its centre `(c, r, from, to)` in the
/// axis's own terms, by Green's theorem with Simpson's rule:
/// `A = ∮ ρ dh`, `∫∫ ρ dA = ∮ ρ²/2 dh`, `∫∫ h dA = −∮ h²/2 dρ`.
fn arc_region(arcs: &[((f64, f64), f64, f64, f64)]) -> (f64, (f64, f64)) {
    let (mut area, mut mr, mut mh) = (0.0, 0.0, 0.0);
    for &((cr, ch), r, from, to) in arcs {
        let steps = 4000;
        let step = (to - from) / f64::from(steps);
        for i in 0..=steps {
            let t = from + step * f64::from(i);
            let (rho, h) = (cr + r * t.cos(), ch + r * t.sin());
            let (drho, dh) = (-r * t.sin(), r * t.cos());
            let w = if i == 0 || i == steps {
                1.0
            } else if i % 2 == 1 {
                4.0
            } else {
                2.0
            } * step
                / 3.0;
            area += w * rho * dh;
            mr += w * rho * rho / 2.0 * dh;
            mh -= w * h * h / 2.0 * drho;
        }
    }
    (area, (mr / area, mh / area))
}

/// Two arcs that meet at a point of the axis, both swept as rational
/// surfaces and so both made in halves in a full turn: a lens between an
/// arc of a horn torus (its circle about (2, 0) touches the axis where the
/// arc ends) and one of a spindle torus (about (0.5, 1.5)), from the axis
/// at (0, 0) to (2, 2). In a full turn the boundary meets itself at that
/// point, and each side has a vertex of its own there (it was one vertex
/// for both, which Euler–Poincaré refused); Pappus with the region worked
/// out by Green's theorem along the arcs.
#[test]
fn two_halved_arcs_that_meet_on_the_axis_each_have_a_vertex_there() {
    let r2 = (0.5_f64 * 0.5 + 1.5 * 1.5).sqrt();
    let (area, (rho, h)) = arc_region(&[
        ((0.5, 1.5), r2, (-1.5_f64).atan2(-0.5), 0.5_f64.atan2(1.5)),
        ((2.0, 0.0), 2.0, FRAC_PI_2, PI),
    ]);
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        let tol = tolerance(k);
        for plane in frames(k) {
            let axis = Axis {
                point: [-2.0 * k, 0.5 * k],
                direction: [-1.0, 0.4],
            };
            for side in [1.0, -1.0] {
                let q = |rho: f64, h: f64| (rho * k, h * k);
                let curves = vec![
                    lathe_arc(1, axis, side, q(2.0, 0.0), q(2.0, 2.0), q(0.0, 0.0)),
                    lathe_arc(2, axis, side, q(0.5, 1.5), q(0.0, 0.0), q(2.0, 2.0)),
                ];
                let region = (area * k * k, lathe(axis, side, rho * k, h * k));
                for (from, to) in SWEEPS {
                    let what = format!("scale {k}, side {side}, {from}..{to}");
                    let result = build(&curves, &plane, axis, (from, to), &tol)
                        .unwrap_or_else(|e| panic!("{what}: {e}"));
                    assert_pappus(&result, &curves, &[region], axis, to - from, k, &what);
                    assert_eq!(result.body.shells.len(), 1, "{what}");
                    assert_eq!(genus(&result.body), Some(0), "{what}");
                    if to - from == TAU {
                        // Two vertices at the point of the axis, one per
                        // side, and two more where the arcs meet off it
                        // (at the start and half a turn on).
                        let apex = lathe(axis, side, 0.0, 0.0);
                        let apex = plane.point(apex[0], apex[1], 0.0);
                        let at_apex = result
                            .body
                            .vertices
                            .iter()
                            .filter(|v| v.point.distance(apex) <= 1e-9 * k)
                            .count();
                        assert_eq!(at_apex, 2, "{what}");
                        assert_eq!(result.body.vertices.len(), 4, "{what}");
                    }
                    count += 1;
                }
            }
        }
    }
    println!("{count} revolutions of two arcs meeting on the axis");
}

fn origin(role: Role, sources: &[u64]) -> Origin {
    Origin::new(role, sources)
}

fn face_kind(result: &OpResult, role: Role, sources: &[u64]) -> Option<&'static str> {
    let wanted = origin(role, sources);
    let found: Vec<usize> = (0..result.origins.faces.len())
        .filter(|&i| result.origins.faces[i] == wanted)
        .collect();
    assert!(found.len() <= 1, "{wanted:?}");
    found.first().map(|&i| result.body.faces[i].surface.kind())
}

/// Which surface each kind of curve sweeps (the table in the module
/// documentation), in a full and in a partial revolution, either side.
#[test]
fn every_curve_sweeps_the_surface_its_position_gives() {
    let tol = tolerance(1.0);
    let plane = frames(1.0)[1];
    let axis = Axis {
        point: [0.3, 0.1],
        direction: [1.0, 2.0],
    };
    for side in [1.0, -1.0] {
        for sweep in [(0.0, TAU), (0.5, 2.0)] {
            // 1: perpendicular, 2: parallel, 3: oblique, 4: oblique to the
            // axis, 5: on the axis.
            let polygon = lathe_polygon(
                axis,
                side,
                &[(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 2.0), (0.0, 3.0)],
            );
            let result = build(&polygon, &plane, axis, sweep, &tol).unwrap();
            let kind = |id: u64| face_kind(&result, Role::Side, &[id]);
            assert_eq!(kind(1), Some("plane"));
            assert_eq!(kind(2), Some("cylinder"));
            assert_eq!(kind(3), Some("cone"));
            assert_eq!(kind(4), Some("cone"));
            assert_eq!(kind(5), None, "a line on the axis sweeps nothing");

            // A quarter disc about the point at height `h` on the axis,
            // `rho` away from it: the arc (1) from its outermost point to
            // its top, the line down to the centre (2) and the radius (3).
            let quarter = |rho: f64| {
                let (centre, out, top) = (
                    lathe(axis, side, rho, 1.0),
                    lathe(axis, side, rho + 2.0, 1.0),
                    lathe(axis, side, rho, 3.0),
                );
                if side < 0.0 {
                    vec![
                        arc(1, centre, out, top),
                        line(2, top, centre),
                        line(3, centre, out),
                    ]
                } else {
                    vec![
                        arc(1, centre, top, out),
                        line(2, centre, top),
                        line(3, out, centre),
                    ]
                }
            };
            let result = build(&quarter(0.0), &plane, axis, sweep, &tol).unwrap();
            assert_eq!(face_kind(&result, Role::Side, &[1]), Some("sphere"));
            assert_eq!(face_kind(&result, Role::Side, &[2]), None);
            assert_eq!(face_kind(&result, Role::Side, &[3]), Some("plane"));
            let result = build(&quarter(4.0), &plane, axis, sweep, &tol).unwrap();
            assert_eq!(face_kind(&result, Role::Side, &[1]), Some("torus"));
            assert_eq!(face_kind(&result, Role::Side, &[2]), Some("cylinder"));
            assert_eq!(face_kind(&result, Role::Side, &[3]), Some("plane"));
        }
    }
}

/// Turning from a to b and then from b to c is turning from a to c.
#[test]
fn consecutive_sweeps_add_up() {
    let tol = tolerance(1.0);
    let plane = frames(1.0)[1];
    for case in catalogue(1.0) {
        let axis = clear_axis(&case.curves, [0.4, 1.0], 1.0);
        let volume = |from: f64, to: f64| {
            let result = build(&case.curves, &plane, axis, (from, to), &tol).unwrap();
            mass_properties(&result.body).unwrap().volume
        };
        let (a, b, c) = (-1.1, 0.6, 3.9);
        close(volume(a, b) + volume(b, c), volume(a, c), 1e-11, case.name);
        // And four quarters are the full turn.
        let quarters: f64 = (0..4)
            .map(|i| volume(f64::from(i) * FRAC_PI_2, f64::from(i + 1) * FRAC_PI_2))
            .sum();
        close(quarters, volume(0.0, TAU), 1e-11, case.name);
    }
}

/// The same input gives the same body bit for bit, whatever order the
/// curves are given in and whichever way the lines were drawn; a full turn
/// does not depend on where it starts.
#[test]
fn revolution_is_deterministic() {
    let tol = tolerance(1.0);
    let plane = frames(1.0)[1];
    let mut rng = Rng(7);
    for case in catalogue(1.0) {
        let axis = clear_axis(&case.curves, [-0.2, 1.0], 0.75);
        for sweep in [(0.3, 2.9), (0.0, TAU)] {
            let first = build(&case.curves, &plane, axis, sweep, &tol).unwrap();
            // Debug prints every float so that it reads back exactly: equal
            // text is equal bits (and tells −0 from 0, which == does not).
            let text = format!("{first:?}");
            let again = build(&case.curves, &plane, axis, sweep, &tol).unwrap();
            assert_eq!(format!("{again:?}"), text, "{}", case.name);
            let mut shuffled = lines_reversed(&case.curves);
            rng.shuffle(&mut shuffled);
            let other = build(&shuffled, &plane, axis, sweep, &tol).unwrap();
            assert_eq!(format!("{other:?}"), text, "{}", case.name);
        }
        let full = build(&case.curves, &plane, axis, (0.0, TAU), &tol).unwrap();
        let later = build(&case.curves, &plane, axis, (1.3, 1.3 + TAU), &tol).unwrap();
        assert_eq!(format!("{later:?}"), format!("{full:?}"), "{}", case.name);
    }
}

fn sorted(origins: &[Origin]) -> Vec<Origin> {
    let mut all = origins.to_vec();
    all.sort();
    all
}

fn vertex_at(result: &OpResult, role: Role, sources: &[u64]) -> Vec3 {
    let wanted = origin(role, sources);
    let found: Vec<usize> = (0..result.origins.vertices.len())
        .filter(|&i| result.origins.vertices[i] == wanted)
        .collect();
    assert_eq!(found.len(), 1, "{wanted:?}");
    result.body.vertices[found[0]].point
}

fn near(a: Vec3, b: Vec3) {
    assert!(a.distance(b) < 1e-12, "{a:?} vs {b:?}");
}

/// The origins of a rectangle's revolution, worked out by hand: clear of
/// the axis and with a side on it, partial and full.
#[test]
fn origins_are_those_of_an_extrusion_bent_around_the_axis() {
    use Role::{EndCap, EndEdge, EndVertex, LateralEdge, Side, StartCap, StartEdge, StartVertex};
    let tol = tolerance(1.0);
    let plane = Frame::WORLD;
    let axis = Axis {
        point: [0.0, 0.0],
        direction: [0.0, 1.0],
    };
    // Ids 10 (bottom), 20 (far side), 30 (top), 40 (near side), drawn
    // counter-clockwise to the right of the axis.
    let clear = rectangle(10, 10, (1.0, 3.0), (0.0, 2.0));
    let corners = [[40, 10], [10, 20], [20, 30], [30, 40]];
    let all = |role: Role| -> Vec<Origin> { corners.iter().map(|c| origin(role, c)).collect() };
    let sides = |role: Role| -> Vec<Origin> {
        [10, 20, 30, 40]
            .iter()
            .map(|&c| origin(role, &[c]))
            .collect()
    };

    let part = build(&clear, &plane, axis, (0.0, 1.0), &tol).unwrap();
    let mut faces = sides(Side);
    faces.push(origin(StartCap, &[10, 20, 30, 40]));
    faces.push(origin(EndCap, &[10, 20, 30, 40]));
    assert_eq!(sorted(&part.origins.faces), sorted(&faces));
    let mut edges = sides(StartEdge);
    edges.extend(sides(EndEdge));
    edges.extend(all(LateralEdge));
    assert_eq!(sorted(&part.origins.edges), sorted(&edges));
    let mut vertices = all(StartVertex);
    vertices.extend(all(EndVertex));
    assert_eq!(sorted(&part.origins.vertices), sorted(&vertices));

    // Where things are. The profile is to the right of an axis pointing
    // up (+y), so by the right-hand rule it turns towards −z: after a
    // quarter turn the corner (3, 0) between the bottom and the far side
    // is at (0, 0, −3).
    let quarter = build(&clear, &plane, axis, (0.0, FRAC_PI_2), &tol).unwrap();
    near(
        vertex_at(&quarter, StartVertex, &[10, 20]),
        Vec3::new(3.0, 0.0, 0.0),
    );
    near(
        vertex_at(&quarter, EndVertex, &[10, 20]),
        Vec3::new(0.0, 0.0, -3.0),
    );
    near(
        vertex_at(&quarter, StartVertex, &[30, 40]),
        Vec3::new(1.0, 2.0, 0.0),
    );
    near(
        vertex_at(&quarter, EndVertex, &[30, 40]),
        Vec3::new(0.0, 2.0, -1.0),
    );
    // The arc a vertex sweeps joins its start vertex to its end vertex; a
    // curve's edges join the vertices at its ends.
    for (i, edge_origin) in quarter.origins.edges.iter().enumerate() {
        let edge = &quarter.body.edges[i];
        let (start, end) = (
            quarter.body.vertices[edge.start.0 as usize].point,
            quarter.body.vertices[edge.end.0 as usize].point,
        );
        if edge_origin.role == LateralEdge {
            near(
                start,
                vertex_at(&quarter, StartVertex, &edge_origin.sources),
            );
            near(end, vertex_at(&quarter, EndVertex, &edge_origin.sources));
        }
    }
    // The bottom (10) at the start runs from (1, 0) to (3, 0), at the end
    // from (0, 0, −1) to (0, 0, −3).
    for (role, from, to) in [
        (
            StartEdge,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
        ),
        (
            EndEdge,
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, -3.0),
        ),
    ] {
        let wanted = origin(role, &[10]);
        let i = quarter
            .origins
            .edges
            .iter()
            .position(|o| *o == wanted)
            .unwrap();
        let edge = &quarter.body.edges[i];
        near(edge.curve.eval(edge.range.0), from);
        near(edge.curve.eval(edge.range.1), to);
    }
    assert_eq!(face_kind(&quarter, Side, &[10]), Some("plane"));
    assert_eq!(face_kind(&quarter, Side, &[20]), Some("cylinder"));
    // The caps: the profile's own plane, looking back along the turn (+z
    // here), and the plane a quarter turn on (x = 0), looking forward (−x).
    for (role, normal) in [(StartCap, Vec3::Z), (EndCap, -Vec3::X)] {
        let wanted = origin(role, &[10, 20, 30, 40]);
        let i = quarter
            .origins
            .faces
            .iter()
            .position(|o| *o == wanted)
            .unwrap();
        let face = &quarter.body.faces[i];
        let Surface::Plane(p) = &face.surface else {
            panic!("a cap is flat");
        };
        assert!(!face.reversed);
        near(p.frame().z(), normal);
    }

    // A full turn: no caps and no profile edges; one circle and one vertex
    // per corner, both named after the corner.
    let full = build(&clear, &plane, axis, (0.0, TAU), &tol).unwrap();
    assert_eq!(sorted(&full.origins.faces), sorted(&sides(Side)));
    assert_eq!(sorted(&full.origins.edges), sorted(&all(LateralEdge)));
    assert_eq!(sorted(&full.origins.vertices), sorted(&all(StartVertex)));
    assert_eq!(genus(&full.body), Some(1));
    // The one vertex of each circle is the profile's own corner.
    near(
        vertex_at(&full, StartVertex, &[10, 20]),
        Vec3::new(3.0, 0.0, 0.0),
    );

    // With the near side (40) on the axis: it sweeps no face, its two ends
    // are one vertex each and sweep no arc, and it is one edge that both
    // caps use.
    let touching = rectangle(10, 10, (0.0, 3.0), (0.0, 2.0));
    let part = build(&touching, &plane, axis, (0.0, 1.0), &tol).unwrap();
    let mut faces: Vec<Origin> = [10, 20, 30].iter().map(|&c| origin(Side, &[c])).collect();
    faces.push(origin(StartCap, &[10, 20, 30, 40]));
    faces.push(origin(EndCap, &[10, 20, 30, 40]));
    assert_eq!(sorted(&part.origins.faces), sorted(&faces));
    let mut edges = sides(StartEdge);
    edges.extend([10, 20, 30].iter().map(|&c| origin(EndEdge, &[c])));
    edges.push(origin(LateralEdge, &[10, 20]));
    edges.push(origin(LateralEdge, &[20, 30]));
    assert_eq!(sorted(&part.origins.edges), sorted(&edges));
    let mut vertices = all(StartVertex);
    vertices.push(origin(EndVertex, &[10, 20]));
    vertices.push(origin(EndVertex, &[20, 30]));
    assert_eq!(sorted(&part.origins.vertices), sorted(&vertices));
    assert_eq!(counts(&part.body), (6, 9, 5));

    let full = build(&touching, &plane, axis, (0.0, TAU), &tol).unwrap();
    assert_eq!(
        sorted(&full.origins.faces),
        sorted(&[
            origin(Side, &[10]),
            origin(Side, &[20]),
            origin(Side, &[30])
        ])
    );
    assert_eq!(
        sorted(&full.origins.edges),
        sorted(&[
            origin(LateralEdge, &[10, 20]),
            origin(LateralEdge, &[20, 30])
        ])
    );
    assert_eq!(
        sorted(&full.origins.vertices),
        sorted(&[
            origin(StartVertex, &[10, 20]),
            origin(StartVertex, &[20, 30])
        ])
    );
}

/// An unrelated curve elsewhere in the sketch changes no origin of what was
/// already there.
#[test]
fn origins_do_not_depend_on_other_curves() {
    let tol = tolerance(1.0);
    let plane = frames(1.0)[1];
    let axis = Axis {
        point: [-1.0, 0.0],
        direction: [0.0, 1.0],
    };
    let alone = rectangle(10, 10, (1.0, 3.0), (0.0, 2.0));
    let mut more = vec![circle(5, [10.0, 1.0], 0.5)];
    more.extend(alone.clone());
    more.extend(polygon(100, 1, &[[5.0, 5.0], [7.0, 5.0], [6.0, 8.0]]));
    for sweep in [(0.0, 2.0), (0.0, TAU)] {
        let a = build(&alone, &plane, axis, sweep, &tol).unwrap();
        let b = build(&more, &plane, axis, sweep, &tol).unwrap();
        // Every entity of the rectangle alone is there again, in the same
        // place, under the same origin.
        for (i, o) in a.origins.vertices.iter().enumerate() {
            let j = b.origins.vertices.iter().position(|x| x == o).unwrap();
            assert_eq!(a.body.vertices[i], b.body.vertices[j], "{o:?}");
        }
        for (i, o) in a.origins.edges.iter().enumerate() {
            let j = b.origins.edges.iter().position(|x| x == o).unwrap();
            assert_eq!(a.body.edges[i].curve, b.body.edges[j].curve, "{o:?}");
        }
        for (i, o) in a.origins.faces.iter().enumerate() {
            let j = b.origins.faces.iter().position(|x| x == o).unwrap();
            assert_eq!(a.body.faces[i].surface, b.body.faces[j].surface, "{o:?}");
        }
    }
}

/// What cannot be revolved is an error that says why and names the curves.
#[test]
fn bad_input_is_an_error_with_a_reason() {
    use craft3d_kernel::KernelError as E;
    let tol = tolerance(1.0);
    let plane = Frame::WORLD;
    let up = Axis {
        point: [0.0, 0.0],
        direction: [0.0, 1.0],
    };
    let square = rectangle(10, 10, (1.0, 3.0), (0.0, 2.0));
    let profiles = detect_profiles(&square, &tol).unwrap();
    let run = |point: P, direction: P, from: f64, to: f64| {
        revolve(&profiles, &plane, point, direction, from, to, &tol)
    };
    // The axis.
    for direction in [
        [0.0, 0.0],
        [f64::NAN, 1.0],
        [f64::INFINITY, 0.0],
        [0.0, -0.0],
    ] {
        let error = run([0.0, 0.0], direction, 0.0, 1.0).unwrap_err();
        assert!(
            matches!(error, E::InvalidAxis { .. }),
            "{direction:?}: {error}"
        );
    }
    for point in [[f64::NAN, 0.0], [0.0, f64::NEG_INFINITY]] {
        let error = run(point, [0.0, 1.0], 0.0, 1.0).unwrap_err();
        assert!(matches!(error, E::InvalidAxis { .. }), "{point:?}: {error}");
    }
    // The angles: backwards, none, too little to move anything by the
    // tolerance, more than a turn, not numbers.
    for (from, to) in [
        (1.0, 0.5),
        (1.0, 1.0),
        (0.0, 1e-12),
        (0.0, 1e-8),
        (0.0, TAU + 1e-6),
        (-4.0, 4.0),
        (f64::NAN, 1.0),
        (0.0, f64::INFINITY),
        (f64::NEG_INFINITY, f64::INFINITY),
    ] {
        let error = run([0.0, 0.0], [0.0, 1.0], from, to).unwrap_err();
        // Compared as text: NaN is not equal to itself.
        assert_eq!(
            format!("{error:?}"),
            format!("{:?}", E::InvalidAngles { from, to }),
            "{from}..{to}"
        );
    }
    // A full turn within the angular tolerance is a full turn.
    for to in [TAU - 1e-10, TAU, TAU + 1e-10] {
        let result = run([0.0, 0.0], [0.0, 1.0], 0.0, to).unwrap();
        assert_eq!(result.origins.faces.len(), 4, "to {to}");
    }
    assert_eq!(
        revolve(&[], &plane, [0.0, 0.0], [0.0, 1.0], 0.0, 1.0, &tol).unwrap_err(),
        E::NoProfile
    );
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        for broken in [
            Tolerance {
                linear: bad,
                angular: 1e-9,
            },
            Tolerance {
                linear: 1e-6,
                angular: bad,
            },
        ] {
            let error =
                revolve(&profiles, &plane, [0.0, 0.0], [0.0, 1.0], 0.0, 1.0, &broken).unwrap_err();
            assert!(matches!(error, E::InvalidSize { .. }), "{bad}: {error}");
        }
    }

    // An axis through the profile: the curves that reach across to the
    // side with less of it. The square spans x = 1..3; at x = 1.5 the far
    // side (20) is wholly on the larger side and is not named.
    let through = |x: f64| run([x, 0.0], [0.0, 1.0], 0.0, 1.0).unwrap_err();
    assert_eq!(
        through(1.5),
        E::CrossesAxis {
            curves: vec![10, 30, 40]
        }
    );
    assert_eq!(
        through(2.5),
        E::CrossesAxis {
            curves: vec![10, 20, 30]
        }
    );
    assert!(through(1.5).to_string().contains("curves 10, 30, 40"));
    // Two profiles either side of the axis.
    let mut both = square.clone();
    both.extend(rectangle(50, 1, (-2.0, -1.0), (0.0, 1.0)));
    assert_eq!(
        build(&both, &plane, up, (0.0, 1.0), &tol).unwrap_err(),
        E::CrossesAxis {
            curves: vec![50, 51, 52, 53]
        }
    );
    // A circle about a point of the axis has half of itself on each side.
    let disc = [circle(7, [0.0, 1.0], 2.0)];
    assert_eq!(
        build(&disc, &plane, up, (0.0, TAU), &tol).unwrap_err(),
        E::CrossesAxis { curves: vec![7] }
    );

    // A circle that touches the axis (a horn torus) pinches there, and so
    // does an arc that touches it between its ends.
    let horn = [circle(7, [2.0, 1.0], 2.0)];
    let error = build(&horn, &plane, up, (0.0, TAU), &tol).unwrap_err();
    assert!(
        matches!(error, E::UnsupportedCurve { id: 7, .. }),
        "{error}"
    );
    assert!(error.to_string().contains("curve 7"), "{error}");
    assert!(error.to_string().contains("touches the axis"), "{error}");
    let touching = [
        arc(5, [2.0, 1.0], [2.0, 3.0], [2.0, -1.0]),
        line(6, [2.0, -1.0], [2.0, 3.0]),
    ];
    for sweep in [(0.0, 1.0), (0.0, TAU)] {
        let error = build(&touching, &plane, up, sweep, &tol).unwrap_err();
        assert!(
            matches!(error, E::UnsupportedCurve { id: 5, .. }),
            "{error}"
        );
        assert!(error.to_string().contains("between its ends"), "{error}");
    }
    // A D whose arc is part of a circle that crosses the axis (a spindle
    // torus) is clear of the axis itself: a rational surface, not a torus.
    let spindle = [
        arc(3, [0.5, 0.0], [0.5, -2.0], [0.5, 2.0]),
        line(4, [0.5, 2.0], [0.5, -2.0]),
    ];
    let made = build(&spindle, &plane, up, (0.0, 1.0), &tol).unwrap();
    assert_eq!(face_kind(&made, Role::Side, &[3]), Some("nurbs surface"));
    // Clear of the axis by more than its radius it is a torus.
    let clear = Axis {
        point: [-2.0, 0.0],
        ..up
    };
    let made = build(&spindle, &plane, clear, (0.0, 1.0), &tol).unwrap();
    assert_eq!(face_kind(&made, Role::Side, &[3]), Some("torus"));

    // A profile that `detect_profiles` would not have made.
    let mut twice = profiles.clone();
    twice.extend(profiles.clone());
    let error = revolve(&twice, &plane, [0.0, 0.0], [0.0, 1.0], 0.0, 1.0, &tol).unwrap_err();
    assert!(matches!(error, E::InvalidProfile { .. }), "{error}");
    let mut open = profiles.clone();
    open[0].outer.segments.truncate(1);
    let error = revolve(&open, &plane, [0.0, 0.0], [0.0, 1.0], 0.0, 1.0, &tol).unwrap_err();
    assert!(matches!(error, E::InvalidProfile { .. }), "{error}");
    let mut gap = profiles.clone();
    gap[0].outer.segments.truncate(3);
    let error = revolve(&gap, &plane, [0.0, 0.0], [0.0, 1.0], 0.0, 1.0, &tol).unwrap_err();
    assert!(
        matches!(error, E::InvalidProfile { .. } | E::InvalidBody { .. }),
        "{error}"
    );
    let mut wild = profiles.clone();
    wild[0].outer.segments[1].end = [f64::NAN, 2.0];
    assert!(revolve(&wild, &plane, [0.0, 0.0], [0.0, 1.0], 0.0, 1.0, &tol).is_err());
    let mut bent = profiles.clone();
    bent[0].outer.segments[1].shape = craft3d_kernel::profile::Shape::Arc {
        center: [f64::INFINITY, 0.0],
        radius: -1.0,
    };
    assert!(revolve(&bent, &plane, [0.0, 0.0], [0.0, 1.0], 0.0, 1.0, &tol).is_err());
}

/// Any numbers at all for the axis and the angles: an error or a valid
/// body, never a panic.
#[test]
fn hostile_numbers_never_panic() {
    let tol = tolerance(1.0);
    let plane = frames(1.0)[1];
    let hostile = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.0,
        -0.0,
        1e-320,
        1e-300,
        1e300,
        -1e300,
        f64::MAX,
        1.0,
        -2.5,
        TAU,
    ];
    let mut cases = catalogue(1.0);
    cases.truncate(6);
    let (mut ok, mut refused) = (0, 0);
    for case in &cases {
        let profiles = detect_profiles(&case.curves, &tol).unwrap();
        for &a in &hostile {
            for &b in &hostile {
                for (point, direction, from, to) in [
                    ([a, b], [0.0, 1.0], 0.0, 1.0),
                    ([-5.0, 0.0], [a, b], 0.0, 1.0),
                    ([-5.0, 0.0], [0.0, 1.0], a, b),
                    ([a, 0.0], [b, 1.0], 0.0, TAU),
                    ([-5.0, a], [1.0, 0.1], b, b + 1.0),
                ] {
                    match revolve(&profiles, &plane, point, direction, from, to, &tol) {
                        Ok(result) => {
                            assert_eq!(check(&result.body, &tol), vec![]);
                            ok += 1;
                        }
                        Err(_) => refused += 1,
                    }
                }
            }
        }
    }
    println!("hostile numbers: {ok} valid bodies, {refused} errors");
    assert!(ok > 0 && refused > 0);
}

/// The turn is counter-clockwise about the axis direction (right-hand
/// rule) from wherever it starts: with the axis along +y a point at
/// x = 3 is at z = −3 after a quarter turn and at x = −3 after half a
/// turn. And where one sweep ends the next begins.
#[test]
fn a_turn_starts_where_it_is_told_to() {
    use Role::{EndVertex, StartVertex};
    let tol = tolerance(1.0);
    let axis = Axis {
        point: [0.0, 0.0],
        direction: [0.0, 1.0],
    };
    let clear = rectangle(10, 10, (1.0, 3.0), (0.0, 2.0));
    let second = build(&clear, &Frame::WORLD, axis, (FRAC_PI_2, PI), &tol).unwrap();
    near(
        vertex_at(&second, StartVertex, &[10, 20]),
        Vec3::new(0.0, 0.0, -3.0),
    );
    near(
        vertex_at(&second, EndVertex, &[10, 20]),
        Vec3::new(-3.0, 0.0, 0.0),
    );
    // The same with the axis the other way: the profile is then to its
    // left and turns towards +z.
    let down = Axis {
        point: [0.0, 0.0],
        direction: [0.0, -1.0],
    };
    let other = build(&clear, &Frame::WORLD, down, (FRAC_PI_2, PI), &tol).unwrap();
    near(
        vertex_at(&other, StartVertex, &[10, 20]),
        Vec3::new(0.0, 0.0, 3.0),
    );
    near(
        vertex_at(&other, EndVertex, &[10, 20]),
        Vec3::new(-3.0, 0.0, 0.0),
    );
    let plane = frames(1.0)[1];
    let oblique = Axis {
        point: [-1.0, 0.5],
        direction: [0.3, 1.0],
    };
    let first = build(&clear, &plane, oblique, (-0.4, 1.1), &tol).unwrap();
    let next = build(&clear, &plane, oblique, (1.1, 2.0), &tol).unwrap();
    for corner in [[40, 10], [10, 20], [20, 30], [30, 40]] {
        near(
            vertex_at(&first, EndVertex, &corner),
            vertex_at(&next, StartVertex, &corner),
        );
    }
}

/// A half disc drawn loosely: the ends of its arc and its centre are on
/// the axis only within the tolerance. Still a ball, and the three
/// quarters of it that a three-quarter turn sweeps: the arc's ends are the
/// poles of the sphere, not points near them.
#[test]
fn a_loosely_drawn_half_disc_is_still_a_ball() {
    let tol = tolerance(1.0);
    let plane = frames(1.0)[1];
    let axis = Axis {
        point: [0.0, 0.0],
        direction: [0.0, 1.0],
    };
    for slack in [0.0, 1e-9, 3e-7, 8e-7] {
        let curves = [
            arc(1, [slack, 0.0], [-slack, -2.0], [slack * 0.5, 2.0]),
            line(2, [slack * 0.5, 2.0], [-slack, -2.0]),
        ];
        for (from, to) in [(0.0, TAU), (0.3, 0.3 + 1.5 * PI), (1.0, 1.4)] {
            let result = build(&curves, &plane, axis, (from, to), &tol)
                .unwrap_or_else(|e| panic!("slack {slack}, {from}..{to}: {e}"));
            let mass = mass_properties(&result.body).unwrap();
            let share = (to - from) / TAU;
            // As exact as the drawing: 2e-5 for the loosest.
            let allowed = 1e-11 + 20.0 * slack;
            close(
                mass.volume,
                share * 4.0 * PI * 8.0 / 3.0,
                allowed,
                &format!("slack {slack}, {from}..{to}: volume"),
            );
            let caps = if share < 1.0 { 4.0 * PI } else { 0.0 };
            close(
                mass.area,
                share * 16.0 * PI + caps,
                allowed,
                &format!("slack {slack}, {from}..{to}: area"),
            );
        }
    }
}
