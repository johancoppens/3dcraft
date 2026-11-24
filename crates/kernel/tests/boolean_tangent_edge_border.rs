//! A tool's tangent edge that crosses the spline border of the face it
//! lies on (random designs 103578, 110541, 121649 and 124545, group B of
//! the invalid bodies; docs/design/booleans.md › *A tangent edge across a
//! spline border*).
//!
//! The tool a fillet of a rim cuts away (the corner of the rim's section
//! turned about the rim's axis) lies on the wall along a strip of its own,
//! and its round face touches the wall along its tangent circle. Where the
//! wall's other border is a spline (the curve in which the wall meets a
//! ball, a cone or a torus) and the circle crosses it, the circle must be
//! cut where it leaves the wall, and the curve in which the ball meets the
//! round face must end there. Both places were taken from the spline's own
//! hit on the round face: a touch, placed where the spline comes nearest
//! the round face, which is anywhere in the valley over which it stays
//! within its fit of it (31 and 6.6 tolerances along the border from the
//! crossing in the seed, and off the circle). The circle went uncut and the
//! wall undivided, so the boolean's own check refused what it built
//! (*"edge … is used 1 times"*), or the spline was split where nothing
//! ends (*"an imprinted curve that does not fit the edges it should
//! coincide with"*). The touch is now taken to where the curve the spline
//! stands for reaches the round face (`corner::touch`).
//!
//! Every body here is held to `topo::check`, to the boolean's full check
//! and to its volume: written without the kernel where the radial set is
//! a quadratic's ([`turning_volume`]: the radial integral in closed form,
//! the angle in stretches between the places where the set changes form,
//! the height adaptively; the same in Python in the workshop,
//! `plan/tangent-edge-border/tools/quad_kinks.py`), and for the torus by
//! the identity that cutting the rim first gives the same body. At scales
//! 1e-3 to 1e6, turned out of the axes, and with the tool's seam in several
//! places.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

mod common;

