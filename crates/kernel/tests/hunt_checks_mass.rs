//! A seeded hunt of the measure of bodies (`craft3d_topo::mass_bound`,
//! `inertia`: what `document.measure` reports) against closed forms of the
//! test's own: blocks, cylinders, cones, balls, rings (tori), prisms of
//! random star polygons (a wedge among them), prisms with a spline wall,
//! polygons and spline profiles revolved about an axis (Pappus, with the
//! profile's integrals by Green's theorem and Gauss–Legendre, exact for
//! the polynomials they are), a block with a hole through it and a blind
//! one, two balls united and one bitten out of another (caps and lenses),
//! and two blocks united (inclusion–exclusion); each in a random frame
//! turned about a random axis and moved up to 20 000 sizes from the
//! origin, at scales 1e-3 to 1e6, and again in the world frame.
//!
//! The oracles:
//! - the volume and the area are within the bound the integrator reports
//!   (`mass_bound`: "its own estimate, not a guarantee"; a value outside it
//!   is reported as wrong all the same, the bound being what
//!   `document.measure {bound}` shows a user), beyond the rounding of the
//!   body's own coordinates far from the origin;
//! - the measure does not move with the frame by more than the two bounds;
//! - the centroid and the inertia tensor (about the centroid, along the
//!   world's axes) are within 1e-9 of the size (the centroid) and 1e-8 of
//!   the tensor's size; no bound is reported for them, so these are the
//!   test's own allowance, printed with the worst seen.
//!
//! The gate runs a fixed set; `-- --ignored` the wide sweep (`HUNT_CASES`
//! per kind, default 200). Wrong cases are pinned in [`KNOWN`].

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_range_loop
)]

mod boolean_common;

use boolean_common::Rng;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, check, inertia, mass_bound};
use std::f64::consts::PI;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Block,
    Cylinder,
    Cone,
    Ball,
    Ring,
    Prism,
    SplinePrism,
    Revolved,
    SplineRevolved,
    Hole,
    Blind,
    Balls,
    Bite,
    Blocks,
    /// Two cylinders crossing at an angle, united: the edges where they
    /// meet are approximate curves.
    Cross,
    /// A cylinder with a narrower one taken out across it.
    CrossCut,
}

const KINDS: [Kind; 16] = [
    Kind::Block,
    Kind::Cylinder,
    Kind::Cone,
    Kind::Ball,
    Kind::Ring,
    Kind::Prism,
    Kind::SplinePrism,
    Kind::Revolved,
    Kind::SplineRevolved,
    Kind::Hole,
    Kind::Blind,
    Kind::Balls,
    Kind::Bite,
    Kind::Blocks,
    Kind::Cross,
    Kind::CrossCut,
];

const SCALES: [f64; 5] = [1e-3, 1.0, 1.0, 1e3, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

type M3 = [[f64; 3]; 3];

/// What a body is, in its frame's coordinates: volume, area (when known),
/// centroid, and the second moments `J = ∫ x xᵀ dV` about the centroid.
#[derive(Clone, Copy, Debug)]
struct Exact {
    volume: f64,
    area: Option<f64>,
    centroid: [f64; 3],
    /// `None` where the test has no closed form for it.
    moments: Option<M3>,
}

fn diag(a: f64, b: f64, c: f64) -> M3 {
    [[a, 0.0, 0.0], [0.0, b, 0.0], [0.0, 0.0, c]]
}

fn add(a: M3, b: M3, s: f64) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| a[i][j] + s * b[i][j]))
}

fn outer(c: [f64; 3], v: f64) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| v * c[i] * c[j]))
}

/// The second moments about the origin of a part whose moments about its
/// centroid `c` are `j`.
fn about_origin(e: &Exact) -> M3 {
    add(e.moments.unwrap(), outer(e.centroid, e.volume), 1.0)
}

/// Inclusion–exclusion: `a + b − both` (each with its moments).
fn combine(parts: &[(Exact, f64)]) -> Exact {
    let volume: f64 = parts.iter().map(|(e, s)| s * e.volume).sum();
    let mut m = [0.0; 3];
    for (e, s) in parts {
        for i in 0..3 {
            m[i] += s * e.volume * e.centroid[i];
        }
    }
    let centroid = m.map(|x| x / volume);
    let moments = if parts.iter().all(|(e, _)| e.moments.is_some()) {
        let mut j = [[0.0; 3]; 3];
        for (e, s) in parts {
            j = add(j, about_origin(e), *s);
        }
        Some(add(j, outer(centroid, volume), -1.0))
    } else {
        None
    };
    Exact {
        volume,
        area: None,
        centroid,
        moments,
    }
}

fn block_exact(lo: [f64; 3], size: [f64; 3]) -> Exact {
    let v = size[0] * size[1] * size[2];
    Exact {
        volume: v,
        area: Some(2.0 * (size[0] * size[1] + size[1] * size[2] + size[2] * size[0])),
        centroid: std::array::from_fn(|i| lo[i] + 0.5 * size[i]),
        moments: Some(diag(
            v * size[0] * size[0] / 12.0,
            v * size[1] * size[1] / 12.0,
            v * size[2] * size[2] / 12.0,
        )),
    }
}

