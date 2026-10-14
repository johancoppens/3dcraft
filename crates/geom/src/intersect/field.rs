//! Surfaces as distance fields, and as charts with derivative bounds.
//!
//! The numeric intersection walks one surface in its parameters (the
//! [`Chart`]) and measures how far each point is from the other (the
//! [`Field`]). For the analytic kinds the field is the exact signed distance
//! in closed form; for Bézier and NURBS surfaces it is the distance to the
//! closest point, found by Gauss–Newton from the nearest of a grid of
//! samples.

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::closure::closes;
use crate::pole::{Pole, poles};
use crate::{BezierPatch, BezierSpan, DerivativeBounds, GeomError, NurbsSurface, Sphere, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};
use std::f64::consts::{FRAC_PI_2, TAU};
use std::sync::{Arc, Mutex, PoisonError};

/// What a field knows at a point.
#[derive(Clone, Copy, Debug)]
pub(super) struct Sample {
    /// Distance to the surface, positive on the side its normal points to.
    pub signed: f64,
    /// Distance to the closest point of the surface; larger than `|signed|`
    /// only past the edge of a bounded surface.
    pub dist: f64,
    /// The gradient of `signed`: the unit normal at the closest point.
    pub normal: Vec3,
}

/// A grid of samples of a freeform surface, to start closest-point searches.
pub(super) struct Projector {
    surface: Surface,
    u: (f64, f64),
    v: (f64, f64),
    grid: Vec<(f64, f64, Vec3)>,
    /// An estimate of the largest normal curvature (not a bound).
    curvature: f64,
    /// The same estimate per grid node: from how fast the normal turns
    /// along the grid lines that meet there.
    turning: Vec<f64>,
    /// The longest step between neighbouring grid nodes.
    spacing: f64,
    /// The borders of the surface that are one point.
    poles: Vec<Pole>,
    /// The grid's nodes in order of their x coordinate (ties by index):
    /// the nodes within some distance of a point lie in a window of it.
    by_x: Vec<(f64, usize)>,
}

const PROJECTOR_GRID: u32 = 24;

/// Projectors made lately, by the surface they are of (written out in
/// full, so that two surfaces are the same key only when every number of
/// them is, bit for bit), the most recently used last.
static SHELF: Mutex<Vec<(String, Arc<Projector>)>> = Mutex::new(Vec::new());
/// How many projectors are kept.
const SHELF_SIZE: usize = 32;

impl Projector {
    /// The projector of `surface`, shared with every intersection before
    /// that asked for the same surface: a projector is a function of its
    /// surface alone, and the faces of one body, intersected with each of
    /// the faces of another, ask for the same few many times over.
    pub(super) fn shared(surface: &Surface) -> Result<Arc<Self>, GeomError> {
        let key = format!("{surface:?}");
        {
            let mut shelf = SHELF.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(at) = shelf.iter().position(|(k, _)| *k == key) {
                let entry = shelf.remove(at);
                let projector = Arc::clone(&entry.1);
                shelf.push(entry);
                return Ok(projector);
            }
        }
        let projector = Arc::new(Projector::new(surface)?);
        let mut shelf = SHELF.lock().unwrap_or_else(PoisonError::into_inner);
        if !shelf.iter().any(|(k, _)| *k == key) {
            shelf.push((key, Arc::clone(&projector)));
        }
        while shelf.len() > SHELF_SIZE {
            let _forgotten = shelf.remove(0);
        }
        Ok(projector)
    }

    pub(super) fn new(surface: &Surface) -> Result<Self, GeomError> {
        let (u, v) = surface.domain();
        if ![u.0, u.1, v.0, v.1].iter().all(|x| x.is_finite()) {
            return Err(GeomError::Unsupported(
                "a freeform surface with an unbounded domain",
            ));
        }
        let n = PROJECTOR_GRID;
        let poles = poles(surface);
        // At a pole the surface has the normal it is reached with along
        // the grid line through it; its own formula has none there.
        let normal_at = |a: f64, b: f64| match poles.iter().find(|pole| pole.holds(surface, a, b)) {
            Some(pole) => pole.normal_along(surface, if pole.across_v() { a } else { b }),
            None => surface.normal(a, b),
        };
        let mut grid = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
        let mut normals = Vec::with_capacity(grid.capacity());
        // The pole each node is on, if any.
        let mut at_pole: Vec<Option<usize>> = Vec::with_capacity(grid.capacity());
        for i in 0..=n {
            for j in 0..=n {
                let a = u.0 + (u.1 - u.0) * f64::from(i) / f64::from(n);
                let b = v.0 + (v.1 - v.0) * f64::from(j) / f64::from(n);
                let p = surface.eval(a, b);
                if !p.is_finite() {
                    return Err(GeomError::InvalidParameter(
                        "a surface with a non-finite point",
                    ));
                }
                grid.push((a, b, p));
                normals.push(normal_at(a, b));
                at_pole.push(poles.iter().position(|pole| pole.holds(surface, a, b)));
            }
        }
        // Curvature from how fast the normal turns between neighbours.
        let stride = (n + 1) as usize;
        let mut curvature = 0.0_f64;
        let mut turning = vec![0.0_f64; grid.len()];
        let mut spacing = 0.0_f64;
        for (k, (&(_, _, p), n0)) in grid.iter().zip(&normals).enumerate() {
            for step in [1, stride] {
                if step == 1 && (k + 1) % stride == 0 {
                    continue;
                }
                let (Some(&(_, _, q)), Some(n1)) = (grid.get(k + step), normals.get(k + step))
                else {
                    continue;
                };
                // Two nodes on one pole are one point (apart by rounding
                // only), each with the limit normal of its own meridian:
                // how far apart those normals are is no curvature, and
                // divided by a distance of rounding it would be taken for
                // one of ten million per unit length, which no cell near
                // the pole would ever be simple enough for.
                if let (Some(Some(a)), Some(Some(b))) = (at_pole.get(k), at_pole.get(k + step))
                    && a == b
                {
                    continue;
                }
                let apart = p.distance(q);
                spacing = spacing.max(apart);
                let (Some(n0), Some(n1)) = (n0, n1) else {
                    continue;
                };
                if apart > 0.0 {
                    let turn = n0.distance(*n1) / apart;
                    curvature = curvature.max(turn);
                    for node in [k, k + step] {
                        if let Some(slot) = turning.get_mut(node) {
                            *slot = slot.max(turn);
                        }
                    }
                }
            }
        }
        let mut by_x: Vec<(f64, usize)> = grid
            .iter()
            .enumerate()
            .map(|(k, &(_, _, p))| (p.x, k))
            .collect();
        by_x.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        Ok(Self {
            by_x,
            surface: surface.clone(),
            u,
            v,
            grid,
            curvature: 2.0 * curvature,
            turning,
            spacing,
            poles,
        })
    }

