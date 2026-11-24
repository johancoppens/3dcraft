//! The review's tests of a spline's touch moved onto the border of the
//! face it touches (`corner::touch`; docs/design/booleans.md › *A tangent
//! edge across a spline border*): cases built to make the move go wrong.
//! None of them did, at four scales, turned and with the tool's seam in
//! several places (the review's probe, with the move switched off and on in
//! one process: Check 45334254056 in the workshop).
//!
//! The wall's border a closed spline (a ball's hole in the wall) that the
//! tangent circle crosses twice, each crossing a touch of its own: `main`
//! refused most of them, and they build. Two crossings so close together
//! that one touch stands for both (the border barely above the circle):
//! refused, on `main` and here, never a body that is wrong. A plane whose
//! border is a spline (a ring's trace) crossed by a straight tangent edge
//! (a fillet's tool along a block's edge): built on `main`, built the same
//! here. Every body is held to `topo::check`, to the boolean's full check
//! and to a volume found without the move: a quadrature for the wall (the
//! method of `boolean_tangent_edge_border.rs`), the same set built in the
//! other order for the plane.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

mod common;

use common::{arc, line};
use craft3d_kernel::boolean::BooleanOp::Subtract;
use craft3d_kernel::boolean::{VerifyOptions, boolean, verify};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{SketchCurve, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_testkit::parallel::map_parallel;
use craft3d_topo::{Body, check, mass_properties};
use std::f64::consts::{PI, TAU};

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

// ---------------------------------------------------------------------
// Volumes written without the kernel (as in boolean_tangent_edge_border.rs).

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

/// The wall about the z axis from z = 10 to 15, its top rim's corner cut
/// away by a fillet's tool of radius `rf`, less the ball about (0, 8, `zc`)
/// of squared radius `rb2`: along the ray at angle `θ` and height `z`, the
/// radial integral in closed form and the functions whose zeros are where
/// the ray's set changes form (the ray tangent to the ball; the ball's
/// surface on the wall).
struct Holed {
    zc: f64,
    rb2: f64,
    rf: f64,
}

impl Holed {
    /// The ball's hole in the wall: about height `zc`, reaching `half`
    /// above and below it where the wall comes nearest the ball's centre.
    fn hole(zc: f64, half: f64, rf: f64) -> Holed {
        Holed {
            zc,
            rb2: 9.0 + half * half,
            rf,
        }
    }

    /// The seed's family: the ball about (0, 8, 8) whose hole's highest
    /// point is at `top`.
    fn top(top: f64, rf: f64) -> Holed {
        Holed::hole(8.0, top - 8.0, rf)
    }

    fn wall(&self, z: f64) -> f64 {
        if z <= 15.0 - self.rf {
            5.0
        } else {
            let e = self.rf - (15.0 - z);
            5.0 - (self.rf - (self.rf * self.rf - e * e).max(0.0).sqrt())
        }
    }

    fn ball(&self, z: f64) -> f64 {
        64.0 + (z - self.zc) * (z - self.zc) - self.rb2
    }

    fn section(&self, theta: f64, z: f64) -> f64 {
        let m = self.wall(z);
        let gone = at_most_zero(1.0, -16.0 * theta.psin(), self.ball(z), (0.0, m));
        0.5 * m * m - radial(&gone)
    }

    fn kinks(&self, theta: f64, z: f64) -> [f64; 2] {
        let (m, s, c) = (self.wall(z), theta.psin(), self.ball(z));
        [64.0 * s * s - c, m * m - 16.0 * m * s + c]
    }

    /// `∫∫ section dθ dz`, the angle cut where a kink function changes sign
    /// (found among 2048 places, and halved), so that each stretch is
    /// smooth; the height in stretches between the rim, the hole's ends and
    /// the wall's.
    fn volume(&self) -> f64 {
        const PLACES: u32 = 2048;
        let around = |z: f64| {
            let at = |k: u32| TAU * f64::from(k) / f64::from(PLACES);
            let values: Vec<[f64; 2]> = (0..=PLACES).map(|k| self.kinks(at(k), z)).collect();
            let mut stops = vec![0.0, TAU];
            for (k, pair) in (0..PLACES).zip(values.windows(2)) {
                for (j, (&before, &after)) in pair[0].iter().zip(&pair[1]).enumerate() {
                    let positive = before > 0.0;
                    if (after > 0.0) == positive {
                        continue;
                    }
                    let (mut a, mut b) = (at(k), at(k + 1));
                    for _ in 0..60 {
                        let m = 0.5 * (a + b);
                        if (self.kinks(m, z)[j] > 0.0) == positive {
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
                .map(|w| integral(&|t| self.section(t, z), w[0], w[1], (1e-13, 1e-13), 0))
                .sum::<f64>()
        };
        let half = (self.rb2 - 9.0).max(0.0).sqrt();
        let mut heights = [
            10.0,
            15.0 - self.rf,
            (self.zc - half).clamp(10.0, 15.0),
            (self.zc + half).clamp(10.0, 15.0),
            15.0,
        ];
        heights.sort_by(f64::total_cmp);
        heights
            .windows(2)
            .filter(|w| w[1] > w[0])
            .map(|w| integral(&around, w[0], w[1], (1e-12, 1e-11), 0))
            .sum()
    }
}

// ---------------------------------------------------------------------
// The bodies.

#[derive(Clone, Copy, Debug)]
struct Place {
    k: f64,
    turned: bool,
}

const PLACES: [Place; 5] = [
    Place {
        k: 1e-3,
        turned: false,
    },
    Place {
        k: 1.0,
        turned: false,
    },
    Place {
        k: 1e6,
        turned: false,
    },
    Place {
        k: 1.0,
        turned: true,
    },
    Place {
        k: 1e3,
        turned: true,
    },
];

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

/// The corner of a rim at (`s`, `r`) of a section that a fillet of radius
/// `rf` cuts away, the material towards `(ds, dr)` (each ±1).
fn rim_corner(s: f64, r: f64, rf: f64, (ds, dr): (f64, f64)) -> Vec<SketchCurve> {
    let (along, across) = ([s + ds * rf, r], [s, r + dr * rf]);
    let center = [s + ds * rf, r + dr * rf];
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

/// The tool of a fillet of radius `rf` of the wall's top rim, turned a
/// whole turn about the z axis from `seam` radians.
fn rim_tool(place: Place, rf: f64, seam: f64) -> Body {
    let tol = tolerance(place.k);
    let sketch = Frame::new(Vec3::ZERO, Vec3::Y, Vec3::Z).unwrap();
    let curves = common::scale_curves(&rim_corner(15.0, 5.0, rf, (-1.0, -1.0)), place.k);
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let body = revolve(
        &profiles,
        &sketch,
        [0.0, 0.0],
        [1.0, 0.0],
        seam,
        seam + TAU,
        &tol,
    )
    .unwrap()
    .body;
    place.put(body)
}

/// The wall less the ball of `holed`.
fn wall_less_ball(place: Place, holed: &Holed) -> Body {
    let k = place.k;
    let cylinder = primitives::cylinder(
        &place.frame([0.0, 0.0, 10.0], Vec3::Z, Vec3::X),
        5.0 * k,
        5.0 * k,
    )
    .unwrap();
    let ball = primitives::sphere(
        &place.frame([0.0, 8.0, holed.zc], Vec3::Z, Vec3::X),
        holed.rb2.sqrt() * k,
    )
    .unwrap();
    boolean(
        &place.put(cylinder),
        &place.put(ball),
        Subtract,
        &tolerance(k),
    )
    .unwrap()
    .body
}

// ---------------------------------------------------------------------
// What a case came to.

#[derive(Debug)]
enum Came {
    /// A body that passes both checks, and its volume and area.
    Built {
        volume: f64,
        area: f64,
    },
    /// A body that does not: what the checks found.
    Broken(Vec<String>),
    Refused(String),
}

/// `target` less `tool`, held to `topo::check` and the boolean's full
/// check.
fn cut(target: &Body, tool: &Body, k: f64) -> Came {
    let tol = tolerance(k);
    match boolean(target, tool, Subtract, &tol) {
        Ok(result) => {
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
            if problems.is_empty() {
                let mass = mass_properties(&result.body).unwrap();
                Came::Built {
                    volume: mass.volume,
                    area: mass.area,
                }
            } else {
                Came::Broken(problems)
            }
        }
        Err(e) => Came::Refused(e.to_string()),
    }
}

/// What is wrong with `came`: a body that fails a check, or whose volume
/// is not `want` within the tolerance times its area; and, unless
/// `refusals` allows them, a refusal.
fn wrong(label: &str, came: &Came, want: f64, k: f64, refusals: bool) -> Option<String> {
    println!("{label}: {came:?}");
    match came {
        Came::Built { volume, area } => {
            let allowed = 1e-6 * k * area;
            ((volume - want).abs() > allowed).then(|| {
                format!(
                    "{label}: volume {volume} where {want} (off by {:e}, allowed {allowed:e})",
                    (volume - want).abs()
                )
            })
        }
        Came::Broken(problems) => Some(format!("{label}: built, but {problems:?}")),
        Came::Refused(words) => (!refusals).then(|| format!("{label}: refused: {words}")),
    }
}

fn held(found: Vec<Option<String>>) {
    let found: Vec<String> = found.into_iter().flatten().collect();
    assert!(found.is_empty(), "{}", found.join("\n"));
}

// ---------------------------------------------------------------------
// The cases.

/// A ball whose hole in the wall is a closed spline (its seam wherever the
/// trace began), crossed twice by the tangent circle at z = 13.75, each
/// crossing a touch of its own: across the hole's middle, nearer its top,
/// and with its top 0.05 above the circle. On `main` the boolean refused
/// 48 of these 64 (*"an imprinted curve that does not fit"*, *"two points
/// closer together than the tolerance can tell apart"*, *"boolean produced
/// an invalid body"*) and built the other 16; all of them build, at four
/// scales, turned and with the tool's seam at four places.
#[test]
fn a_closed_spline_border_crossed_twice_by_the_tangent_circle_divides_the_wall() {
    let rf = 1.25;
    let holes = [(13.0, 1.5), (13.75, 0.5), (13.2, 0.6), (12.9, 0.9)];
    let seams = [0.0, 0.5 * PI, 0.5 * PI + 0.2, PI];
    let expected: Vec<f64> = map_parallel(holes.to_vec(), |(zc, half)| {
        Holed::hole(zc, half, rf).volume()
    });
    let mut cases = Vec::new();
    for (i, &(zc, half)) in holes.iter().enumerate() {
        for seam in seams {
            for place in PLACES.into_iter().take(4) {
                cases.push((zc, half, seam, place, expected[i]));
            }
        }
    }
    held(map_parallel(cases, |(zc, half, seam, place, want)| {
        let k = place.k;
        let target = wall_less_ball(place, &Holed::hole(zc, half, rf));
        let came = cut(&target, &rim_tool(place, rf, seam), k);
        let label = format!("hole {zc} {half} seam {seam:.3} {place:?}");
        wrong(&label, &came, want * k * k * k, k, false)
    }));
}

/// The seed's family with the border's top from on the tangent circle to
/// 3e-3 above it: the two crossings come together, and from about 5e-5
/// below 1.6e-3 above one touch of the border stands for both, which the
/// move may take to one of them. Never a body that fails a check or its
/// volume, at four scales, turned and with the seam beside the crossings;
/// refused where it is not built (as on `main`, which refused every one).
#[test]
fn a_touch_standing_for_two_crossings_never_makes_a_wrong_body() {
    let rf = 1.25;
    let rises = [
        0.0, 1e-6, 1e-5, 4e-5, 1e-4, 2e-4, 4e-4, 7e-4, 1e-3, 1.4e-3, 2e-3, 3e-3,
    ];
    let seams = [0.0, 0.5 * PI - 0.01, 0.5 * PI + 0.03];
    let expected: Vec<f64> = map_parallel(rises.to_vec(), |h| Holed::top(13.75 + h, rf).volume());
    let mut cases = Vec::new();
    for (i, &h) in rises.iter().enumerate() {
        for seam in seams {
            for place in PLACES {
                cases.push((h, seam, place, expected[i]));
            }
        }
    }
    held(map_parallel(cases, |(h, seam, place, want)| {
        let k = place.k;
        let target = wall_less_ball(place, &Holed::top(13.75 + h, rf));
        let came = cut(&target, &rim_tool(place, rf, seam), k);
        let label = format!("rise {h:e} seam {seam:.3} {place:?}");
        wrong(&label, &came, want * k * k * k, k, true)
    }));
}

/// A straight tangent edge across a spline border on a plane: the block
/// x 0 to 10, y −6 to 6, z 0 to 5 less a ring about the x axis (its trace
/// on the top face a spline), less the tool of a fillet of the edge x = 10,
/// z = 5 (the rim's corner along y), whose tangent line on the top crosses
/// that trace. Built on `main` and here; held to the same set built the
/// other way round (the block less the tool, then less the ring: no spline
/// edge in either operand there).
#[test]
fn a_straight_tangent_edge_across_a_spline_border_on_a_plane() {
    let mut cases = Vec::new();
    for rf in [1.0, 1.25, 1.5] {
        for cz in [6.5, 7.0, 7.5] {
            for place in PLACES {
                cases.push((rf, cz, place));
            }
        }
    }
    held(map_parallel(cases, |(rf, cz, place)| {
        let k = place.k;
        let tol = tolerance(k);
        let block = place.put(
            primitives::block(
                &place.frame([0.0, -6.0, 0.0], Vec3::Z, Vec3::X),
                10.0 * k,
                12.0 * k,
                5.0 * k,
            )
            .unwrap(),
        );
        let ring = place.put(
            primitives::torus(
                &place.frame([10.0, 0.0, cz], Vec3::X, Vec3::Y),
                5.0 * k,
                3.0 * k,
            )
            .unwrap(),
        );
        // The sketch's (u, v) is the world's (x, z), drawn on y = 7 and
        // extruded along −y.
        let sketch = Frame::new(Vec3::new(0.0, 7.0, 0.0) * k, -Vec3::Y, Vec3::X).unwrap();
        let curves = common::scale_curves(&rim_corner(10.0, 5.0, rf, (-1.0, -1.0)), k);
        let profiles = detect_profiles(&curves, &tol).unwrap();
        let tool = place.put(
            extrude(&profiles, &sketch, 0.0, 14.0 * k, &tol)
                .unwrap()
                .body,
        );
        let target = boolean(&block, &ring, Subtract, &tol).unwrap().body;
        let other = boolean(
            &boolean(&block, &tool, Subtract, &tol).unwrap().body,
            &ring,
            Subtract,
            &tol,
        )
        .unwrap()
        .body;
        let want = mass_properties(&other).unwrap().volume;
        let came = cut(&target, &tool, k);
        wrong(
            &format!("plane rf {rf} cz {cz} {place:?}"),
            &came,
            want,
            k,
            false,
        )
    }));
}
