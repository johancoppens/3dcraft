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

/// (Empty since 2026-10-14: the tessellator meshes all eight that were
/// listed here, cuts through and around a pole.)
///
/// Results of this file that the tessellator does not mesh yet, by what
/// their names contain, each with how it fails. They are valid bodies
/// that measure as their oracle says; the mesh is the tessellator's to
/// get right (`crates/tess`: a face whose boundary passes through a pole
/// of its patch on the way of an edge, or runs around one), and
/// `the_tessellator_meshes_cuts_through_a_pole` below reproduces them.
/// An entry that meshes is to be removed here: the test says so.
const UNMESHED: &[(&[&str], &str)] = &[];

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
/// into that face, "imprint lost"; see `architecture.md` §5.3.)
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
/// (`architecture.md` §5.3 › *Freeform faces with a pole*). Held to the
/// closed forms from the profile where the tool cuts across the axis
/// (`π ∫ ρ² dz` and `2π ∫ ρ ds` up to the cut), to the identities of
/// sets for every tool (around the point, a leaning block around it, a
/// ball, and away from it), to the exact membership of 400 seeded points
/// (the solid is the points within `ρ(y)` of the axis), and every result
/// is meshed. Refused still: a face that holds the axis through the
/// point (a plane through it along the axis), which the needle touches
/// there: on the needle's chart the distance to that face vanishes to the
/// second order all along the border the point is, and the marcher takes
/// that for a contact along a curve.
#[test]
fn a_cusp_is_an_operand() {
    const CUSP: [P; 4] = [[0.0, 0.0], [0.0, 2.0], [3.0, 4.0], [3.0, 6.0]];
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
        let along = half_space(k, Vec3::ZERO, -Vec3::X, Vec3::Y);
        for op in [Union, Intersect, Subtract] {
            match boolean(&cusp, &along, op, &tolerance(k)) {
                Err(KernelError::Unsupported { reason, .. }) => {
                    assert!(reason.contains("touch along a curve"), "{reason}");
                }
                other => panic!(
                    "a plane along the axis through the cusp, {op:?} at {k}: {:?}; hold it to the half solid",
                    other.map(|r| counts(&r))
                ),
            }
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