    /// The estimate of the largest normal curvature of the part of the
    /// surface within `radius` of `p`: twice the fastest turn of the
    /// normal along the grid lines there (every grid cell that reaches
    /// into the ball has a corner within two steps of it), and the
    /// estimate of the whole surface where no node is that near.
    fn curvature_near(&self, p: Vec3, radius: f64) -> f64 {
        let within = radius + 2.0 * self.spacing;
        let mut most: Option<f64> = None;
        let mut take = |k: usize| {
            if let (Some(&(_, _, q)), Some(&turn)) = (self.grid.get(k), self.turning.get(k))
                && q.distance(p) <= within
            {
                most = Some(most.map_or(turn, |m: f64| m.max(turn)));
            }
        };
        // Only the nodes whose x is within reach can be: one further
        // along x than a little over `within` is further than `within`
        // whatever rounding does. (The largest of the turns taken does not
        // depend on the order they are taken in.)
        let reach = within * (1.0 + 1e-9);
        if reach.is_finite() && p.x.is_finite() {
            // (The difference as the distance has it, which rounds the
            // same way for every node: a node is passed over only when the
            // x of its offset from `p` is beyond the reach.)
            let first = self.by_x.partition_point(|&(x, _)| x - p.x < -reach);
            for &(x, k) in self.by_x.iter().skip(first) {
                if x - p.x > reach {
                    break;
                }
                take(k);
            }
        } else {
            for k in 0..self.grid.len() {
                take(k);
            }
        }
        most.map_or(self.curvature, |m| 2.0 * m)
    }

    /// The node of the grid nearest to `p`: the first of them in the
    /// grid's order when several are as near. `None` when none is nearer
    /// than infinity.
    fn nearest_node(&self, p: Vec3) -> Option<(f64, f64, Vec3)> {
        // From the place of `p` in the order along x outwards, both ways:
        // a node whose x alone is further than the nearest so far is
        // further whatever its other coordinates (the squared distance is
        // a sum of squares, and rounding keeps a sum at least as large as
        // any of its terms), and so is every node beyond it.
        let mut best: (f64, usize) = (f64::INFINITY, usize::MAX);
        let consider = |k: usize, best: &mut (f64, usize)| -> bool {
            let Some(&(_, _, q)) = self.grid.get(k) else {
                return true;
            };
            let d = q - p;
            if d.x * d.x > best.0 {
                return false;
            }
            let squared = d.length_squared();
            if squared < best.0 || (squared == best.0 && best.1 != usize::MAX && k < best.1) {
                *best = (squared, k);
            }
            true
        };
        let split = self.by_x.partition_point(|&(x, _)| x < p.x);
        let (mut left, mut right) = (split, split);
        let (mut left_open, mut right_open) = (true, true);
        while left_open || right_open {
            if right_open {
                match self.by_x.get(right) {
                    Some(&(_, k)) if consider(k, &mut best) => right += 1,
                    _ => right_open = false,
                }
            }
            if left_open {
                match left.checked_sub(1).and_then(|l| self.by_x.get(l)) {
                    Some(&(_, k)) if consider(k, &mut best) => left -= 1,
                    _ => left_open = false,
                }
            }
        }
        self.grid.get(best.1).copied()
    }

    /// Gauss–Newton from `(u, v)` towards the point closest to `p`.
    ///
    /// At a pole the derivative along its border is zero, so Newton
    /// cannot turn from the meridian it is on to another: from a pole, or
    /// to one, it keeps to the meridian it started on, and for a point on
    /// the far side of the pole from that meridian it stops at the pole,
    /// which is then taken for the nearest point when the surface comes
    /// nearer elsewhere (a point a millimetre from the pole of a dome was
    /// a millimetre from the dome). An end on a pole is tried again from
    /// the meridian that leaves the pole towards `p`, and the nearer of
    /// the two kept.
    pub(super) fn foot_from(&self, p: Vec3, u: f64, v: f64) -> (f64, f64, Vec3) {
        let first = self.newton(p, u, v, None);
        let row = 1.0 / f64::from(PROJECTOR_GRID);
        let Some(pole) = self
            .poles
            .iter()
            .find(|pole| !(pole.share_off(&self.surface, first.0, first.1) > row))
        else {
            return first;
        };
        let along = pole.meridian_towards(&self.surface, p);
        let (a, b) = pole.off_pole(&self.surface, along, row);
        // Along the meridian first: next to the pole the derivative along
        // the border is small, and a step in both parameters may throw the
        // point onto another meridian, and back onto the pole.
        let down = self.newton(p, a, b, Some(!pole.across_v()));
        let polished = self.newton(p, down.0, down.1, None);
        let again = if (polished.2 - p).length_squared() < (down.2 - p).length_squared() {
            polished
        } else {
            down
        };
        if (again.2 - p).length_squared() < (first.2 - p).length_squared() {
            again
        } else {
            first
        }
    }

