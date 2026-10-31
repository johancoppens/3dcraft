//! `Surface::project` against two oracles that need nothing of it.
//!
//! **Points with a known answer.** `P = S(u, v) + d n`, the surface's own
//! point pushed out along its normal by less than the surface curves
//! with, has `S(u, v)` as its nearest point: `project(P)` must return a
//! point no further than `|d|`, and, unless it found one nearer (which
//! happens where another part of the surface is closer), that very point
//! and those very parameters, up to the surface's periods.
//!
//! **Points anywhere.** The nearest of 200 × 200 samples of the surface
//! is within one sample spacing of the nearest point there is, wherever
//! `P` lies: `project(P)` may not be further than that.
//!
//! On every kind of surface, the freeform ones with everything that makes
//! a search go wrong: seams of surfaces that close on themselves one way
//! and both ways, poles and collapsed patch edges, borders, a
//! parametrisation that is warped between its knot lines.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{
    BezierPatch, Cone, Cylinder, NurbsCurve, NurbsSurface, Plane, Sphere, Surface, Torus,
};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_testkit::teapot::teapot;

/// SplitMix64: seeded, no dependency.
struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next()
    }
}

/// A surface with what the tests need to know about it.
struct Fixture {
    name: String,
    surface: Surface,
    /// The stretch of each parameter that is tried.
    range: ((f64, f64), (f64, f64)),
    /// Its size, for tolerances and offsets.
    size: f64,
}

/// The surface a profile in the half-plane (ρ, z) sweeps about the z axis
/// of `frame`: u around the axis as the nine-point rational circle goes,
/// v along the profile (*The NURBS Book* §8.5).
fn revolved(
    frame: &Frame,
    degree: usize,
    knots: &[f64],
    profile: &[(f64, f64)],
    profile_weights: &[f64],
) -> Surface {
    let unit = NurbsCurve::circle(&Frame::WORLD, 1.0).unwrap();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (around, &w_u) in unit.control_points().iter().zip(unit.weights()) {
        for (&(rho, z), &w_v) in profile.iter().zip(profile_weights) {
            ctrl.push(frame.point(around.x * rho, around.y * rho, z));
            weights.push(w_u * w_v);
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            2,
            degree,
            unit.knots().to_vec(),
            knots.to_vec(),
            9,
            profile.len(),
            ctrl,
            weights,
        )
        .unwrap(),
    )
}

