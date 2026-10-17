//! Booleans with a rational freeform face (a NURBS surface whose weights
//! are not all one).
//!
//! The oracle is the strong point here: a rational surface can be a piece
//! of a cylinder, a cone, a sphere or a torus *exactly*. So every solid of
//! this file exists twice: with its curved faces on the analytic surfaces,
//! which the kernel handles in closed form and the boolean corpus has
//! measured against FreeCAD, and with the same faces on rational patches
//! (`rational_common`). The two are one solid; a boolean of the rational
//! one must give what the same boolean of the analytic one gives, and both
//! must give the closed form where there is one.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;
mod rational_common;

use boolean_common::*;
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::{self, Intersect, Subtract, Union};
use craft3d_kernel::boolean::boolean;
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::trimmed::domed_block;
use craft3d_topo::{Body, check};
use rational_common::{rationalized, skewed_apart, span_weight_ratio};
use std::f64::consts::PI;
use std::time::Instant;

/// A solid with curved faces that stay clear of poles and seams, its
/// volume and its area at scale 1.
struct Solid {
    name: &'static str,
    body: Body,
    volume: f64,
    area: f64,
    /// How many of its faces are curved.
    curved: usize,
}

/// A quarter of a rod: radius 2, height 3.
fn quarter_rod(k: f64) -> Solid {
    let rod = rod(k, [0.0, 0.0, 0.0], 2.0, 3.0);
    let corner = boxed(k, [0.0, 0.0, -1.0], [3.0, 3.0, 4.0]);
    Solid {
        name: "quarter rod",
        body: solve(&rod, &corner, Intersect, k).body,
        volume: PI * 4.0 * 3.0 / 4.0,
        area: 2.0 * PI + 3.0 * PI + 12.0,
        curved: 1,
    }
}

/// A quarter of the zone of a ball of radius 2 between the heights −1 and
/// 1: a sphere face between two latitudes, clear of the poles.
fn ball_zone(k: f64) -> Solid {
    let ball = ball(k, [0.0, 0.0, 0.0], 2.0);
    let slab = boxed(k, [0.0, 0.0, -1.0], [3.0, 3.0, 1.0]);
    let side = 3.0_f64.sqrt() + 2.0 * PI / 3.0;
    Solid {
        name: "ball zone",
        body: solve(&ball, &slab, Intersect, k).body,
        volume: PI * (8.0 - 2.0 / 3.0) / 4.0,
        area: 2.0 * PI + 2.0 * PI * 3.0 / 4.0 + 2.0 * side,
        curved: 1,
    }
}

/// A quarter of a frustum: a cone of base radius 2 and height 4 between
/// the heights 0.5 and 2.
fn frustum(k: f64) -> Solid {
    let cone = spike(k, [0.0, 0.0, 0.0], 2.0, 4.0);
    let slab = boxed(k, [0.0, 0.0, 0.5], [3.0, 3.0, 2.0]);
    let (low, high, height) = (1.75_f64, 1.0_f64, 1.5_f64);
    let slant = height.hypot(low - high);
    Solid {
        name: "frustum",
        body: solve(&cone, &slab, Intersect, k).body,
        volume: PI * height * (low * low + low * high + high * high) / 3.0 / 4.0,
        area: PI * (low + high) * slant / 4.0
            + PI * (low * low + high * high) / 4.0
            + (low + high) * height,
        curved: 1,
    }
}

/// A quarter of the slice of a ring (major 3, minor 1) between the heights
/// −0.5 and 0.5: an outer and an inner face on the torus.
fn ring_slice(k: f64) -> Solid {
    let ring = ring(k, [0.0, 0.0, 0.0], 3.0, 1.0);
    let slab = boxed(k, [0.0, 0.0, -0.5], [5.0, 5.0, 0.5]);
    let section = 3.0_f64.sqrt() / 2.0 + PI / 3.0;
    let half_width = 0.75_f64.sqrt();
    Solid {
        name: "ring slice",
        body: solve(&ring, &slab, Intersect, k).body,
        // Pappus: the section turns about the axis at the distance 3.
        volume: PI / 2.0 * 3.0 * section,
        area: PI * PI + 2.0 * 3.0 * PI * half_width + 2.0 * section,
        curved: 2,
    }
}

fn solids(k: f64) -> Vec<Solid> {
    vec![quarter_rod(k), ball_zone(k), frustum(k), ring_slice(k)]
}

fn rational_faces(body: &Body) -> usize {
    body.faces
        .iter()
        .filter(|f| match &f.surface {
            Surface::Nurbs(s) => s.weights().iter().any(|w| (w - 1.0).abs() > 1e-9),
            _ => false,
        })
        .count()
}

/// A tilt and a shift that leave nothing along an axis.
fn tilt(k: f64) -> Transform {
    Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
}

#[track_caller]
fn near(actual: f64, expected: f64, rel: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= rel * expected.abs().max(actual.abs()),
        "{what}: {actual} is not {expected} (off by {:e})",
        (actual - expected).abs() / expected.abs().max(actual.abs())
    );
}

/// The mesh of a body is closed and encloses its volume.
#[track_caller]
fn meshes(body: &Body, k: f64, what: &str) {
    let exact = mass(body);
    let chord = 5e-3 * k;
    let params = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
    let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert!(welded.is_closed(), "{what}: the mesh is not watertight");
    assert!(
        (welded.volume() - exact.volume).abs() <= exact.area * chord,
        "{what}: mesh volume {} against {}",
        welded.volume(),
        exact.volume
    );
}