/// A cylinder along z from its base at `base`.
fn cylinder_exact(base: [f64; 3], r: f64, h: f64) -> Exact {
    let v = PI * r * r * h;
    Exact {
        volume: v,
        area: Some(2.0 * PI * r * (r + h)),
        centroid: [base[0], base[1], base[2] + 0.5 * h],
        moments: Some(diag(v * r * r / 4.0, v * r * r / 4.0, v * h * h / 12.0)),
    }
}

/// Integrals over a plane region bounded counter-clockwise (or clockwise:
/// the sign is set by the area) by line segments and cubic Bézier curves:
/// `∫ u^p v^q dA` for p + q ≤ 3, by Green's theorem (`∮ u^(p+1)/(p+1) v^q
/// dv`) with Gauss–Legendre of ten points per piece (exact for the
/// polynomials of degree up to 19 they are), and the length of each piece
/// with a weight (`∮ w(u, v) ds`, composite Gauss–Legendre: lengths are
/// not polynomial).
#[derive(Clone)]
enum Piece {
    Line([f64; 2], [f64; 2]),
    Cubic([[f64; 2]; 4]),
}

impl Piece {
    fn at(&self, t: f64) -> [f64; 2] {
        match self {
            Piece::Line(a, b) => [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])],
            Piece::Cubic(p) => {
                let s = 1.0 - t;
                let w = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
                [
                    (0..4).map(|i| w[i] * p[i][0]).sum(),
                    (0..4).map(|i| w[i] * p[i][1]).sum(),
                ]
            }
        }
    }
    fn d(&self, t: f64) -> [f64; 2] {
        match self {
            Piece::Line(a, b) => [b[0] - a[0], b[1] - a[1]],
            Piece::Cubic(p) => {
                let s = 1.0 - t;
                let w = [3.0 * s * s, 6.0 * s * t, 3.0 * t * t];
                [
                    (0..3).map(|i| w[i] * (p[i + 1][0] - p[i][0])).sum(),
                    (0..3).map(|i| w[i] * (p[i + 1][1] - p[i][1])).sum(),
                ]
            }
        }
    }
}

/// Gauss–Legendre nodes and weights on [0, 1], ten points.
fn gauss10() -> [(f64, f64); 10] {
    let x = [
        0.148_874_338_981_631_21,
        0.433_395_394_129_247_2,
        0.679_409_568_299_024_4,
        0.865_063_366_688_984_5,
        0.973_906_528_517_171_7,
    ];
    let w = [
        0.295_524_224_714_752_87,
        0.269_266_719_309_996_35,
        0.219_086_362_515_982_04,
        0.149_451_349_150_580_6,
        0.066_671_344_308_688_14,
    ];
    let mut out = [(0.0, 0.0); 10];
    for i in 0..5 {
        out[2 * i] = (0.5 * (1.0 - x[i]), 0.5 * w[i]);
        out[2 * i + 1] = (0.5 * (1.0 + x[i]), 0.5 * w[i]);
    }
    out
}

/// `∫ u^p v^q dA` over the region the pieces bound, positive whichever way
/// they run.
fn moment(pieces: &[Piece], p: i32, q: i32) -> f64 {
    let g = gauss10();
    let raw = |p: i32, q: i32| -> f64 {
        let mut sum = 0.0;
        for piece in pieces {
            for &(t, w) in &g {
                let [u, v] = piece.at(t);
                let d = piece.d(t);
                sum += w * u.ppowi(p + 1) / f64::from(p + 1) * v.ppowi(q) * d[1];
            }
        }
        sum
    };
    let sign = raw(0, 0).signum();
    sign * raw(p, q)
}

/// `∮ w(u, v) ds` along the pieces (composite Gauss–Legendre, 64 panels of
/// ten points per piece).
fn along(pieces: &[Piece], w: impl Fn(f64, f64) -> f64) -> f64 {
    let g = gauss10();
    let mut sum = 0.0;
    for piece in pieces {
        let n = if matches!(piece, Piece::Line(..)) {
            1
        } else {
            64
        };
        for k in 0..n {
            for &(t, wt) in &g {
                let t = (f64::from(k) + t) / f64::from(n);
                let [u, v] = piece.at(t);
                let d = piece.d(t);
                sum += wt / f64::from(n) * w(u, v) * d[0].phypot(d[1]);
            }
        }
    }
    sum
}

/// The sketch curves of the pieces, as the kernel takes them.
fn curves(pieces: &[Piece]) -> Vec<SketchCurve> {
    pieces
        .iter()
        .enumerate()
        .map(|(i, p)| SketchCurve {
            id: i as u64 + 1,
            curve: match p {
                Piece::Line(a, b) => Curve2::Line { start: *a, end: *b },
                Piece::Cubic(c) => Curve2::Spline(Spline2 {
                    degree: 3,
                    knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                    control: c.to_vec(),
                }),
            },
        })
        .collect()
}

/// A prism of the region extruded `h` along z: in the frame's own
/// coordinates.
fn prism_exact(pieces: &[Piece], h: f64) -> Exact {
    let a = moment(pieces, 0, 0);
    let (mu, mv) = (moment(pieces, 1, 0), moment(pieces, 0, 1));
    let (cu, cv) = (mu / a, mv / a);
    let uu = moment(pieces, 2, 0) - a * cu * cu;
    let vv = moment(pieces, 0, 2) - a * cv * cv;
    let uv = moment(pieces, 1, 1) - a * cu * cv;
    let perimeter = along(pieces, |_, _| 1.0);
    Exact {
        volume: a * h,
        area: Some(2.0 * a + perimeter * h),
        centroid: [cu, cv, 0.5 * h],
        moments: Some([
            [h * uu, h * uv, 0.0],
            [h * uv, h * vv, 0.0],
            [0.0, 0.0, a * h * h * h / 12.0],
        ]),
    }
}

