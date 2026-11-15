//! The nearest point of a freeform surface (`Surface::project` and
//! `Surface::project_from`) against the closed form, on sheets that make a
//! search go wrong: lopsided (one side up to a million times the other),
//! parameters that run at very different speeds (knots stretched by up to
//! 1e9, or clustered), weights from 1e-3 to 1e3, several degrees, at
//! scales 1e-3, 1 and 1e6, in the world's frame, a turned one and turned
//! ones far from the origin. The sheets are planes, cylinders and spheres
//! written as splines, so the true foot is known without the code: for a
//! plane the point's orthogonal projection, for a cylinder and a sphere
//! the radial one. The work is counted (`crate::allowance`).
//!
//! `the_feet_of_points_near_lopsided_sheets_table` (ignored) prints the
//! whole sweep as a table, by kind and by place; `project_short` holds
//! parts of it.

// A foot that is not a number is off: `!(off <= allowed)` says so.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::*;
use crate::allowance;
use std::fmt::Write as _;

/// SplitMix64: seeded, no dependency.
pub struct Random(pub u64);

impl Random {
    pub fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next()
    }
}

/// How the knots of one parameter are spaced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Knots {
    /// Evenly, over [0, 1].
    Even,
    /// Evenly, over [0, factor].
    Stretched(f64),
    /// Crowded towards the start: the inner knots at (k / n)⁴.
    Clustered,
}

impl Knots {
    fn name(self) -> String {
        match self {
            Knots::Even => "even".into(),
            Knots::Stretched(k) => format!("x{k:.0e}"),
            Knots::Clustered => "clustered".into(),
        }
    }

    /// A clamped knot vector of `degree` with `inner` inner knots.
    fn vector(self, degree: usize, inner: usize) -> Vec<f64> {
        let (reach, clustered) = match self {
            Knots::Even => (1.0, false),
            Knots::Stretched(k) => (k, false),
            Knots::Clustered => (1.0, true),
        };
        let mut knots = vec![0.0; degree + 1];
        for k in 1..=inner {
            let t = k as f64 / (inner + 1) as f64;
            knots.push(reach * if clustered { t.ppowi(4) } else { t });
        }
        knots.extend(std::iter::repeat_n(reach, degree + 1));
        knots
    }
}

/// The Greville abscissae of a clamped knot vector, as shares of its
/// reach: control points there give a spline of degree one or more the
/// line's own pace (linear precision).
fn greville(knots: &[f64], degree: usize, count: usize) -> Vec<f64> {
    let (low, high) = (knots[0], knots[knots.len() - 1]);
    (0..count)
        .map(|i| {
            let sum: f64 = knots[i + 1..=i + degree].iter().sum();
            (sum / degree as f64 - low) / (high - low)
        })
        .collect()
}

/// Where a sheet stands.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub name: &'static str,
    /// Turned, or the world's axes.
    pub turned: bool,
    /// How many of its sizes from the origin.
    pub far: f64,
}

pub const PLACES: [Place; 4] = [
    Place {
        name: "world",
        turned: false,
        far: 0.0,
    },
    Place {
        name: "turned",
        turned: true,
        far: 0.0,
    },
    Place {
        name: "far 1e3",
        turned: true,
        far: 1e3,
    },
    Place {
        name: "far 1e6",
        turned: true,
        far: 1e6,
    },
];

fn frame(place: Place, size: f64) -> Frame {
    let origin = Vec3::new(0.37, -0.81, 0.46) * (place.far * size);
    if place.turned {
        Frame::new(
            origin,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.27, -0.1),
        )
        .unwrap()
    } else {
        Frame::WORLD.at(origin).unwrap()
    }
}

/// A sheet and its closed form.
pub struct Sheet {
    pub surface: Surface,
    pub frame: Frame,
    pub shape: Shape,
    /// Its size, for what counts as off.
    pub size: f64,
    /// Its smaller size: the scale, of which the default tolerance is a
    /// millionth.
    pub small: f64,
}