#[test]
fn the_rational_twins_are_valid_and_measure_as_the_closed_forms() {
    for k in SCALES {
        let tol = tolerance(k);
        for solid in solids(k) {
            // The analytic body first: the oracle is right.
            let m = mass(&solid.body);
            assert_close(m.volume, solid.volume * k * k * k, solid.name);
            assert_close(m.area, solid.area * k * k, solid.name);
            for (pad, edges) in [(0.0, false), (0.25, false), (0.0, true), (0.1, true)] {
                for tilted in [false, true] {
                    let what = format!(
                        "{} at {k}, pad {pad}, edges {edges}, tilted {tilted}",
                        solid.name
                    );
                    let (mut twin, replaced) = rationalized(&solid.body, pad, edges);
                    assert_eq!(replaced, solid.curved, "{what}");
                    assert_eq!(rational_faces(&twin), solid.curved, "{what}");
                    if tilted {
                        twin = twin.transformed(&tilt(k)).unwrap();
                    }
                    assert_eq!(check(&twin, &tol), [], "{what}");
                    let t = mass(&twin);
                    near(t.volume, solid.volume * k * k * k, 1e-9, &what);
                    near(t.area, solid.area * k * k, 1e-9, &what);
                    let mut centre = m.centroid.unwrap();
                    if tilted {
                        centre = tilt(k).point(centre);
                    }
                    assert!(
                        t.centroid.unwrap().distance(centre) <= 1e-8 * k,
                        "{what}: centroid"
                    );
                    meshes(&twin, k, &what);
                }
            }
        }
    }
}

/// Weights far apart (2026-10-17): the rational twins with their weights
/// skewed by a Möbius reparametrisation of every span (`skewed`), so that
/// the weights of a span are 10, 100, 1000 and 10,000 times further apart
/// along u, along v or both, while the faces stay the same point sets and
/// the closed forms still hold. Before, `topo::mass_properties` settled on
/// a cylinder skewed a thousandfold along its axis 7e-6 short (the inner
/// panels stopped doubling before the boundary's did) and refused it at
/// 10,000; now each inner stretch is summed in the Möbius parameter that
/// evens out its end weights.
#[test]
fn twins_with_weights_far_apart_measure_as_the_closed_forms() {
    for k in SCALES {
        for solid in solids(k) {
            let what = format!("{} at {k}", solid.name);
            let (twin, _) = rationalized(&solid.body, 0.2, true);
            let centre = mass(&solid.body).centroid.unwrap();
            for ratio in [1.0_f64, 10.0, 100.0, 1e3, 1e4] {
                for (along_u, along_v) in [(ratio, 1.0), (1.0, ratio), (ratio, ratio)] {
                    let body = skewed_apart(&twin, along_u, along_v);
                    let what = format!("{what}, weights {along_u} and {along_v} apart");
                    let skew = body
                        .faces
                        .iter()
                        .filter_map(|f| match &f.surface {
                            Surface::Nurbs(s) => Some(span_weight_ratio(s)),
                            _ => None,
                        })
                        .fold(1.0, f64::max);
                    assert!(skew >= 0.5 * ratio, "{what}: {skew}");
                    assert_eq!(check(&body, &tolerance(k)), [], "{what}");
                    let m = mass(&body);
                    near(m.volume, solid.volume * k * k * k, 1e-9, &what);
                    near(m.area, solid.area * k * k, 1e-9, &what);
                    assert!(
                        m.centroid.unwrap().distance(centre) <= 1e-9 * k,
                        "{what}: centroid"
                    );
                }
            }
        }
    }
}

/// A block whose top is a bilinear rational patch with the weights 1, h,
/// h, 1 at its corners: the same rectangle as the plain top, drawn so
/// unevenly that no reparametrisation takes it out (the ratio of the
/// products of the diagonals' weights, `1 / h²`, is the same in every
/// parametrisation). Most of the top's area then lies, seen from the
/// boundary integral, on a stretch of an edge `1 / h²` of it long next to
/// a heavy corner. Before 2026-10-17 `topo::mass_properties` gave up at
/// a hundred apart (every panel doubled seven times, and the sums still
/// crept); the panels are now halved only where they disagree. At ten
/// thousand apart (a stretch 1e-8 long) the sums scatter at the rounding
/// of the boundary's parameters and the face is refused, not guessed;
/// beyond that `topo::MAX_UNEVEN` refuses it before anything is summed.
#[test]
fn a_plate_with_weights_far_apart_measures_as_the_block() {
    for k in SCALES {
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let corner = |x: f64, y: f64| Vec3::new(x, y, 2.0) * k;
        let weighted = |heavy: f64| {
            let mut body = plate.clone();
            body.faces[1].surface = Surface::Nurbs(
                NurbsSurface::new(
                    1,
                    1,
                    vec![0.0, 0.0, 1.0, 1.0],
                    vec![0.0, 0.0, 1.0, 1.0],
                    2,
                    2,
                    vec![
                        corner(0.0, 0.0),
                        corner(0.0, 3.0),
                        corner(4.0, 0.0),
                        corner(4.0, 3.0),
                    ],
                    vec![1.0, heavy, heavy, 1.0],
                )
                .unwrap(),
            );
            assert_eq!(check(&body, &tolerance(k)), []);
            body
        };
        for heavy in [1.0, 10.0, 100.0, 1000.0] {
            let what = format!("weights {heavy} apart at {k}");
            let body = weighted(heavy);
            assert_eq!(
                craft3d_topo::mass_bound(&body).map(|(m, _)| m),
                craft3d_topo::mass_properties(&body),
                "{what}"
            );
            let m = mass(&body);
            near(m.volume, 24.0 * k * k * k, 1e-12, &what);
            near(m.area, 52.0 * k * k, 1e-12, &what);
            assert!(
                m.centroid.unwrap().distance(Vec3::new(2.0, 1.5, 1.0) * k) <= 1e-12 * k,
                "{what}: centroid"
            );
        }
        if k == 1.0 {
            // At ten thousand, the bound of the measure: refused by the
            // bound, or by sums that do not settle (which is what refuses
            // it without the bound), never measured wrong.
            let refused = craft3d_topo::mass_properties(&weighted(1e4));
            assert!(
                matches!(&refused, Err(craft3d_topo::MassError::Unsupported { what, .. }) if what.contains("does not settle"))
                    || matches!(
                        &refused,
                        Err(craft3d_topo::MassError::UnevenWeights { face: 1, .. })
                    ),
                "{refused:?}"
            );
            // Further apart the measure is held for nothing, and is not
            // tried (from 1e7 its sums settled wrong: 22 for 24). The
            // bodies need not pass `check` any more (from 1e5 the edges
            // leave such a surface by more than the tolerance).
            for heavy in [1e5, 1e7, 1e12] {
                let mut body = plate.clone();
                body.faces[1].surface = weighted(1.0).faces[1].surface.clone();
                if let Surface::Nurbs(s) = &body.faces[1].surface {
                    body.faces[1].surface = Surface::Nurbs(
                        NurbsSurface::new(
                            1,
                            1,
                            s.knots_u().to_vec(),
                            s.knots_v().to_vec(),
                            2,
                            2,
                            s.control_points().to_vec(),
                            vec![1.0, heavy, heavy, 1.0],
                        )
                        .unwrap(),
                    );
                }
                let refused = craft3d_topo::mass_properties(&body);
                assert!(
                    matches!(&refused, Err(craft3d_topo::MassError::UnevenWeights { face: 1, ratio }) if (ratio / heavy - 1.0).abs() < 1e-9),
                    "{heavy}: {refused:?}"
                );
            }
        }
    }
}