use common::{arc, circle, line};
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract};
use craft3d_kernel::boolean::{BooleanOp, VerifyOptions, boolean, verify};
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{SketchCurve, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_testkit::parallel::map_parallel;
use craft3d_topo::{Body, EdgeId, check, mass_properties};
use std::f64::consts::{PI, TAU};

/// The scales the cases are built at; the tolerance is a millionth of
/// the scale.
const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

// ---------------------------------------------------------------------
// Volumes written without the kernel.

/// Eight-point Gauss–Legendre on `[a, b]`.
fn gauss(f: &dyn Fn(f64) -> f64, a: f64, b: f64) -> f64 {
    const NODES: [(f64, f64); 4] = [
        (0.183_434_642_495_649_8, 0.362_683_783_378_362),
        (0.525_532_409_916_329, 0.313_706_645_877_887_3),
        (0.796_666_477_413_626_7, 0.222_381_034_453_374_5),
        (0.960_289_856_497_536_2, 0.101_228_536_290_376_3),
    ];
    let (middle, half) = (0.5 * (a + b), 0.5 * (b - a));
    half * NODES
        .iter()
        .map(|&(x, w)| w * (f(middle - half * x) + f(middle + half * x)))
        .sum::<f64>()
}

/// `∫ f` over `[a, b]`: halved until eight points on the whole and on its
/// halves agree to `rel` of the sum or to `abs`.
fn integral(f: &dyn Fn(f64) -> f64, a: f64, b: f64, (rel, abs): (f64, f64), depth: u32) -> f64 {
    let middle = 0.5 * (a + b);
    let (whole, halves) = (gauss(f, a, b), gauss(f, a, middle) + gauss(f, middle, b));
    if depth >= 30 || (whole - halves).abs() <= (rel * halves.abs()).max(abs) {
        halves
    } else {
        integral(f, a, middle, (rel, 0.5 * abs), depth + 1)
            + integral(f, middle, b, (rel, 0.5 * abs), depth + 1)
    }
}

/// The pieces of `[lo, hi]` where `a r² + b r + c ≤ 0`.
fn at_most_zero(a: f64, b: f64, c: f64, (lo, hi): (f64, f64)) -> Vec<(f64, f64)> {
    let mut stops = vec![lo, hi];
    let mut root = |r: f64| {
        if r > lo && r < hi {
            stops.push(r);
        }
    };
    if a != 0.0 {
        let d = b * b - 4.0 * a * c;
        if d > 0.0 {
            let q = -0.5 * (b + d.sqrt().copysign(b));
            root(q / a);
            if q != 0.0 {
                root(c / q);
            }
        }
    } else if b != 0.0 {
        root(-c / b);
    }
    stops.sort_by(f64::total_cmp);
    stops
        .windows(2)
        .filter(|w| {
            let m = 0.5 * (w[0] + w[1]);
            w[1] > w[0] && a * m * m + b * m + c <= 0.0
        })
        .map(|w| (w[0], w[1]))
        .collect()
}

/// `∫ r dr` over `pieces`.
fn radial(pieces: &[(f64, f64)]) -> f64 {
    pieces.iter().map(|&(a, b)| 0.5 * (b * b - a * a)).sum()
}

/// What a solid described about an axis is, along the ray at angle `θ` and
/// height `s`: `∫ r dr` over its points there, and functions of `θ`, each
/// continuous, whose zeros are where that set changes form (a root of
/// its quadratic appears, or meets a bound): there the integrand has a
/// kink.
trait About: Sync {
    fn section(&self, theta: f64, s: f64) -> f64;
    fn kinks(&self, theta: f64, s: f64) -> Vec<f64>;
}

/// The volume of `solid`: `∫∫ section(θ, s) dθ ds`, `s` along the axis over
/// the stretches between `heights`, `θ` round a whole turn, cut where a
/// kink function changes sign (found among 2048 places, and halved), so
/// that each stretch is smooth. (Cut where the set's form changes, read at
/// 512 places, two changes a thousandth of a radian apart were one, and
/// the halves agreed by chance across the other: 8e-3 off at one height.)
fn turning_volume(heights: &[f64], solid: &dyn About) -> f64 {
    const PLACES: u32 = 2048;
    let around = |s: f64| {
        let at = |k: u32| TAU * f64::from(k) / f64::from(PLACES);
        let values: Vec<Vec<f64>> = (0..=PLACES).map(|k| solid.kinks(at(k), s)).collect();
        let mut stops = vec![0.0, TAU];
        for j in 0..values[0].len() {
            for k in 0..PLACES {
                let positive = values[k as usize][j] > 0.0;
                if (values[k as usize + 1][j] > 0.0) == positive {
                    continue;
                }
                let (mut a, mut b) = (at(k), at(k + 1));
                for _ in 0..60 {
                    let m = 0.5 * (a + b);
                    if (solid.kinks(m, s)[j] > 0.0) == positive {
                        a = m;
                    } else {
                        b = m;
                    }
                }
                stops.push(0.5 * (a + b));
            }
        }
        stops.sort_by(f64::total_cmp);
        stops
            .windows(2)
            .filter(|w| w[1] > w[0])
            .map(|w| integral(&|t| solid.section(t, s), w[0], w[1], (1e-13, 1e-13), 0))
            .sum::<f64>()
    };
    let mut heights = heights.to_vec();
    heights.sort_by(f64::total_cmp);
    heights
        .windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| integral(&around, w[0], w[1], (1e-12, 1e-11), 0))
        .sum()
}

/// How far into a rim's section of radius `rf`, at `d` from the corner of
/// the arc's square: `rf − √(rf² − (rf − d)²)`, the arc's offset.
fn arc_offset(rf: f64, d: f64) -> f64 {
    let e = rf - d;
    rf - (rf * rf - e * e).max(0.0).sqrt()
}

/// The seed's body (scale 1): the cylinder of radius 5 about the z axis
/// from z = 10 to 15, less the ball about (0, 8, 8) whose surface meets
/// the wall at y = 5 at height `top` (the highest point of the wall's
/// lower border, a spline) or, `exact`, less the half-space below the plane
/// z = y + top − 5 (an ellipse as high there), less the corner of the top
/// rim a fillet of radius `rf` cuts away.
struct Seed {
    top: f64,
    rf: f64,
    exact: bool,
}

impl Seed {
    fn wall(&self, z: f64) -> f64 {
        if z <= 15.0 - self.rf {
            5.0
        } else {
            5.0 - arc_offset(self.rf, 15.0 - z)
        }
    }

