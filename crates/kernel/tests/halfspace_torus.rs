//! A torus cut by a plane parallel to its axis and then by a plane square
//! to it (fuzz seed 124506, reduced; docs/design/booleans.md › *A torus
//! cut beside its axis and then square to it*). The first plane leaves the
//! torus's face bounded by one closed spline, the oval in which the plane
//! meets the tube; the second meets the torus in two circles about its
//! axis, the outer one crossing the oval. The boolean grouped the loops of
//! the torus's face in a chart that cuts the torus open along a circle
//! about its axis, and a point stepped in from the inner circle by half its
//! width landed past that cut, where the circle winds about what it does
//! not bound: the band above the plane lost a loop and the boolean's own
//! check refused what it had made (`outline::left_of`, `Chart::beyond_cut`).
//!
//! Every piece is held to its volume and area, written here without the
//! kernel (the inner integrals in closed form, the outer by adaptive
//! Gauss–Legendre: [`Pieces`]), to `topo::check` and to the boolean's full
//! check against the block each plane is cut with. On the analytic torus:
//! first planes from 5.5 to 9.5 from the axis, both sides of each plane,
//! second planes from −2.4 to 2.4, at three scales, in the axes and turned
//! out of them, and the torus turned about its own axis; on the same torus
//! as one rational NURBS face closed both ways (its chart cut open along a
//! line of one share), with its seams in two places.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod rational_common;

use boolean_common::{SCALES, tolerance};
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, BooleanOptions, VerifyOptions, verify};
use craft3d_kernel::halfspace::{Kept, Tool, keep_below, tool};
use craft3d_kernel::primitives;
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_testkit::parallel::map_parallel;
use craft3d_topo::{Body, BodyKind, check, mass_properties};
use rational_common::{arc, revolved};
use std::f64::consts::{PI, TAU};

const MAJOR: f64 = 7.5;
const MINOR: f64 = 2.5;

/// Which side of the plane y = −d is kept: the ring (y > −d), or the cap
/// the plane cuts off the tube (y < −d).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Beside {
    Ring,
    Cap,
}

/// Which side of the plane z = h is kept.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Level {
    Above,
    Below,
}

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
/// halves agree to 1e-14 of the sum.
fn integral(f: &dyn Fn(f64) -> f64, a: f64, b: f64, depth: u32) -> f64 {
    let middle = 0.5 * (a + b);
    let (whole, halves) = (gauss(f, a, b), gauss(f, a, middle) + gauss(f, middle, b));
    if depth >= 40 || (whole - halves).abs() <= 1e-14 * halves.abs().max(1.0) {
        halves
    } else {
        integral(f, a, middle, depth + 1) + integral(f, middle, b, depth + 1)
    }
}

/// `∫ f` from the first of `stops` to the last, in stretches between them
/// (where `f` has a kink).
fn over(stops: &mut [f64], f: &dyn Fn(f64) -> f64) -> f64 {
    stops.sort_by(f64::total_cmp);
    stops
        .windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| integral(f, w[0], w[1], 0))
        .sum()
}

/// The solid torus about the z axis (radii [`MAJOR`] and [`MINOR`]) cut by
/// the plane y = −d and by the plane z = h, in closed form where it has
/// one. A point at distance ρ from the axis lies beyond y = −d over an
/// angle of `2 acos(d / ρ)` about the axis (none where ρ ≤ d), and
/// `∫ ρ acos(d / ρ) dρ = ρ² acos(d / ρ) / 2 − d √(ρ² − d²) / 2`, so the
/// level section of either piece at a height, between the radii `R ± w`
/// with `w = √(r² − z²)`, is closed. Over the height (z = r sin a, which
/// takes the square roots out of the ends) and around the tube (the torus
/// face, `r ρ(v)` per unit of angle about the axis and of angle v round the
/// tube) by [`integral`], in stretches between the kinks where the tube's
/// section reaches the plane y = −d. The flat face in that plane is the
/// chord of the tube's level section through it, the flat face in z = h a
/// level section.
struct Pieces {
    d: f64,
}

impl Pieces {
    /// The angle about the axis the piece takes at distance `rho`.
    fn kept(&self, beside: Beside, rho: f64) -> f64 {
        let cap = if rho > self.d {
            2.0 * (self.d / rho).pacos()
        } else {
            0.0
        };
        match beside {
            Beside::Ring => TAU - cap,
            Beside::Cap => cap,
        }
    }

    /// `∫ ρ kept(ρ) dρ` over `[lo, hi]`: the piece's part of a level
    /// section between those radii.
    fn section(&self, beside: Beside, lo: f64, hi: f64) -> f64 {
        let d = self.d;
        let anti =
            |rho: f64| 0.5 * rho * rho * (d / rho).pacos() - 0.5 * d * (rho * rho - d * d).sqrt();
        let from = lo.max(d);
        let cap = if hi > from {
            2.0 * (anti(hi) - anti(from))
        } else {
            0.0
        };
        match beside {
            Beside::Ring => PI * (hi * hi - lo * lo) - cap,
            Beside::Cap => cap,
        }
    }