/// The three booleans of the rational twin of `solid` and a tool, held to
/// the same booleans of the analytic solid.
fn agree(solid: &Solid, tool: &Body, k: f64, pad: f64, edges: bool, tilted: bool, what: &str) {
    let (mut twin, _) = rationalized(&solid.body, pad, edges);
    let (mut plain, mut tool) = (solid.body.clone(), tool.clone());
    if tilted {
        twin = twin.transformed(&tilt(k)).unwrap();
        plain = plain.transformed(&tilt(k)).unwrap();
        tool = tool.transformed(&tilt(k)).unwrap();
    }
    agree_with(&plain, &twin, &tool, k, what);
}

/// The three booleans of `twin` (a rational twin of `plain`) and a tool,
/// held to the same booleans of `plain`.
fn agree_with(plain: &Body, twin: &Body, tool: &Body, k: f64, what: &str) {
    agree_meshed(plain, twin, tool, k, what, true);
}

/// [`agree_with`], meshed only with `meshed`: the tessellator does not
/// mesh every face drawn as unevenly as the twins with weights far apart
/// (its grid is even in the parameters: too many segments per side, a
/// refinement that cannot go on, a side shared by two faces' meshes;
/// 2026-10-17, open in `tess`).
fn agree_meshed(plain: &Body, twin: &Body, tool: &Body, k: f64, what: &str, meshed: bool) {
    let (plain, twin, tool) = (plain.clone(), twin.clone(), tool.clone());
    for op in [Union, Intersect, Subtract] {
        let what = format!("{what}, {op:?}");
        let started = Instant::now();
        let expected = solve(&plain, &tool, op, k);
        let analytic = started.elapsed();
        let started = Instant::now();
        let result = match boolean(&twin, &tool, op, &tolerance(k)) {
            Ok(result) => result,
            Err(e) => panic!("{what}: {e}"),
        };
        let rational = started.elapsed();
        assert_eq!(check(&result.body, &tolerance(k)), [], "{what}");
        let (want, got) = (mass(&expected.body), mass(&result.body));
        if std::env::var_os("RATIONAL_TIMINGS").is_some() {
            eprintln!(
                "{what}: analytic {analytic:.3?}, rational {rational:.3?}, volume off {:.1e}, area off {:.1e}",
                (got.volume - want.volume).abs() / want.volume,
                (got.area - want.area).abs() / want.area
            );
        }
        near(got.volume, want.volume, 1e-6, &format!("{what}: volume"));
        near(got.area, want.area, 1e-6, &format!("{what}: area"));
        assert!(
            got.centroid.unwrap().distance(want.centroid.unwrap()) <= 1e-5 * k,
            "{what}: centroid"
        );
        assert_eq!(
            result.body.shells.len(),
            expected.body.shells.len(),
            "{what}: shells"
        );
        if op == Intersect {
            // The case is what it says: the tool goes through the
            // rational face, so what they have in common is bounded by a
            // piece of it and is less than the tool.
            assert!(rational_faces(&result.body) > 0, "{what}: no rational face");
            assert!(
                got.volume < 0.999 * mass(&tool).volume,
                "{what}: all of the tool"
            );
        }
        if meshed {
            meshes(&result.body, k, &what);
        }
    }
}