/// The region turned a full turn about the sketch's v axis (u > 0): by
/// Pappus and the moments of the profile. In the frame's coordinates (the
/// axis is the frame's y).
fn revolved_exact(pieces: &[Piece]) -> Exact {
    let i_u = moment(pieces, 1, 0);
    let volume = 2.0 * PI * i_u;
    let cv = moment(pieces, 1, 1) / i_u;
    // The faces: every piece not on the axis sweeps 2π u ds.
    let area = along(pieces, |u, _| 2.0 * PI * u);
    let axial = 2.0 * PI * moment(pieces, 3, 0);
    let along_axis =
        2.0 * PI * (moment(pieces, 1, 2) - 2.0 * cv * moment(pieces, 1, 1)) + volume * cv * cv;
    // J = ∫ x xᵀ: the radial directions each half of ∫ρ², the axis ∫(v −
    // cv)².
    Exact {
        volume,
        area: Some(area),
        centroid: [0.0, cv, 0.0],
        moments: Some(diag(0.5 * axial, along_axis, 0.5 * axial)),
    }
}

/// A body made for a case: in its frame, with what it is.
struct Made {
    body: Body,
    exact: Exact,
    frame: Frame,
    size: f64,
    what: String,
}

fn random_frame(rng: &mut Rng, k: f64) -> Frame {
    if rng.below(4) == 0 {
        return Frame::WORLD;
    }
    let far = [0.0, 30.0, 1000.0, 20000.0][rng.below(4) as usize];
    let dir = |rng: &mut Rng| {
        Vec3::new(
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
        )
    };
    let o = dir(rng);
    let o = o.normalized().unwrap_or(Vec3::new(1.0, 0.0, 0.0)) * (far * k);
    Frame::new(o, dir(rng), dir(rng)).unwrap_or(Frame::WORLD)
}

/// A star polygon about the origin: `n` corners at sorted angles, radii
/// 0.4 to 1 of `r`.
fn star(rng: &mut Rng, r: f64, shift: [f64; 2]) -> Vec<Piece> {
    let n = 3 + rng.below(7) as usize;
    let mut angles: Vec<f64> = (0..n)
        .map(|i| (i as f64 + rng.between(0.1, 0.9)) * 2.0 * PI / n as f64)
        .collect();
    angles.sort_by(f64::total_cmp);
    let points: Vec<[f64; 2]> = angles
        .iter()
        .map(|a| {
            let rr = r * rng.between(0.4, 1.0);
            [shift[0] + rr * a.pcos(), shift[1] + rr * a.psin()]
        })
        .collect();
    (0..n)
        .map(|i| Piece::Line(points[i], points[(i + 1) % n]))
        .collect()
}