    /// Gauss–Newton from `(u, v)` towards the point closest to `p`.
    /// With `only`, along one parameter: `Some(true)` u, `Some(false)` v.
    fn newton(&self, p: Vec3, u: f64, v: f64, only: Option<bool>) -> (f64, f64, Vec3) {
        let (mut u, mut v) = (u.clamp(self.u.0, self.u.1), v.clamp(self.v.0, self.v.1));
        let (span_u, span_v) = (self.u.1 - self.u.0, self.v.1 - self.v.0);
        for _ in 0..40 {
            let (s, su, sv) = self.surface.derivatives(u, v);
            let r = p - s;
            let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
            let det = a * c - b * b;
            let (ru, rv) = (r.dot(su), r.dot(sv));
            let (du, dv) = if let Some(along_u) = only {
                match (along_u, a > 0.0, c > 0.0) {
                    (true, true, _) => (ru / a, 0.0),
                    (false, _, true) => (0.0, rv / c),
                    _ => break,
                }
            } else if det.is_finite() && det > 1e-14 * a * c {
                ((c * ru - b * rv) / det, (a * rv - b * ru) / det)
            } else if a > 0.0 && a >= c {
                (ru / a, 0.0) // A collapsed edge: move along the direction left.
            } else if c > 0.0 {
                (0.0, rv / c)
            } else {
                break;
            };
            let next_u = (u + du).clamp(self.u.0, self.u.1);
            let next_v = (v + dv).clamp(self.v.0, self.v.1);
            let moved = (next_u - u).abs() / span_u + (next_v - v).abs() / span_v;
            (u, v) = (next_u, next_v);
            // The distance is stationary at the foot: its error is the
            // square of what is left here.
            if !(moved > 1e-11) {
                break;
            }
        }
        (u, v, self.surface.eval(u, v))
    }

    /// The closest point of the surface to `p`: `(u, v, point)`.
    pub(super) fn foot(&self, p: Vec3) -> (f64, f64, Vec3) {
        let best = match self.nearest_node(p) {
            Some((u, v, q)) => ((q - p).length_squared(), u, v, q),
            None => (f64::INFINITY, 0.0, 0.0, p),
        };
        let (u, v, q) = self.foot_from(p, best.1, best.2);
        if (q - p).length_squared() <= best.0 {
            (u, v, q)
        } else {
            (best.1, best.2, best.3)
        }
    }

    fn sample(&self, p: Vec3) -> Option<Sample> {
        let (u, v, q) = self.foot(p);
        let d = p - q;
        let dist = d.length();
        // A foot at a pole: the normal of the meridian that leaves it
        // nearest to the direction of `p` (`Pole::normal_towards` says
        // why that one tells the side).
        let normal = match self
            .poles
            .iter()
            .find(|pole| pole.holds(&self.surface, u, v))
        {
            Some(pole) => pole.normal_towards(&self.surface, p),
            None => self.surface.normal(u, v),
        };
        match normal {
            Some(normal) => Some(Sample {
                signed: d.dot(normal),
                dist,
                normal,
            }),
            // No normal at the foot (a collapsed patch): the direction to it.
            None => d.normalized().map(|normal| Sample {
                signed: dist,
                dist,
                normal,
            }),
        }
    }
}

/// The signed distance to a surface.
pub(super) enum Field {
    Plane {
        origin: Vec3,
        normal: Vec3,
    },
    Cylinder {
        frame: Frame,
        radius: f64,
    },
    /// `axis` points from the apex into the cone; `sin`, `cos` are of the
    /// (positive) half-angle.
    Cone {
        apex: Vec3,
        axis: Vec3,
        x: Vec3,
        sin: f64,
        cos: f64,
    },
    Sphere {
        center: Vec3,
        x: Vec3,
        radius: f64,
    },
    Torus {
        frame: Frame,
        major: f64,
        minor: f64,
    },
    Free(Arc<Projector>),
}

impl Field {
    pub(super) fn new(surface: &Surface) -> Result<Self, GeomError> {
        Ok(match surface {
            Surface::Plane(p) => Field::Plane {
                origin: p.frame().origin(),
                normal: p.frame().z(),
            },
            Surface::Cylinder(c) => Field::Cylinder {
                frame: *c.frame(),
                radius: c.radius(),
            },
            Surface::Cone(c) => {
                let (apex, axis) = cone_apex_axis(c);
                let (sin, cos) = c.half_angle().abs().sin_cos();
                Field::Cone {
                    apex,
                    axis,
                    x: c.frame().x(),
                    sin,
                    cos,
                }
            }
            Surface::Sphere(s) => Field::Sphere {
                center: s.frame().origin(),
                x: s.frame().x(),
                radius: s.radius(),
            },
            Surface::Torus(t) => Field::Torus {
                frame: *t.frame(),
                major: t.radii().0,
                minor: t.radii().1,
            },
            Surface::Bezier(_) | Surface::Nurbs(_) => Field::Free(Projector::shared(surface)?),
        })
    }