/// Tools in general position, each crossing the curved face of the solid
/// it is listed for.
fn tools(name: &str, k: f64) -> Vec<(&'static str, Body)> {
    match name {
        "quarter rod" => vec![
            ("block", boxed(k, [1.0, 0.7, 1.1], [2.6, 2.3, 2.2])),
            ("rod", log(k, [0.6, 1.2, 1.6], 0.5, 3.0)),
            ("ball", ball(k, [1.5, 1.3, 1.4], 0.8)),
            ("spike", spike(k, [1.3, 1.5, 0.6], 0.7, 1.9)),
            ("ring", ring(k, [1.45, 1.35, 1.5], 0.8, 0.3)),
        ],
        "ball zone" => vec![
            ("block", boxed(k, [0.9, 0.8, -0.4], [2.4, 2.1, 0.5])),
            ("rod", log(k, [0.5, 1.1, 0.1], 0.45, 3.0)),
            ("ball", ball(k, [1.5, 1.3, 0.1], 0.6)),
            ("spike", spike(k, [1.3, 1.5, -0.6], 0.6, 1.3)),
            ("ring", ring(k, [1.45, 1.35, 0.05], 0.7, 0.25)),
        ],
        "frustum" => vec![
            ("block", boxed(k, [0.6, 0.5, 0.9], [2.0, 1.6, 1.6])),
            ("rod", log(k, [0.3, 0.7, 1.2], 0.3, 3.0)),
            ("ball", ball(k, [1.0, 0.9, 1.2], 0.5)),
            ("spike", spike(k, [0.9, 1.0, 0.8], 0.45, 0.9)),
            ("ring", ring(k, [1.0, 0.95, 1.25], 0.5, 0.15)),
        ],
        "ring slice" => vec![
            ("block", boxed(k, [2.3, 1.4, -0.3], [3.6, 2.4, 0.35])),
            ("rod", log(k, [1.2, 2.0, 0.05], 0.3, 3.5)),
            ("ball", ball(k, [2.9, 2.6, 0.05], 0.4)),
            ("spike", spike(k, [2.8, 2.7, -0.4], 0.35, 0.8)),
            ("ring", ring(k, [2.85, 2.65, 0.02], 0.45, 0.15)),
        ],
        _ => panic!("no tools for {name}"),
    }
}

fn general(name: &str) {
    for k in SCALES {
        let solid = solids(k).into_iter().find(|s| s.name == name).unwrap();
        for (tool_name, tool) in tools(name, k) {
            for (pad, edges, tilted) in [(0.0, false, false), (0.2, true, true)] {
                let what = format!("{name} and {tool_name} at {k}, pad {pad}, tilted {tilted}");
                agree(&solid, &tool, k, pad, edges, tilted, &what);
            }
        }
    }
}

#[test]
fn a_rational_quarter_rod_against_every_kind_of_tool() {
    general("quarter rod");
}

#[test]
fn a_rational_ball_zone_against_every_kind_of_tool() {
    general("ball zone");
}

#[test]
fn a_rational_frustum_against_every_kind_of_tool() {
    general("frustum");
}

#[test]
fn a_rational_ring_slice_against_every_kind_of_tool() {
    general("ring slice");
}

/// Bodies in contact with the curved face of the quarter rod.
fn contacts(k: f64) -> Vec<(&'static str, Body)> {
    let d = Vec3::new(0.5_f64.sqrt(), 0.5_f64.sqrt(), 0.0);
    let across = Vec3::Z.cross(d);
    let flat = craft3d_math::Frame::new((d * 2.0 - across + Vec3::Z) * k, Vec3::Z, d).unwrap();
    let r2 = 2.0_f64.sqrt();
    vec![
        (
            "a block face tangent to it",
            craft3d_kernel::primitives::block(&flat, k, 2.0 * k, k).unwrap(),
        ),
        (
            "a ball tangent outside",
            ball(k, [2.5 * d.x, 2.5 * d.y, 1.5], 0.5),
        ),
        (
            "a ball tangent inside",
            ball(k, [1.5 * d.x, 1.5 * d.y, 1.5], 0.5),
        ),
        (
            "the rod it lies on, higher",
            rod(k, [0.0, 0.0, 1.0], 2.0, 4.0),
        ),
        (
            "a block edge on it, inside",
            boxed(k, [0.5, 0.5, 1.0], [r2, r2, 2.0]),
        ),
        (
            "a block edge on it, outside",
            boxed(k, [r2, r2, 1.0], [3.0, 3.0, 2.0]),
        ),
    ]
}

/// What a boolean came to.
#[derive(Debug)]
enum Outcome {
    Body {
        volume: f64,
        area: f64,
        centroid: Vec3,
        shells: usize,
        /// Refused as zero thickness (parts that touch, since
        /// 2026-10-15); the rest is the body made without the refusal.
        zero_thickness: bool,
    },
    Empty,
    Refused(String),
}

fn outcome(a: &Body, b: &Body, op: BooleanOp, k: f64, what: &str) -> Outcome {
    let unrefused = craft3d_kernel::boolean::BooleanOptions {
        refuse_zero_thickness: false,
        ..Default::default()
    };
    let (made, at) = match boolean(a, b, op, &tolerance(k)) {
        Err(KernelError::ZeroThickness { at, .. }) => (
            craft3d_kernel::boolean::boolean_with(a, b, op, &tolerance(k), &unrefused),
            Some(at),
        ),
        other => (other, None),
    };
    let zero_thickness = at.is_some();
    match made {
        Ok(result) => {
            assert_eq!(check(&result.body, &tolerance(k)), [], "{what}");
            if let Some(at) = at {
                assert!(
                    on_parts(&result.body, at, &tolerance(k)) >= 2,
                    "{what}: {at:?}"
                );
            }
            meshes(&result.body, k, what);
            let m = mass(&result.body);
            Outcome::Body {
                volume: m.volume,
                area: m.area,
                centroid: m.centroid.unwrap(),
                shells: result.body.shells.len(),
                zero_thickness,
            }
        }
        Err(KernelError::EmptyResult { .. }) => Outcome::Empty,
        Err(KernelError::Unsupported { reason, .. }) => Outcome::Refused(reason.to_string()),
        Err(other) => panic!("{what}: {other}"),
    }
}

