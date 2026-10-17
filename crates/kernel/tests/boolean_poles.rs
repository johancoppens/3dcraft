//! Booleans with a freeform face that has a pole: a border of its patch
//! that is one point, where the parameters stall and the surface is a
//! surface all the same.
//!
//! Two oracles. An octant of a ball exists twice, as in
//! `boolean_rational.rs`: with its curved face on the analytic sphere and
//! on the rational patch that equals it, whose border at the pole is that
//! one point; a boolean of the one must give what the same boolean of the
//! other gives. The dome and the drop that a spline sweeps when it ends on
//! the axis have no analytic twin: they are held to the identities of
//! sets and to the closed forms of the cuts that have one (Pappus, from
//! the profile by an integral of its own).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;
mod rational_common;

use boolean_common::*;
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::{boolean, contains_points};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, check};
use rational_common::rationalized;
use std::f64::consts::{PI, TAU};

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

fn mesh_of(body: &Body, k: f64) -> Result<craft3d_tess::Mesh, craft3d_tess::TessError> {
    let params = TessParams {
        chordal_tolerance: 5e-3 * k,
        ..TessParams::default()
    };
    tessellate_body(body, &params)
}

/// Whether the mesh of a body is closed and encloses its volume, or why
/// not.
fn try_mesh(body: &Body, k: f64) -> Result<(), String> {
    let exact = mass(body);
    let chord = 5e-3 * k;
    let mesh = mesh_of(body, k).map_err(|e| e.to_string())?;
    let welded = mesh.weld().map_err(|e| format!("{e:?}"))?;
    if !welded.is_closed() {
        return Err("the mesh is not watertight".to_string());
    }
    if (welded.volume() - exact.volume).abs() > exact.area * chord {
        return Err(format!(
            "mesh volume {} against {}",
            welded.volume(),
            exact.volume
        ));
    }
    Ok(())
}

/// (Empty from 2026-10-14, when the tessellator meshed all eight that
/// were listed here, cuts through and around a pole, to 2026-10-17: the
/// cuts along the axis of a cusp and of the cone that a spline sweeps,
/// below.)
///
/// Results of this file that the tessellator does not mesh yet, by what
/// their names contain, each with how it fails. They are valid bodies
/// that measure as their oracle says; the mesh is the tessellator's to
/// get right (`crates/tess`: a face whose boundary passes through a pole
/// of its patch on the way of an edge, or runs around one), and
/// `the_tessellator_meshes_cuts_through_a_pole` below reproduces them.
/// An entry that meshes is to be removed here: the test says so.
const UNMESHED: &[(&[&str], &str)] = &[
    // The needle's face next to its point, where the cut's edges end:
    // the face's mesh leaves the edge's chords unused there.
    (&["cusp, a plane"], "is not closed along edge"),
    (
        &["cusp, a cylinder through the axis"],
        "is not closed along edge",
    ),
    (&["cusp, a block edge", "union"], "is not closed along edge"),
    (&["cusp, a block edge", "rest"], "is not closed along edge"),
    (
        &["cusp, a block edge", "turned 0.4", "common"],
        "is not closed along edge",
    ),
    // The cone a straight spline sweeps from the axis, cut through its
    // apex along the axis (a pole of the first order: the same at 1e-3
    // meshes).
    (
        &["twin cone, a plane through the axis, at 1,"],
        "folds three distinct points",
    ),
    (
        &["twin cone, a plane through the axis, at 1000000,"],
        "folds three distinct points",
    ),
    (
        &["twin cone, a block edge along the axis, at 1,"],
        "folds three distinct points",
    ),
    (
        &["twin cone, a block edge along the axis, at 1000000, common"],
        "folds three distinct points",
    ),
];

/// The mesh of a body is closed and encloses its volume, unless it is
/// one of the [`UNMESHED`], which must then fail as listed.
#[track_caller]
fn meshes(body: &Body, k: f64, what: &str) {
    let known = UNMESHED
        .iter()
        .find(|(needles, _)| needles.iter().all(|needle| what.contains(needle)));
    match (try_mesh(body, k), known) {
        (Ok(()), None) => {}
        (Err(why), Some((_, how))) if why.contains(how) => {}
        (Ok(()), Some(_)) => panic!("{what}: meshes now; remove it from UNMESHED"),
        (Err(why), _) => panic!("{what}: {why}"),
    }
}

fn freeform_faces(body: &Body) -> usize {
    body.faces
        .iter()
        .filter(|f| matches!(f.surface, Surface::Nurbs(_) | Surface::Bezier(_)))
        .count()
}

/// The octant of a ball of radius 2 about the origin with x, y and z
/// positive: its pole, (0, 0, 2), is the corner where two of the curved
/// face's three arcs meet.
fn octant(k: f64) -> Body {
    craft3d_testkit::trimmed::sphere_octant(&Frame::WORLD, 2.0 * k)
        .unwrap()
        .body
}

/// The same octant with its curved face on a rational patch whose border
/// at the pole is one point.
fn twin(k: f64) -> Body {
    let (twin, replaced) = rationalized(&octant(k), 0.0, false);
    assert_eq!(replaced, 1);
    twin
}

/// A block on a frame through `origin` (in units of the scale): `dx` by
/// `dy` by `dz` from there along the frame's axes.
fn block_on(k: f64, origin: Vec3, z: Vec3, x: Vec3, size: [f64; 3]) -> Body {
    let frame = Frame::new(origin * k, z, x).unwrap();
    primitives::block(&frame, size[0] * k, size[1] * k, size[2] * k).unwrap()
}

const POLE: Vec3 = Vec3::new(0.0, 0.0, 2.0);

/// Tools that cut the curved face of the octant.
fn tools(k: f64) -> Vec<(&'static str, Body)> {
    // A frame that leans: for blocks with the pole on a face or an edge.
    let down = Vec3::new(0.6, 0.5, -0.62).normalized().unwrap();
    let across = Vec3::new(1.0, -1.0, 0.0);
    let leaning = Frame::new(Vec3::ZERO, down, across).unwrap();
    let (lx, ly) = (leaning.x(), leaning.y());
    let edge = Vec3::new(0.2, 0.2, -1.0).normalized().unwrap();
    vec![
        // Through the face, away from the pole.
        ("away: block", boxed(k, [0.8, 0.7, 0.9], [2.5, 2.5, 2.5])),
        ("away: rod", log(k, [0.5, 1.1, 0.9], 0.45, 3.0)),
        ("away: ball", ball(k, [1.3, 1.2, 1.1], 0.6)),
        ("away: spike", spike(k, [1.1, 1.2, 0.6], 0.6, 1.3)),
        ("away: ring", ring(k, [1.2, 1.1, 1.0], 0.7, 0.25)),
        // The pole inside the tool: the cut goes around it.
        (
            "around: block",
            boxed(k, [-0.5, -0.4, 1.5], [0.7, 0.6, 2.6]),
        ),
        ("around: rod", log(k, [-1.0, 0.2, 1.8], 0.5, 3.0)),
        ("around: ball", ball(k, [0.2, 0.3, 1.9], 0.6)),
        ("around: spike", spike(k, [0.1, 0.2, 1.2], 0.7, 1.5)),
        ("around: ring", ring(k, [0.6, 0.1, 1.9], 0.7, 0.3)),
        // The pole on a face of the tool: the cut runs through it.
        (
            "through: block face",
            block_on(k, POLE - lx * 0.5 - ly * 0.4, down, across, [1.4, 1.2, 0.9]),
        ),
        // An edge of a block through the pole, from outside the ball
        // into it, with the block's two faces there either side of it.
        (
            "through: block edge",
            block_on(
                k,
                POLE - edge * 0.5,
                Vec3::new(1.0, 0.0, 0.2),
                edge,
                [1.6, 1.1, 1.2],
            ),
        ),
        ("through: rod", log(k, [-1.0, 0.3, 1.6], 0.5, 3.0)),
        ("through: ball", ball(k, [0.3, 0.4, 1.4], 0.61_f64.sqrt())),
        ("through: spike", spike(k, [0.3, 0.0, 1.5], 0.45, 1.5)),
        ("through: ring", ring(k, [0.6, 0.0, 2.0], 0.9, 0.3)),
    ]
}