#[derive(Clone, Copy, Debug)]
pub enum Shape {
    /// The rectangle [0, length] × [0, width] of the frame's xy plane.
    Plane { length: f64, width: f64 },
    /// About the frame's z axis, z from 0 to `length`.
    Cylinder { radius: f64, length: f64 },
    /// About the frame's origin.
    Sphere { radius: f64 },
}

/// Weights from 1e-3 to 1e3, even in their logarithm.
fn weight(random: &mut Random) -> f64 {
    (6.0 * (random.next() - 0.5) * std::f64::consts::LN_10).pexp()
}

/// A plane rectangle `length` by `width` as a spline of `degrees` with
/// `inner` inner knots each way, its knots spaced as `knots`, weighted
/// from 1e-3 to 1e3 when `weighted`.
#[allow(clippy::too_many_arguments)]
pub fn plane(
    place: Place,
    (length, width): (f64, f64),
    (pu, pv): (usize, usize),
    (iu, iv): (usize, usize),
    (ku, kv): (Knots, Knots),
    weighted: bool,
    random: &mut Random,
) -> Sheet {
    let size = length.max(width);
    let f = frame(place, size);
    let (knots_u, knots_v) = (ku.vector(pu, iu), kv.vector(pv, iv));
    let (nu, nv) = (pu + 1 + iu, pv + 1 + iv);
    let (gu, gv) = (greville(&knots_u, pu, nu), greville(&knots_v, pv, nv));
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for &x in &gu {
        for &y in &gv {
            ctrl.push(f.point(length * x, width * y, 0.0));
            weights.push(if weighted { weight(random) } else { 1.0 });
        }
    }
    Sheet {
        surface: Surface::Nurbs(
            NurbsSurface::new(pu, pv, knots_u, knots_v, nu, nv, ctrl, weights).unwrap(),
        ),
        frame: f,
        shape: Shape::Plane { length, width },
        size,
        small: length.min(width),
    }
}

/// The nine points and weights of the rational quadratic circle of radius
/// `r`, and its knots over [0, `reach`].
fn circle(r: f64, reach: f64) -> (Vec<(f64, f64)>, Vec<f64>, Vec<f64>) {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let points = vec![
        (r, 0.0),
        (r, r),
        (0.0, r),
        (-r, r),
        (-r, 0.0),
        (-r, -r),
        (0.0, -r),
        (r, -r),
        (r, 0.0),
    ];
    let weights = vec![1.0, h, 1.0, h, 1.0, h, 1.0, h, 1.0];
    let knots = [0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 4.0]
        .iter()
        .map(|k| k / 4.0 * reach)
        .collect();
    (points, weights, knots)
}

/// A cylinder of `radius` and `length` as a spline: around it the
/// rational circle, its knots over [0, `around`]; along it degree `pv`
/// with `inner` inner knots spaced as `kv`, each row weighted from 1e-3 to
/// 1e3 when `weighted` (weights of a row times the circle's keep the
/// cylinder, at another pace along it).
#[allow(clippy::too_many_arguments)]
pub fn cylinder(
    place: Place,
    (radius, length): (f64, f64),
    around: f64,
    pv: usize,
    inner: usize,
    kv: Knots,
    weighted: bool,
    random: &mut Random,
) -> Sheet {
    let size = radius.max(length);
    let f = frame(place, size);
    let (points, round, knots_u) = circle(radius, around);
    let knots_v = kv.vector(pv, inner);
    let nv = pv + 1 + inner;
    let gv = greville(&knots_v, pv, nv);
    let along: Vec<f64> = (0..nv)
        .map(|_| if weighted { weight(random) } else { 1.0 })
        .collect();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (&(x, y), &w) in points.iter().zip(&round) {
        for (&z, &c) in gv.iter().zip(&along) {
            ctrl.push(f.point(x, y, length * z));
            weights.push(w * c);
        }
    }
    Sheet {
        surface: Surface::Nurbs(
            NurbsSurface::new(2, pv, knots_u, knots_v, 9, nv, ctrl, weights).unwrap(),
        ),
        frame: f,
        shape: Shape::Cylinder { radius, length },
        size,
        small: radius.min(length),
    }
}