/// The rational outcome is the analytic one: the same solid to the
/// tolerance, or nothing where that is nothing.
#[track_caller]
fn same(rational: &Outcome, analytic: &Outcome, k: f64, what: &str) {
    match (rational, analytic) {
        (Outcome::Empty, Outcome::Empty) => {}
        (
            Outcome::Body {
                volume,
                area,
                centroid,
                shells,
                zero_thickness,
            },
            Outcome::Body {
                volume: v,
                area: a,
                centroid: c,
                shells: s,
                zero_thickness: z,
            },
        ) => {
            assert_eq!(zero_thickness, z, "{what}: zero thickness");
            near(*volume, *v, 1e-6, &format!("{what}: volume"));
            near(*area, *a, 1e-6, &format!("{what}: area"));
            assert!(centroid.distance(*c) <= 1e-5 * k, "{what}: centroid");
            assert_eq!(shells, s, "{what}: shells");
        }
        _ => panic!("{what}: {rational:?} against {analytic:?}"),
    }
}

/// Contact that is not coincidence: the rational face tangent to a plane
/// and to a ball (from outside and from inside), and an edge of a block on
/// it (from inside and from outside). The kernel settles each as it does
/// for the cylinder the face is.
#[test]
fn a_rational_face_in_tangent_contact_and_with_an_edge_on_it() {
    for k in SCALES {
        let solid = quarter_rod(k);
        for (name, tool) in contacts(k) {
            if name.starts_with("the rod") {
                continue;
            }
            for (pad, tilted) in [(0.0, false), (0.2, true)] {
                let (mut twin, _) = rationalized(&solid.body, pad, false);
                let (mut plain, mut tool) = (solid.body.clone(), tool.clone());
                if tilted {
                    twin = twin.transformed(&tilt(k)).unwrap();
                    plain = plain.transformed(&tilt(k)).unwrap();
                    tool = tool.transformed(&tilt(k)).unwrap();
                }
                for op in [Union, Intersect, Subtract] {
                    let what = format!("{name}, {op:?} at {k}, tilted {tilted}");
                    let analytic = outcome(&plain, &tool, op, k, &what);
                    assert!(
                        !matches!(analytic, Outcome::Refused(_)),
                        "{what}: the analytic twin is refused: {analytic:?}"
                    );
                    same(&outcome(&twin, &tool, op, k, &what), &analytic, k, &what);
                }
            }
        }
    }
}

/// A rational face that lies on the surface of a face of the other body:
/// a rational quarter cylinder against the analytic cylinder it is a
/// piece of, against its analytic twin lifted along the axis, and against
/// itself lifted (two freeform faces on one surface); and a rational
/// quarter of a zone of a ball against the ball. Each is held to the
/// analytic twin. The first and the last are built since 2026-10-14:
/// geometry finds that the patch lies on the cylinder, a point of the
/// cylinder beyond the patch is off the face rather than placed in the
/// patch's chart, and a face on a patch is not merged with the other
/// body's face on the same surface (it would run off the patch). The
/// other two since 2026-10-15: an edge that lies on a patch for a while
/// and leaves it where the patch ends is cut there (it was cut at a place
/// found by bisecting a distance that is zero all along, "an imprinted
/// curve that does not fit"), and a patch lies on a patch where their
/// samples over each other are within the tolerance, those past the end
/// of the other left out ("touch along a curve without a closed form").
/// With patches that end where the faces do and patches a fifth larger,
/// also each against the other (two patches of one surface with other
/// parameters), upright and tilted.
#[test]
fn a_rational_face_on_the_surface_it_equals_is_right() {
    for k in SCALES {
        let solid = quarter_rod(k);
        let (twin, _) = rationalized(&solid.body, 0.0, false);
        let (wide, _) = rationalized(&solid.body, 0.2, false);
        let lift = Transform::translation(Vec3::new(0.0, 0.0, k));
        let lifted = |body: &Body| body.transformed(&lift).unwrap();
        let zone = ball_zone(k);
        let (zone_twin, _) = rationalized(&zone.body, 0.0, false);
        let cases: Vec<(&str, &Body, &Body, Body, Body)> = vec![
            (
                "the rod it lies on, higher",
                &twin,
                &solid.body,
                rod(k, [0.0, 0.0, 1.0], 2.0, 4.0),
                rod(k, [0.0, 0.0, 1.0], 2.0, 4.0),
            ),
            (
                "its analytic twin, lifted",
                &twin,
                &solid.body,
                lifted(&solid.body),
                lifted(&solid.body),
            ),
            (
                "itself, lifted",
                &twin,
                &solid.body,
                lifted(&twin),
                lifted(&solid.body),
            ),
            (
                "a wider patch of itself, lifted",
                &twin,
                &solid.body,
                lifted(&wide),
                lifted(&solid.body),
            ),
            (
                "wider patches, lifted",
                &wide,
                &solid.body,
                lifted(&wide),
                lifted(&solid.body),
            ),
            (
                "a rational ball zone and the ball it lies on",
                &zone_twin,
                &zone.body,
                ball(k, [0.0, 0.0, 0.0], 2.0),
                ball(k, [0.0, 0.0, 0.0], 2.0),
            ),
        ];
        for (name, twin, plain, tool, plain_tool) in cases {
            // (Tilted, the analytic union of the rod with the rod above
            // it does not mesh: a seam of the tessellator's.)
            let tilts: &[bool] = if name.contains("lifted") {
                &[false, true]
            } else {
                &[false]
            };
            for &tilted in tilts {
                let turn = |body: &Body| {
                    if tilted {
                        body.transformed(&tilt(k)).unwrap()
                    } else {
                        body.clone()
                    }
                };
                let (twin, plain, tool, plain_tool) =
                    (turn(twin), turn(plain), turn(&tool), turn(&plain_tool));
                for op in [Union, Intersect, Subtract] {
                    let what = format!("{name}, {op:?} at {k}, tilted {tilted}");
                    let analytic = outcome(&plain, &plain_tool, op, k, &what);
                    if let Outcome::Refused(reason) = &analytic {
                        panic!("{what}: the analytic twin is refused: {reason}");
                    }
                    same(&outcome(&twin, &tool, op, k, &what), &analytic, k, &what);
                }
            }
        }
    }
}