    fn half_width(z: f64) -> f64 {
        (MINOR * MINOR - z * z).max(0.0).sqrt()
    }

    /// Volume and area at scale 1 of the piece on side `beside` of y = −d
    /// and side `level` of z = h.
    fn measures(&self, beside: Beside, level: Level, h: f64) -> (f64, f64) {
        let (big, r, d) = (MAJOR, MINOR, self.d);
        let (za, zb) = match level {
            Level::Above => (h.max(-r), r),
            Level::Below => (-r, h.min(r)),
        };
        if zb <= za {
            return (0.0, 0.0);
        }
        let (aa, ab) = ((za / r).pasin(), (zb / r).pasin());
        // Where the tube's level section reaches the plane y = −d.
        let mut heights = vec![aa, ab];
        for w in [d - big, big - d] {
            if (0.0..r).contains(&w) {
                let z = (r * r - w * w).sqrt();
                for a in [(z / r).pasin(), (-z / r).pasin()] {
                    if a > aa && a < ab {
                        heights.push(a);
                    }
                }
            }
        }
        let volume = over(&mut heights.clone(), &|a| {
            let (sin, cos) = a.psin_cos();
            let w = Self::half_width(r * sin);
            self.section(beside, big - w, big + w) * r * cos
        });
        let chord = |rho: f64| (rho * rho - d * d).max(0.0).sqrt();
        let in_plane = over(&mut heights, &|a| {
            let (sin, cos) = a.psin_cos();
            let w = Self::half_width(r * sin);
            2.0 * (chord(big + w) - chord(big - w)) * r * cos
        });
        let tube = |v: f64| {
            let rho = big + r * v.pcos();
            r * rho * self.kept(beside, rho)
        };
        let mut torus = 0.0;
        for (p, q) in [(aa, ab), (PI - ab, PI - aa)] {
            let mut stops = vec![p, q];
            if (d - big).abs() < r {
                let v = ((d - big) / r).pacos();
                for v in [v, -v, TAU - v] {
                    if v > p && v < q {
                        stops.push(v);
                    }
                }
            }
            torus += over(&mut stops, &tube);
        }
        let level_face = if h > -r && h < r {
            let w = Self::half_width(h);
            self.section(beside, big - w, big + w)
        } else {
            0.0
        };
        (volume, torus + in_plane + level_face)
    }
}

/// The quadrature against what is known without it: the whole torus, its
/// halves by a plane through the axis and by its equator, and Pappus for a
/// slab of it.
#[test]
fn the_pieces_of_a_torus_measure_as_known() {
    let close = |a: f64, b: f64, what: &str| {
        assert!((a - b).abs() <= 1e-12 * b.abs(), "{what}: {a} against {b}");
    };
    let whole = (
        2.0 * PI * PI * MAJOR * MINOR * MINOR,
        4.0 * PI * PI * MAJOR * MINOR,
    );
    // The plane y = −d beyond the tube: the ring is all of it.
    let far = Pieces { d: 11.0 };
    let (v, a) = far.measures(Beside::Ring, Level::Above, -3.0);
    close(v, whole.0, "whole volume");
    close(a, whole.1, "whole area");
    // y = 0 through the axis: half, and two discs of the tube.
    let half = Pieces { d: 0.0 };
    let (v, a) = half.measures(Beside::Ring, Level::Above, -3.0);
    close(v, 0.5 * whole.0, "half volume");
    close(a, 0.5 * whole.1 + 2.0 * PI * MINOR * MINOR, "half area");
    // The equator: half, and an annulus.
    let (v, a) = far.measures(Beside::Ring, Level::Above, 0.0);
    close(v, 0.5 * whole.0, "upper volume");
    let annulus = PI * ((MAJOR + MINOR).ppowi(2) - (MAJOR - MINOR).ppowi(2));
    close(a, 0.5 * whole.1 + annulus, "upper area");
    // The two sides of each plane add up.
    for d in [5.5, 7.0, 8.0, 9.5] {
        let p = Pieces { d };
        for h in [-1.3, 0.5, 2.2] {
            let parts = [
                (Beside::Ring, Level::Above),
                (Beside::Ring, Level::Below),
                (Beside::Cap, Level::Above),
                (Beside::Cap, Level::Below),
            ];
            let total: f64 = parts.iter().map(|&(b, l)| p.measures(b, l, h).0).sum();
            close(total, whole.0, &format!("d {d}, h {h}: four pieces"));
        }
    }
}