/// A sphere of `radius` as a spline: the rational circle around, its
/// knots over [0, `around`], and the rational half circle from pole to
/// pole, its knots over [0, `meridian`].
pub fn sphere(place: Place, radius: f64, (around, meridian): (f64, f64)) -> Sheet {
    let f = frame(place, radius);
    let (points, round, knots_u) = circle(1.0, around);
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let half = [
        (0.0, -radius, 1.0),
        (radius, -radius, h),
        (radius, 0.0, 1.0),
        (radius, radius, h),
        (0.0, radius, 1.0),
    ];
    let knots_v: Vec<f64> = [0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0]
        .iter()
        .map(|k| k * meridian)
        .collect();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (&(x, y), &w) in points.iter().zip(&round) {
        for &(rho, z, c) in &half {
            ctrl.push(f.point(x * rho, y * rho, z));
            weights.push(w * c);
        }
    }
    Sheet {
        surface: Surface::Nurbs(
            NurbsSurface::new(2, 2, knots_u, knots_v, 9, 5, ctrl, weights).unwrap(),
        ),
        frame: f,
        shape: Shape::Sphere { radius },
        size: radius,
        small: radius,
    }
}

/// A point near the sheet, by its closed form, with its true foot: `lift`
/// is the height over the sheet as a share of its smaller size.
pub fn point_near(sheet: &Sheet, lift: f64, random: &mut Random) -> (Vec3, Vec3) {
    let f = &sheet.frame;
    match sheet.shape {
        Shape::Plane { length, width } => {
            let (x, y) = (random.between(0.0, length), random.between(0.0, width));
            let h = lift * width * if random.next() < 0.5 { -1.0 } else { 1.0 };
            (f.point(x, y, h), f.point(x, y, 0.0))
        }
        Shape::Cylinder { radius, length } => {
            let (s, c) = random.between(0.0, TAU).psin_cos();
            let z = random.between(0.0, length);
            let h = lift * radius * if random.next() < 0.5 { -1.0 } else { 1.0 };
            let rho = radius + h;
            (
                f.point(rho * c, rho * s, z),
                f.point(radius * c, radius * s, z),
            )
        }
        Shape::Sphere { radius } => {
            let (s, c) = random.between(0.0, TAU).psin_cos();
            let (sl, cl) = random.between(-1.4, 1.4).psin_cos();
            let h = lift * radius * if random.next() < 0.5 { -1.0 } else { 1.0 };
            let at = |rho: f64| f.point(rho * cl * c, rho * cl * s, rho * sl);
            (at(radius + h), at(radius))
        }
    }
}

/// A point on the border of a plane sheet (on one of its four sides),
/// `lift` over it; any other sheet's point as [`point_near`].
pub fn point_on_border(sheet: &Sheet, lift: f64, random: &mut Random) -> (Vec3, Vec3) {
    let Shape::Plane { length, width } = sheet.shape else {
        return point_near(sheet, lift, random);
    };
    let f = &sheet.frame;
    let t = random.next();
    let (x, y) = match (4.0 * random.next()) as usize {
        0 => (length * t, 0.0),
        1 => (length * t, width),
        2 => (0.0, width * t),
        _ => (length, width * t),
    };
    let h = lift * width * if random.next() < 0.5 { -1.0 } else { 1.0 };
    (f.point(x, y, h), f.point(x, y, 0.0))
}