    /// `None` for a non-finite point.
    pub(super) fn sample(&self, p: Vec3) -> Option<Sample> {
        if !p.is_finite() {
            return None;
        }
        let exact = |signed: f64, normal: Vec3| Sample {
            signed,
            dist: signed.abs(),
            normal,
        };
        let sample = match self {
            Field::Plane { origin, normal } => exact((p - *origin).dot(*normal), *normal),
            Field::Cylinder { frame, radius } => {
                let (x, y, _) = frame.local(p);
                let rho = x.hypot(y);
                let radial = if rho > 0.0 {
                    frame.vector(x / rho, y / rho, 0.0)
                } else {
                    frame.x()
                };
                exact(rho - radius, radial)
            }
            Field::Cone {
                apex,
                axis,
                x,
                sin,
                cos,
            } => {
                let q = p - *apex;
                let h = q.dot(*axis);
                let w = q - *axis * h;
                let rho = w.length();
                let radial = if rho > 0.0 { w * (1.0 / rho) } else { *x };
                if rho * sin + h * cos >= 0.0 {
                    exact(rho * cos - h * sin, radial * *cos - *axis * *sin)
                } else {
                    // Behind the apex: the apex is the closest point.
                    let dist = q.length();
                    Sample {
                        signed: dist,
                        dist,
                        normal: q.normalized().unwrap_or(-*axis),
                    }
                }
            }
            Field::Sphere { center, x, radius } => {
                let q = p - *center;
                let d = q.length();
                exact(d - radius, q.normalized().unwrap_or(*x))
            }
            Field::Torus {
                frame,
                major,
                minor,
            } => {
                let (x, y, z) = frame.local(p);
                let rho = x.hypot(y);
                let radial = if rho > 0.0 {
                    frame.vector(x / rho, y / rho, 0.0)
                } else {
                    frame.x()
                };
                let off = rho - major;
                let d = off.hypot(z);
                let normal = if d > 0.0 {
                    radial * (off / d) + frame.z() * (z / d)
                } else {
                    radial
                };
                exact(d - minor, normal)
            }
            Field::Free(projector) => projector.sample(p)?,
        };
        (sample.signed.is_finite() && sample.dist.is_finite() && sample.normal.is_finite())
            .then_some(sample)
    }

    /// A bound on how fast the field's gradient turns (the norm of its
    /// second derivative) within `reach` of `p`; `None` where there is none
    /// (near an axis, a centre or an apex). An estimate for freeform
    /// surfaces.
    /// `dist` is the distance from `p` to the surface (the `dist` of its
    /// [`Sample`]): a freeform surface then needs no second search for
    /// the closest point.
    pub(super) fn bend_at(&self, p: Vec3, reach: f64, dist: f64) -> Option<f64> {
        let within = |radius: f64| (radius > 2.0 * reach).then(|| 1.0 / (radius - reach));
        match self {
            Field::Plane { .. } => Some(0.0),
            Field::Cylinder { frame, .. } => {
                let (x, y, _) = frame.local(p);
                within(x.hypot(y))
            }
            Field::Cone {
                apex,
                axis,
                sin,
                cos,
                ..
            } => {
                let q = p - *apex;
                let h = q.dot(*axis);
                let rho = (q - *axis * h).length();
                if rho * sin + h * cos <= 2.0 * reach {
                    return None;
                }
                within(rho)
            }
            Field::Sphere { center, .. } => within(p.distance(*center)),
            Field::Torus { frame, major, .. } => {
                let (x, y, z) = frame.local(p);
                let rho = x.hypot(y);
                let tube = within((rho - major).hypot(z))?;
                Some(tube.max(within(rho)?))
            }
            // The closest points of everything within `reach` of `p` lie
            // within `2 (dist + reach)` of the closest point of `p`, so
            // within `3 dist + 2 reach` of `p`. At a distance `d` from a
            // surface of curvature `k` the field bends by `k / (1 − k d)`,
            // at most `2 k` here; that factor is kept whole as the margin
            // of an estimate (with `k / (1 − k d)` instead, a branch of
            // parallel/dome-dome/001 of the boolean corpus was missed).
            Field::Free(projector) => {
                let k = projector.curvature_near(p, 3.0 * dist + 2.0 * reach);
                let away = k * (dist + reach);
                (away < 0.5).then_some(2.0 * k)
            }
        }
    }

    /// The apex, when the field is a cone's.
    pub(super) fn apex(&self) -> Option<Vec3> {
        match self {
            Field::Cone { apex, .. } => Some(*apex),
            _ => None,
        }
    }
}

/// The apex of a cone and the unit axis from it into the cone.
pub(super) fn cone_apex_axis(c: &crate::Cone) -> (Vec3, Vec3) {
    let f = c.frame();
    let apex = f.point(0.0, 0.0, c.apex_v());
    let axis = if c.half_angle() > 0.0 { f.z() } else { -f.z() };
    (apex, axis)
}

/// Bounds on the lengths of a chart's partial derivatives.
#[derive(Clone, Copy, Debug)]
pub(super) struct Bounds {
    pub lu: f64,
    pub lv: f64,
    pub muu: f64,
    pub muv: f64,
    pub mvv: f64,
}

/// A surface with the rectangle of its parameters that covers everything of
/// it inside a box.
pub(super) struct Chart {
    pub surface: Surface,
    pub u: (f64, f64),
    pub v: (f64, f64),
    pub periodic_u: bool,
    pub periodic_v: bool,
    pub bounds: Bounds,
}

impl Chart {
    /// The point and first derivatives of the surface at `(u, v)`. A
    /// freeform surface that closes on itself is walked around, past the
    /// ends of its domain, and is no surface there: its parameters are
    /// taken around into the domain (the analytic kinds go around by
    /// their formulas). Elsewhere `(u, v)` as given, beyond the domain
    /// too (the marcher corrects onto what lies past a border).
    pub(super) fn at(&self, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
        let (u, v) = self.wrapped(u, v);
        self.surface.derivatives(u, v)
    }

    /// `(u, v)` taken around into the domain where a freeform surface
    /// closes on itself (see [`Chart::at`]).
    pub(super) fn wrapped(&self, u: f64, v: f64) -> (f64, f64) {
        if matches!(self.surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
            let around = |x: f64, periodic: bool, (lo, hi): (f64, f64)| {
                if periodic && x.is_finite() && !(x >= lo && x <= hi) {
                    (lo + (x - lo).rem_euclid(hi - lo)).clamp(lo, hi)
                } else {
                    x
                }
            };
            (
                around(u, self.periodic_u, self.u),
                around(v, self.periodic_v, self.v),
            )
        } else {
            (u, v)
        }
    }