/// What a cut leaves: a body that passes `topo::check` and the boolean's
/// full check against the block it was cut with, the body itself where
/// the plane misses it, or nothing; or the refusal in its words.
fn cut(body: &Body, plane: &Frame, k: f64) -> Result<Option<Body>, String> {
    let tol = tolerance(k);
    match keep_below(body, plane, &tol, &BooleanOptions::default()) {
        Ok(Kept::Whole) => Ok(Some(body.clone())),
        Ok(Kept::Cut(result)) => {
            let defects = check(&result.body, &tol);
            if !defects.is_empty() {
                return Err(format!("topo::check: {defects:?}"));
            }
            let Ok(Tool::Block(block)) = tool(body, plane, &tol) else {
                return Err("a cut without a block".to_string());
            };
            let found = verify(
                body,
                &block,
                BooleanOp::Intersect,
                &result.body,
                &tol,
                &VerifyOptions::default(),
            )
            .map_err(|e| format!("the boolean check cannot run: {e}"))?;
            if let Some(first) = found.violations.first() {
                return Err(format!("the boolean check: {first}"));
            }
            Ok(Some(result.body))
        }
        Err(KernelError::EmptyResult { .. }) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// One cut of the grid: the torus (made in `frame`, turned about its own
/// axis by `turn` radians, which moves where its curves start), the first
/// plane at `d` keeping `beside`, the second at `h` keeping `level` (the
/// planes in `frame`).
#[derive(Clone, Copy, Debug)]
struct Case {
    k: f64,
    frame: Frame,
    turn: f64,
    freeform: Option<(f64, f64)>,
    d: f64,
    beside: Beside,
    h: f64,
    level: Level,
}

/// The torus of radii 7.5 and 2.5 times `k` about the z axis of `frame`:
/// on its analytic surface, or (`freeform`, the angles where its turns
/// start about the axis and round the tube) as one face on the rational
/// NURBS surface that is the same torus, closed both ways.
fn torus(frame: &Frame, k: f64, freeform: Option<(f64, f64)>) -> Body {
    let Some((from, tube_from)) = freeform else {
        return primitives::torus(frame, MAJOR * k, MINOR * k).unwrap();
    };
    let mut profile = arc(tube_from, tube_from + TAU, 0.0, 1.0);
    for point in &mut profile.points {
        *point = ((MAJOR + MINOR * point.0) * k, MINOR * point.1 * k);
    }
    let surface = Surface::Nurbs(revolved(frame, &profile, from, from + TAU));
    let mut body = Body::new(BodyKind::Solid);
    // `∂u × ∂v` points out: u goes about the axis, v round the tube
    // counter-clockwise in its own plane.
    let face = body.add_face(surface, false, Vec::new());
    body.add_shell(vec![face]);
    body
}

/// What a case leaves, against its closed forms: when it is right (or
/// nothing where nothing is left), the volume's and the area's errors
/// over what the tolerance allows them; else what is wrong.
fn judged(case: &Case) -> Result<(f64, f64), String> {
    let Case {
        k,
        frame,
        turn,
        freeform,
        d,
        beside,
        h,
        level,
    } = *case;
    let (sin, cos) = turn.psin_cos();
    let turned = Frame::new(frame.origin(), frame.z(), frame.vector(cos, sin, 0.0)).unwrap();
    let body = torus(&turned, k, freeform);
    let first = Frame::from_z(
        frame.point(0.0, -d * k, 0.0),
        frame.vector(0.0, if beside == Beside::Ring { -1.0 } else { 1.0 }, 0.0),
    )
    .unwrap();
    let second = Frame::from_z(
        frame.point(0.0, 0.0, h * k),
        frame.vector(0.0, 0.0, if level == Level::Above { -1.0 } else { 1.0 }),
    )
    .unwrap();
    let first = cut(&body, &first, k)
        .map_err(|e| format!("the first plane: {e}"))?
        .ok_or("the first plane leaves nothing")?;
    let kept = cut(&first, &second, k).map_err(|e| format!("the second plane: {e}"))?;
    let (volume, area) = Pieces { d }.measures(beside, level, h);
    let Some(kept) = kept else {
        return if volume < 1e-9 {
            Ok((0.0, 0.0))
        } else {
            Err(format!("nothing left, where {volume} is"))
        };
    };
    let m = mass_properties(&kept).map_err(|e| format!("not measured: {e}"))?;
    let t = tolerance(k).linear;
    // The cut curves are fitted within the tolerance of the surfaces: the
    // volume may be off by the tolerance over the faces, the area by the
    // tolerance along the edges (no edge set of these pieces is longer
    // than the two circles of the tube's rims and a turn round it twice).
    let (volume, area) = (volume * k * k * k, area * k * k);
    let edges = (4.0 * PI * (MAJOR + MINOR) + 4.0 * PI * MINOR) * k;
    let (dv, da) = ((m.volume - volume).abs(), (m.area - area).abs());
    let worst = (dv / (t * area), da / (t * edges));
    if !(worst.0 <= 1.0 && worst.1 <= 1.0) {
        return Err(format!(
            "volume {} against {volume}, area {} against {area}",
            m.volume, m.area
        ));
    }
    Ok(worst)
}

/// Runs the cases, prints what came of them, and fails with every case
/// that is wrong.
fn held(cases: Vec<Case>, what: &str) {
    let count = cases.len();
    let found = map_parallel(cases, |case| (case, judged(&case)));
    let mut wrong = Vec::new();
    let mut worst = (0.0_f64, 0.0_f64);
    for (case, outcome) in &found {
        match outcome {
            Ok(w) => worst = (worst.0.max(w.0), worst.1.max(w.1)),
            Err(e) => wrong.push(format!("{case:?}: {e}")),
        }
    }
    println!(
        "{what}: {count} cuts, {} wrong; the measures' worst errors over the tolerance's {worst:?}",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{what}:\n{}", wrong.join("\n"));
}

/// The frames the torus is made in: the world's, and one turned out of the
/// axes and moved.
fn frames(k: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(1.5, -2.0, 0.7) * k,
            Vec3::new(0.3, -0.5, 0.8),
            Vec3::new(1.0, 0.2, -0.1),
        )
        .unwrap(),
    ]
}

/// Heights of the second plane, none where it would touch the cap's oval
/// at its highest or lowest point (1.5, 2, √5.25, √6 and 2.5 from the
/// equator for the first planes below).
const LEVELS: [f64; 19] = [
    -2.4, -2.2, -1.9, -1.6, -1.3, -1.0, -0.7, -0.5, -0.3, 0.0, 0.3, 0.5, 0.7, 1.0, 1.3, 1.6, 1.9,
    2.2, 2.4,
];

const BESIDE: [Beside; 2] = [Beside::Ring, Beside::Cap];
const LEVEL: [Level; 2] = [Level::Above, Level::Below];

/// Seed 124506 reduced: the first plane 8 from the axis, the second 0.5
/// above the equator; and every first plane from 5.5 to 9.5 with every
/// second plane, both sides of each, at three scales in both frames.
#[test]
fn a_torus_cut_beside_its_axis_then_square_to_it() {
    let mut cases = Vec::new();
    for k in SCALES {
        for frame in frames(k) {
            for d in [5.5, 6.0, 6.5, 7.0, 7.5, 8.0, 8.5, 9.0, 9.5] {
                for beside in BESIDE {
                    for h in LEVELS {
                        for level in LEVEL {
                            cases.push(Case {
                                k,
                                frame,
                                turn: 0.0,
                                freeform: None,
                                d,
                                beside,
                                h,
                                level,
                            });
                        }
                    }
                }
            }
        }
    }
    held(cases, "the analytic torus");
}

/// The seed's two planes, and two more pairs, with the torus turned about
/// its own axis in twelve steps under planes that stay where they are:
/// where the circles the second plane cuts start moves with the torus,
/// and so does the place a point is stepped in from them (the middle of
/// each circle). Before the repair the seed's planes built at one turn
/// of the twelve, where that middle faces the first plane and the oval's
/// pieces are nearer than the circle to the first step from it; in both
/// frames, at three scales.
#[test]
fn a_torus_turned_about_its_axis_is_cut_the_same() {
    let mut cases = Vec::new();
    for k in SCALES {
        for frame in frames(k) {
            for turn in 0..12 {
                for (d, h) in [(8.0, 0.5), (8.0, 1.0), (6.0, -0.5)] {
                    for level in LEVEL {
                        cases.push(Case {
                            k,
                            frame,
                            turn: f64::from(turn) * PI / 6.0,
                            freeform: None,
                            d,
                            beside: Beside::Ring,
                            h,
                            level,
                        });
                    }
                }
            }
        }
    }
    held(cases, "the torus turned about its axis");
}

/// The same torus as one face on a rational NURBS surface closed both
/// ways, whose chart cuts it open along a line of one share: the seams
/// about the axis and round the tube in two places, fewer planes, at
/// three scales in both frames.
#[test]
fn a_freeform_torus_cut_beside_its_axis_then_square_to_it() {
    let mut cases = Vec::new();
    for k in SCALES {
        for frame in frames(k) {
            for freeform in [(0.0, 0.0), (2.0, 2.5)] {
                for d in [5.5, 6.5, 8.0, 9.5] {
                    for beside in BESIDE {
                        for h in [-1.9, -0.5, 0.0, 0.5, 1.3, 2.2] {
                            for level in LEVEL {
                                cases.push(Case {
                                    k,
                                    frame,
                                    turn: 0.0,
                                    freeform: Some(freeform),
                                    d,
                                    beside,
                                    h,
                                    level,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    held(cases, "the freeform torus");
}