    /// `64 + (z − 8)² − r²`: the ray meets the ball where `ρ² − 16 ρ sin θ`
    /// plus this is negative.
    fn ball(&self, z: f64) -> f64 {
        64.0 + (z - 8.0) * (z - 8.0) - (9.0 + (self.top - 8.0) * (self.top - 8.0))
    }

    fn volume(&self) -> f64 {
        turning_volume(
            &[10.0, 15.0 - self.rf, self.top.clamp(10.0, 15.0), 15.0],
            self,
        )
    }
}

impl About for Seed {
    fn section(&self, theta: f64, z: f64) -> f64 {
        let m = self.wall(z);
        let gone = if self.exact {
            // Gone where z − top + 5 − ρ sin θ < 0.
            at_most_zero(0.0, -theta.psin(), z - self.top + 5.0, (0.0, m))
        } else {
            at_most_zero(1.0, -16.0 * theta.psin(), self.ball(z), (0.0, m))
        };
        0.5 * m * m - radial(&gone)
    }

    fn kinks(&self, theta: f64, z: f64) -> Vec<f64> {
        let (m, s) = (self.wall(z), theta.psin());
        if self.exact {
            vec![s, z - self.top + 5.0 - m * s]
        } else {
            let c = self.ball(z);
            // The ray tangent to the ball; the ball's surface on the wall.
            vec![64.0 * s * s - c, m * m - 16.0 * m * s + c]
        }
    }
}

fn seed_volume(top: f64, rf: f64, exact: bool) -> f64 {
    Seed { top, rf, exact }.volume()
}

/// Random design 110541 as reduced (scale 1): the cone of base radius 7.5
/// at z = 3.75 about (−2.5, c), apex at z = 16.25, less the hole of
/// radius 2.5 about the z axis, less the corner of the hole's rim in the
/// base a fillet of radius `rf` cuts away. Along a ray the cone is where
/// `ρ² + 2 b ρ + d ≤ 0`.
struct ConeHole {
    rf: f64,
    c: f64,
}

impl ConeHole {
    fn hole(&self, z: f64) -> f64 {
        if z >= 3.75 + self.rf {
            2.5
        } else {
            2.5 + arc_offset(self.rf, z - 3.75)
        }
    }

    fn cone(&self, theta: f64, z: f64) -> (f64, f64) {
        let rc = 7.5 * (16.25 - z) / 12.5;
        (
            2.5 * theta.pcos() - self.c * theta.psin(),
            6.25 + self.c * self.c - rc * rc,
        )
    }
}

impl About for ConeHole {
    fn section(&self, theta: f64, z: f64) -> f64 {
        let (b, d) = self.cone(theta, z);
        radial(&at_most_zero(1.0, 2.0 * b, d, (self.hole(z), 1e3)))
    }

    fn kinks(&self, theta: f64, z: f64) -> Vec<f64> {
        let ((b, d), lo) = (self.cone(theta, z), self.hole(z));
        vec![b * b - d, lo * lo + 2.0 * b * lo + d]
    }
}

fn cone_hole_volume(rf: f64, c: f64) -> f64 {
    turning_volume(&[3.75, 3.75 + rf, 16.25], &ConeHole { rf, c })
}

/// Random design 121649 as reduced (scale 1): the cylinder of radius 2.5
/// about the x axis from x = −10 to −7.5, less the cone of base radius 2.5
/// in z = 0 about (−7.5, 2.5) with its apex at z = 10, less the corner of
/// the rim x = −10 a fillet of radius `rf` cuts away. About the x axis,
/// y = ρ cos θ and z = ρ sin θ; the cone is where `(x + 7.5)² + (ρ cos θ −
/// 2.5)² ≤ (2.5 (1 − ρ sin θ / 10))²` and z ≥ 0, a quadratic `a ρ² + b ρ +
/// c ≤ 0`.
struct CylinderCone {
    rf: f64,
}

impl CylinderCone {
    fn wall(&self, x: f64) -> f64 {
        if x >= -10.0 + self.rf {
            2.5
        } else {
            2.5 - arc_offset(self.rf, x + 10.0)
        }
    }