    /// `None` when no part of the surface can be inside the box. `other`
    /// is the field of the surface it will be intersected with (a sphere is
    /// turned so that its poles are away from it).
    pub(super) fn new(
        surface: &Surface,
        within: &Bbox3,
        tol: &Tolerance,
        other: &Field,
    ) -> Result<Option<Self>, GeomError> {
        let Some((lo, hi)) = within.corners() else {
            return Ok(None);
        };
        let corners = || {
            (0..8).map(move |k| {
                Vec3::new(
                    if k & 1 == 0 { lo.x } else { hi.x },
                    if k & 2 == 0 { lo.y } else { hi.y },
                    if k & 4 == 0 { lo.z } else { hi.z },
                )
            })
        };
        let center = lo.lerp(hi, 0.5);
        let half = lo.distance(hi) * 0.5;
        let margin = tol.linear;
        // The range of a coordinate over the corners, grown by the margin.
        let range = |f: &dyn Fn(Vec3) -> f64| {
            let (mut a, mut b) = (f64::INFINITY, f64::NEG_INFINITY);
            for c in corners() {
                let x = f(c);
                a = a.min(x);
                b = b.max(x);
            }
            (a - margin, b + margin)
        };
        // The angles about an axis a box can reach, when it is clear of it.
        let turn = |frame: &Frame| {
            let (x, y, _) = frame.local(center);
            let away = x.hypot(y);
            if away > half * 1.001 + margin && away > 0.0 {
                let spread = ((half + margin) / away).min(1.0).asin() * 1.001 + 1e-9;
                let mid = y.atan2(x);
                ((mid - spread, mid + spread), false)
            } else {
                ((0.0, TAU), true)
            }
        };
        let chart = match surface {
            Surface::Plane(p) => {
                let f = *p.frame();
                Chart {
                    surface: surface.clone(),
                    u: range(&|c| f.local(c).0),
                    v: range(&|c| f.local(c).1),
                    periodic_u: false,
                    periodic_v: false,
                    bounds: Bounds {
                        lu: 1.0,
                        lv: 1.0,
                        muu: 0.0,
                        muv: 0.0,
                        mvv: 0.0,
                    },
                }
            }
            Surface::Cylinder(c) => {
                let f = *c.frame();
                let (u, periodic_u) = turn(&f);
                let r = c.radius();
                Chart {
                    surface: surface.clone(),
                    u,
                    v: range(&|c| f.local(c).2),
                    periodic_u,
                    periodic_v: false,
                    bounds: Bounds {
                        lu: r,
                        lv: 1.0,
                        muu: r,
                        muv: 0.0,
                        mvv: 0.0,
                    },
                }
            }
            Surface::Cone(c) => {
                let f = *c.frame();
                let (u, periodic_u) = turn(&f);
                let (mut v0, mut v1) = range(&|c| f.local(c).2);
                // Stop just short of the apex, where the chart collapses.
                let slope = c.half_angle().tan();
                let short = 0.25 * tol.linear * c.half_angle().cos().abs() / slope.abs();
                if c.half_angle() > 0.0 {
                    v0 = v0.max(c.apex_v() + short);
                } else {
                    v1 = v1.min(c.apex_v() - short);
                }
                if !(v0 < v1) {
                    return Ok(None);
                }
                let widest = (c.radius() + v0 * slope).max(c.radius() + v1 * slope);
                Chart {
                    surface: surface.clone(),
                    u,
                    v: (v0, v1),
                    periodic_u,
                    periodic_v: false,
                    bounds: Bounds {
                        lu: widest,
                        lv: 1.0 / c.half_angle().cos().abs(),
                        muu: widest,
                        muv: slope.abs(),
                        mvv: 0.0,
                    },
                }
            }
            Surface::Sphere(s) => {
                // Any axis through the centre gives the same sphere: take the
                // one whose poles are furthest from the other surface, so
                // that no curve runs through a pole.
                let f = *s.frame();
                let r = s.radius();
                let candidates = [
                    (f.z(), f.x()),
                    (f.x(), f.y()),
                    (f.y(), f.z()),
                    (f.vector(1.0, 1.0, 1.0), f.x()),
                    (f.vector(1.0, -1.0, 1.0), f.x()),
                    (f.vector(-1.0, 1.0, 1.0), f.x()),
                    (f.vector(1.0, 1.0, -1.0), f.x()),
                ];
                let mut best: Option<(f64, Frame)> = None;
                for (z, x) in candidates {
                    let Some(frame) = Frame::new(f.origin(), z, x) else {
                        continue;
                    };
                    let clear = [1.0, -1.0]
                        .iter()
                        .map(|&side| {
                            other
                                .sample(frame.point(0.0, 0.0, side * r))
                                .map_or(0.0, |s| s.dist)
                        })
                        .fold(f64::INFINITY, f64::min);
                    if best.is_none_or(|(c, _)| clear > c) {
                        best = Some((clear, frame));
                    }
                }
                let frame = best.map_or(f, |(_, frame)| frame);
                Chart {
                    surface: Surface::Sphere(Sphere::new(frame, r)?),
                    u: (0.0, TAU),
                    v: (-FRAC_PI_2, FRAC_PI_2),
                    periodic_u: true,
                    periodic_v: false,
                    bounds: Bounds {
                        lu: r,
                        lv: r,
                        muu: r,
                        muv: r,
                        mvv: r,
                    },
                }
            }
            Surface::Torus(t) => {
                let (major, minor) = t.radii();
                Chart {
                    surface: surface.clone(),
                    u: (0.0, TAU),
                    v: (0.0, TAU),
                    periodic_u: true,
                    periodic_v: true,
                    bounds: Bounds {
                        lu: major + minor,
                        lv: minor,
                        muu: major + minor,
                        muv: minor,
                        mvv: minor,
                    },
                }
            }
            Surface::Bezier(b) => {
                let p = |i: usize, j: usize| b.control_point(i, j).unwrap_or(Vec3::ZERO);
                let (m, n) = (b.degree_u(), b.degree_v());
                let (mut du, mut dv) = (0.0_f64, 0.0_f64);
                for i in 0..=m {
                    for j in 0..=n {
                        if i < m {
                            du = du.max(p(i + 1, j).distance(p(i, j)));
                        }
                        if j < n {
                            dv = dv.max(p(i, j + 1).distance(p(i, j)));
                        }
                    }
                }
                let (muu, muv, mvv) = b.second_derivative_bounds();
                let [periodic_u, periodic_v] = closes(surface);
                Chart {
                    surface: surface.clone(),
                    u: (0.0, 1.0),
                    v: (0.0, 1.0),
                    periodic_u,
                    periodic_v,
                    bounds: Bounds {
                        lu: m as f64 * du,
                        lv: n as f64 * dv,
                        muu,
                        muv,
                        mvv,
                    },
                }
            }
            Surface::Nurbs(s) => {
                let (u, v) = s.domain();
                let (count_u, count_v) = s.counts();
                // A surface that closes on itself is walked around: its
                // parameters are wrapped into the domain where it is
                // evaluated (`Chart::at`).
                let [periodic_u, periodic_v] = closes(surface);
                Chart {
                    surface: surface.clone(),
                    u,
                    v,
                    periodic_u,
                    periodic_v,
                    bounds: nurbs_bounds(s, surface, u, v, count_u, count_v),
                }
            }
        };
        let chart = short_of_poles(chart, tol);
        let finite = [chart.u.0, chart.u.1, chart.v.0, chart.v.1]
            .iter()
            .all(|x| x.is_finite());
        let b = chart.bounds;
        if !(finite
            && [b.lu, b.lv, b.muu, b.muv, b.mvv]
                .iter()
                .all(|x| x.is_finite()))
        {
            return Err(GeomError::InvalidParameter(
                "a surface or box that is not finite",
            ));
        }
        Ok((chart.u.0 < chart.u.1 && chart.v.0 < chart.v.1).then_some(chart))
    }
}