/// What is still refused of the cuts in this file, by what the name of
/// the case contains, with the reason given: work owed. (Empty since
/// 2026-10-15: the torus through the pole, tilted, at a million, lost the
/// lobe of the figure eight in which it meets the octant's flat face back
/// into that face, "imprint lost"; see docs/design/booleans.md.)
const REFUSED: &[(&str, &str)] = &[];

/// The three booleans of the twin and a tool, held to the same booleans
/// of the analytic octant.
fn agree(tool: &Body, k: f64, tilted: bool, what: &str) -> (f64, f64) {
    let (mut twin, mut plain, mut tool) = (twin(k), octant(k), tool.clone());
    if tilted {
        twin = twin.transformed(&tilt(k)).unwrap();
        plain = plain.transformed(&tilt(k)).unwrap();
        tool = tool.transformed(&tilt(k)).unwrap();
    }
    let mut worst = (0.0_f64, 0.0_f64);
    for op in [Union, Intersect, Subtract] {
        let what = format!("{what}, {op:?}");
        let expected = solve(&plain, &tool, op, k);
        let started = std::time::Instant::now();
        let refused = REFUSED
            .iter()
            .find(|(needle, _)| what.contains(needle))
            .map(|(_, why)| why);
        let result = match (boolean(&twin, &tool, op, &tolerance(k)), refused) {
            (Ok(result), None) => result,
            (Err(KernelError::Unsupported { reason, .. }), Some(why)) if reason.contains(why) => {
                continue;
            }
            (Ok(_), Some(_)) => panic!("{what}: taken now; remove it from REFUSED"),
            (Err(e), _) => panic!("{what}: {e}"),
        };
        let took = started.elapsed();
        assert_eq!(check(&result.body, &tolerance(k)), [], "{what}");
        let (want, got) = (mass(&expected.body), mass(&result.body));
        let off = (
            (got.volume - want.volume).abs() / want.volume,
            (got.area - want.area).abs() / want.area,
        );
        if std::env::var_os("POLE_TIMINGS").is_some() {
            eprintln!(
                "{what}: {took:.3?}, volume off {:.1e}, area off {:.1e}",
                off.0, off.1
            );
        }
        worst = (worst.0.max(off.0), worst.1.max(off.1));
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
            assert!(freeform_faces(&result.body) > 0, "{what}: no freeform face");
        }
        // (The tessellator declines some results whatever surface the
        // octant's face is on: a torus face with two loops around its
        // tube. Where it meshes the analytic result it must mesh this
        // one.)
        if mesh_of(&expected.body, k).is_ok() {
            meshes(&result.body, k, &what);
        }
    }
    worst
}

fn octant_against(kind: &str) {
    let mut worst = (0.0_f64, 0.0_f64);
    for k in SCALES {
        for (name, tool) in tools(k) {
            if !name.starts_with(kind) {
                continue;
            }
            for tilted in [false, true] {
                let what = format!("octant, {name} at {k}, tilted {tilted}");
                let off = agree(&tool, k, tilted, &what);
                worst = (worst.0.max(off.0), worst.1.max(off.1));
            }
        }
    }
    println!(
        "worst against the analytic twin: volume {:e}, area {:e}",
        worst.0, worst.1
    );
}

#[test]
fn the_twin_is_the_octant() {
    for k in SCALES {
        let (twin, plain) = (twin(k), octant(k));
        assert_eq!(check(&twin, &tolerance(k)), []);
        let (t, p) = (mass(&twin), mass(&plain));
        near(t.volume, p.volume, 1e-9, "volume");
        near(t.area, p.area, 1e-9, "area");
        assert!(t.centroid.unwrap().distance(p.centroid.unwrap()) <= 1e-9 * k);
        meshes(&twin, k, "the twin");
    }
}

#[test]
fn a_patch_with_a_pole_is_cut_away_from_the_pole() {
    octant_against("away");
}

#[test]
fn a_patch_with_a_pole_is_cut_around_the_pole() {
    octant_against("around");
}

#[test]
fn a_patch_with_a_pole_is_cut_through_the_pole() {
    octant_against("through");
}

/// A plane through the pole that holds the tangent of the patch's border
/// there (the face of the block whose edge goes through the pole) meets
/// the patch in an arc of a circle that leaves the pole along that
/// border, within `x = y² / 10R` of it (`x` across the border's plane,
/// `y` from the pole along it, `R` the ball's radius): within the
/// tolerance of the border over thousands of tolerances. Where along that
/// stretch the march stops on the border is rounding, and a seed further
/// along it once started a second trace of the same arc, which the
/// boolean took for two curves (`regions::untie`: "two curves leave a
/// point in the same direction", the octant cut through its pole by the
/// block's edge at 1e-3 when each pair of faces was intersected in the
/// box of the two faces). In 40 boxes about the pole, upright and tilted,
/// at three scales: one curve, from the pole or from where it is still
/// within the tolerance of the border, on the circle the plane cuts from
/// the ball (closed form) to the tolerance.
#[test]
fn a_plane_along_the_border_at_the_pole_meets_the_patch_once() {
    use craft3d_geom::Plane;
    use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
    use craft3d_math::Bbox3;
    for k in SCALES {
        let tol = tolerance(k);
        for tilted in [false, true] {
            let place = if tilted { tilt(k) } else { Transform::IDENTITY };
            let patch = twin(k).transformed(&place).unwrap();
            let patch = &patch
                .faces
                .iter()
                .find(|f| matches!(f.surface, Surface::Nurbs(_)))
                .unwrap()
                .surface;
            let pole = place.point(POLE * k);
            let centre = place.point(Vec3::ZERO);
            let normal = place.vector(Vec3::new(1.0, 0.0, 0.2)).normalized().unwrap();
            let plane = Surface::Plane(Plane::new(Frame::from_z(pole, normal).unwrap()));
            // The normal of the plane of the patch's border x = 0.
            let border = place.vector(Vec3::X);
            // The circle the plane cuts from the ball.
            let lift = normal.dot(pole - centre);
            let middle = centre + normal * lift;
            let radius = ((2.0 * k).powi(2) - lift * lift).sqrt();
            let mut rng = Rng(0x0b0e_5eed ^ k.to_bits() ^ u64::from(tilted));
            for _ in 0..40 {
                let mut reach = || {
                    let r = rng.between(0.02, 2.5) * k;
                    Vec3::new(
                        r * rng.between(0.2, 1.0),
                        r * rng.between(0.2, 1.0),
                        r * rng.between(0.2, 1.0),
                    )
                };
                let (below, above) = (reach(), reach());
                let within = Bbox3::from_points([pole - below, pole + above]);
                let what = format!("at {k}, tilted {tilted}, in {within:?}");
                let met = meet_surfaces(patch, &plane, &within, &tol)
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
                let SurfaceMeeting::Meets(meeting) = met else {
                    panic!("{what}: no curve");
                };
                assert_eq!(meeting.curves.len(), 1, "{what}: the arc more than once");
                let curve = &meeting.curves[0];
                let (t0, t1) = curve.range;
                // It ends at the pole, or on the border where the arc is
                // within the tolerance of it: `y² / 10R` of the plane
                // x = 0 for y up to √(10 R tol), R = 2k.
                let ends = [curve.curve.eval(t0), curve.curve.eval(t1)];
                let stretch = (20.0 * k * tol.linear).sqrt();
                assert!(
                    ends.iter().any(|&end| {
                        end.distance(pole) <= stretch && border.dot(end - pole).abs() <= tol.linear
                    }),
                    "{what}: the arc does not leave the pole: {ends:?}"
                );
                for i in 0..=400 {
                    let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(i) / 400.0);
                    let across = normal.dot(p - middle);
                    let around = ((p - middle) - normal * across).length() - radius;
                    assert!(
                        across.hypot(around) <= tol.linear,
                        "{what}: {:e} off the circle",
                        across.hypot(around)
                    );
                }
            }
        }
    }
}