/// An octant of a ball as a rational patch has a border collapsed into
/// the pole, where its parameters stall (as those of a polynomial patch
/// with a collapsed border do). It was refused as an operand until
/// 2026-10-14; a pole is an ordinary point of the face's chart now, and
/// the patch is cut like the sphere it is (`boolean_poles.rs` has the
/// cuts around and through the pole).
#[test]
fn a_rational_patch_with_a_pole_is_a_valid_body_and_an_operand() {
    for k in SCALES {
        // The octant of `craft3d_testkit`: its sphere's pole is the
        // corner where two of the face's three arcs meet.
        let octant = craft3d_testkit::trimmed::sphere_octant(&Frame::WORLD, 2.0 * k)
            .unwrap()
            .body;
        let (twin, replaced) = rationalized(&octant, 0.0, false);
        assert_eq!(replaced, 1);
        assert_eq!(check(&twin, &tolerance(k)), []);
        // The patch is the octant: a border of it is the pole.
        let Some(Surface::Nurbs(patch)) = twin
            .faces
            .iter()
            .map(|f| &f.surface)
            .find(|s| matches!(s, Surface::Nurbs(_)))
        else {
            panic!("no patch");
        };
        let pole = Vec3::new(0.0, 0.0, 2.0 * k);
        for u in [0.0, 1.0, 2.2, 3.0] {
            assert!(patch.eval(u, 1.0).distance(pole) < 1e-12 * k);
        }
        let tool = boxed(k, [0.8, 0.7, 0.9], [2.5, 2.5, 2.5]);
        for op in [Union, Intersect, Subtract] {
            let what = format!("{op:?} at {k}");
            let expected = solve(&octant, &tool, op, k);
            let result = solve(&twin, &tool, op, k);
            let (want, got) = (mass(&expected.body), mass(&result.body));
            near(got.volume, want.volume, 1e-6, &format!("{what}: volume"));
            near(got.area, want.area, 1e-6, &format!("{what}: area"));
            meshes(&result.body, k, &what);
        }
    }
}

// ------------------------------------------------- not a quadric

const DX: f64 = 4.0;
const DY: f64 = 3.0;
const HEIGHT: f64 = 2.0;

/// The weight of control point `(i, j)` of the weighted roof: 1, 2, 0.5
/// and 3 in turn, without symmetry.
fn roof_weight(i: usize, j: usize) -> f64 {
    [1.0, 2.0, 0.5, 3.0][(i + 3 * j + i * j) % 4]
}

/// The domed block of `craft3d_testkit` with weights on its bicubic roof:
/// a rational surface that is no quadric, with the four roof edges as the
/// rational curves its borders are. The roof itself comes along.
fn weighted_dome(k: f64) -> (Body, NurbsSurface) {
    let mut body = domed_block(&Frame::WORLD, DX * k, DY * k, HEIGHT * k, true)
        .unwrap()
        .body;
    let index = body
        .faces
        .iter()
        .position(|f| matches!(f.surface, Surface::Nurbs(_)))
        .unwrap();
    let Surface::Nurbs(plain) = body.faces[index].surface.clone() else {
        panic!("the roof is a spline surface")
    };
    let net = plain.control_points().to_vec();
    let weights: Vec<f64> = (0..16).map(|n| roof_weight(n / 4, n % 4)).collect();
    let roof = NurbsSurface::new(
        3,
        3,
        plain.knots_u().to_vec(),
        plain.knots_v().to_vec(),
        4,
        4,
        net.clone(),
        weights,
    )
    .unwrap();
    body.faces[index].surface = Surface::Nurbs(roof.clone());
    // The borders: a row or a column of the net with its weights.
    let mut replaced = 0;
    for edge in &mut body.edges {
        let Curve3::Nurbs(curve) = &edge.curve else {
            continue;
        };
        let ends = (curve.control_points()[0], curve.control_points()[3]);
        let border = [0, 3]
            .into_iter()
            .flat_map(|fixed| {
                [
                    (0..4).map(|i| (i, fixed)).collect::<Vec<_>>(),
                    (0..4).map(|j| (fixed, j)).collect::<Vec<_>>(),
                ]
            })
            .find(|line| {
                let at = |n: usize| net[line[n].0 * 4 + line[n].1];
                (at(0), at(3)) == ends
            })
            .unwrap();
        edge.curve = Curve3::Nurbs(
            NurbsCurve::new(
                3,
                curve.knots().to_vec(),
                curve.control_points().to_vec(),
                border.iter().map(|&(i, j)| roof_weight(i, j)).collect(),
            )
            .unwrap(),
        );
        replaced += 1;
    }
    assert_eq!(replaced, 4);
    (body, roof)
}