/// The chart of a freeform surface stopped just short of its poles, as a
/// cone's is of its apex: at a border that is one point the chart
/// collapses (every parameter along the border is the same point, and a
/// surface through that point is zero all along it). The border is moved
/// in by a quarter of the linear tolerance over the bound on the speed
/// across it, so that everything left out is within a quarter of the
/// tolerance of the pole (a point moves no further than its speed allows),
/// and a curve that runs into the pole ends within that of it.
fn short_of_poles(mut chart: Chart, tol: &Tolerance) -> Chart {
    for pole in poles(&chart.surface) {
        let (range, speed) = if pole.across_v() {
            (&mut chart.v, chart.bounds.lv)
        } else {
            (&mut chart.u, chart.bounds.lu)
        };
        let short = 0.25 * tol.linear / speed;
        // A surface no larger than the tolerance keeps its chart: nothing
        // of it would be left.
        if !(short > 0.0 && short < 0.25 * (range.1 - range.0)) {
            continue;
        }
        match pole.border {
            crate::pole::Border::UStart | crate::pole::Border::VStart => range.0 += short,
            crate::pole::Border::UEnd | crate::pole::Border::VEnd => range.1 -= short,
        }
    }
    chart
}

/// Derivative bounds of a surface, estimated from samples (four per control
/// point and direction) with a margin. Not a proof: the last resort of
/// `nurbs_bounds`.
fn sampled_bounds(
    surface: &Surface,
    u: (f64, f64),
    v: (f64, f64),
    count_u: usize,
    count_v: usize,
) -> Bounds {
    let nu = (count_u.saturating_mul(4)).clamp(8, 64);
    let nv = (count_v.saturating_mul(4)).clamp(8, 64);
    let (hu, hv) = ((u.1 - u.0) / nu as f64, (v.1 - v.0) / nv as f64);
    let at = |i: usize, j: usize| {
        let (_, su, sv) = surface.derivatives(u.0 + hu * i as f64, v.0 + hv * j as f64);
        (su, sv)
    };
    let mut b = Bounds {
        lu: 0.0,
        lv: 0.0,
        muu: 0.0,
        muv: 0.0,
        mvv: 0.0,
    };
    for i in 0..=nu {
        for j in 0..=nv {
            let (su, sv) = at(i, j);
            b.lu = b.lu.max(su.length());
            b.lv = b.lv.max(sv.length());
            if i < nu {
                let (next_u, next_v) = at(i + 1, j);
                b.muu = b.muu.max(next_u.distance(su) / hu);
                b.muv = b.muv.max(next_v.distance(sv) / hu);
            }
            if j < nv {
                let (next_u, next_v) = at(i, j + 1);
                b.muv = b.muv.max(next_u.distance(su) / hv);
                b.mvv = b.mvv.max(next_v.distance(sv) / hv);
            }
        }
    }
    Bounds {
        lu: 1.5 * b.lu,
        lv: 1.5 * b.lv,
        muu: 2.0 * b.muu,
        muv: 2.0 * b.muv,
        mvv: 2.0 * b.mvv,
    }
}

/// How often each Bézier span of a rational surface is quartered for its
/// derivative bounds: the bounds of a rational piece tighten as it shrinks
/// (its weights even out).
const RATIONAL_QUARTERINGS: usize = 2;