// ------------------------------------------------- the dome and the drop

type P = [f64; 2];

/// A dome from the axis at (0, 0) out and back to the axis at (0, 6): it
/// leaves the axis square to it at both ends, so the solid it sweeps is
/// smooth at both poles.
const DOME: [P; 5] = [[0.0, 0.0], [4.0, 0.0], [6.0, 3.0], [3.0, 6.0], [0.0, 6.0]];
/// A drop: from the axis at (0, 0), where it leaves at a slant (the solid
/// comes to a point there), to a foot at (3, 5).
const DROP: [P; 4] = [[0.0, 0.0], [2.0, 1.0], [5.0, 2.0], [3.0, 5.0]];

fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

/// De Boor's algorithm: the point of a spline at knot value `u`.
fn de_boor(degree: usize, knots: &[f64], control: &[P], u: f64) -> P {
    let last = control.len() - 1;
    let mut span = degree;
    while span < last && u >= knots[span + 1] {
        span += 1;
    }
    let mut d: Vec<P> = (0..=degree).map(|j| control[span - degree + j]).collect();
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = span - degree + j;
            let (lo, hi) = (knots[i], knots[i + degree + 1 - r]);
            let a = if hi > lo { (u - lo) / (hi - lo) } else { 0.0 };
            d[j] = [
                d[j - 1][0] * (1.0 - a) + d[j][0] * a,
                d[j - 1][1] * (1.0 - a) + d[j][1] * a,
            ];
        }
    }
    d[degree]
}

/// A cubic spline in the half-plane (ρ away from the axis, z along it),
/// with what the oracle integrates along it.
struct Meridian {
    control: Vec<P>,
    knots: Vec<f64>,
}

impl Meridian {
    fn new(points: &[P], k: f64) -> Self {
        let control: Vec<P> = points.iter().map(|p| [p[0] * k, p[1] * k]).collect();
        Meridian {
            knots: knots(3, control.len()),
            control,
        }
    }

    fn at(&self, t: f64) -> P {
        de_boor(3, &self.knots, &self.control, t)
    }

    /// The derivative: the spline of one degree less over the scaled
    /// differences of the control points.
    fn rate(&self, t: f64) -> P {
        let steps: Vec<P> = (0..self.control.len() - 1)
            .map(|i| {
                let span = self.knots[i + 4] - self.knots[i + 1];
                let f = if span > 0.0 { 3.0 / span } else { 0.0 };
                [
                    (self.control[i + 1][0] - self.control[i][0]) * f,
                    (self.control[i + 1][1] - self.control[i][1]) * f,
                ]
            })
            .collect();
        let inner = &self.knots[1..self.knots.len() - 1];
        de_boor(2, inner, &steps, t)
    }

    /// `∫ f(point, rate) dt` from `from` to `to`: five Gauss points on 64
    /// stretches of every knot span.
    fn integral(&self, from: f64, to: f64, f: &dyn Fn(P, P) -> f64) -> f64 {
        const NODES: [(f64, f64); 5] = [
            (0.0, 0.568_888_888_888_888_9),
            (-0.538_469_310_105_683_1, 0.478_628_670_499_366_47),
            (0.538_469_310_105_683_1, 0.478_628_670_499_366_47),
            (-0.906_179_845_938_664, 0.236_926_885_056_189_08),
            (0.906_179_845_938_664, 0.236_926_885_056_189_08),
        ];
        let mut stops = vec![from];
        stops.extend(self.knots.iter().copied().filter(|&k| k > from && k < to));
        stops.push(to);
        stops.dedup();
        let mut sum = 0.0;
        for pair in stops.windows(2) {
            let width = (pair[1] - pair[0]) / 64.0;
            for i in 0..64 {
                let (a, b) = (pair[0] + width * i as f64, pair[0] + width * (i + 1) as f64);
                for (x, w) in NODES {
                    let t = 0.5 * (a + b) + 0.5 * (b - a) * x;
                    sum += f(self.at(t), self.rate(t)) * w * 0.5 * (b - a);
                }
            }
        }
        sum
    }

    /// The parameter at which the spline is at the height `z` (it rises
    /// all the way).
    fn at_height(&self, z: f64) -> f64 {
        let (mut a, mut b) = (0.0, 1.0);
        for _ in 0..200 {
            let m = 0.5 * (a + b);
            if self.at(m)[1] < z {
                a = m;
            } else {
                b = m;
            }
        }
        0.5 * (a + b)
    }

    /// The volume and the curved area that the stretch `from` to `to`
    /// sweeps in a whole turn about the axis: `π ∫ ρ² dz` and
    /// `2π ∫ ρ ds`.
    fn swept(&self, from: f64, to: f64) -> (f64, f64) {
        (
            PI * self.integral(from, to, &|p, d| p[0] * p[0] * d[1]),
            TAU * self.integral(from, to, &|p, d| p[0] * d[0].hypot(d[1])),
        )
    }

    /// The area between the stretch and the axis: `∫ ρ dz`.
    fn section(&self, from: f64, to: f64) -> f64 {
        self.integral(from, to, &|p, d| p[0] * d[1])
    }
}

/// The solid a spline sweeps in a whole turn about the y axis of the
/// world, with the lines that close its profile: the dome between two
/// points of the axis, the drop from a point of the axis to a flat top.
fn swept_solid(points: &[P], k: f64) -> Body {
    let tol = tolerance(k);
    let scaled: Vec<P> = points.iter().map(|p| [p[0] * k, p[1] * k]).collect();
    let (first, last) = (scaled[0], scaled[scaled.len() - 1]);
    let mut curves = vec![SketchCurve {
        id: 3,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: knots(3, scaled.len()),
            control: scaled,
        }),
    }];
    let mut id = 5;
    let mut line = |start: P, end: P| {
        curves.push(SketchCurve {
            id,
            curve: Curve2::Line { start, end },
        });
        id += 2;
    };
    if last[0] != 0.0 {
        line(last, [0.0, last[1]]);
    }
    line([0.0, last[1]], first);
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let body = revolve(
        &profiles,
        &Frame::WORLD,
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        TAU,
        &tol,
    )
    .unwrap()
    .body;
    assert_eq!(check(&body, &tol), []);
    body
}