    fn cone(theta: f64, x: f64) -> (f64, f64, f64) {
        let (s, c) = (theta.psin(), theta.pcos());
        (
            c * c - s * s / 16.0,
            -5.0 * c + 1.25 * s,
            (x + 7.5) * (x + 7.5),
        )
    }
}

impl About for CylinderCone {
    fn section(&self, theta: f64, x: f64) -> f64 {
        let m = self.wall(x);
        if theta.psin() < 0.0 {
            // Below the cone's base.
            return 0.5 * m * m;
        }
        let (a, b, c) = Self::cone(theta, x);
        0.5 * m * m - radial(&at_most_zero(a, b, c, (0.0, m)))
    }

    fn kinks(&self, theta: f64, x: f64) -> Vec<f64> {
        let m = self.wall(x);
        let (a, b, c) = Self::cone(theta, x);
        vec![theta.psin(), a * m * m + b * m + c, b * b - 4.0 * a * c, a]
    }
}

fn cylinder_cone_volume(rf: f64) -> f64 {
    turning_volume(&[-10.0, -10.0 + rf, -7.5], &CylinderCone { rf })
}

// ---------------------------------------------------------------------
// The bodies, built where a case says.

/// Where a case is built: at scale `k`, and turned out of the axes (about
/// a slanted axis, then moved) or not.
#[derive(Clone, Copy, Debug)]
struct Place {
    k: f64,
    turned: bool,
}

impl Place {
    fn put(self, body: Body) -> Body {
        if self.turned {
            let turn = Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
                .unwrap()
                .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * self.k));
            body.transformed(&turn).unwrap()
        } else {
            body
        }
    }

    fn frame(self, origin: [f64; 3], z: Vec3, hint: Vec3) -> Frame {
        Frame::new(Vec3::new(origin[0], origin[1], origin[2]) * self.k, z, hint).unwrap()
    }
}

/// The sketch frame of a rim tool about the z axis: its x axis the
/// world's z, its y axis the world's x; profile points are (height,
/// radius).
fn about_z() -> Frame {
    Frame::new(Vec3::ZERO, Vec3::Y, Vec3::Z).unwrap()
}

/// The sketch frame of a rim tool about the x axis: the world's xy plane;
/// profile points are (x, radius).
fn about_x() -> Frame {
    Frame::WORLD
}

/// The closed section `curves` (scale 1) turned a whole turn about the
/// sketch's x axis from `seam` radians, scaled and placed.
fn turned(place: Place, sketch: Frame, curves: &[SketchCurve], seam: f64) -> Body {
    let k = place.k;
    let tol = tolerance(k);
    let profiles = detect_profiles(&common::scale_curves(curves, k), &tol).unwrap();
    place.put(
        revolve(
            &profiles,
            &sketch,
            [0.0, 0.0],
            [1.0, 0.0],
            seam,
            seam + TAU,
            &tol,
        )
        .unwrap()
        .body,
    )
}

/// The corner of a rim at (`s`, `r`) of a section (along the axis,
/// radius) that a fillet of radius `rf` cuts away, the material towards
/// `(ds, dr)` (each ±1).
fn rim_corner(s: f64, r: f64, rf: f64, (ds, dr): (f64, f64)) -> Vec<SketchCurve> {
    let (along, across) = ([s + ds * rf, r], [s, r + dr * rf]);
    let center = [s + ds * rf, r + dr * rf];
    // The arc runs counter-clockwise from one foot to the other.
    let (start, end) = if ds * dr > 0.0 {
        (across, along)
    } else {
        (along, across)
    };
    vec![
        line(1, [s, r], across),
        arc(2, center, start, end),
        line(3, along, [s, r]),
    ]
}

fn op(a: &Body, b: &Body, which: BooleanOp, k: f64) -> Body {
    boolean(a, b, which, &tolerance(k)).unwrap().body
}