/// The sheets of the long block of `classify_far.rs` (`block_with_weighted_
/// sides`): a plane rectangle as a bicubic spline of six by six control
/// points on knots at thirds, whose weights run from 1e-3 to 1e3, drawn by
/// that test's generator from `state`.
pub fn pin_sheet(place: Place, (length, width): (f64, f64), state: &mut u64) -> Sheet {
    let mut next = || {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*state >> 11) as f64 / (1u64 << 53) as f64
    };
    let size = length.max(width);
    let f = frame(place, size);
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..6 {
        for j in 0..6 {
            ctrl.push(f.point(length * i as f64 / 5.0, width * j as f64 / 5.0, 0.0));
            weights.push((6.0 * (next() - 0.5) * std::f64::consts::LN_10).pexp());
        }
    }
    Sheet {
        surface: Surface::Nurbs(
            NurbsSurface::new(3, 3, knots.clone(), knots, 6, 6, ctrl, weights).unwrap(),
        ),
        frame: f,
        shape: Shape::Plane { length, width },
        size,
        small: length.min(width),
    }
}

/// What a sweep of feet found.
#[derive(Default, Clone, Copy)]
pub struct Tally {
    pub tried: usize,
    /// Feet farther from the true one than [`allowed`] allows.
    pub missed: usize,
    /// Feet farther from the true one than the default tolerance at the
    /// sheet's scale (1e-6 of its smaller size), where the coordinates
    /// hold that.
    pub beyond: usize,
    /// Errors (`project_from`: the walk did not settle).
    pub errors: usize,
    /// The worst distance from the true foot, as a share of the size.
    pub worst: f64,
    /// The work, in units.
    pub units: u64,
}

impl Tally {
    pub fn add(&mut self, o: Tally) {
        self.tried += o.tried;
        self.missed += o.missed;
        self.beyond += o.beyond;
        self.errors += o.errors;
        self.worst = self.worst.max(o.worst);
        self.units += o.units;
    }

    pub fn row(&self, name: &str) -> String {
        format!(
            "{name:<44} {:>6} {:>6} {:>6} {:>5} {:>10.2e} {:>8.1}",
            self.tried,
            self.missed,
            self.beyond,
            self.errors,
            self.worst,
            self.units as f64 / self.tried.max(1) as f64
        )
    }
}

/// What counts as off: a millionth of a millionth of the size, or what
/// the coordinates there can hold.
pub fn allowed(sheet: &Sheet, p: Vec3) -> f64 {
    1e-12 * sheet.size + 256.0 * f64::EPSILON * (p.length() + sheet.size)
}

/// A point near the sheet over the point of its parameters `(u, v)`,
/// with that point, its true foot: `lift` as for [`point_near`].
pub fn point_over(
    sheet: &Sheet,
    (u, v): (f64, f64),
    lift: f64,
    random: &mut Random,
) -> (Vec3, Vec3) {
    let f = &sheet.frame;
    let foot = sheet.surface.eval(u, v);
    let sign = if random.next() < 0.5 { -1.0 } else { 1.0 };
    let (x, y, z) = f.local(foot);
    let (normal, small) = match sheet.shape {
        Shape::Plane { width, .. } => (f.z(), width),
        Shape::Cylinder { radius, .. } => {
            let rho = x.phypot(y);
            (f.vector(x / rho, y / rho, 0.0), radius)
        }
        Shape::Sphere { radius } => {
            let r = (x * x + y * y + z * z).sqrt();
            (f.vector(x / r, y / r, z / r), radius)
        }
    };
    (foot + normal * (sign * lift * small), foot)
}

/// What the parameters can tell apart at `(u, v)`: how far the surface
/// moves for four units in the last place of the larger of the parameter
/// and its domain's ends (where a spline races, the next parameter is a
/// visible way off).
pub fn parameter_rounding(surface: &Surface, (u, v): (f64, f64)) -> f64 {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let (_, su, sv) = surface.derivatives(u, v);
    let reach = |x: f64, (low, high): (f64, f64)| x.abs().max(low.abs()).max(high.abs());
    4.0 * f64::EPSILON * (su.length() * reach(u, (u0, u1)) + sv.length() * reach(v, (v0, v1)))
}

/// How a foot is looked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Search {
    /// `project`, from the grid.
    Whole,
    /// `project_from`, from the true foot's parameters moved by this share
    /// of the domain.
    From(f64),
}