/// The three booleans of a solid with a pole and a tool, held to the
/// identities of sets: what the two have in common and what is left of
/// the solid make the solid; the union is both less what they have in
/// common, in volume and (the faces crossing) in area. Gives the three
/// results: union, intersection, difference.
fn identities(solid: &Body, tool: &Body, k: f64, what: &str) -> [Body; 3] {
    let run = |op| {
        let started = std::time::Instant::now();
        let result = match boolean(solid, tool, op, &tolerance(k)) {
            Ok(result) => result,
            Err(e) => panic!("{what}, {op:?}: {e}"),
        };
        if std::env::var_os("POLE_TIMINGS").is_some() {
            eprintln!("{what}, {op:?}: {:.3?}", started.elapsed());
        }
        assert_eq!(check(&result.body, &tolerance(k)), [], "{what}, {op:?}");
        result.body
    };
    let (both, common, rest) = (run(Union), run(Intersect), run(Subtract));
    let (a, b) = (mass(solid), mass(tool));
    let (u, i, d) = (mass(&both), mass(&common), mass(&rest));
    near(
        i.volume + d.volume,
        a.volume,
        1e-6,
        &format!("{what}: parts"),
    );
    near(
        u.volume + i.volume,
        a.volume + b.volume,
        1e-6,
        &format!("{what}: union"),
    );
    near(
        u.area + i.area,
        a.area + b.area,
        1e-6,
        &format!("{what}: area"),
    );
    assert!(i.volume > 1e-4 * a.volume, "{what}: nothing in common");
    assert!(d.volume > 1e-4 * a.volume, "{what}: nothing left");
    for (body, name) in [(&both, "union"), (&common, "common"), (&rest, "rest")] {
        let what = format!("{what}, {name}");
        meshes(body, k, &what);
    }
    [both, common, rest]
}

/// A block that is everything on one side of the plane through `origin`
/// with the normal `into` (as far as the solids here reach).
fn half_space(k: f64, origin: Vec3, into: Vec3, hint: Vec3) -> Body {
    let frame = Frame::new(origin * k, into, hint).unwrap();
    let corner = frame.point(-20.0 * k, -20.0 * k, 0.0);
    primitives::block(&frame.at(corner).unwrap(), 40.0 * k, 40.0 * k, 20.0 * k).unwrap()
}

/// Tools in general position against the dome (which turns about the y
/// axis from y = 0 to y = 6, at most 4.6 from it): through its side away
/// from the poles, around a pole, and with a pole on a face, on an edge
/// or on the surface of the tool.
fn dome_tools(k: f64) -> Vec<(&'static str, Body)> {
    let top = Vec3::new(0.0, 6.0, 0.0);
    let lean = Vec3::new(0.5, -0.7, 0.3).normalized().unwrap();
    let edge = Vec3::new(0.3, -1.0, 0.2).normalized().unwrap();
    vec![
        ("away: block", boxed(k, [2.0, 1.5, -1.0], [8.0, 3.5, 1.0])),
        ("away: rod", log(k, [1.0, 2.5, 0.7], 0.8, 7.0)),
        ("away: ball", ball(k, [3.5, 3.6, 1.2], 1.6)),
        ("away: spike", spike(k, [3.0, 1.0, 1.5], 1.2, 5.0)),
        ("away: ring", ring(k, [3.9, 3.0, 0.4], 1.5, 0.5)),
        ("around: ball", ball(k, [0.3, 5.8, 0.2], 1.4)),
        (
            "around: block",
            boxed(k, [-1.1, -0.7, -0.9], [1.3, 0.8, 1.2]),
        ),
        (
            "through: block face",
            block_on(
                k,
                top - Vec3::X * 0.7 - Vec3::Z * 0.6,
                lean,
                Vec3::X,
                [2.0, 2.0, 1.5],
            ),
        ),
        (
            "through: block edge",
            block_on(
                k,
                top - edge * 0.8,
                Vec3::new(1.0, 0.3, 0.1),
                edge,
                [2.5, 1.4, 1.3],
            ),
        ),
        ("through: ball", ball(k, [0.6, 5.2, 0.0], 1.0)),
        // A rod whose side passes through the pole, across the dome's own
        // edges there (they leave the pole along x).
        ("through: rod", {
            let out = Vec3::new(0.6, -0.7, 0.2).normalized().unwrap();
            let along = out.cross(Vec3::Z).normalized().unwrap();
            let base = (top + out - along * 4.0) * k;
            let frame = Frame::new(base, along, out).unwrap();
            primitives::cylinder(&frame, k, 8.0 * k).unwrap()
        }),
    ]
}

fn dome_against(kind: &str) {
    for k in SCALES {
        let dome = swept_solid(&DOME, k);
        for (name, tool) in dome_tools(k) {
            if name.starts_with(kind) {
                identities(&dome, &tool, k, &format!("dome, {name} at {k}"));
                // And nowhere along an axis, at one scale.
                if k == 1.0 {
                    let (dome, tool) = (
                        dome.transformed(&tilt(k)).unwrap(),
                        tool.transformed(&tilt(k)).unwrap(),
                    );
                    identities(&dome, &tool, k, &format!("tilted dome: {name} at {k}"));
                }
            }
        }
    }
}

#[test]
fn a_dome_is_cut_away_from_its_poles() {
    dome_against("away");
}

#[test]
fn a_dome_is_cut_around_a_pole() {
    dome_against("around");
}

#[test]
fn a_dome_is_cut_through_a_pole() {
    dome_against("through");
}

/// Cuts with a closed form. A plane across the axis leaves the part of
/// the dome above it: what the upper stretch of the spline sweeps, and a
/// disc. A plane through the axis (through both poles, and not the plane
/// the dome's own edges lie in) leaves half the dome and the whole
/// section either side of the axis.
#[test]
fn a_dome_cut_by_a_plane_is_what_its_profile_says() {
    for k in SCALES {
        let dome = swept_solid(&DOME, k);
        let meridian = Meridian::new(&DOME, k);
        let whole = meridian.swept(0.0, 1.0);
        let m = mass(&dome);
        near(m.volume, whole.0, 1e-9, "the dome's volume");
        near(m.area, whole.1, 1e-9, "the dome's area");
        // Across the axis at the height 2.3.
        let above = half_space(k, Vec3::new(0.0, 2.3, 0.0), Vec3::Y, Vec3::X);
        let [_, common, rest] = identities(&dome, &above, k, &format!("dome above 2.3 at {k}"));
        let t = meridian.at_height(2.3 * k);
        let (volume, curved) = meridian.swept(t, 1.0);
        let disc = PI * meridian.at(t)[0].powi(2);
        near(mass(&common).volume, volume, 1e-7, "above: volume");
        near(mass(&common).area, curved + disc, 1e-7, "above: area");
        near(mass(&rest).volume, whole.0 - volume, 1e-7, "below: volume");
        near(
            mass(&rest).area,
            whole.1 - curved + disc,
            1e-7,
            "below: area",
        );
        // Through the axis, a third of a turn from the dome's own edges.
        let normal = Vec3::new(0.5, 0.0, 0.75_f64.sqrt());
        let beside = half_space(k, Vec3::ZERO, normal, Vec3::Y);
        let [_, common, rest] = identities(&dome, &beside, k, &format!("half a dome at {k}"));
        let section = 2.0 * meridian.section(0.0, 1.0);
        for half in [&common, &rest] {
            near(mass(half).volume, whole.0 / 2.0, 1e-7, "half: volume");
            near(mass(half).area, whole.1 / 2.0 + section, 1e-7, "half: area");
        }
    }
}