/// Derivative bounds of a rational surface, proven: the largest of the
/// bounds of the pieces of its spans
/// ([`RationalBezierPatch::derivative_bounds`]), each scaled by the part
/// of the surface's domain the piece covers. `None` when a piece has none
/// (a weight that is not positive).
fn rational_bounds(spans: &[BezierSpan]) -> Option<Bounds> {
    let mut all: Option<DerivativeBounds> = None;
    for span in spans {
        let mut pieces = vec![span.patch.clone()];
        for _ in 0..RATIONAL_QUARTERINGS {
            pieces = pieces
                .iter()
                .flat_map(|piece| {
                    let (left, right) = piece.split_u(0.5);
                    let ((a, b), (c, d)) = (left.split_v(0.5), right.split_v(0.5));
                    [a, b, c, d]
                })
                .collect();
        }
        let across = (1_usize << RATIONAL_QUARTERINGS) as f64;
        let (du, dv) = (
            (span.u.1 - span.u.0) / across,
            (span.v.1 - span.v.0) / across,
        );
        if !(du > 0.0 && dv > 0.0) {
            return None;
        }
        for piece in &pieces {
            let bound = piece.derivative_bounds()?.over(du, dv);
            all = Some(all.map_or(bound, |so_far| so_far.max(&bound)));
        }
    }
    let b = all?;
    let bounds = Bounds {
        lu: b.first_u,
        lv: b.first_v,
        muu: b.second_uu,
        muv: b.second_uv,
        mvv: b.second_vv,
    };
    [bounds.lu, bounds.lv, bounds.muu, bounds.muv, bounds.mvv]
        .iter()
        .all(|x| x.is_finite())
        .then_some(bounds)
}