/// The seed's target: the wall less the ball, or less the half-space below
/// the plane (`exact`).
fn seed_target(place: Place, top: f64, exact: bool) -> Body {
    let k = place.k;
    let cylinder = place.put(
        primitives::cylinder(
            &place.frame([0.0, 0.0, 10.0], Vec3::Z, Vec3::X),
            5.0 * k,
            5.0 * k,
        )
        .unwrap(),
    );
    let cutter = if exact {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let (z, y) = (Vec3::new(0.0, s, -s), Vec3::new(0.0, -s, -s));
        let origin = Vec3::new(-20.0, 5.0, top) - y * 20.0;
        primitives::block(
            &place.frame([origin.x, origin.y, origin.z], z, Vec3::X),
            40.0 * k,
            40.0 * k,
            40.0 * k,
        )
        .unwrap()
    } else {
        let r = (9.0 + (top - 8.0) * (top - 8.0)).sqrt();
        primitives::sphere(&place.frame([0.0, 8.0, 8.0], Vec3::Z, Vec3::X), r * k).unwrap()
    };
    op(&cylinder, &place.put(cutter), Subtract, k)
}

/// The tool of a fillet of radius `rf` of the seed's top rim.
fn seed_tool(place: Place, rf: f64, seam: f64) -> Body {
    turned(
        place,
        about_z(),
        &rim_corner(15.0, 5.0, rf, (-1.0, -1.0)),
        seam,
    )
}

// ---------------------------------------------------------------------
// What a case came to, and whether it is right.

#[derive(Debug)]
enum Came {
    Built {
        volume: f64,
        area: f64,
        problems: Vec<String>,
    },
    Refused(String),
}

/// `target` less `tool`, with what `topo::check` and the boolean's full
/// check find.
fn cut(target: &Body, tool: &Body, k: f64) -> Came {
    let tol = tolerance(k);
    match boolean(target, tool, Subtract, &tol) {
        Ok(result) => {
            let mass = mass_properties(&result.body).unwrap();
            let mut problems: Vec<String> = check(&result.body, &tol)
                .iter()
                .map(ToString::to_string)
                .collect();
            match verify(
                target,
                tool,
                Subtract,
                &result.body,
                &tol,
                &VerifyOptions::default(),
            ) {
                Ok(found) => problems.extend(found.violations.iter().map(ToString::to_string)),
                Err(e) => problems.push(format!("the boolean's check cannot run: {e}")),
            }
            Came::Built {
                volume: mass.volume,
                area: mass.area,
                problems,
            }
        }
        Err(e) => Came::Refused(e.to_string()),
    }
}

/// What is wrong with `came`: a body that passes both checks, whose volume
/// is `expected` (at scale 1) to within the tolerance times its area.
fn wrong(label: &str, came: &Came, expected: f64, k: f64) -> Option<String> {
    println!("{label}: {came:?}");
    match came {
        Came::Built {
            volume,
            area,
            problems,
        } => {
            let want = expected * k * k * k;
            let allowed = 1e-6 * k * area;
            if !problems.is_empty() {
                Some(format!("{label}: built, but {problems:?}"))
            } else if (volume - want).abs() > allowed {
                Some(format!(
                    "{label}: volume {volume} where {want} (off by {:e}, allowed {allowed:e})",
                    (volume - want).abs()
                ))
            } else {
                None
            }
        }
        Came::Refused(words) => Some(format!("{label}: refused: {words}")),
    }
}

fn held(found: Vec<Option<String>>) {
    let found: Vec<String> = found.into_iter().flatten().collect();
    assert!(found.is_empty(), "{}", found.join("\n"));
}

// ---------------------------------------------------------------------
// The cases.

/// The quadrature against what the kernel built before this finding,
/// where it built: the border below the circle (13.749, the circle clear
/// of it) and an exact border (13.84), to the digits printed then. (An
/// earlier form of the quadrature, without the angle cut where the
/// section changes form, was 2.9e-4 off at 13.749: halves that agree by
/// chance across a kink.)
#[test]
fn the_volumes_written_here_meet_bodies_built_before() {
    for (got, kernel) in [
        (seed_volume(13.749, 1.25, false), 348.989_892_123_069),
        (seed_volume(13.749, 1.25, true), 340.762_488_900_750),
        (seed_volume(13.84, 1.25, true), 338.275_274_776_663),
    ] {
        assert!((got - kernel).abs() < 1e-6, "{got} against {kernel}");
    }
}