/// Tools against the drop (about the y axis from its point at the origin
/// to its flat top at y = 5, at most 4 from the axis).
fn drop_tools(k: f64) -> Vec<(&'static str, Body)> {
    vec![
        ("away: block", boxed(k, [1.5, 2.0, -1.0], [7.0, 3.5, 1.2])),
        ("away: rod", log(k, [0.5, 2.6, 0.6], 0.7, 6.0)),
        ("away: ball", ball(k, [3.0, 2.2, 1.0], 1.3)),
        ("away: spike", spike(k, [2.6, 1.0, 1.2], 1.0, 4.0)),
        ("away: ring", ring(k, [3.2, 2.4, 0.3], 1.3, 0.4)),
        ("around: ball", ball(k, [0.2, 0.3, 0.1], 1.1)),
        (
            "around: block",
            boxed(k, [-0.9, -0.8, -1.0], [1.1, 0.7, 0.8]),
        ),
    ]
}

fn drop_against(kind: &str) {
    for k in SCALES {
        let drop = swept_solid(&DROP, k);
        for (name, tool) in drop_tools(k) {
            if name.starts_with(kind) {
                identities(&drop, &tool, k, &format!("drop, {name} at {k}"));
            }
        }
    }
}

#[test]
fn a_drop_is_cut_away_from_its_point() {
    drop_against("away");
}

#[test]
fn a_drop_is_cut_around_its_point() {
    drop_against("around");
}

/// The drop cut by planes: across the axis (what is left has the point),
/// and through the axis, which goes through the point.
#[test]
fn a_drop_cut_by_a_plane_is_what_its_profile_says() {
    for k in SCALES {
        let drop = swept_solid(&DROP, k);
        let meridian = Meridian::new(&DROP, k);
        let whole = meridian.swept(0.0, 1.0);
        let lid = PI * (3.0 * k).powi(2);
        let m = mass(&drop);
        near(m.volume, whole.0, 1e-9, "the drop's volume");
        near(m.area, whole.1 + lid, 1e-9, "the drop's area");
        let above = half_space(k, Vec3::new(0.0, 1.4, 0.0), Vec3::Y, Vec3::X);
        let [_, common, rest] = identities(&drop, &above, k, &format!("drop above 1.4 at {k}"));
        let t = meridian.at_height(1.4 * k);
        let (volume, curved) = meridian.swept(t, 1.0);
        let disc = PI * meridian.at(t)[0].powi(2);
        near(mass(&common).volume, volume, 1e-7, "above: volume");
        near(mass(&common).area, curved + disc + lid, 1e-7, "above: area");
        near(mass(&rest).volume, whole.0 - volume, 1e-7, "below: volume");
        near(
            mass(&rest).area,
            whole.1 - curved + disc,
            1e-7,
            "below: area",
        );
        let normal = Vec3::new(0.5, 0.0, 0.75_f64.sqrt());
        let beside = half_space(k, Vec3::ZERO, normal, Vec3::Y);
        let [_, common, rest] = identities(&drop, &beside, k, &format!("half a drop at {k}"));
        let section = 2.0 * meridian.section(0.0, 1.0);
        for half in [&common, &rest] {
            near(mass(half).volume, whole.0 / 2.0, 1e-7, "half: volume");
            near(
                mass(half).area,
                (whole.1 + lid) / 2.0 + section,
                1e-7,
                "half: area",
            );
        }
    }
}

/// A cusp: a spline that leaves the axis along it sweeps a point the
/// surface comes to as a needle, its width growing as the square of the
/// distance from the point (no fan of meridians). Refused by name until
/// 2026-10-15 ("a cusp"); now its chart is a wedge (or a disc) whose width
/// vanishes to the same order, and the pieces that leave the point, all
/// along the axis, are put in order a little further along
/// (docs/design/booleans.md › *Freeform faces with a pole*). Held to the
/// closed forms from the profile where the tool cuts across the axis
/// (`π ∫ ρ² dz` and `2π ∫ ρ ds` up to the cut), to the identities of
/// sets for every tool (around the point, a leaning block around it, a
/// ball, and away from it), to the exact membership of 400 seeded points
/// (the solid is the points within `ρ(y)` of the axis), and every result
/// is meshed. (A face that holds the axis through the point was refused
/// here until 2026-10-17: see
/// `a_cusp_cut_along_its_axis_is_what_its_profile_says`.)
#[test]
fn a_cusp_is_an_operand() {
    let mut rng = Rng(20_261_016);
    for k in SCALES {
        let cusp = swept_solid(&CUSP, k);
        let meridian = Meridian::new(&CUSP, k);
        let (volume, curved) = meridian.swept(0.0, 1.0);
        let lid = PI * (3.0 * k).powi(2);
        let m = mass(&cusp);
        near(m.volume, volume, 1e-9, "the cusp's volume");
        near(m.area, curved + lid, 1e-9, "the cusp's area");
        let radius = |y: f64| meridian.at(meridian.at_height(y))[0];
        let inside = |p: Vec3| p.y > 0.0 && p.y < 6.0 * k && p.x.hypot(p.z) < radius(p.y);
        let lean = Vec3::new(0.5, -0.7, 0.3).normalized().unwrap();
        let tools = [
            (
                "around: block",
                boxed(k, [-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]),
            ),
            ("away: slab", boxed(k, [1.0, 3.0, -1.0], [7.0, 4.0, 1.0])),
            ("around: ball", ball(k, [0.2, 0.3, -0.1], 0.9)),
            (
                "across",
                half_space(k, Vec3::new(0.0, 2.0, 0.0), -Vec3::Y, Vec3::X),
            ),
            (
                "around: leaning block",
                block_on(
                    k,
                    -Vec3::X * 0.7 - Vec3::Z * 0.6,
                    lean,
                    Vec3::X,
                    [2.0, 2.0, 1.5],
                ),
            ),
        ];
        for (name, tool) in &tools {
            let what = format!("cusp, {name}, at {k}");
            let [both, common, rest] = identities(&cusp, tool, k, &what);
            match *name {
                "around: block" | "across" => {
                    let top = if *name == "across" { 2.0 } else { 1.0 };
                    let (v, a) = meridian.swept(0.0, meridian.at_height(top * k));
                    let cap = PI * radius(top * k).powi(2);
                    let c = mass(&common);
                    near(c.volume, v, 1e-7, &format!("{what}: common volume"));
                    near(c.area, a + cap, 1e-7, &format!("{what}: common area"));
                    let r = mass(&rest);
                    near(r.volume, volume - v, 1e-7, &format!("{what}: rest volume"));
                    near(
                        r.area,
                        curved + lid - a + cap,
                        1e-7,
                        &format!("{what}: rest area"),
                    );
                }
                _ => {}
            }
            // Membership, by ray casting, against the solids' own
            // definitions.
            let points: Vec<Vec3> = (0..400)
                .map(|_| {
                    Vec3::new(
                        rng.between(-3.5, 3.5),
                        rng.between(-1.0, 7.0),
                        rng.between(-3.5, 3.5),
                    ) * k
                })
                .collect();
            let tool_has = contains_points(tool, &points, &tolerance(k)).unwrap();
            for (body, op) in [(&both, "union"), (&common, "common"), (&rest, "rest")] {
                let got = contains_points(body, &points, &tolerance(k)).unwrap();
                for ((p, got), in_tool) in points.iter().zip(&got).zip(&tool_has) {
                    let (Some(got), Some(in_tool)) = (got, in_tool) else {
                        continue;
                    };
                    let a = inside(*p);
                    let want = match op {
                        "union" => a || *in_tool,
                        "common" => a && *in_tool,
                        _ => a && !*in_tool,
                    };
                    assert_eq!(*got, want, "{what}, {op}, at {p:?}");
                }
            }
        }
    }
}