/// Derivative bounds of a NURBS surface: proofs, from the Bézier nets of
/// its spans. For a polynomial one (every weight 1) each span's net bounds
/// its derivatives as a Bézier patch's does, scaled by the span's width in
/// the surface's own parameters; for a rational one see
/// `rational_bounds`. Estimates from samples only where the surface cannot
/// be split into spans or a bound is not finite. (With
/// the estimates, which carry a margin, the marcher could rarely call a
/// cell simple on a B-spline and split down to the floor along the whole
/// curve: a B-spline roof took seconds where the same roof as a Bézier
/// patch took milliseconds.)
fn nurbs_bounds(
    s: &NurbsSurface,
    surface: &Surface,
    u: (f64, f64),
    v: (f64, f64),
    count_u: usize,
    count_v: usize,
) -> Bounds {
    let estimate = || sampled_bounds(surface, u, v, count_u, count_v);
    let Ok(spans) = s.bezier_spans() else {
        return estimate();
    };
    if !spans.iter().all(|span| span.patch.is_polynomial()) {
        return rational_bounds(&spans).unwrap_or_else(estimate);
    }
    let mut b = Bounds {
        lu: 0.0,
        lv: 0.0,
        muu: 0.0,
        muv: 0.0,
        mvv: 0.0,
    };
    for span in &spans {
        let (m, n) = span.patch.degrees();
        let Ok(patch) = BezierPatch::new(m, n, span.patch.control_points()) else {
            return estimate();
        };
        let (du, dv) = (span.u.1 - span.u.0, span.v.1 - span.v.0);
        if !(du > 0.0 && dv > 0.0) {
            return estimate();
        }
        let p = |i: usize, j: usize| patch.control_point(i, j).unwrap_or(Vec3::ZERO);
        let (mut first_u, mut first_v) = (0.0_f64, 0.0_f64);
        for i in 0..=m {
            for j in 0..=n {
                if i < m {
                    first_u = first_u.max(p(i + 1, j).distance(p(i, j)));
                }
                if j < n {
                    first_v = first_v.max(p(i, j + 1).distance(p(i, j)));
                }
            }
        }
        let (muu, muv, mvv) = patch.second_derivative_bounds();
        b.lu = b.lu.max(m as f64 * first_u / du);
        b.lv = b.lv.max(n as f64 * first_v / dv);
        b.muu = b.muu.max(muu / (du * du));
        b.muv = b.muv.max(muv / (du * dv));
        b.mvv = b.mvv.max(mvv / (dv * dv));
    }
    if [b.lu, b.lv, b.muu, b.muv, b.mvv]
        .iter()
        .all(|x| x.is_finite())
    {
        b
    } else {
        estimate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BezierPatch;

    /// A wavy bicubic patch; with `pole`, its first row is one point, so
    /// that a row of the projector's grid is one node many times over.
    fn patch(pole: bool, k: f64) -> Surface {
        let mut ctrl = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let (x, y) = (f64::from(i), f64::from(j));
                let z = 0.3 * (1.7 * x + 0.9 * y).sin() + 0.1 * x * y;
                ctrl.push(if pole && i == 0 {
                    Vec3::new(0.0, 1.5 * k, 0.2 * k)
                } else {
                    Vec3::new(x * k, y * k, z * k)
                });
            }
        }
        Surface::Bezier(BezierPatch::new(3, 3, ctrl).unwrap())
    }

    /// The scans the projector made before it kept its nodes in order
    /// along x: every node, in the grid's order.
    fn nearest_node_by_scan(projector: &Projector, p: Vec3) -> (f64, f64, f64, Vec3) {
        let mut best = (f64::INFINITY, 0.0, 0.0, p);
        for &(u, v, q) in &projector.grid {
            let d = (q - p).length_squared();
            if d < best.0 {
                best = (d, u, v, q);
            }
        }
        best
    }

    fn curvature_near_by_scan(projector: &Projector, p: Vec3, radius: f64) -> f64 {
        let within = radius + 2.0 * projector.spacing;
        let mut most: Option<f64> = None;
        for (&(_, _, q), &turn) in projector.grid.iter().zip(&projector.turning) {
            if q.distance(p) <= within {
                most = Some(most.map_or(turn, |m: f64| m.max(turn)));
            }
        }
        most.map_or(projector.curvature, |m| 2.0 * m)
    }

    #[test]
    fn the_nearest_node_and_the_curvature_near_a_point_are_what_a_scan_of_every_node_gives() {
        let mut seed = 7_u64;
        let mut unit = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        for k in [1e-3, 1.0, 1e6] {
            for pole in [false, true] {
                let projector = Projector::new(&patch(pole, k)).unwrap();
                // Points around the patch, near and far, the nodes
                // themselves (ties between equal nodes at the pole) and
                // points halfway between two nodes (ties of distance).
                let mut points: Vec<Vec3> = (0..400)
                    .map(|_| Vec3::new(unit() * 8.0 * k, unit() * 8.0 * k, unit() * 3.0 * k))
                    .collect();
                points.extend(projector.grid.iter().step_by(7).map(|&(_, _, q)| q));
                points.extend(
                    projector
                        .grid
                        .iter()
                        .zip(projector.grid.iter().skip(1))
                        .step_by(11)
                        .map(|(&(_, _, a), &(_, _, b))| a.lerp(b, 0.5)),
                );
                points.push(Vec3::new(1e300, 0.0, 0.0));
                for p in points {
                    let expected = nearest_node_by_scan(&projector, p);
                    let found = match projector.nearest_node(p) {
                        Some((u, v, q)) => ((q - p).length_squared(), u, v, q),
                        None => (f64::INFINITY, 0.0, 0.0, p),
                    };
                    assert_eq!(format!("{found:?}"), format!("{expected:?}"), "{p:?}");
                    for radius in [0.0, 1e-3 * k, 0.4 * k, 3.0 * k] {
                        assert_eq!(
                            projector.curvature_near(p, radius).to_bits(),
                            curvature_near_by_scan(&projector, p, radius).to_bits(),
                            "{p:?} within {radius}"
                        );
                    }
                }
            }
        }
    }

    /// The wavy patch with its pole's control points a rounding apart
    /// (one point by `poles`, as the control points of a rational surface
    /// turned about an axis are).
    fn rounded_pole(k: f64) -> Surface {
        let Surface::Bezier(b) = patch(true, k) else {
            panic!("the patch is a Bézier patch")
        };
        let mut ctrl = b.control_points().to_vec();
        for (j, p) in ctrl.iter_mut().take(4).enumerate() {
            *p += Vec3::new(1e-13, -2e-13, 1e-13) * (k * j as f64);
        }
        Surface::Bezier(BezierPatch::new(3, 3, ctrl).unwrap())
    }

    /// Nodes on one pole are one point with a normal per meridian: no
    /// curvature is read from them. (The limit normals of this patch's
    /// meridians differ by a good part of a radian; read over a rounding
    /// they made a curvature of 1e11 per unit length, and every cell of an
    /// intersection near the pole too bent to be simple.)
    #[test]
    fn the_curvature_is_not_read_between_nodes_on_one_pole() {
        for k in [1e-3, 1.0, 1e6] {
            let surface = rounded_pole(k);
            assert_eq!(poles(&surface).len(), 1);
            let plain = Projector::new(&patch(true, k)).unwrap().curvature;
            let rounded = Projector::new(&surface).unwrap().curvature;
            assert!(rounded * k < 100.0, "{rounded} at {k}");
            assert!(
                (rounded - plain).abs() <= 1e-6 * plain,
                "{rounded} and {plain}"
            );
        }
    }

    /// Half a dome, a bicubic patch: its border u = 0 is the pole on top,
    /// its meridians (v constant) leave it in every direction of a half
    /// turn, from +x to −x.
    fn half_dome(k: f64) -> Surface {
        let (radii, heights) = ([0.0, 0.6, 1.0, 1.0], [1.0, 1.0, 0.6, 0.0]);
        let around = [(1.0, 0.0), (1.0, 4.0 / 3.0), (-1.0, 4.0 / 3.0), (-1.0, 0.0)];
        let mut ctrl = Vec::new();
        for (r, z) in radii.iter().zip(heights) {
            for (x, y) in around {
                ctrl.push(Vec3::new(r * x, r * y, z) * k);
            }
        }
        Surface::Bezier(BezierPatch::new(3, 3, ctrl).unwrap())
    }

    /// Points near a pole on every side of it have their nearest point
    /// near the pole on their own meridian, not on the meridian Newton
    /// started from, which it cannot leave at the pole.
    #[test]
    fn the_nearest_point_near_a_pole_is_on_the_meridian_towards_the_point() {
        for k in [1e-3, 1.0, 1e6] {
            for surface in [half_dome(k), patch(true, k), rounded_pole(k)] {
                let projector = Projector::new(&surface).unwrap();
                for step in 0..=20 {
                    let along = f64::from(step) / 20.0;
                    for off in [1e-4, 1e-3, 1e-2] {
                        let on = surface.eval(off, along);
                        let normal = surface.normal(off, along).unwrap();
                        let p = on + normal * (1e-3 * off * k);
                        let (_, _, foot) = projector.foot(p);
                        // The foot is at most the distance to `on` away.
                        assert!(
                            foot.distance(p) <= 1e-3 * off * k * (1.0 + 1e-6),
                            "at {along}, {off} off the pole, scale {k}: {} from the surface",
                            foot.distance(p) / k
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_projector_is_shared_by_equal_surfaces_only() {
        let a = Projector::shared(&patch(false, 1.0)).unwrap();
        let b = Projector::shared(&patch(false, 1.0)).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        let c = Projector::shared(&patch(false, 1.0 + f64::EPSILON)).unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
    }
}