/// The seed as reduced (the border's highest point 13.84, the circle at
/// 13.75) and others like it: the circle crossing the border at a dozen
/// degrees (13.76) up to well into the wall (14.5), at every scale,
/// turned, and the tool's seam at five places about the axis (at the
/// crossing's side, opposite it, and between). All of them were refused
/// on `main` (*"boolean produced an invalid body"* at 13.84, *"an imprinted
/// curve that does not fit the edges it should coincide with"* at the
/// others).
#[test]
fn a_tangent_edge_across_a_spline_border_divides_the_wall() {
    let rf = 1.25;
    let tops = [13.76, 13.84, 14.0, 14.5];
    let seams = [0.0, 0.65, 0.5 * PI, PI, 1.5 * PI];
    let mut cases = Vec::new();
    for top in tops {
        for seam in seams {
            cases.push((
                Place {
                    k: 1.0,
                    turned: false,
                },
                top,
                seam,
            ));
        }
        for k in SCALES {
            for turned in [false, true] {
                cases.push((Place { k, turned }, top, 0.0));
            }
        }
    }
    for k in SCALES {
        for seam in seams {
            cases.push((Place { k, turned: true }, 13.84, seam));
        }
    }
    let expected: Vec<f64> = tops
        .iter()
        .map(|&top| seed_volume(top, rf, false))
        .collect();
    held(map_parallel(cases, |(place, top, seam)| {
        let came = cut(
            &seed_target(place, top, false),
            &seed_tool(place, rf, seam),
            place.k,
        );
        let at = tops.iter().position(|&t| t == top).unwrap();
        wrong(
            &format!("top {top} seam {seam:.3} {place:?}"),
            &came,
            expected[at],
            place.k,
        )
    }));
}

/// The same with the wall's lower border an ellipse (the half-space below
/// a slanted plane), which built on `main` at these heights: the volumes
/// written here.
#[test]
fn the_same_tool_across_an_exact_border_divides_the_wall_as_before() {
    let rf = 1.25;
    let tops = [13.76, 13.84, 14.0, 14.5];
    let mut cases = Vec::new();
    for top in tops {
        for k in SCALES {
            for turned in [false, true] {
                cases.push((Place { k, turned }, top));
            }
        }
    }
    let expected: Vec<f64> = tops.iter().map(|&top| seed_volume(top, rf, true)).collect();
    held(map_parallel(cases, |(place, top)| {
        let came = cut(
            &seed_target(place, top, true),
            &seed_tool(place, rf, 0.0),
            place.k,
        );
        let at = tops.iter().position(|&t| t == top).unwrap();
        wrong(
            &format!("plane top {top} {place:?}"),
            &came,
            expected[at],
            place.k,
        )
    }));
}

/// Where the border rises above the circle by no more than a few
/// thousandths, the circle crosses it at a degree or less, the border stays
/// within the tolerance of the round face over both crossings (one touch
/// for two places), and the curve in which the ball meets the round face
/// runs within the tolerance of the border all the way between them: the
/// round face's piece there is a sliver a few tolerances tall. Such a cut
/// is built and right, or refused in words that say the edge of the
/// tolerance; never an invalid body (on `main` too, spline or ellipse;
/// building them is owed: docs/design/booleans.md › *A tangent edge across
/// a spline border* › *Owed*).
#[test]
fn a_border_barely_above_the_circle_is_built_right_or_at_the_edge_of_tolerance() {
    let rf = 1.25;
    let tops = [13.750_01, 13.7501, 13.751, 13.755];
    let mut cases = Vec::new();
    for top in tops {
        for exact in [false, true] {
            for k in SCALES {
                cases.push((Place { k, turned: false }, top, exact));
            }
        }
    }
    held(map_parallel(cases, |(place, top, exact)| {
        let came = cut(
            &seed_target(place, top, exact),
            &seed_tool(place, rf, 0.0),
            place.k,
        );
        let label = format!("top {top} exact {exact} {place:?}");
        match &came {
            Came::Refused(words) if words.contains("at the edge of tolerance") => {
                println!("{label}: {came:?}");
                None
            }
            _ => wrong(&label, &came, seed_volume(top, rf, exact), place.k),
        }
    }));
}