/// The needle of [`a_cusp_is_an_operand`]: a spline that leaves the axis
/// along it, `ρ = 9 t² − 6 t³` for `y = 6 t` near the point.
const CUSP: [P; 4] = [[0.0, 0.0], [0.0, 2.0], [3.0, 4.0], [3.0, 6.0]];

/// The exact membership of 400 seeded points in the three results of the
/// needle and a tool, by ray casting, against the solids' own
/// definitions (the needle is the points within `ρ(y)` of the axis).
fn cusp_membership(
    results: &[Body; 3],
    k: f64,
    in_tool: &dyn Fn(Vec3) -> bool,
    rng: &mut Rng,
    what: &str,
) {
    let meridian = Meridian::new(&CUSP, k);
    let radius = |y: f64| meridian.at(meridian.at_height(y))[0];
    let inside = |p: Vec3| p.y > 0.0 && p.y < 6.0 * k && p.x.hypot(p.z) < radius(p.y);
    let points: Vec<Vec3> = (0..400)
        .map(|_| {
            Vec3::new(
                rng.between(-3.5, 3.5),
                rng.between(-1.0, 7.0),
                rng.between(-3.5, 3.5),
            ) * k
        })
        .collect();
    let tol = tolerance(k);
    for (body, kind) in results.iter().zip(["union", "common", "rest"]) {
        let got = contains_points(body, &points, &tol).unwrap();
        for (p, got) in points.iter().zip(&got) {
            let Some(got) = got else { continue };
            let (a, b) = (inside(*p), in_tool(*p));
            let want = match kind {
                "union" => a || b,
                "common" => a && b,
                _ => a && !b,
            };
            assert_eq!(*got, want, "{what}, {kind}, at {p:?}");
        }
    }
}

/// A cusp cut by faces that hold its axis through the point: planes
/// through the axis (along the revolve's seam, a third of a turn from it,
/// and leaning) and a cylinder that has the axis on its side. Refused
/// until 2026-10-17 ("touch along a curve": on the needle's chart the
/// distance to such a face vanishes to the second order all along the
/// border the point is). Held to closed forms from the profile: a plane
/// through the axis leaves half the needle and the section either side
/// (`2 ∫ ρ dz`), and the cylinder of radius `R` through the axis the lens
/// of two discs integrated up the profile (`∫ A(ρ) dz`, `A` the area
/// common to the discs of radius `ρ` and `R` with their centres `R`
/// apart); to the identities of sets, the exact membership of 400 seeded
/// points and a valid result, at three scales. (The meshes of these
/// results are the tessellator's to make: [`UNMESHED`].)
#[test]
fn a_cusp_cut_along_its_axis_is_what_its_profile_says() {
    let mut rng = Rng(20_261_017);
    for k in SCALES {
        let cusp = swept_solid(&CUSP, k);
        let meridian = Meridian::new(&CUSP, k);
        let (volume, curved) = meridian.swept(0.0, 1.0);
        let lid = PI * (3.0 * k).powi(2);
        let section = meridian.section(0.0, 1.0);
        for (name, normal) in [
            ("along the seam", -Vec3::X),
            (
                "a third of a turn off",
                Vec3::new(0.5, 0.0, 0.75_f64.sqrt()),
            ),
            ("leaning", Vec3::new(-0.8, 0.0, -0.6)),
        ] {
            let what = format!("cusp, a plane {name}, at {k}");
            let tool = half_space(k, Vec3::ZERO, normal, Vec3::Y);
            let results = identities(&cusp, &tool, k, &what);
            for (half, part) in [(&results[1], "common"), (&results[2], "rest")] {
                let m = mass(half);
                near(
                    m.volume,
                    volume / 2.0,
                    1e-7,
                    &format!("{what}, {part}: volume"),
                );
                near(
                    m.area,
                    (curved + lid) / 2.0 + 2.0 * section,
                    1e-7,
                    &format!("{what}, {part}: area"),
                );
            }
            cusp_membership(&results, k, &|p| p.dot(normal) > 0.0, &mut rng, &what);
        }
        // A cylinder of radius 2 along the axis with the axis on its side.
        let big = 2.0 * k;
        let what = format!("cusp, a cylinder through the axis, at {k}");
        let frame = Frame::new(Vec3::new(big, -k, 0.0), Vec3::Y, Vec3::X).unwrap();
        let tool = primitives::cylinder(&frame, big, 8.0 * k).unwrap();
        let results = identities(&cusp, &tool, k, &what);
        let lens = |r: f64| {
            if r <= 0.0 {
                return 0.0;
            }
            let d = big;
            let a = (r / (2.0 * big)).clamp(-1.0, 1.0).acos();
            let b = (1.0 - r * r / (2.0 * big * big)).clamp(-1.0, 1.0).acos();
            let k4 = (-d + r + big) * (d + r - big) * (d - r + big) * (d + r + big);
            r * r * a + big * big * b - 0.5 * k4.max(0.0).sqrt()
        };
        let shared = meridian.integral(0.0, 1.0, &|p, d| lens(p[0]) * d[1]);
        near(
            mass(&results[1]).volume,
            shared,
            1e-7,
            &format!("{what}: common volume"),
        );
        near(
            mass(&results[2]).volume,
            volume - shared,
            1e-7,
            &format!("{what}: rest volume"),
        );
        let in_tool = |p: Vec3| (p.x - big).hypot(p.z) < big && p.y > -k && p.y < 7.0 * k;
        cusp_membership(&results, k, &in_tool, &mut rng, &what);
    }
}

/// The needle quartered by a block whose edge is its axis (two faces
/// through the point that hold the axis, and an edge along it), held to
/// a quarter of the volume and of the curved area and lid, and the two
/// sections, at scale 1 (square to the revolve's seams). Slow: the
/// block's edge along the axis is within the tolerance of the needle
/// for 2e-3 from the point, where every point of the axis is as near to
/// a whole circle of the needle and the nearest-point search descends
/// from every piece of it (about 5000 searches at 3 ms; 20 to 30 s per
/// boolean in release). The same at 1e-3 and 1e6 and turned about the
/// axis: [`a_cusp_quartered_along_its_axis_at_every_scale`].
#[test]
fn a_cusp_quartered_along_its_axis_is_what_its_profile_says() {
    cusp_quartered(1.0, 0.0);
}