/// The feet of `count` points near `sheet` (every eighth on a border of a
/// plane sheet): their distance from the true foot, and the work.
pub fn feet(sheet: &Sheet, count: usize, search: Search, random: &mut Random) -> Tally {
    let mut tally = Tally::default();
    let ((u0, u1), (v0, v1)) = sheet.surface.domain();
    for k in 0..count {
        let lift = [0.0, 1e-7, 0.05, 0.3][k % 4];
        let (p, foot, found) = match search {
            Search::Whole => {
                let (p, foot) = if k % 8 == 7 {
                    point_on_border(sheet, lift, random)
                } else {
                    point_near(sheet, lift, random)
                };
                (p, foot, allowance::measured(|| sheet.surface.project(p)))
            }
            Search::From(nudge) => {
                let (u, v) = (random.between(u0, u1), random.between(v0, v1));
                let (p, foot) = point_over(sheet, (u, v), lift, random);
                let start = (
                    (u + nudge * (u1 - u0) * random.between(-1.0, 1.0)).clamp(u0, u1),
                    (v + nudge * (v1 - v0) * random.between(-1.0, 1.0)).clamp(v0, v1),
                );
                (
                    p,
                    foot,
                    allowance::measured(|| sheet.surface.project_from(p, start)),
                )
            }
        };
        tally.tried += 1;
        tally.units += found.1;
        match found.0 {
            Ok((u, v)) => {
                let off = sheet.surface.eval(u, v).distance(foot);
                tally.worst = tally.worst.max(off / sheet.size);
                let allowed = allowed(sheet, p) + parameter_rounding(&sheet.surface, (u, v));
                if !(off <= allowed) {
                    tally.missed += 1;
                }
                if !(off <= (1e-6 * sheet.small).max(allowed)) {
                    tally.beyond += 1;
                }
            }
            Err(_) => tally.errors += 1,
        }
    }
    tally
}

/// The sheets of the sweep: (kind, parametrisation, place) and the
/// sheet. `kinds` names the families: `plane`, `cylinder`, `sphere`,
/// `pin`; `scales` the sizes.
pub fn sheets(
    kinds: &str,
    scales: &[f64],
    random: &mut Random,
) -> Vec<(String, String, Place, Sheet)> {
    let mut out = Vec::new();
    let knots = [
        (Knots::Even, Knots::Even),
        (Knots::Stretched(1e3), Knots::Even),
        (Knots::Stretched(1e6), Knots::Even),
        (Knots::Stretched(1e9), Knots::Even),
        (Knots::Even, Knots::Stretched(1e3)),
        (Knots::Even, Knots::Stretched(1e9)),
        (Knots::Clustered, Knots::Clustered),
    ];
    let mut state = 1_u64;
    for &k in scales {
        for place in PLACES {
            if kinds.contains("plane") {
                for aspect in [1.0, 30.0, 1e3, 1e6] {
                    for degrees in [(1, 1), (3, 3), (5, 2)] {
                        for (ku, kv) in knots {
                            for weighted in [false, true] {
                                let sheet = plane(
                                    place,
                                    (aspect * k, k),
                                    degrees,
                                    (3, 2),
                                    (ku, kv),
                                    weighted,
                                    random,
                                );
                                let kind = format!(
                                    "plane {aspect:.0e}:1{}",
                                    if weighted { " weighted" } else { "" }
                                );
                                let how = format!(
                                    "deg {:?} {}/{} k {k:.0e} {}",
                                    degrees,
                                    ku.name(),
                                    kv.name(),
                                    place.name
                                );
                                out.push((kind, how, place, sheet));
                            }
                        }
                    }
                }
            }
            if kinds.contains("cylinder") {
                for aspect in [1.0, 30.0, 1e3, 1e6] {
                    for (ku, kv) in knots {
                        let around = match ku {
                            Knots::Stretched(s) => s,
                            _ => 1.0,
                        };
                        for weighted in [false, true] {
                            let sheet = cylinder(
                                place,
                                (k, aspect * k),
                                around,
                                3,
                                3,
                                kv,
                                weighted,
                                random,
                            );
                            let kind = format!(
                                "cylinder {aspect:.0e}:1{}",
                                if weighted { " weighted" } else { "" }
                            );
                            let how =
                                format!("{}/{} k {k:.0e} {}", ku.name(), kv.name(), place.name);
                            out.push((kind, how, place, sheet));
                        }
                    }
                }
            }
            if kinds.contains("sphere") {
                for (around, meridian) in [(1.0, 1.0), (1e6, 1.0), (1.0, 1e6), (1e9, 1e-3)] {
                    let sheet = sphere(place, k, (around, meridian));
                    let how = format!("{around:.0e}/{meridian:.0e} k {k:.0e} {}", place.name);
                    out.push(("sphere".into(), how, place, sheet));
                }
            }
            if kinds.contains("pin") {
                for (length, width) in [(30.0, 30.0), (30.0, 900.0), (900.0, 20.0)] {
                    for _ in 0..4 {
                        let sheet = pin_sheet(place, (length * k, width * k), &mut state);
                        let kind = format!("pin's sheet {length}x{width}");
                        let how = format!("k {k:.0e} {}", place.name);
                        out.push((kind, how, place, sheet));
                    }
                }
            }
        }
    }
    out
}