/// Random design 110541 as reduced: the hole's rim in the cone's base
/// less the tool of a fillet of radius 5, whose tangent circle at z = 8.75
/// crosses the hole's upper border (the spline in which the hole meets the
/// cone, as low as 7.92); and of radius 4.5 and 5.5. Refused on `main` as
/// *"boolean produced an invalid body"*.
#[test]
fn seed_110541_the_rim_of_a_hole_in_a_cone() {
    let c = 0.018_227_814_754_864_995;
    let radii = [4.5, 5.0, 5.5];
    let mut cases = Vec::new();
    for rf in radii {
        for k in SCALES {
            for turned in [false, true] {
                cases.push((Place { k, turned }, rf));
            }
        }
    }
    let expected: Vec<f64> = radii.iter().map(|&rf| cone_hole_volume(rf, c)).collect();
    held(map_parallel(cases, |(place, rf)| {
        let k = place.k;
        let hole = place.put(
            primitives::cylinder(&place.frame([0.0; 3], Vec3::Z, Vec3::X), 2.5 * k, 20.0 * k)
                .unwrap(),
        );
        let cone = place.put(
            primitives::cone(
                &place.frame([-2.5, c, 3.75], Vec3::Z, Vec3::Y),
                7.5 * k,
                12.5 * k,
            )
            .unwrap(),
        );
        let target = op(&cone, &hole, Subtract, k);
        let tool = turned(
            place,
            about_z(),
            &rim_corner(3.75, 2.5, rf, (1.0, 1.0)),
            0.0,
        );
        let came = cut(&target, &tool, k);
        let at = radii.iter().position(|&r| r == rf).unwrap();
        wrong(&format!("110541 rf {rf} {place:?}"), &came, expected[at], k)
    }));
}

/// Random design 121649 as reduced: the cylinder about the x axis less a
/// cone whose base circle passes through the rim's vertex, less the tool
/// of a fillet of the rim x = −10 of radius 0.625 (and 0.5): its tangent
/// circle on the wall crosses the wall's border (the spline in which the
/// wall meets the cone). The cylinder's seam at two places. Refused on
/// `main` as *"boolean produced an invalid body"*.
#[test]
fn seed_121649_the_rim_of_a_cylinder_cut_by_a_cone() {
    let radii = [0.5, 0.625];
    let mut cases = Vec::new();
    for rf in radii {
        for k in SCALES {
            for turned in [false, true] {
                for hint in [Vec3::Y, Vec3::Z] {
                    cases.push((Place { k, turned }, rf, hint));
                }
            }
        }
    }
    let expected: Vec<f64> = radii.iter().map(|&rf| cylinder_cone_volume(rf)).collect();
    held(map_parallel(cases, |(place, rf, hint)| {
        let k = place.k;
        let cylinder = place.put(
            primitives::cylinder(
                &place.frame([-10.0, 0.0, 0.0], Vec3::X, hint),
                2.5 * k,
                2.5 * k,
            )
            .unwrap(),
        );
        let cone = place.put(
            primitives::cone(
                &place.frame([-7.5, 2.5, 0.0], Vec3::Z, Vec3::X),
                2.5 * k,
                10.0 * k,
            )
            .unwrap(),
        );
        let target = op(&cylinder, &cone, Subtract, k);
        let tool = turned(
            place,
            about_x(),
            &rim_corner(-10.0, 2.5, rf, (1.0, -1.0)),
            0.0,
        );
        let came = cut(&target, &tool, k);
        let at = radii.iter().position(|&r| r == rf).unwrap();
        wrong(
            &format!("121649 rf {rf} seam {hint:?} {place:?}"),
            &came,
            expected[at],
            k,
        )
    }));
}