fn make(rng: &mut Rng, kind: Kind, k: f64) -> Option<Made> {
    let tol = tolerance(k);
    let frame = random_frame(rng, k);
    let ok = |b: Body| check(&b, &tol).is_empty().then_some(b);
    let (body, exact, size, what) = match kind {
        Kind::Block => {
            let s = [
                rng.between(1.0, 20.0) * k,
                rng.between(1.0, 20.0) * k,
                rng.between(1.0, 20.0) * k,
            ];
            (
                primitives::block(&frame, s[0], s[1], s[2]).ok()?,
                block_exact([0.0; 3], s),
                s[0].max(s[1]).max(s[2]),
                format!("block {:.6e} x {:.6e} x {:.6e}", s[0], s[1], s[2]),
            )
        }
        Kind::Cylinder => {
            let (r, h) = (rng.between(0.5, 10.0) * k, rng.between(0.5, 20.0) * k);
            (
                primitives::cylinder(&frame, r, h).ok()?,
                cylinder_exact([0.0; 3], r, h),
                r.max(h),
                format!("cylinder {r:.6e}, {h:.6e}"),
            )
        }
        Kind::Cone => {
            let (r, h) = (rng.between(0.5, 10.0) * k, rng.between(0.5, 20.0) * k);
            let v = PI * r * r * h / 3.0;
            (
                primitives::cone(&frame, r, h).ok()?,
                Exact {
                    volume: v,
                    area: Some(PI * r * (r + r.phypot(h))),
                    centroid: [0.0, 0.0, 0.25 * h],
                    // ∫x² = ∫y² = 3 V r² / 20, ∫(z − h/4)² = 3 V h² / 80.
                    moments: Some(diag(
                        3.0 * v * r * r / 20.0,
                        3.0 * v * r * r / 20.0,
                        3.0 * v * h * h / 80.0,
                    )),
                },
                r.max(h),
                format!("cone {r:.6e}, {h:.6e}"),
            )
        }
        Kind::Ball => {
            let r = rng.between(0.5, 10.0) * k;
            let v = 4.0 * PI * r * r * r / 3.0;
            (
                primitives::sphere(&frame, r).ok()?,
                Exact {
                    volume: v,
                    area: Some(4.0 * PI * r * r),
                    centroid: [0.0; 3],
                    moments: Some(diag(v * r * r / 5.0, v * r * r / 5.0, v * r * r / 5.0)),
                },
                r,
                format!("ball {r:.6e}"),
            )
        }
        Kind::Ring => {
            let big = rng.between(1.0, 10.0) * k;
            let small = rng.between(0.05, 0.95) * big;
            let v = 2.0 * PI * PI * big * small * small;
            // ∫(x² + y²) = V (R² + 3 r² / 4), ∫z² = V r² / 4.
            let radial = 0.5 * v * (big * big + 0.75 * small * small);
            (
                primitives::torus(&frame, big, small).ok()?,
                Exact {
                    volume: v,
                    area: Some(4.0 * PI * PI * big * small),
                    centroid: [0.0; 3],
                    moments: Some(diag(radial, radial, 0.25 * v * small * small)),
                },
                big + small,
                format!("ring {big:.6e}, {small:.6e}"),
            )
        }
        Kind::Prism => {
            let r = rng.between(1.0, 10.0) * k;
            let shift = [rng.between(-3.0, 3.0) * k, rng.between(-3.0, 3.0) * k];
            let pieces = star(rng, r, shift);
            let h = rng.between(0.5, 20.0) * k;
            let profiles = detect_profiles(&curves(&pieces), &tol).ok()?;
            (
                extrude(&profiles, &frame, 0.0, h, &tol).ok()?.body,
                prism_exact(&pieces, h),
                r.max(h),
                format!(
                    "prism of {} corners, {r:.6e} across, {h:.6e} high",
                    pieces.len()
                ),
            )
        }
        Kind::SplinePrism => {
            let l = rng.between(2.0, 20.0) * k;
            let (h1, h2) = (rng.between(-0.3, 0.8) * l, rng.between(-0.3, 0.8) * l);
            let d = 0.5 * l;
            let pieces = vec![
                Piece::Cubic([[0.0, 0.0], [l / 3.0, h1], [2.0 * l / 3.0, h2], [l, 0.0]]),
                Piece::Line([l, 0.0], [l, -d]),
                Piece::Line([l, -d], [0.0, -d]),
                Piece::Line([0.0, -d], [0.0, 0.0]),
            ];
            let h = rng.between(0.5, 20.0) * k;
            let profiles = detect_profiles(&curves(&pieces), &tol).ok()?;
            (
                extrude(&profiles, &frame, 0.0, h, &tol).ok()?.body,
                prism_exact(&pieces, h),
                l.max(h),
                format!("spline prism {l:.6e} (heights {h1:.4e}, {h2:.4e}), {h:.6e} high"),
            )
        }
        Kind::Revolved => {
            let r = rng.between(1.0, 8.0) * k;
            let shift = [r + rng.between(0.2, 5.0) * k, rng.between(-3.0, 3.0) * k];
            let pieces = star(rng, r, shift);
            let profiles = detect_profiles(&curves(&pieces), &tol).ok()?;
            (
                revolve(
                    &profiles,
                    &frame,
                    [0.0, 0.0],
                    [0.0, 1.0],
                    0.0,
                    2.0 * PI,
                    &tol,
                )
                .ok()?
                .body,
                revolved_exact(&pieces),
                shift[0] + r,
                format!(
                    "polygon of {} corners, {r:.6e} across, {:.6e} off the axis, revolved",
                    pieces.len(),
                    shift[0]
                ),
            )
        }
        Kind::SplineRevolved => {
            let l = rng.between(2.0, 20.0) * k;
            let d = rng.between(0.3, 0.8) * l;
            let u0 = d + rng.between(0.1, 1.0) * l;
            let (h1, h2) = (rng.between(-0.3, 0.8) * d, rng.between(-0.3, 0.8) * d);
            let pieces = vec![
                Piece::Cubic([
                    [u0, 0.0],
                    [u0 + h1, l / 3.0],
                    [u0 + h2, 2.0 * l / 3.0],
                    [u0, l],
                ]),
                Piece::Line([u0, l], [u0 - d, l]),
                Piece::Line([u0 - d, l], [u0 - d, 0.0]),
                Piece::Line([u0 - d, 0.0], [u0, 0.0]),
            ];
            let profiles = detect_profiles(&curves(&pieces), &tol).ok()?;
            (
                revolve(
                    &profiles,
                    &frame,
                    [0.0, 0.0],
                    [0.0, 1.0],
                    0.0,
                    2.0 * PI,
                    &tol,
                )
                .ok()?
                .body,
                revolved_exact(&pieces),
                u0 + l,
                format!("spline profile {l:.6e} long, {u0:.6e} off the axis, revolved"),
            )
        }
        Kind::Hole | Kind::Blind => {
            let s = [
                rng.between(4.0, 20.0) * k,
                rng.between(4.0, 20.0) * k,
                rng.between(2.0, 20.0) * k,
            ];
            let r = rng.between(0.2, 0.4) * s[0].min(s[1]);
            let (cx, cy) = (
                rng.between(r * 1.1, s[0] - r * 1.1),
                rng.between(r * 1.1, s[1] - r * 1.1),
            );
            let blind = kind == Kind::Blind;
            let depth = if blind {
                rng.between(0.2, 0.8) * s[2]
            } else {
                s[2]
            };
            // The drill from below the floor (through) or from the top
            // down to `depth`.
            let (z0, h) = if blind {
                (s[2] - depth, depth + s[2])
            } else {
                (-s[2], 3.0 * s[2])
            };
            let drill_frame = frame.at(frame.point(cx, cy, z0))?;
            let a = primitives::block(&frame, s[0], s[1], s[2]).ok()?;
            let b = primitives::cylinder(&drill_frame, r, h).ok()?;
            let body = boolean(&a, &b, BooleanOp::Subtract, &tol).ok()?.body;
            let cut = cylinder_exact([cx, cy, s[2] - depth], r, depth);
            let mut exact = combine(&[(block_exact([0.0; 3], s), 1.0), (cut, -1.0)]);
            exact.area = Some(
                2.0 * (s[0] * s[1] + s[1] * s[2] + s[2] * s[0]) + 2.0 * PI * r * depth
                    - if blind { 0.0 } else { 2.0 * PI * r * r },
            );
            (
                body,
                exact,
                s[0].max(s[1]).max(s[2]),
                format!(
                    "block {:.4e} x {:.4e} x {:.4e} with a {} hole {r:.6e} {depth:.6e} deep",
                    s[0],
                    s[1],
                    s[2],
                    if blind { "blind" } else { "through" }
                ),
            )
        }
        Kind::Balls | Kind::Bite => {
            let (r1, r2) = (rng.between(1.0, 10.0) * k, rng.between(1.0, 10.0) * k);
            // Centres d apart, the spheres crossing (|r1 − r2| < d < r1 + r2).
            let lo = (r1 - r2).abs();
            let d = lo + rng.between(0.05, 0.95) * (r1 + r2 - lo);
            let dir = Vec3::new(
                rng.between(-1.0, 1.0),
                rng.between(-1.0, 1.0),
                rng.between(-1.0, 1.0),
            )
            .normalized()?;
            let a = primitives::sphere(&frame, r1).ok()?;
            let b =
                primitives::sphere(&frame.at(frame.point(d * dir.x, d * dir.y, d * dir.z))?, r2)
                    .ok()?;
            let bite = kind == Kind::Bite;
            let op = if bite {
                BooleanOp::Subtract
            } else {
                BooleanOp::Union
            };
            let body = boolean(&a, &b, op, &tol).ok()?.body;
            // The plane of the circle where they cross: x1 from the first
            // centre. Cap heights: h1 of the first ball past it, h2 of the
            // second short of it.
            let x1 = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
            let (h1, h2) = (r1 - x1, r2 - (d - x1));
            let cap = |r: f64, h: f64| PI * h * h * (3.0 * r - h) / 3.0;
            // A cap's centroid from its sphere's centre, along its axis.
            let at = |r: f64, h: f64| 3.0 * (2.0 * r - h).ppowi(2) / (4.0 * (3.0 * r - h));
            let (v1, v2) = (4.0 * PI * r1.ppowi(3) / 3.0, 4.0 * PI * r2.ppowi(3) / 3.0);
            let (c1, c2) = (cap(r1, h1), cap(r2, h2));
            let lens = c1 + c2;
            // The lens's moment along the line of centres (from the first).
            let lens_m = c1 * at(r1, h1) + c2 * (d - at(r2, h2));
            let (volume, m, area) = if bite {
                (
                    v1 - lens,
                    -lens_m,
                    4.0 * PI * r1 * r1 - 2.0 * PI * r1 * h1 + 2.0 * PI * r2 * h2,
                )
            } else {
                (
                    v1 + v2 - lens,
                    v2 * d - lens_m,
                    4.0 * PI * (r1 * r1 + r2 * r2) - 2.0 * PI * (r1 * h1 + r2 * h2),
                )
            };
            let t = m / volume;
            (
                body,
                Exact {
                    volume,
                    area: Some(area),
                    centroid: [t * dir.x, t * dir.y, t * dir.z],
                    moments: None,
                },
                r1.max(r2) + d,
                format!(
                    "balls {r1:.6e} and {r2:.6e} {d:.6e} apart, {}",
                    if bite {
                        "the second bitten out"
                    } else {
                        "united"
                    }
                ),
            )
        }
        Kind::Blocks => {
            let s1 = [
                rng.between(2.0, 20.0) * k,
                rng.between(2.0, 20.0) * k,
                rng.between(2.0, 20.0) * k,
            ];
            let s2 = [
                rng.between(2.0, 20.0) * k,
                rng.between(2.0, 20.0) * k,
                rng.between(2.0, 20.0) * k,
            ];
            // The second starts inside the first along every axis, so they
            // share a box; coplanar faces left out (the corner at least a
            // twentieth in).
            let p: [f64; 3] = std::array::from_fn(|i| rng.between(0.05, 0.95) * s1[i]);
            if (0..3).any(|i| (p[i] + s2[i] - s1[i]).abs() < 0.05 * k) {
                return None;
            }
            let a = primitives::block(&frame, s1[0], s1[1], s1[2]).ok()?;
            let b = primitives::block(
                &frame.at(frame.point(p[0], p[1], p[2]))?,
                s2[0],
                s2[1],
                s2[2],
            )
            .ok()?;
            let body = boolean(&a, &b, BooleanOp::Union, &tol).ok()?.body;
            let lo = p;
            let hi: [f64; 3] = std::array::from_fn(|i| (p[i] + s2[i]).min(s1[i]));
            let both = block_exact(lo, std::array::from_fn(|i| hi[i] - lo[i]));
            let exact = combine(&[
                (block_exact([0.0; 3], s1), 1.0),
                (block_exact(p, s2), 1.0),
                (both, -1.0),
            ]);
            (
                body,
                exact,
                (0..3).map(|i| s1[i].max(p[i] + s2[i])).fold(0.0, f64::max),
                format!("blocks {s1:?} and {s2:?} at {p:?}, united"),
            )
        }
        Kind::Cross | Kind::CrossCut => {
            let big = rng.between(1.0, 10.0) * k;
            let small = if rng.below(6) == 0 {
                big
            } else {
                rng.between(0.2, 0.95) * big
            };
            let alpha = if rng.below(3) == 0 {
                90.0_f64
            } else {
                rng.between(25.0, 90.0)
            }
            .to_radians();
            let sa = alpha.psin();
            // Long enough that the crossing is all inside both.
            let reach = (big + small) / sa + big;
            let (l1, l2) = (
                2.0 * reach * rng.between(1.1, 2.0),
                2.0 * reach * rng.between(1.1, 2.0),
            );
            let u = [sa, 0.0, alpha.pcos()];
            let a =
                primitives::cylinder(&frame.at(frame.point(0.0, 0.0, -0.5 * l1))?, big, l1).ok()?;
            let fb = Frame::new(
                frame.point(-0.5 * l2 * u[0], 0.0, -0.5 * l2 * u[2]),
                frame.vector(u[0], u[1], u[2]),
                frame.vector(0.0, 1.0, 0.0),
            )?;
            let b = primitives::cylinder(&fb, small, l2).ok()?;
            let cut = kind == Kind::CrossCut;
            let op = if cut {
                BooleanOp::Subtract
            } else {
                BooleanOp::Union
            };
            let body = boolean(&a, &b, op, &tol).ok()?.body;
            // The crossing of the infinite cylinders: V90 / sin α, V90 =
            // ∫ 4 √(R² − y²) √(r² − y²) dy over |y| < r (y = r sin t).
            let q = |f: &dyn Fn(f64) -> f64, lo: f64, hi: f64| -> f64 {
                let g = gauss10();
                let n = 256;
                let mut sum = 0.0;
                for i in 0..n {
                    let (a, b) = (
                        lo + (hi - lo) * f64::from(i) / f64::from(n),
                        lo + (hi - lo) * f64::from(i + 1) / f64::from(n),
                    );
                    for &(t, w) in &g {
                        sum += w * (b - a) * f(a + (b - a) * t);
                    }
                }
                sum
            };
            let half = 0.5 * PI;
            let ratio = small / big;
            let v90 = q(
                &|t: f64| {
                    let c = t.pcos();
                    4.0 * (big * big - small * small * t.psin() * t.psin())
                        .max(0.0)
                        .sqrt()
                        * small
                        * small
                        * c
                        * c
                },
                -half,
                half,
            );
            let shared = v90 / sa;
            // The narrow wall inside the wide cylinder, and the wide wall
            // inside the narrow (two lobes, sin φ = (r/R) sin ψ).
            let narrow_in = 2.0 * small / sa
                * (0..4)
                    .map(|i| {
                        q(
                            &|t: f64| {
                                (big * big - small * small * t.psin() * t.psin())
                                    .max(0.0)
                                    .sqrt()
                            },
                            f64::from(i) * half,
                            f64::from(i + 1) * half,
                        )
                    })
                    .sum::<f64>();
            let wide_in = 2.0 * big / sa
                * 2.0
                * q(
                    &|t: f64| {
                        let c = t.pcos();
                        small * ratio * c * c
                            / (1.0 - ratio * ratio * t.psin() * t.psin())
                                .max(1e-300)
                                .sqrt()
                    },
                    -half,
                    half,
                );
            let (va, vb) = (PI * big * big * l1, PI * small * small * l2);
            let (aa, ab) = (2.0 * PI * big * (big + l1), 2.0 * PI * small * (small + l2));
            let (volume, area) = if cut {
                (va - shared, aa - wide_in + narrow_in)
            } else {
                (va + vb - shared, aa + ab - wide_in - narrow_in)
            };
            (
                body,
                Exact {
                    volume,
                    area: Some(area),
                    centroid: [0.0; 3],
                    moments: None,
                },
                l1.max(l2),
                format!(
                    "cylinders {big:.6e} and {small:.6e} crossing at {:.3} deg, {}",
                    alpha.to_degrees(),
                    if cut {
                        "the narrow one taken out"
                    } else {
                        "united"
                    }
                ),
            )
        }
    };
    let body = ok(body)?;
    Some(Made {
        body,
        exact,
        frame,
        size,
        what,
    })
}