/// The sweep as a table: per kind, then per place, then per kind and
/// parametrisation where a foot was missed (when `detail`).
pub fn table(kinds: &str, scales: &[f64], count: usize, search: Search, detail: bool) -> String {
    let mut random = Random(0x5EED_F007);
    let mut by_kind: Vec<(String, Tally)> = Vec::new();
    let mut by_place: Vec<(String, Tally)> = Vec::new();
    let mut rows = String::new();
    for (kind, how, place, sheet) in sheets(kinds, scales, &mut random) {
        let tally = feet(&sheet, count, search, &mut random);
        if detail && (tally.missed > 0 || tally.errors > 0) {
            let _ = writeln!(rows, "  {}", tally.row(&format!("{kind} {how}")));
        }
        for (key, list) in [
            (kind, &mut by_kind),
            (place.name.to_string(), &mut by_place),
        ] {
            match list.iter_mut().find(|(k, _)| *k == key) {
                Some((_, t)) => t.add(tally),
                None => list.push((key, tally)),
            }
        }
    }
    let mut out = format!(
        "{:<44} {:>6} {:>6} {:>6} {:>5} {:>10} {:>8}\n",
        "kind", "tried", "missed", "> tol", "err", "worst/size", "units"
    );
    for (key, tally) in by_kind.iter().chain(&by_place) {
        let _ = writeln!(out, "{}", tally.row(key));
    }
    out + &rows
}

fn env<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

/// The table of the feet, by hand: `PROJECT_SWEEP` the families (all four
/// by default), `PROJECT_SWEEP_POINTS` the points per sheet (16),
/// `PROJECT_SWEEP_FROM` a share of the domain to start `project_from` that
/// far from the true foot (by default `project`), `PROJECT_SWEEP_SCALE` one
/// scale (by default 1e-3, 1 and 1e6).
#[test]
#[ignore = "a sweep, run by hand (a minute or two)"]
fn the_feet_of_points_near_lopsided_sheets_table() {
    let kinds: String = env("PROJECT_SWEEP").unwrap_or_else(|| "plane,cylinder,sphere,pin".into());
    let count = env("PROJECT_SWEEP_POINTS").unwrap_or(16);
    let search = env("PROJECT_SWEEP_FROM").map_or(Search::Whole, Search::From);
    let scales = env::<f64>("PROJECT_SWEEP_SCALE").map_or(vec![1e-3, 1.0, 1e6], |k| vec![k]);
    println!("{}", table(&kinds, &scales, count, search, true));
}