/// Random design 124545 as reduced: a torus about z through (0, 8.75,
/// −7.5) intersected with the cylinder of radius 7.5 about the x axis from
/// x = 0 to 10, less the tool of a fillet of the rim x = 0 of radius 2.5
/// (and 1.25 and 2): its tangent circle on the wall crosses the wall's
/// border (the spline in which the wall meets the torus). Held to the
/// same body made the other way round: the cylinder's rim cut first, then
/// the torus taken (no tangent edge then lies on a face with a spline
/// border). Refused on `main` as *"boolean produced an invalid body"*.
#[test]
fn seed_124545_the_rim_of_a_cylinder_cut_by_a_torus() {
    let radii = [1.25, 2.0, 2.5];
    let mut cases = Vec::new();
    for rf in radii {
        for k in SCALES {
            for turned in [false, true] {
                cases.push((Place { k, turned }, rf));
            }
        }
    }
    held(map_parallel(cases, |(place, rf)| {
        let k = place.k;
        let torus = place.put(
            primitives::torus(
                &place.frame([0.0, 8.75, -7.5], Vec3::Z, Vec3::Y),
                7.5 * k,
                2.5 * k,
            )
            .unwrap(),
        );
        let cylinder = place.put(
            primitives::cylinder(&place.frame([0.0; 3], Vec3::X, Vec3::Y), 7.5 * k, 10.0 * k)
                .unwrap(),
        );
        let tool = turned(
            place,
            about_x(),
            &rim_corner(0.0, 7.5, rf, (1.0, -1.0)),
            0.0,
        );
        let target = op(&cylinder, &torus, Intersect, k);
        let came = cut(&target, &tool, k);
        let other = op(&op(&cylinder, &tool, Subtract, k), &torus, Intersect, k);
        let expected = mass_properties(&other).unwrap().volume / (k * k * k);
        wrong(&format!("124545 rf {rf} {place:?}"), &came, expected, k)
    }));
}

/// The edge of a body nearest `p`.
fn edge_near(body: &Body, p: Vec3) -> EdgeId {
    let distance = |i: usize| {
        let e = &body.edges[i];
        (0..=32)
            .map(|j| {
                e.curve
                    .eval(e.range.0 + (e.range.1 - e.range.0) * f64::from(j) / 32.0)
            })
            .map(|q| q.distance(p))
            .fold(f64::INFINITY, f64::min)
    };
    let nearest = (0..body.edges.len())
        .min_by(|&a, &b| distance(a).total_cmp(&distance(b)))
        .unwrap();
    EdgeId(u32::try_from(nearest).unwrap())
}

/// Random design 121704 as reduced, owed: half a torus (a circle of
/// radius 7.5 about (12.5, 2.5) in z = −10 turned half a turn about the
/// sketch's y axis, so that both end caps lie in z = −10), its end rim
/// filleted by 2.5, less a copy moved 7.5 along y. The two caps lie on one
/// plane and each is bounded by its fillet's tangent edge, a spline: the
/// copy's tangent edge crosses the original cap's border, and both
/// fillets are tangent to the plane there. The plane's meeting with the
/// copy's fillet is marched as a crossing curve, cut where the original's
/// tangent edge touches the fillet: one of the two crossings is found
/// (−8.85, 6.25), the other (−15.61, 6.25) not, both stretches are left
/// out, and the original's cap is not divided along the copy's tangent
/// edge (*"edge 8 is used 1 times …"*). Where two tangent edges cross on
/// one plane no surface meets either of them across it; the place is
/// where the two curves cross on the plane.
#[test]
#[ignore = "owed: two fillets' tangent edges that cross on a plane both caps lie on"]
fn owed_seed_121704_two_tangent_edges_cross_on_a_shared_cap() {
    for k in SCALES {
        let tol = tolerance(k);
        let sketch = Frame::WORLD.at(Vec3::new(0.0, 0.0, -10.0 * k)).unwrap();
        let profiles = detect_profiles(&[circle(1, [12.5 * k, 2.5 * k], 7.5 * k)], &tol).unwrap();
        let half = revolve(&profiles, &sketch, [0.0, 0.0], [0.0, 1.0], 0.0, PI, &tol)
            .unwrap()
            .body;
        let rim = edge_near(&half, Vec3::new(-20.0, 2.5, -10.0) * k);
        let rounded = fillet(
            &half,
            &[FilletEdge {
                edge: rim,
                radius: 2.5 * k,
            }],
            &tol,
        )
        .unwrap()
        .body;
        let copy = rounded
            .transformed(&Transform::translation(Vec3::new(0.0, 7.5, 0.0) * k))
            .unwrap();
        let result = boolean(&rounded, &copy, Subtract, &tol);
        let body = match result {
            Ok(result) => result.body,
            Err(e) => panic!("scale {k}: {e}"),
        };
        assert_eq!(check(&body, &tol), [], "scale {k}");
        let found = verify(
            &rounded,
            &copy,
            Subtract,
            &body,
            &tol,
            &VerifyOptions::default(),
        )
        .unwrap();
        assert!(
            found.violations.is_empty(),
            "scale {k}: {:?}",
            found.violations
        );
    }
}