/// The frame's axes as the columns of a rotation.
fn rotation(f: &Frame) -> M3 {
    let (x, y, z) = (f.x(), f.y(), f.z());
    [[x.x, y.x, z.x], [x.y, y.y, z.y], [x.z, y.z, z.z]]
}

/// `R J Rᵀ`, then the inertia tensor `tr(J) 1 − J`.
fn tensor_in_world(j: M3, r: M3) -> M3 {
    let rj: M3 =
        std::array::from_fn(|i| std::array::from_fn(|k| (0..3).map(|m| r[i][m] * j[m][k]).sum()));
    let w: M3 =
        std::array::from_fn(|i| std::array::from_fn(|k| (0..3).map(|m| rj[i][m] * r[k][m]).sum()));
    let tr = w[0][0] + w[1][1] + w[2][2];
    std::array::from_fn(|i| std::array::from_fn(|k| if i == k { tr - w[i][k] } else { -w[i][k] }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Right,
    Declined,
    Wrong,
    Panic,
    Frame,
}

impl Verdict {
    fn letter(self) -> char {
        match self {
            Verdict::Right => 'R',
            Verdict::Declined => 'D',
            Verdict::Wrong => 'W',
            Verdict::Panic => 'P',
            Verdict::Frame => 'F',
        }
    }
}

/// What the measure says of a body: the volume, area and their bounds,
/// the centroid and the inertia tensor.
struct Measured {
    volume: f64,
    area: f64,
    bound_v: f64,
    bound_a: f64,
    centroid: Vec3,
    tensor: Option<M3>,
}

fn measure(body: &Body) -> Result<Result<Measured, String>, String> {
    catch_unwind(AssertUnwindSafe(|| {
        let (mp, mb) = mass_bound(body).map_err(|e| format!("{e:?}"))?;
        let tensor = inertia(body).ok().map(|i| i.tensor);
        Ok(Measured {
            volume: mp.volume,
            area: mp.area,
            bound_v: mb.volume,
            bound_a: mb.area,
            centroid: mp.centroid.ok_or("no centroid")?,
            tensor,
        })
    }))
    .map_err(|p| {
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    })
}

/// The worst seen, relative, over a sweep (for the allowances without a
/// bound).
#[derive(Default, Debug)]
struct Worst {
    centroid: f64,
    tensor: f64,
    /// The largest error of the volume as a share of its bound.
    volume_over_bound: f64,
    area_over_bound: f64,
}

fn judge(made: &Made, k: f64, rng: &mut Rng, worst: &mut Worst) -> (Verdict, String) {
    let e = &made.exact;
    let reach = made.frame.origin().length();
    let m = match measure(&made.body) {
        Err(p) => return (Verdict::Panic, format!("panicked: {p}")),
        Ok(Err(e)) => return (Verdict::Declined, format!("refused: {e}")),
        Ok(Ok(m)) => m,
    };
    let mut wrong = Vec::new();
    let area = e.area.unwrap_or(m.area);
    // The rounding of the body's own coordinates: each point off by about
    // 2e-16 of its distance from the origin.
    let slack_v = 4e-16 * area * (reach + made.size) + 1e-15 * e.volume;
    let slack_a = 4e-16 * area * (reach + made.size) / made.size + 1e-15 * area;
    let dv = (m.volume - e.volume).abs();
    worst.volume_over_bound = worst.volume_over_bound.max(dv / m.bound_v.max(1e-300));
    if dv > m.bound_v + slack_v {
        wrong.push(format!(
            "volume {:.17e} for {:.17e}: off by {dv:.3e}, bound {:.3e}",
            m.volume, e.volume, m.bound_v
        ));
    }
    if let Some(a) = e.area {
        let da = (m.area - a).abs();
        worst.area_over_bound = worst.area_over_bound.max(da / m.bound_a.max(1e-300));
        if da > m.bound_a + slack_a {
            wrong.push(format!(
                "area {:.17e} for {a:.17e}: off by {da:.3e}, bound {:.3e}",
                m.area, m.bound_a
            ));
        }
    }
    let c = made
        .frame
        .point(e.centroid[0], e.centroid[1], e.centroid[2]);
    let dc = (m.centroid - c).length();
    worst.centroid = worst.centroid.max(dc / made.size);
    if dc > 1e-9 * made.size + 4e-16 * reach {
        wrong.push(format!(
            "centroid {:?} for {c:?}: {dc:.3e} off ({:.3e} of the size)",
            m.centroid,
            dc / made.size
        ));
    }
    if let (Some(j), Some(t)) = (e.moments, m.tensor) {
        let want = tensor_in_world(j, rotation(&made.frame));
        let scale = (0..3).map(|i| want[i][i].abs()).fold(0.0, f64::max);
        let diff = (0..3)
            .flat_map(|i| (0..3).map(move |j| (i, j)))
            .map(|(i, j)| (t[i][j] - want[i][j]).abs())
            .fold(0.0, f64::max);
        worst.tensor = worst.tensor.max(diff / scale);
        if diff > 1e-8 * scale {
            wrong.push(format!(
                "inertia off by {:.3e} of its size: {t:?} for {want:?}",
                diff / scale
            ));
        }
    }
    // The same body moved: by the frame it was made in against a body made
    // in the world frame from the same draws is not possible here (the
    // draws are spent), so the body is moved rigidly instead and measured
    // again; the two may differ by their two bounds.
    let mut frame_note = None;
    if let Some(moved) = moved_copy(&made.body, rng, k)
        && let Ok(Ok(m2)) = measure(&moved)
    {
        let dv = (m2.volume - m.volume).abs();
        if dv > m.bound_v + m2.bound_v + 2.0 * slack_v + 4e-16 * area * 20000.0 * k {
            frame_note = Some(format!(
                "moved, the volume is {:.17e} against {:.17e}: {dv:.3e} apart, bounds {:.3e} and {:.3e}",
                m2.volume, m.volume, m.bound_v, m2.bound_v
            ));
        }
    }
    let verdict = if !wrong.is_empty() {
        Verdict::Wrong
    } else if frame_note.is_some() {
        Verdict::Frame
    } else {
        Verdict::Right
    };
    let mut lines = wrong;
    lines.extend(frame_note);
    (verdict, lines.join("; "))
}

/// The body turned about a random axis and moved up to 20 000 times the
/// scale away (or back to near the origin), as a rigid motion of its
/// entities.
fn moved_copy(body: &Body, rng: &mut Rng, k: f64) -> Option<Body> {
    let axis = Vec3::new(
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
    );
    let turn = craft3d_math::Transform::rotation_about(
        Vec3::new(0.0, 0.0, 0.0),
        axis,
        rng.between(-3.0, 3.0),
    )?;
    let away = Vec3::new(
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
    ) * ([0.0, 100.0, 20000.0][rng.below(3) as usize] * k);
    let t = turn.then(&craft3d_math::Transform::translation(away));
    body.transformed(&t).ok()
}

#[derive(Default)]
struct Tally {
    cases: usize,
    counts: [usize; 5],
    notes: Vec<(String, char, String)>,
    letters: String,
}

fn sweep(kind: Kind, seed: u64, count: usize, worst: &mut Worst) -> Tally {
    let mut rng = Rng(seed ^ (kind as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut tally = Tally::default();
    let mut index = 0;
    let mut draws = 0;
    while index < count && draws < 8 * count {
        draws += 1;
        let k = SCALES[rng.below(SCALES.len() as u64) as usize];
        let Some(made) = make(&mut rng, kind, k) else {
            continue;
        };
        let (verdict, why) = judge(&made, k, &mut rng, worst);
        let id = format!("{kind:?} {index}");
        let letter = verdict.letter();
        tally.letters.push(letter);
        tally.cases += 1;
        tally.counts[match verdict {
            Verdict::Right => 0,
            Verdict::Declined => 1,
            Verdict::Wrong => 2,
            Verdict::Panic => 3,
            Verdict::Frame => 4,
        }] += 1;
        if verdict != Verdict::Right {
            let line = format!(
                "{} at scale {k:e}, frame at {:.3e}: {why}",
                made.what,
                made.frame.origin().length()
            );
            println!("{letter} {id}: {line}");
            tally.notes.push((id, letter, line));
        }
        index += 1;
    }
    tally
}

fn summary(name: &str, t: &Tally) -> String {
    format!(
        "{name}: {} cases, {} right, {} declined, {} WRONG, {} panic, {} frame-dependent",
        t.cases, t.counts[0], t.counts[1], t.counts[2], t.counts[3], t.counts[4]
    )
}

/// Wrong cases of the gate's sweep, by kind and index, with what is wrong.
const KNOWN: [(Kind, usize, &str); 0] = [];

const GATE_SEED: u64 = 0x5EED_4A55_0001;

fn hold(kinds: &[Kind], count: usize) {
    let mut worst = Worst::default();
    for &kind in kinds {
        let tally = sweep(kind, GATE_SEED, count, &mut worst);
        println!("{}", summary(&format!("{kind:?}"), &tally));
        println!("  letters: {}", tally.letters);
        for (id, letter, what) in &tally.notes {
            let index: usize = id.rsplit(' ').next().unwrap().parse().unwrap();
            let known = KNOWN.iter().any(|(k, i, _)| *k == kind && *i == index);
            assert!(
                *letter == 'D' || (known && *letter == 'W'),
                "{id} {letter}: {what}"
            );
        }
        for (k, i, _) in KNOWN.iter().filter(|(k, ..)| *k == kind) {
            let id = format!("{k:?} {i}");
            assert!(
                tally.notes.iter().any(|(n, l, _)| *n == id && *l == 'W'),
                "{id} is listed as known wrong but is not: take it off KNOWN"
            );
        }
    }
    println!("worst: {worst:?}");
}

const GATE_CASES: usize = 12;

#[test]
fn primitives_measure_to_their_closed_forms() {
    hold(
        &[
            Kind::Block,
            Kind::Cylinder,
            Kind::Cone,
            Kind::Ball,
            Kind::Ring,
        ],
        GATE_CASES,
    );
}

#[test]
fn prisms_and_revolved_profiles_measure_to_their_closed_forms() {
    hold(
        &[
            Kind::Prism,
            Kind::SplinePrism,
            Kind::Revolved,
            Kind::SplineRevolved,
        ],
        GATE_CASES,
    );
}

#[test]
fn holes_unions_and_bites_measure_to_their_closed_forms() {
    hold(
        &[
            Kind::Hole,
            Kind::Blind,
            Kind::Balls,
            Kind::Bite,
            Kind::Blocks,
        ],
        GATE_CASES,
    );
}

#[test]
fn crossing_cylinders_measure_to_their_closed_forms() {
    hold(&[Kind::Cross, Kind::CrossCut], GATE_CASES);
}

#[test]
#[ignore]
fn wide() {
    let count: usize = std::env::var("HUNT_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let seed: u64 = std::env::var("HUNT_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5EED_4A55_1000);
    let mut worst = Worst::default();
    let mut all = Tally::default();
    for kind in KINDS {
        let t = sweep(kind, seed, count, &mut worst);
        println!("{}", summary(&format!("{kind:?}"), &t));
        all.cases += t.cases;
        for i in 0..5 {
            all.counts[i] += t.counts[i];
        }
    }
    println!("{}", summary("all", &all));
    println!("worst: {worst:?}");
}