/// [`a_cusp_quartered_along_its_axis_is_what_its_profile_says`] at all
/// three scales and turned: green on 2026-10-17 (release, 5.5 min).
#[test]
#[ignore = "slow: nearest points on the axis of a cusp (about 3 ms each)"]
fn a_cusp_quartered_along_its_axis_at_every_scale() {
    for k in SCALES {
        for turn in [0.0, 0.4] {
            cusp_quartered(k, turn);
        }
    }
}

fn cusp_quartered(k: f64, turn: f64) {
    let cusp = swept_solid(&CUSP, k);
    let meridian = Meridian::new(&CUSP, k);
    let (volume, curved) = meridian.swept(0.0, 1.0);
    let lid = PI * (3.0 * k).powi(2);
    let section = meridian.section(0.0, 1.0);
    let what = format!("cusp, a block edge along the axis (turned {turn}), at {k}");
    let (x, z) = (
        Vec3::new(turn.cos(), 0.0, -turn.sin()),
        Vec3::new(turn.sin(), 0.0, turn.cos()),
    );
    let frame = Frame::new(-Vec3::Y * k, z, x).unwrap();
    let tool = primitives::block(&frame, 10.0 * k, 10.0 * k, 8.0 * k).unwrap();
    let results = identities(&cusp, &tool, k, &what);
    let c = mass(&results[1]);
    near(
        c.volume,
        volume / 4.0,
        1e-7,
        &format!("{what}: common volume"),
    );
    near(
        c.area,
        (curved + lid) / 4.0 + 2.0 * section,
        1e-7,
        &format!("{what}: common area"),
    );
    let r = mass(&results[2]);
    near(
        r.volume,
        0.75 * volume,
        1e-7,
        &format!("{what}: rest volume"),
    );
    near(
        r.area,
        0.75 * (curved + lid) + 2.0 * section,
        1e-7,
        &format!("{what}: rest area"),
    );
    let mut rng = Rng(20_261_018);
    cusp_membership(
        &results,
        k,
        &|p| p.dot(x) > 0.0 && p.dot(z) > 0.0,
        &mut rng,
        &what,
    );
}

/// A tool, its name and its signed distance (near enough; positive
/// inside).
type Probe = (String, Body, Box<dyn Fn(Vec3) -> f64>);

/// Surfaces through the cusp's point that do not hold its axis, or hold
/// it only within the strip's width: a ball of radius `c` centred on the
/// axis at `c` (its surface through the point, swallowing the tip up to
/// `2 c`; `c` = 2e-3 and 3e-3), a cone about the axis with its apex on
/// the point, planes through the point a thousandth and a hundredth of a
/// radian off the axis, a cylinder and a ball of radius 1 whose surface
/// passes through the point and crosses the axis again at `y0` = 4e-3
/// (they meet the needle in a closed loop from about 2.7e-3 to 8e-3,
/// wholly inside the strip), and a ball of radius 1 tangent to the axis
/// at 1e-3. Each of the three booleans is right or refused by name, never
/// a body that is wrong: valid, and every one of 400 seeded points, of
/// points along the axis next to the point and of points 1.8 tolerances
/// deep in both solids next to the loop is where it should be by the
/// solids' own definitions (points within half the tolerance of either
/// surface are not asked). (The chart left the strip next to the point
/// out whenever the other surface passed through the point until the
/// review of 2026-10-17, and then wherever the other surface stayed
/// within the strip's width of a meridian: the needle less the ball, and
/// later less the chord's cylinder, came back whole, its tip through the
/// tool.)
#[test]
fn surfaces_through_a_cusp_that_leave_its_axis_are_cut_or_refused() {
    let mut rng = Rng(20_261_019);
    for k in SCALES {
        let cusp = swept_solid(&CUSP, k);
        let tol = tolerance(k);
        // Signed distances, near enough: positive inside.
        let rho = |y: f64| y * y / 4.0 - y * y * y / 36.0;
        let needle = move |p: Vec3| {
            let y = p.y / k;
            (k * rho(y.clamp(0.0, 6.0)) - p.x.hypot(p.z))
                .min(p.y)
                .min(6.0 * k - p.y)
        };
        let mut tools: Vec<Probe> = Vec::new();
        for c in [2e-3, 3e-3] {
            let centre = Vec3::Y * (c * k);
            let ball = primitives::sphere(&Frame::WORLD.at(centre).unwrap(), c * k).unwrap();
            tools.push((
                format!("a ball of {c} through the point"),
                ball,
                Box::new(move |p: Vec3| c * k - p.distance(centre)),
            ));
        }
        let frame = Frame::new(Vec3::Y * (2.0 * k), -Vec3::Y, Vec3::X).unwrap();
        let cone = primitives::cone(&frame, k, 2.0 * k).unwrap();
        tools.push((
            "a cone about the axis, its apex on the point".to_string(),
            cone,
            Box::new(move |p: Vec3| {
                ((0.5 * p.y - p.x.hypot(p.z)) * 0.8_f64.sqrt())
                    .min(p.y)
                    .min(2.0 * k - p.y)
            }),
        ));
        for angle in [1e-3_f64, 1e-2] {
            let normal = Vec3::new(-angle.cos(), angle.sin(), 0.0);
            tools.push((
                format!("a plane through the point {angle} off the axis"),
                half_space(k, Vec3::ZERO, normal, Vec3::Z),
                Box::new(move |p: Vec3| p.dot(normal)),
            ));
        }
        let (big, y0) = (1.0, 4e-3);
        let centre = Vec3::new((big * big - y0 * y0 / 4.0_f64).sqrt(), y0 / 2.0, 0.0) * k;
        let frame = Frame::new(centre - Vec3::Z * (2.0 * k), Vec3::Z, Vec3::X).unwrap();
        tools.push((
            "a chord's cylinder through the point and the axis at 4e-3".to_string(),
            primitives::cylinder(&frame, big * k, 4.0 * k).unwrap(),
            Box::new(move |p: Vec3| {
                (big * k - (p.x - centre.x).hypot(p.y - centre.y))
                    .min(p.z + 2.0 * k)
                    .min(2.0 * k - p.z)
            }),
        ));
        tools.push((
            "a chord's ball through the point and the axis at 4e-3".to_string(),
            primitives::sphere(&Frame::WORLD.at(centre).unwrap(), big * k).unwrap(),
            Box::new(move |p: Vec3| big * k - p.distance(centre)),
        ));
        let touching = Vec3::new(1.0, 1e-3, 0.0) * k;
        tools.push((
            "a ball tangent to the axis at 1e-3".to_string(),
            primitives::sphere(&Frame::WORLD.at(touching).unwrap(), k).unwrap(),
            Box::new(move |p: Vec3| k - p.distance(touching)),
        ));
        for (name, tool, in_tool) in &tools {
            let what = format!("cusp, {name}, at {k}");
            let mut points: Vec<Vec3> = (0..400)
                .map(|_| {
                    Vec3::new(
                        rng.between(-3.5, 3.5),
                        rng.between(-1.0, 7.0),
                        rng.between(-3.5, 3.5),
                    ) * k
                })
                .collect();
            // Next to the point, where the strip is: along the axis, a
            // little off it, and 1.8 tolerances deep next to the loop.
            for y in [
                1e-4, 3e-4, 1e-3, 2e-3, 3e-3, 3.5e-3, 4e-3, 4.5e-3, 5e-3, 6e-3, 8e-3, 1e-2,
            ] {
                points.push(Vec3::new(0.0, y, 0.0) * k);
                let r = rho(y);
                points.push(Vec3::new(0.3 * r, y, -0.2 * r) * k);
                points.push(Vec3::new(-0.6 * r, y, 0.5 * r) * k);
            }
            for (x, y) in [
                (1.92e-6, 3.96e-3),
                (1.55e-6, 3.885e-3),
                (0.88e-6, 3.75e-3),
                (1.1e-6, 4.11e-3),
                (0.12e-6, 3.405e-3),
                (1.0e-6, 3.9e-3),
                (2.0e-6, 4.0e-3),
            ] {
                points.push(Vec3::new(x, y, 0.0) * k);
            }
            let points: Vec<Vec3> = points
                .into_iter()
                .filter(|p| {
                    needle(*p).abs() > 0.5 * tol.linear && in_tool(*p).abs() > 0.5 * tol.linear
                })
                .collect();
            for op in [Union, Intersect, Subtract] {
                let body = match boolean(&cusp, tool, op, &tol) {
                    Ok(result) => result.body,
                    Err(KernelError::Unsupported { reason, .. }) => {
                        if std::env::var_os("POLE_TIMINGS").is_some() {
                            eprintln!("{what}, {op:?}: refused: {reason}");
                        }
                        continue;
                    }
                    Err(e) => panic!("{what}, {op:?}: {e}"),
                };
                assert_eq!(check(&body, &tol), [], "{what}, {op:?}");
                if std::env::var_os("POLE_TIMINGS").is_some() {
                    eprintln!(
                        "{what}, {op:?}: {} faces, volume {}",
                        body.faces.len(),
                        mass(&body).volume
                    );
                }
                let got = contains_points(&body, &points, &tol).unwrap();
                for (p, got) in points.iter().zip(&got) {
                    let Some(got) = got else { continue };
                    let (a, b) = (needle(*p) > 0.0, in_tool(*p) > 0.0);
                    let want = match op {
                        Union => a || b,
                        Intersect => a && b,
                        _ => a && !b,
                    };
                    assert_eq!(*got, want, "{what}, {op:?}, at {p:?}");
                }
            }
        }
    }
}