/// A square in the xy plane of `frame` as a biquadratic spline whose
/// inner control points are pushed about in the plane: still the square,
/// with parameters that are smooth only between its knot lines.
fn warped_square(frame: &Frame, half: f64, multiplicity: usize, random: &mut Random) -> Surface {
    let mut knots = vec![0.0; 3];
    for knot in [0.35, 0.7] {
        knots.extend(std::iter::repeat_n(knot, multiplicity));
    }
    knots.extend([1.0; 3]);
    let count = knots.len() - 3;
    let home = |i: usize| -half + 2.0 * half * (knots[i + 1] + knots[i + 2]) / 2.0;
    let push = 0.15 * 2.0 * half / count as f64;
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..count {
        for j in 0..count {
            let inside = i > 0 && i + 1 < count && j > 0 && j + 1 < count;
            let (dx, dy) = if inside {
                (random.between(-push, push), random.between(-push, push))
            } else {
                (0.0, 0.0)
            };
            ctrl.push(frame.point(home(i) + dx, home(j) + dy, 0.0));
            weights.push(if inside {
                random.between(0.6, 1.6)
            } else {
                1.0
            });
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(2, 2, knots.clone(), knots, count, count, ctrl, weights).unwrap(),
    )
}

fn fixtures(k: f64, random: &mut Random) -> Vec<Fixture> {
    let frame = Frame::new(
        Vec3::new(1.0, -2.0, 0.5) * k,
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap();
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let turn = (0.0, std::f64::consts::TAU);
    let half_turn = std::f64::consts::FRAC_PI_2;
    let unit = ((0.0, 1.0), (0.0, 1.0));
    let mut all = vec![
        Fixture {
            name: "plane".into(),
            surface: Surface::Plane(Plane::new(frame)),
            range: ((-3.0 * k, 3.0 * k), (-3.0 * k, 3.0 * k)),
            size: 3.0 * k,
        },
        Fixture {
            name: "cylinder".into(),
            surface: Surface::Cylinder(Cylinder::new(frame, 2.0 * k).unwrap()),
            range: (turn, (-3.0 * k, 3.0 * k)),
            size: 3.0 * k,
        },
        Fixture {
            name: "cone, widening".into(),
            surface: Surface::Cone(Cone::new(frame, 1.5 * k, 0.4).unwrap()),
            // From the apex, at −1.5 k / tan 0.4, upwards.
            range: (turn, (-1.5 * k / 0.4_f64.ptan(), 3.0 * k)),
            size: 3.0 * k,
        },
        Fixture {
            name: "cone, narrowing".into(),
            surface: Surface::Cone(Cone::new(frame, 1.5 * k, -0.7).unwrap()),
            range: (turn, (-3.0 * k, 1.5 * k / 0.7_f64.ptan())),
            size: 3.0 * k,
        },
        Fixture {
            name: "sphere".into(),
            surface: Surface::Sphere(Sphere::new(frame, 3.0 * k).unwrap()),
            range: (turn, (-half_turn, half_turn)),
            size: 3.0 * k,
        },
        Fixture {
            name: "torus".into(),
            surface: Surface::Torus(Torus::new(frame, 4.0 * k, 1.0 * k).unwrap()),
            range: (turn, turn),
            size: 5.0 * k,
        },
    ];
    // A bumpy bicubic patch.
    let mut net = [[Vec3::ZERO; 4]; 4];
    for (i, row) in net.iter_mut().enumerate() {
        for (j, p) in row.iter_mut().enumerate() {
            let (x, y) = (i as f64, j as f64 * 1.5);
            *p = frame.point(
                x * k,
                y * k,
                (((i * 7 + j * 3) % 5) as f64 * 0.3 - 0.2 * x) * k,
            );
        }
    }
    let bumpy = BezierPatch::bicubic(net).unwrap();
    all.push(Fixture {
        name: "bumpy patch".into(),
        surface: Surface::Bezier(bumpy.clone()),
        range: unit,
        size: 4.0 * k,
    });
    all.push(Fixture {
        name: "bumpy patch as a spline".into(),
        surface: Surface::Nurbs(NurbsSurface::from_bezier(&bumpy).unwrap()),
        range: unit,
        size: 4.0 * k,
    });
    // The teapot: 32 patches, eight of them with a collapsed edge.
    for (i, patch) in teapot().unwrap().iter().enumerate() {
        let ctrl = patch
            .control_points()
            .iter()
            .map(|c| frame.point(c.x * k, c.y * k, c.z * k))
            .collect();
        all.push(Fixture {
            name: format!("teapot patch {i}"),
            surface: Surface::Bezier(BezierPatch::new(3, 3, ctrl).unwrap()),
            range: unit,
            size: 3.0 * k,
        });
    }
    // Splines that close on themselves: a can (one way), a sphere (one
    // way, with two poles), a torus (both ways).
    all.push(Fixture {
        name: "spline can".into(),
        surface: revolved(
            &frame,
            1,
            &[0.0, 0.0, 0.2, 0.9, 1.0, 1.0],
            &[
                (1.5 * k, 0.0),
                (1.5 * k, 0.7 * k),
                (1.5 * k, 1.1 * k),
                (1.5 * k, 2.0 * k),
            ],
            &[1.0; 4],
        ),
        range: unit,
        size: 2.0 * k,
    });
    let r = 1.7 * k;
    all.push(Fixture {
        name: "spline sphere".into(),
        surface: revolved(
            &frame,
            2,
            &[0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
            &[(0.0, -r), (r, -r), (r, 0.0), (r, r), (0.0, r)],
            &[1.0, s, 1.0, s, 1.0],
        ),
        range: unit,
        size: r,
    });
    let (major, minor) = (3.0 * k, 0.8 * k);
    let square = [
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (-1.0, 1.0),
        (-1.0, 0.0),
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (1.0, 0.0),
    ];
    let tube: Vec<(f64, f64)> = square
        .iter()
        .map(|&(x, z)| (major + minor * x, minor * z))
        .collect();
    all.push(Fixture {
        name: "spline torus".into(),
        surface: revolved(
            &frame,
            2,
            &[
                0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
            ],
            &tube,
            &[1.0, s, 1.0, s, 1.0, s, 1.0, s, 1.0],
        ),
        range: unit,
        size: major + minor,
    });
    for multiplicity in [1, 2] {
        all.push(Fixture {
            name: format!("warped square, knots of multiplicity {multiplicity}"),
            surface: warped_square(&frame, 2.0 * k, multiplicity, random),
            range: unit,
            size: 2.0 * k,
        });
    }
    all
}

/// The width of a parameter's range if the surface closes on itself in
/// that direction, as far as a look at it can tell: its period.
fn periods(fixture: &Fixture) -> (Option<f64>, Option<f64>) {
    let surface = &fixture.surface;
    if !matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
        return (surface.period_u(), surface.period_v());
    }
    let ((u0, u1), (v0, v1)) = fixture.range;
    let same = |a: Vec3, b: Vec3| a.distance(b) <= 1e-9 * fixture.size;
    let around_u = (0..=10).all(|j| {
        let v = v0 + (v1 - v0) * f64::from(j) / 10.0;
        same(surface.eval(u0, v), surface.eval(u1, v))
    });
    let around_v = (0..=10).all(|i| {
        let u = u0 + (u1 - u0) * f64::from(i) / 10.0;
        same(surface.eval(u, v0), surface.eval(u, v1))
    });
    (around_u.then_some(u1 - u0), around_v.then_some(v1 - v0))
}

/// `a − b`, less whole periods.
fn apart(a: f64, b: f64, period: Option<f64>) -> f64 {
    let d = a - b;
    match period {
        Some(period) => (d - (d / period).round() * period).abs(),
        None => d.abs(),
    }
}

/// A parameter to try: anywhere in its range, at either end, or a hair
/// (1e-12 to 1e-3 of the range) inside either end.
fn parameter(random: &mut Random, (low, high): (f64, f64)) -> f64 {
    let width = high - low;
    let hair = [1e-12, 1e-9, 1e-6, 1e-3][(random.next() * 4.0) as usize % 4] * width;
    match (random.next() * 10.0) as usize {
        0 => low,
        1 => high,
        2 => low + hair,
        3 => high - hair,
        _ => random.between(low, high),
    }
}

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

#[test]
fn a_point_pushed_off_a_surface_along_its_normal_comes_back_to_where_it_was() {
    let mut random = Random(0x9A0C_2E11);
    let mut tried = 0;
    let mut nearer = 0;
    let mut wrong = Vec::new();
    for k in SCALES {
        for fixture in fixtures(k, &mut random) {
            let surface = &fixture.surface;
            let size = fixture.size;
            let (period_u, period_v) = periods(&fixture);
            let ((u0, u1), (v0, v1)) = fixture.range;
            for _ in 0..150 {
                let (u, v) = (
                    parameter(&mut random, fixture.range.0),
                    parameter(&mut random, fixture.range.1),
                );
                let (origin, su, sv) = surface.derivatives(u, v);
                // Where a whole range of parameters is one point (a pole,
                // a collapsed edge, an apex) the normal is the cross
                // product of a tangent that is zero but for rounding:
                // nothing to push along. The point itself is tried.
                let (wu, wv) = (u1 - u0, v1 - v0);
                let (lu, lv) = (su.length() * wu, sv.length() * wv);
                let told_apart = lu > 1e-3 * size
                    && lv > 1e-3 * size
                    && su.cross(sv).length() * wu * wv > 1e-3 * lu * lv;
                let offsets: &[f64] = match surface.normal(u, v) {
                    Some(_) if told_apart => &[0.0, 1e-6, -1e-6, 1e-3, -1e-3, 1e-2, -1e-2],
                    _ => &[0.0],
                };
                for &offset in offsets {
                    let d = offset * size;
                    let normal = surface.normal(u, v).unwrap_or(Vec3::ZERO);
                    let p = origin + normal * d;
                    let what = format!(
                        "{} at scale {k}: ({u:e}, {v:e}) pushed out by {d:e}",
                        fixture.name
                    );
                    tried += 1;
                    let (pu, pv) = match surface.project(p) {
                        Ok(found) => found,
                        Err(error) => {
                            wrong.push(format!("{what}: {error}"));
                            continue;
                        }
                    };
                    let found = surface.eval(pu, pv);
                    let distance = found.distance(p);
                    // A NaN counts as wrong.
                    if distance.is_nan() || distance > d.abs() * (1.0 + 1e-9) + 1e-12 * size {
                        wrong.push(format!(
                            "{what}: came back {distance:e} away, at ({pu:e}, {pv:e})"
                        ));
                        continue;
                    }
                    if distance < d.abs() * (1.0 - 1e-9) - 1e-12 * size {
                        // Another part of the surface is nearer: so be it.
                        nearer += 1;
                        continue;
                    }
                    // 1e-9 of the size in space. (A parameter 1e-9 of its
                    // range from a border is about that far from it, so
                    // this tells a point from the border next to it only
                    // by half: the parameters below do the rest.)
                    if found.distance(origin) > 2e-9 * size {
                        wrong.push(format!(
                            "{what}: came back {:e} from where it was, at ({pu:e}, {pv:e})",
                            found.distance(origin)
                        ));
                        continue;
                    }
                    // The parameters too, where they are told apart.
                    if told_apart
                        && !(apart(pu, u, period_u) <= 1e-9 * wu
                            && apart(pv, v, period_v) <= 1e-9 * wv)
                    {
                        wrong.push(format!(
                            "{what}: the right point with other parameters, ({pu:e}, {pv:e})"
                        ));
                    }
                }
            }
        }
    }
    println!(
        "{tried} points, {nearer} with a nearer point than their own, {} wrong",
        wrong.len()
    );
    assert!(
        wrong.is_empty(),
        "{} of {tried} points wrong, the first of them:\n{}",
        wrong.len(),
        wrong
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    // The offsets are small against every fixture: few points have a
    // nearer one.
    assert!(nearer * 50 < tried, "{nearer} of {tried}");
}

/// The nearest of `n × n` samples of a fixture to `p`, and how far apart
/// neighbouring samples are at most.
fn sampled(fixture: &Fixture, grid: &[Vec<Vec3>], p: Vec3) -> f64 {
    let _ = fixture;
    grid.iter()
        .flatten()
        .map(|q| q.distance(p))
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_point_anywhere_is_brought_no_further_than_dense_sampling_brings_it() {
    const N: usize = 200;
    let mut random = Random(0x0BEE_F00D);
    let mut tried = 0;
    let mut behind = 0;
    let mut worst = 0.0_f64;
    for k in SCALES {
        for fixture in fixtures(k, &mut random) {
            let surface = &fixture.surface;
            let ((u0, u1), (v0, v1)) = fixture.range;
            let grid: Vec<Vec<Vec3>> = (0..=N)
                .map(|i| {
                    (0..=N)
                        .map(|j| {
                            surface.eval(
                                u0 + (u1 - u0) * i as f64 / N as f64,
                                v0 + (v1 - v0) * j as f64 / N as f64,
                            )
                        })
                        .collect()
                })
                .collect();
            // How far a point of the surface can be from its nearest
            // sample: half the diagonal of the largest cell.
            let mut spacing = 0.0_f64;
            for i in 0..N {
                for j in 0..N {
                    spacing = spacing
                        .max(grid[i][j].distance(grid[i + 1][j + 1]))
                        .max(grid[i + 1][j].distance(grid[i][j + 1]));
                }
            }
            let analytic = !matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_));
            let centre = grid[N / 2][N / 2];
            for _ in 0..40 {
                // Near the surface and far from it, all around it.
                let reach =
                    fixture.size * [0.05, 0.5, 2.0, 10.0][(random.next() * 4.0) as usize % 4];
                let from =
                    grid[(random.next() * N as f64) as usize][(random.next() * N as f64) as usize];
                let p = from
                    + Vec3::new(
                        random.between(-reach, reach),
                        random.between(-reach, reach),
                        random.between(-reach, reach),
                    );
                let _ = centre;
                let (u, v) = surface.project(p).unwrap();
                assert!(
                    u.is_finite() && v.is_finite(),
                    "{}: ({u}, {v})",
                    fixture.name
                );
                let found = surface.eval(u, v).distance(p);
                let reference = sampled(&fixture, &grid, p);
                tried += 1;
                // The analytic kinds are unbounded where the samples are
                // not, and may find a nearer point outside them.
                let allowed = reference + spacing;
                assert!(
                    found <= allowed || (analytic && found <= reference),
                    "{} at scale {k}: {p:?} brought {found:e} away, sampling finds {reference:e} \
                     (samples {spacing:e} apart)",
                    fixture.name
                );
                if found > reference {
                    behind += 1;
                    worst = worst.max((found - reference) / spacing);
                }
            }
        }
    }
    println!(
        "{tried} points; {behind} further than the nearest sample, by {worst:.3} of the sample spacing at most"
    );
}

/// Bad input is an error or a plain answer, never a panic or a NaN.
#[test]
fn hostile_points_are_errors_or_plain_answers() {
    let mut random = Random(7);
    for fixture in fixtures(1.0, &mut random) {
        for p in [
            Vec3::new(f64::NAN, 0.0, 0.0),
            Vec3::new(f64::INFINITY, 0.0, 0.0),
            Vec3::new(0.0, f64::NEG_INFINITY, 1.0),
        ] {
            assert!(fixture.surface.project(p).is_err(), "{}", fixture.name);
        }
        for p in [
            Vec3::ZERO,
            Vec3::new(1e300, -1e300, 1e300),
            Vec3::new(1e-300, 0.0, 0.0),
            Vec3::new(1e15, 1e15, 1e15),
        ] {
            if let Ok((u, v)) = fixture.surface.project(p) {
                assert!(
                    u.is_finite() && v.is_finite(),
                    "{}: ({u}, {v})",
                    fixture.name
                );
            }
        }
    }
}

/// How long a projection takes, per kind of surface: points on the
/// surface and points off it. `cargo test --release -p craft3d-geom
/// --test project -- --ignored --nocapture timing`.
#[test]
#[ignore = "a timing, not a check"]
fn timing() {
    use std::time::Instant;
    let mut random = Random(99);
    for fixture in fixtures(1.0, &mut random) {
        if fixture.name.starts_with("teapot patch")
            && !["teapot patch 0", "teapot patch 20", "teapot patch 28"]
                .contains(&fixture.name.as_str())
        {
            continue;
        }
        let surface = &fixture.surface;
        let points: Vec<Vec3> = (0..200)
            .map(|i| {
                let (u, v) = (
                    random.between(fixture.range.0.0, fixture.range.0.1),
                    random.between(fixture.range.1.0, fixture.range.1.1),
                );
                let off = if i % 2 == 0 { 0.0 } else { 1e-2 * fixture.size };
                surface.eval(u, v) + surface.normal(u, v).unwrap_or(Vec3::ZERO) * off
            })
            .collect();
        let mut best = f64::INFINITY;
        for _ in 0..3 {
            let start = Instant::now();
            for _ in 0..20 {
                for p in &points {
                    std::hint::black_box(surface.project(std::hint::black_box(*p)).unwrap());
                }
            }
            best = best.min(start.elapsed().as_secs_f64() / (20.0 * points.len() as f64));
        }
        println!(
            "TIMING {:40} {:9.2} µs per projection",
            fixture.name,
            best * 1e6
        );
    }
}

/// A point beyond a border of an open patch has its nearest point on the
/// border: the nearest point of the border curve, which a search along
/// that one curve finds on its own (the nearest of 4000 samples, then
/// narrowed between its neighbours by thirds). `project` must come as
/// near, and say the border's parameter exactly.
#[test]
fn a_point_beyond_a_border_is_brought_to_the_border() {
    let mut random = Random(0x00B0_DEE5);
    let mut tried = 0;
    let mut on_border = 0;
    for k in SCALES {
        for fixture in fixtures(k, &mut random) {
            let surface = &fixture.surface;
            if !matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
                continue;
            }
            let (period_u, period_v) = periods(&fixture);
            let ((u0, u1), (v0, v1)) = fixture.range;
            let size = fixture.size;
            // The four borders: which parameter is fixed, at what, and
            // the direction in which that parameter leaves the domain.
            for (fixed_u, at, sign, open) in [
                (true, u0, -1.0, period_u.is_none()),
                (true, u1, 1.0, period_u.is_none()),
                (false, v0, -1.0, period_v.is_none()),
                (false, v1, 1.0, period_v.is_none()),
            ] {
                if !open {
                    continue;
                }
                let along = if fixed_u { (v0, v1) } else { (u0, u1) };
                let on = |t: f64| if fixed_u { (at, t) } else { (t, at) };
                for _ in 0..6 {
                    let t = random.between(along.0, along.1);
                    let (u, v) = on(t);
                    let (origin, su, sv) = surface.derivatives(u, v);
                    let (across, beside) = if fixed_u { (su, sv) } else { (sv, su) };
                    // A collapsed border is a point, not a border.
                    if across.length() * (u1 - u0).max(v1 - v0) < 1e-3 * size
                        || beside.length() * (along.1 - along.0) < 1e-3 * size
                    {
                        continue;
                    }
                    // Out of the patch, square to the border, and a
                    // little off the surface.
                    let Some(beside) = beside.normalized() else {
                        continue;
                    };
                    let out = across * sign - beside * (across.dot(beside) * sign);
                    let (Some(out), Some(normal)) = (out.normalized(), surface.normal(u, v)) else {
                        continue;
                    };
                    let p = origin
                        + out * (random.between(0.01, 0.2) * size)
                        + normal * (random.between(-0.05, 0.05) * size);
                    // The nearest point of the border curve.
                    let distance = |t: f64| {
                        let (u, v) = on(t);
                        surface.eval(u, v).distance(p)
                    };
                    const N: usize = 4000;
                    let step = (along.1 - along.0) / N as f64;
                    let nearest = (0..=N)
                        .map(|i| along.0 + step * i as f64)
                        .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
                        .unwrap();
                    let (mut low, mut high) =
                        ((nearest - step).max(along.0), (nearest + step).min(along.1));
                    for _ in 0..200 {
                        let (a, b) = (low + (high - low) / 3.0, high - (high - low) / 3.0);
                        if distance(a) < distance(b) {
                            high = b;
                        } else {
                            low = a;
                        }
                    }
                    let reference = distance(0.5 * (low + high));

                    let (pu, pv) = surface.project(p).unwrap();
                    let found = surface.eval(pu, pv).distance(p);
                    let what = format!(
                        "{} at scale {k}, beyond {} = {at}: {p:?}",
                        fixture.name,
                        if fixed_u { "u" } else { "v" }
                    );
                    tried += 1;
                    assert!(
                        found <= reference * (1.0 + 1e-9) + 1e-12 * size,
                        "{what}: brought {found:e} away, the border is {reference:e} away"
                    );
                    // Unless the patch curls back to somewhere nearer, it
                    // is that point of the border, and on the border.
                    if found >= reference * (1.0 - 1e-9) - 1e-12 * size {
                        on_border += 1;
                        let fixed = if fixed_u { pu } else { pv };
                        assert!(
                            fixed == at,
                            "{what}: at ({pu:e}, {pv:e}), not on the border"
                        );
                    }
                }
            }
        }
    }
    println!("{tried} points, {on_border} with their nearest point on the border");
    assert!(on_border * 2 > tried, "{on_border} of {tried}");
}

/// `project_from` the parameters of the point before it along a walk on
/// the surface lands where `project` does: on the point itself, pushed
/// off the surface by no more than an edge is, with the same parameters
/// to rounding. Where it does not (from a start at a pole, where the
/// parameters say nothing about where to go, the walk may be held at the
/// seam of a surface that closes on itself, or stuck), what it returns is
/// plainly no foot of the point: further off than an edge is from its
/// face, or with the offset not square to the surface, which is what a
/// caller checks before it takes the result over the search's. The walk
/// stays within the domain, as the nodes along one piece of an edge do.
#[test]
fn a_projection_from_the_point_before_lands_where_the_search_does() {
    let mut random = Random(0x5EED_F00D);
    let mut tried = 0;
    let mut missed = 0;
    let mut wrong = Vec::new();
    for k in SCALES {
        for fixture in fixtures(k, &mut random) {
            let surface = &fixture.surface;
            if !matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
                continue;
            }
            let size = fixture.size;
            let (period_u, period_v) = periods(&fixture);
            let ((u0, u1), (v0, v1)) = fixture.range;
            let (wu, wv) = (u1 - u0, v1 - v0);
            for _ in 0..4 {
                // A walk of a hundred steps, each 1/150 of the range in a
                // direction that drifts.
                let (mut u, mut v) = (random.between(u0, u1), random.between(v0, v1));
                let mut angle = random.between(0.0, std::f64::consts::TAU);
                let mut last: Option<(f64, f64)> = None;
                for _ in 0..100 {
                    angle += random.between(-0.5, 0.5);
                    u = (u + angle.pcos() * wu / 150.0).clamp(u0, u1);
                    v = (v + angle.psin() * wv / 150.0).clamp(v0, v1);
                    let (origin, su, sv) = surface.derivatives(u, v);
                    let (lu, lv) = (su.length() * wu, sv.length() * wv);
                    let told_apart = lu > 1e-3 * size
                        && lv > 1e-3 * size
                        && su.cross(sv).length() * wu * wv > 1e-3 * lu * lv;
                    let Some(start) = last.replace((u, v)) else {
                        continue;
                    };
                    if !told_apart {
                        continue;
                    }
                    let normal = surface.normal(u, v).unwrap_or(Vec3::ZERO);
                    for offset in [0.0, 1e-6, -1e-6] {
                        let p = origin + normal * (offset * size);
                        let what = format!(
                            "{} at scale {k}: ({u:e}, {v:e}) pushed out by {offset:e} of the size, from ({:e}, {:e})",
                            fixture.name, start.0, start.1
                        );
                        tried += 1;
                        let Ok((pu, pv)) = surface.project_from(p, start) else {
                            missed += 1;
                            continue;
                        };
                        let (found, fu, fv) = surface.derivatives(pu, pv);
                        let landed = found.distance(origin) <= 2e-9 * size
                            && apart(pu, u, period_u) <= 1e-9 * wu
                            && apart(pv, v, period_v) <= 1e-9 * wv;
                        if !landed {
                            // Then it may not look like a foot: within
                            // 1e-5 of the size and square to rounding.
                            let r = p - found;
                            let slack = 64.0
                                * f64::EPSILON
                                * (p.length() + fu.length() * wu + fv.length() * wv);
                            let square = |t: Vec3| r.dot(t).abs() <= slack * t.length();
                            if r.length() <= 1e-5 * size && square(fu) && square(fv) {
                                wrong.push(format!(
                                    "{what}: a foot {:e} from where it was, at ({pu:e}, {pv:e})",
                                    found.distance(origin)
                                ));
                            } else {
                                missed += 1;
                            }
                            continue;
                        }
                        // And the same as the search, where the search
                        // found this point (it may find a nearer one
                        // elsewhere, and does not say which it took).
                        let Ok((su_, sv_)) = surface.project(p) else {
                            continue;
                        };
                        let searched = surface.eval(su_, sv_);
                        if searched.distance(origin) <= 2e-9 * size
                            && !(apart(su_, pu, period_u) <= 1e-9 * wu
                                && apart(sv_, pv, period_v) <= 1e-9 * wv)
                        {
                            wrong.push(format!(
                                "{what}: ({pu:e}, {pv:e}) where the search gives ({su_:e}, {sv_:e})"
                            ));
                        }
                    }
                }
            }
        }
    }
    println!("{tried} points, {missed} missed, {} wrong", wrong.len());
    assert!(
        wrong.is_empty(),
        "{} of {tried} points wrong, the first of them:\n{}",
        wrong.len(),
        wrong
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    // Misses come from starts at a pole: one walk in a thousand at most.
    assert!(missed * 1000 < tried, "{missed} of {tried} missed");
}

/// A long, narrow sheet far from the origin (a kayak's panel, 5 m long
/// and 15 cm across, 2.5 m off: 2026-10-27): the rounding of the
/// positions there is many steps of the rounding of the parameter across.
/// A projection from a start next to the answer settles there (it did not
/// within its steps when it waited for the parameters' rounding alone),
/// at every scale.
#[test]
fn a_projection_onto_a_long_narrow_sheet_far_from_the_origin_settles() {
    let mut random = Random(0x4B41_5941);
    for k in SCALES {
        // Degree 3 along, 1 across, 24 rows: a wavy strip.
        let rows = 24;
        let knots_u: Vec<f64> = {
            let inner = rows - 4;
            let mut knots = vec![0.0; 4];
            knots.extend((1..=inner).map(|i| i as f64 / (inner + 1) as f64));
            knots.extend([1.0; 4]);
            knots
        };
        let mut ctrl = Vec::new();
        for i in 0..rows {
            let x = 5000.0 * i as f64 / (rows - 1) as f64;
            let wave = (x / 700.0).psin();
            let low = Vec3::new(2500.0 + x, 40.0 * wave, 200.0 + 10.0 * wave);
            let high = low + Vec3::new(3.0 * wave, 120.0, 90.0 + 20.0 * wave);
            ctrl.push(low * k);
            ctrl.push(high * k);
        }
        let surface = Surface::Nurbs(
            NurbsSurface::new(
                3,
                1,
                knots_u,
                vec![0.0, 0.0, 1.0, 1.0],
                rows,
                2,
                ctrl,
                vec![1.0; 2 * rows],
            )
            .unwrap(),
        );
        let mut unsettled = Vec::new();
        for _ in 0..400 {
            let (u, v) = (random.between(0.0, 1.0), random.between(0.0, 1.0));
            let normal = surface.normal(u, v).unwrap();
            let offset = random.between(-1e-7, 1e-7) * k;
            let p = surface.eval(u, v) + normal * offset;
            let start = (
                (u + random.between(-1e-3, 1e-3)).clamp(0.0, 1.0),
                (v + random.between(-1e-2, 1e-2)).clamp(0.0, 1.0),
            );
            match surface.project_from(p, start) {
                Ok((pu, pv)) => {
                    let foot = surface.eval(pu, pv);
                    assert!(
                        foot.distance(p) <= offset.abs() + 1e-9 * 5000.0 * k,
                        "scale {k}: ({u}, {v}) came back {} off",
                        foot.distance(p)
                    );
                }
                Err(e) => unsettled.push(format!("({u}, {v}) from {start:?}: {e}")),
            }
        }
        assert!(
            unsettled.is_empty(),
            "scale {k}: {} of 400 did not settle, the first: {:?}",
            unsettled.len(),
            unsettled.first()
        );
    }
}

/// On the analytic kinds `project_from` is `project`, whatever the start;
/// a start or a point that is not finite is an error, and a start outside
/// the domain is as good as one inside.
#[test]
fn a_projection_from_a_start_is_the_search_on_analytic_surfaces_and_never_panics() {
    let mut random = Random(0x0DD5);
    for fixture in fixtures(1.0, &mut random) {
        let surface = &fixture.surface;
        let ((u0, u1), (v0, v1)) = fixture.range;
        for _ in 0..20 {
            let p = Vec3::new(
                random.between(-5.0, 5.0),
                random.between(-5.0, 5.0),
                random.between(-5.0, 5.0),
            );
            let start = (random.between(u0, u1), random.between(v0, v1));
            if !matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
                assert_eq!(
                    surface.project_from(p, start).ok(),
                    surface.project(p).ok(),
                    "{}",
                    fixture.name
                );
            }
            for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                assert!(
                    surface.project_from(p, (bad, start.1)).is_err(),
                    "{}",
                    fixture.name
                );
                assert!(
                    surface.project_from(p, (start.0, bad)).is_err(),
                    "{}",
                    fixture.name
                );
                assert!(
                    surface
                        .project_from(Vec3::new(bad, 0.0, 0.0), start)
                        .is_err(),
                    "{}",
                    fixture.name
                );
            }
            // Outside the domain: brought to its border first.
            let far = (u0 - 3.0 * (u1 - u0), v1 + 2.0 * (v1 - v0));
            if let Ok((u, v)) = surface.project_from(p, far) {
                assert!(
                    u.is_finite() && v.is_finite(),
                    "{}: ({u}, {v})",
                    fixture.name
                );
            }
        }
    }
}