/// The parameters and the height of the weighted roof above `(x, y)`, at
/// scale 1, by Newton's iteration from `start`.
fn roof_above(roof: &NurbsSurface, x: f64, y: f64, start: (f64, f64)) -> ((f64, f64), f64) {
    let (mut u, mut v) = start;
    for _ in 0..50 {
        let (p, su, sv) = roof.derivatives(u, v);
        let (rx, ry) = (p.x - x, p.y - y);
        if rx.hypot(ry) < 1e-13 {
            return ((u, v), p.z);
        }
        let det = su.x * sv.y - su.y * sv.x;
        assert!(det > 0.0, "the roof is no height field at ({u}, {v})");
        u = (u - (rx * sv.y - ry * sv.x) / det).clamp(0.0, 1.0);
        v = (v - (su.x * ry - su.y * rx) / det).clamp(0.0, 1.0);
    }
    panic!("no point of the roof above ({x}, {y})");
}

/// The integral of `f(x, y, roof height)` over a region of the floor,
/// given by its sample points and their weights.
fn under_roof(
    roof: &NurbsSurface,
    samples: impl Iterator<Item = (f64, f64, f64)>,
    f: impl Fn(f64, f64, f64) -> f64,
) -> f64 {
    let mut at = (0.5, 0.5);
    let mut sum = 0.0;
    for (x, y, weight) in samples {
        // From the middle when the last sample is far (a new row).
        let (found, z) = match std::panic::catch_unwind(|| roof_above(roof, x, y, at)) {
            Ok(found) => found,
            Err(_) => roof_above(roof, x, y, (0.5, 0.5)),
        };
        at = found;
        sum += weight * f(x, y, z);
    }
    sum
}

fn rectangle(lo: (f64, f64), hi: (f64, f64)) -> impl Iterator<Item = (f64, f64, f64)> {
    const N: usize = 400;
    let (w, h) = ((hi.0 - lo.0) / N as f64, (hi.1 - lo.1) / N as f64);
    (0..N).flat_map(move |i| {
        (0..N).map(move |j| {
            (
                lo.0 + (i as f64 + 0.5) * w,
                lo.1 + (j as f64 + 0.5) * h,
                w * h,
            )
        })
    })
}

/// A disc, sampled evenly in the angle from the vertical of the ball it
/// is the shadow of: the height of the ball over the disc is then a
/// smooth function of the samples up to the rim, where it has a square
/// root in the distance from the centre.
fn disc(center: (f64, f64), radius: f64) -> impl Iterator<Item = (f64, f64, f64)> {
    // Fine, because the integrand has a kink where the roof leaves the
    // ball: the error falls with the square of the step only.
    const N: usize = 1600;
    let (dp, da) = (0.5 * PI / N as f64, 2.0 * PI / N as f64);
    (0..N).flat_map(move |i| {
        let (sine, cosine) = ((i as f64 + 0.5) * dp).sin_cos();
        let r = radius * sine;
        (0..N).map(move |j| {
            let a = (j as f64 + 0.5) * da;
            (
                center.0 + r * a.cos(),
                center.1 + r * a.sin(),
                radius * radius * sine * cosine * dp * da,
            )
        })
    })
}

#[test]
fn a_weighted_roof_is_valid_and_measures_as_its_integral() {
    let (_, roof) = weighted_dome(1.0);
    // The volume under the roof, by a quadrature of its own (not the
    // kernel's): the roof is a height field.
    let volume = under_roof(&roof, rectangle((0.0, 0.0), (DX, DY)), |_, _, z| z);
    // It is no quadric and not the polynomial roof: the weights move it.
    let plain = domed_block(&Frame::WORLD, DX, DY, HEIGHT, true).unwrap();
    assert!((volume - plain.volume).abs() > 0.05, "{volume}");
    for k in SCALES {
        let (body, _) = weighted_dome(k);
        assert_eq!(rational_faces(&body), 1);
        for tilted in [false, true] {
            let body = if tilted {
                body.transformed(&tilt(k)).unwrap()
            } else {
                body.clone()
            };
            let what = format!("the weighted dome at {k}, tilted {tilted}");
            assert_eq!(check(&body, &tolerance(k)), [], "{what}");
            near(mass(&body).volume, volume * k * k * k, 1e-7, &what);
            meshes(&body, k, &what);
        }
    }
}

/// The three booleans of two bodies, held to the identities of sets:
/// what is in both and what is in either add up to the two, and what is
/// left of the first and what is in both add up to the first. In volume
/// and, where no faces coincide, in area. Every result is checked and
/// meshed; the volume in both comes back.
fn identities(a: &Body, b: &Body, k: f64, what: &str) -> f64 {
    let (ma, mb) = (mass(a), mass(b));
    let started = Instant::now();
    let both = solve(a, b, Intersect, k);
    let either = solve(a, b, Union, k);
    let rest = solve(a, b, Subtract, k);
    if std::env::var_os("RATIONAL_TIMINGS").is_some() {
        eprintln!("{what}: three booleans in {:.3?}", started.elapsed());
    }
    let (mi, mu, md) = (mass(&both.body), mass(&either.body), mass(&rest.body));
    let size = ma.volume + mb.volume;
    assert!(
        (mu.volume + mi.volume - size).abs() <= 1e-6 * size,
        "{what}: union {} + common {} against {} + {}",
        mu.volume,
        mi.volume,
        ma.volume,
        mb.volume
    );
    assert!(
        (md.volume + mi.volume - ma.volume).abs() <= 1e-6 * size,
        "{what}: difference {} + common {} against {}",
        md.volume,
        mi.volume,
        ma.volume
    );
    let skin = ma.area + mb.area;
    assert!(
        (mu.area + mi.area - skin).abs() <= 1e-6 * skin,
        "{what}: areas {} + {} against {} + {}",
        mu.area,
        mi.area,
        ma.area,
        mb.area
    );
    for (name, result) in [("common", &both), ("union", &either), ("difference", &rest)] {
        meshes(&result.body, k, &format!("{what}, {name}"));
    }
    mi.volume
}