/// The same cuts of a cone, whose apex is a pole of the first order (the
/// surface leaves it in a fan): the analytic cone and its twin that a
/// straight spline from the axis sweeps, halved by a plane through the
/// axis and quartered by a block whose edge is the axis, held to `π r²
/// h / 3`, `π r s + π r²` and the triangles of the section, at three
/// scales.
#[test]
fn a_cone_cut_along_its_axis_is_what_it_measures() {
    for k in SCALES {
        let (r, h) = (3.0 * k, 6.0 * k);
        let volume = PI * r * r * h / 3.0;
        let area = PI * r * r.hypot(h) + PI * r * r;
        let triangle = 0.5 * r * h;
        // The analytic cone stands on z = 0 with its apex up; the twin
        // stands on its apex at the origin of the y axis.
        let analytic = primitives::cone(&Frame::WORLD, r, h).unwrap();
        let twin = swept_solid(&[[0.0, 0.0], [1.0, 2.0], [2.0, 4.0], [3.0, 6.0]], k);
        for (name, solid, axis, across) in [
            ("analytic", &analytic, Vec3::Z, Vec3::X),
            ("twin", &twin, Vec3::Y, Vec3::X),
        ] {
            let m = mass(solid);
            near(m.volume, volume, 1e-9, &format!("{name} cone: volume"));
            near(m.area, area, 1e-9, &format!("{name} cone: area"));
            let what = format!("{name} cone, a plane through the axis, at {k}");
            let normal = Vec3::new(0.5, 0.0, 0.75_f64.sqrt());
            let normal = if axis == Vec3::Z {
                Vec3::new(normal.x, normal.z, 0.0)
            } else {
                normal
            };
            let tool = half_space(k, Vec3::ZERO, normal, axis);
            let [_, common, rest] = identities(solid, &tool, k, &what);
            for (half, part) in [(&common, "common"), (&rest, "rest")] {
                let m = mass(half);
                near(
                    m.volume,
                    volume / 2.0,
                    1e-7,
                    &format!("{what}, {part}: volume"),
                );
                near(
                    m.area,
                    area / 2.0 + 2.0 * triangle,
                    1e-7,
                    &format!("{what}, {part}: area"),
                );
            }
            let what = format!("{name} cone, a block edge along the axis, at {k}");
            let side = across.cross(axis);
            let frame = Frame::new(-axis * k, side, across).unwrap();
            let tool = primitives::block(&frame, 10.0 * k, 10.0 * k, 8.0 * k).unwrap();
            let [_, common, rest] = identities(solid, &tool, k, &what);
            let c = mass(&common);
            near(
                c.volume,
                volume / 4.0,
                1e-7,
                &format!("{what}: common volume"),
            );
            near(
                c.area,
                area / 4.0 + 2.0 * triangle,
                1e-7,
                &format!("{what}: common area"),
            );
            let d = mass(&rest);
            near(
                d.volume,
                0.75 * volume,
                1e-7,
                &format!("{what}: rest volume"),
            );
            near(
                d.area,
                0.75 * area + 2.0 * triangle,
                1e-7,
                &format!("{what}: rest area"),
            );
        }
    }
}

/// The reproducer for `crates/tess`: valid bodies with a freeform face
/// through a pole that the tessellator declines or meshes with a gap
/// (see [`UNMESHED`]). Each passes `check` and measures as its oracle
/// says. (Ignored until the tessellator took them, 2026-10-14.)
#[test]
fn the_tessellator_meshes_cuts_through_a_pole() {
    let k = 1.0;
    let tol = tolerance(k);
    // An octant's patch and a ring around its pole: the common part is
    // not watertight.
    let ring_around = ring(k, [0.6, 0.1, 1.9], 0.7, 0.3);
    let common = boolean(&twin(k), &ring_around, Intersect, &tol)
        .unwrap()
        .body;
    assert_eq!(check(&common, &tol), []);
    assert_eq!(
        try_mesh(&common, k),
        Ok(()),
        "octant and a ring around its pole"
    );
    // A dome and a block with an edge through its pole: what they have
    // in common needs more segments than the limit, and the rest is not
    // watertight.
    let dome = swept_solid(&DOME, k);
    let (_, block) = dome_tools(k)
        .into_iter()
        .find(|(name, _)| *name == "through: block edge")
        .unwrap();
    for op in [Intersect, Subtract] {
        let body = boolean(&dome, &block, op, &tol).unwrap().body;
        assert_eq!(check(&body, &tol), []);
        assert_eq!(try_mesh(&body, k), Ok(()), "dome and a block edge, {op:?}");
    }
}