#[test]
fn a_weighted_roof_against_a_block_and_a_ball() {
    let (_, roof) = weighted_dome(1.0);
    // What a block from z = 1 up and a ball have in common with the dome,
    // by the quadrature: the roof is above 1 over the block, and cuts the
    // ball in a curve around its lower half (the ball's rim is clear of
    // the roof: the integrand has no root at the rim).
    let in_block = under_roof(&roof, rectangle((0.5, 0.5), (3.5, 2.5)), |_, _, z| {
        assert!(z > 1.0 && z < 3.0);
        z - 1.0
    });
    let (centre, radius) = ([2.1, 1.4, 2.0], 0.8);
    let in_ball = under_roof(&roof, disc((centre[0], centre[1]), radius), |x, y, z| {
        let half = (radius * radius - (x - centre[0]).powi(2) - (y - centre[1]).powi(2))
            .max(0.0)
            .sqrt();
        (z.min(centre[2] + half) - (centre[2] - half)).max(0.0)
    });
    for k in SCALES {
        let (dome, _) = weighted_dome(k);
        let tools = [
            (
                "a block through the roof",
                boxed(k, [0.5, 0.5, 1.0], [3.5, 2.5, 3.0]),
                Some(in_block),
            ),
            ("a ball in the roof", ball(k, centre, radius), Some(in_ball)),
            // Through two walls and the roof's corner: the rational roof
            // edges are cut.
            (
                "a block over a corner",
                boxed(k, [3.0, 2.0, 1.0], [5.0, 4.0, 3.5]),
                None,
            ),
            (
                "a rod through the roof",
                log(k, [-1.0, 1.6, 1.9], 0.6, 6.0),
                None,
            ),
        ];
        for (name, tool, common) in tools {
            for tilted in [false, true] {
                let what = format!("the weighted dome and {name} at {k}, tilted {tilted}");
                let (a, b) = if tilted {
                    (
                        dome.transformed(&tilt(k)).unwrap(),
                        tool.transformed(&tilt(k)).unwrap(),
                    )
                } else {
                    (dome.clone(), tool.clone())
                };
                let volume = identities(&a, &b, k, &what);
                if let Some(common) = common {
                    // The ball's quadrature has a kink where the roof
                    // leaves the ball: good to 1e-6.
                    near(volume, common * k * k * k, 1e-6, &what);
                }
            }
        }
    }
}

#[test]
#[ignore = "timing: prints how long one boolean takes"]
fn time_one() {
    let k = 1.0;
    let which = std::env::var("RATIONAL_CASE").unwrap_or_default();
    let block = boxed(k, [0.5, 0.5, 1.0], [3.5, 2.5, 3.0]);
    let plain = domed_block(&Frame::WORLD, DX, DY, HEIGHT, true)
        .unwrap()
        .body;
    let (weighted, _) = weighted_dome(k);
    for (name, a) in [("polynomial roof", &plain), ("weighted roof", &weighted)] {
        if !which.is_empty() && !name.starts_with(&which) {
            continue;
        }
        for op in [Union, Intersect, Subtract] {
            let started = Instant::now();
            let result = boolean(a, &block, op, &tolerance(k)).unwrap();
            eprintln!(
                "{name}, {op:?}: {:.3?} ({} faces)",
                started.elapsed(),
                result.body.faces.len()
            );
        }
    }
}

/// Weights far apart in booleans (2026-10-17): the twins with the weights
/// of every span skewed 10, 100 and 1000 times further apart along u, along
/// v and both (a thousandfold both ways is 1.4e6 within a span), each the
/// same solid, against the tools that cross their curved faces, held to
/// the same booleans of the analytic solid as in [`agree`]. Two tools per
/// ratio and direction, taken in turn, so that every tool meets every
/// ratio. Before, a thousand apart was refused at the tree ("rational …
/// weights"), and a hundred apart along u by the surface intersection
/// ("tangent or nearly coincident over too large an area": the marcher's
/// cells were judged by the derivative bounds of the whole surface; then
/// "stalled", at a knot where the spans are drawn at paces a hundred
/// apart).
fn skewed_against_tools(name: &str) {
    for k in SCALES {
        let solid = solids(k).into_iter().find(|s| s.name == name).unwrap();
        let (twin, _) = rationalized(&solid.body, 0.2, true);
        let tools = tools(name, k);
        let mut turn = 0;
        for ratio in [10.0, 100.0, 1000.0] {
            for (along_u, along_v) in [(ratio, 1.0), (1.0, ratio), (ratio, ratio)] {
                let body = skewed_apart(&twin, along_u, along_v);
                assert_eq!(check(&body, &tolerance(k)), [], "{name}");
                for pick in [turn, turn + 2] {
                    let (tool_name, tool) = &tools[pick % tools.len()];
                    let what =
                        format!("{name} skewed {along_u} by {along_v}, and {tool_name} at {k}");
                    // Meshed where the tessellator meshes such faces: ten
                    // apart (from a hundred it does not always, open in
                    // `tess`).
                    agree_meshed(&solid.body, &body, tool, k, &what, ratio <= 10.0);
                }
                turn += 1;
            }
        }
    }
}

#[test]
fn a_quarter_rod_with_weights_far_apart_against_tools() {
    skewed_against_tools("quarter rod");
}

#[test]
fn a_ball_zone_with_weights_far_apart_against_tools() {
    skewed_against_tools("ball zone");
}

#[test]
fn a_frustum_with_weights_far_apart_against_tools() {
    skewed_against_tools("frustum");
}

#[test]
fn a_ring_slice_with_weights_far_apart_against_tools() {
    skewed_against_tools("ring slice");
}
