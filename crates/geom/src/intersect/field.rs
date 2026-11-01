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
use craft3d_math::Portable;
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

/// Lets go of the projectors and the pieces of surfaces kept (each is made
/// again, the same, when asked for), and says how many there were.
pub(super) fn forget_shelves() -> usize {
    let projectors = std::mem::take(&mut *SHELF.lock().unwrap_or_else(PoisonError::into_inner));
    let pieces = std::mem::take(&mut *PIECE_SHELF.lock().unwrap_or_else(PoisonError::into_inner));
    projectors.len().saturating_add(pieces.len())
}

/// How many projectors and pieces of surfaces are kept.
pub(super) fn shelves_len() -> usize {
    let projectors = SHELF.lock().unwrap_or_else(PoisonError::into_inner).len();
    let pieces = PIECE_SHELF
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    projectors.saturating_add(pieces)
}

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
        // Made uncounted: whether it is on the shelf depends on what ran
        // before (`crate::allowance::uncounted`).
        let projector = Arc::new(crate::allowance::uncounted(|| Projector::new(surface))?);
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
                // (Two poles on borders across different parameters share
                // a corner, and are one point: a pole at a corner.)
                if let (Some(Some(a)), Some(Some(b))) = (at_pole.get(k), at_pole.get(k + step))
                    && (a == b
                        || poles.get(*a).map(Pole::across_v) != poles.get(*b).map(Pole::across_v))
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
        // Every pole the end is next to: two at a corner of the
        // parameters, where the meridians leave from both borders.
        let mut best = first;
        for pole in self
            .poles
            .iter()
            .filter(|pole| !(pole.share_off(&self.surface, first.0, first.1) > row))
        {
            let along = pole.meridian_towards(&self.surface, p);
            let (a, b) = pole.off_pole(&self.surface, along, row);
            // Along the meridian first: next to the pole the derivative
            // along the border is small, and a step in both parameters may
            // throw the point onto another meridian, and back onto the
            // pole.
            let down = self.newton(p, a, b, Some(!pole.across_v()));
            let polished = self.newton(p, down.0, down.1, None);
            let again = if (polished.2 - p).length_squared() < (down.2 - p).length_squared() {
                polished
            } else {
                down
            };
            if (again.2 - p).length_squared() < (best.2 - p).length_squared() {
                best = again;
            }
        }
        best
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

    /// Whether the nearest point of the surface to `p` is on a border of
    /// its parameters (to a billionth of the range): `p` is then past the
    /// surface, or beside a seam or a pole.
    pub(super) fn on_border(&self, p: Vec3) -> bool {
        let (u, v, _) = self.foot(p);
        let at = |x: f64, (lo, hi): (f64, f64)| {
            let near = 1e-9 * (hi - lo);
            !((x - lo).abs() > near) || !((hi - x).abs() > near)
        };
        at(u, self.u) || at(v, self.v)
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
                let (sin, cos) = c.half_angle().abs().psin_cos();
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
                let rho = x.phypot(y);
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
                let rho = x.phypot(y);
                let radial = if rho > 0.0 {
                    frame.vector(x / rho, y / rho, 0.0)
                } else {
                    frame.x()
                };
                let off = rho - major;
                let d = off.phypot(z);
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
                within(x.phypot(y))
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
                let rho = x.phypot(y);
                let tube = within((rho - major).phypot(z))?;
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

impl Bounds {
    /// The larger of each bound.
    fn max(&self, o: &Bounds) -> Bounds {
        Bounds {
            lu: self.lu.max(o.lu),
            lv: self.lv.max(o.lv),
            muu: self.muu.max(o.muu),
            muv: self.muv.max(o.muv),
            mvv: self.mvv.max(o.mvv),
        }
    }

    /// The smaller of each bound.
    fn min(&self, o: &Bounds) -> Bounds {
        Bounds {
            lu: self.lu.min(o.lu),
            lv: self.lv.min(o.lv),
            muu: self.muu.min(o.muu),
            muv: self.muv.min(o.muv),
            mvv: self.mvv.min(o.mvv),
        }
    }

    fn is_finite(&self) -> bool {
        [self.lu, self.lv, self.muu, self.muv, self.mvv]
            .iter()
            .all(|x| x.is_finite())
    }

    fn of(b: &DerivativeBounds) -> Bounds {
        Bounds {
            lu: b.first_u,
            lv: b.first_v,
            muu: b.second_uu,
            muv: b.second_uv,
            mvv: b.second_vv,
        }
    }
}

/// How often a piece of a freeform surface is halved at most, from its
/// Bézier span, for the bounds of its pieces ([`Pieces`]), in u and in v
/// together: next to the light end of a span whose weights are `r` apart
/// the weights even out within a piece about `1 / r` of it wide, some
/// twenty halvings for the ratios a span may have ([`crate`]'s callers
/// refuse more), each way.
const PIECE_DEPTH: usize = 48;
/// How many pieces the bounds of one surface keep at most.
const PIECE_BUDGET: usize = 1 << 13;
/// A piece is halved (the way that gains more) only where one of its
/// halves has a bound on a first derivative more than this many times
/// tighter than the piece's. A derivative that grows in proportion to the distance from a
/// point (a fan at a pole) tightens by two at every halving, and the
/// cells' own halving already pays for that: the budget would go on it
/// all along such a border. Weights far apart tighten the pieces next to
/// a light corner by orders of magnitude.
const PIECE_GAIN: f64 = 1.5;
/// How many pieces a query of [`Pieces::over`] looks at before it takes
/// the bounds of the whole surface.
const PIECE_LOOKS: usize = 1 << 12;

/// The derivative bounds of a freeform surface piece by piece: a tree of
/// pieces over its parameters, each with the bounds of its Bézier net
/// ([`crate::RationalBezierPatch::derivative_bounds`], in the surface's own
/// parameters), split where its quarters' bounds are much tighter
/// ([`PIECE_GAIN`]), in halves. A node holds the larger of its own bounds and of the
/// largest of its leaves', whichever is smaller. The marcher's cells take
/// the bounds of the pieces under them ([`Chart::bounds_in`]): where a
/// parametrisation is far from uniform (rational weights far apart) the
/// bounds over the whole surface are orders of magnitude above what a cell
/// near a heavy corner needs.
pub(super) struct Pieces {
    nodes: Vec<PieceNode>,
    roots: Vec<usize>,
    u: (f64, f64),
    v: (f64, f64),
    /// The roots' spans along u and along v when the roots are their grid
    /// (root `i · vs.len() + j` over `us[i] × vs[j]`, both in increasing
    /// order), as a surface's Bézier spans are: [`Pieces::over`] then looks
    /// only at the roots that meet the part asked about.
    grid: Option<RootGrid>,
}

/// The spans along u and along v of [`Pieces::grid`].
type RootGrid = (Vec<(f64, f64)>, Vec<(f64, f64)>);

/// A gain ordered by `total_cmp`, for a heap.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Gain(f64);

impl Eq for Gain {}

impl PartialOrd for Gain {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Gain {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

struct PieceNode {
    u: (f64, f64),
    v: (f64, f64),
    bounds: Bounds,
    children: Option<Vec<usize>>,
}

/// The pieces of the surfaces charted lately, by the surface (written out
/// in full, as for [`Projector::shared`]), the most recently used last.
type PieceShelf = Vec<(String, Option<Arc<Pieces>>)>;
static PIECE_SHELF: Mutex<PieceShelf> = Mutex::new(Vec::new());

impl Pieces {
    /// The pieces of `surface`, shared with every chart before that was
    /// of the same surface: a function of the surface alone, asked for
    /// again for every face it meets (measured on the benchmark's turbofan,
    /// 1312 charts took 0.9 s of pieces made anew).
    fn shared(surface: &Surface) -> Option<Arc<Pieces>> {
        let key = format!("{surface:?}");
        {
            let mut shelf = PIECE_SHELF.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(at) = shelf.iter().position(|(k, _)| *k == key) {
                let entry = shelf.remove(at);
                let pieces = entry.1.clone();
                shelf.push(entry);
                return pieces;
            }
        }
        let pieces = crate::allowance::uncounted(|| Pieces::new(surface)).map(Arc::new);
        let mut shelf = PIECE_SHELF.lock().unwrap_or_else(PoisonError::into_inner);
        if !shelf.iter().any(|(k, _)| *k == key) {
            shelf.push((key, pieces.clone()));
        }
        while shelf.len() > SHELF_SIZE {
            let _forgotten = shelf.remove(0);
        }
        pieces
    }

    /// The pieces of a Bézier or NURBS surface; `None` for another kind,
    /// or where a piece has no finite bound.
    fn new(surface: &Surface) -> Option<Pieces> {
        let spans: Vec<BezierSpan> = match surface {
            Surface::Bezier(b) => vec![BezierSpan {
                u: (0.0, 1.0),
                v: (0.0, 1.0),
                patch: crate::RationalBezierPatch::from_bezier(b),
            }],
            Surface::Nurbs(s) => s.bezier_spans_shared().ok()?.as_ref().clone(),
            _ => return None,
        };
        let (u, v) = surface.domain();
        let bound = |patch: &crate::RationalBezierPatch, u: (f64, f64), v: (f64, f64)| {
            let (wu, wv) = (u.1 - u.0, v.1 - v.0);
            if !(wu > 0.0 && wv > 0.0) {
                return None;
            }
            let b = Bounds::of(&patch.derivative_bounds()?.over(wu, wv));
            b.is_finite().then_some(b)
        };
        let mut pieces = Pieces {
            nodes: Vec::new(),
            roots: Vec::new(),
            u,
            v,
            grid: None,
        };
        // The split of a piece that gains more (its halves along u, along
        // v, or its quarters), and how much.
        type Part = (crate::RationalBezierPatch, (f64, f64), (f64, f64), Bounds);
        let best_split = |patch: &crate::RationalBezierPatch,
                          nu: (f64, f64),
                          nv: (f64, f64),
                          own: Bounds|
         -> Option<(f64, Vec<Part>)> {
            let (um, vm) = (0.5 * (nu.0 + nu.1), 0.5 * (nv.0 + nv.1));
            let (cut_u, cut_v) = (um > nu.0 && um < nu.1, vm > nv.0 && vm < nv.1);
            type Cut = (crate::RationalBezierPatch, (f64, f64), (f64, f64));
            let parts = |pieces: Vec<Cut>| {
                pieces
                    .into_iter()
                    .map(|(p, u, v)| Some((bound(&p, u, v)?.min(&own), p, u, v)))
                    .map(|made| made.map(|(b, p, u, v)| (p, u, v, b)))
                    .collect::<Option<Vec<Part>>>()
            };
            let mut options: Vec<Vec<Part>> = Vec::new();
            if cut_u {
                let (a, b) = patch.split_u(0.5);
                options.extend(parts(vec![(a, (nu.0, um), nv), (b, (um, nu.1), nv)]));
            }
            if cut_v {
                let (a, b) = patch.split_v(0.5);
                options.extend(parts(vec![(a, nu, (nv.0, vm)), (b, nu, (vm, nv.1))]));
            }
            if cut_u && cut_v {
                let (left, right) = patch.split_u(0.5);
                let ((ll, lh), (rl, rh)) = (left.split_v(0.5), right.split_v(0.5));
                options.extend(parts(vec![
                    (ll, (nu.0, um), (nv.0, vm)),
                    (lh, (nu.0, um), (vm, nv.1)),
                    (rl, (um, nu.1), (nv.0, vm)),
                    (rh, (um, nu.1), (vm, nv.1)),
                ]));
            }
            let gain = |parts: &[Part]| {
                parts
                    .iter()
                    .map(|h| (own.lu / h.3.lu).max(own.lv / h.3.lv))
                    .fold(0.0, f64::max)
            };
            // The first of the best (halves before quarters on a tie), by
            // the gain per halving: quarters are two.
            let mut best: Option<(f64, Vec<Part>)> = None;
            for option in options {
                let g = if option.len() > 2 {
                    gain(&option).sqrt()
                } else {
                    gain(&option)
                };
                if best.as_ref().is_none_or(|(b, _)| g > *b) {
                    best = Some((g, option));
                }
            }
            // (Not `>`-negated: a NaN gains nothing.)
            best.filter(|(g, _)| *g > PIECE_GAIN)
        };
        // The pieces that gain most are halved first, so that the budget
        // goes where the bounds tighten most (next to the light end of a
        // span whose weights are far apart), not evenly over the surface.
        let mut heap = std::collections::BinaryHeap::new();
        let mut pending: std::collections::BTreeMap<usize, (Vec<Part>, usize)> =
            std::collections::BTreeMap::new();
        for span in spans {
            let b = bound(&span.patch, span.u, span.v)?;
            let index = pieces.nodes.len();
            pieces.roots.push(index);
            pieces.nodes.push(PieceNode {
                u: span.u,
                v: span.v,
                bounds: b,
                children: None,
            });
            if let Some((gain, halves)) = best_split(&span.patch, span.u, span.v, b) {
                heap.push((Gain(gain), std::cmp::Reverse(index)));
                pending.insert(index, (halves, 0));
            }
        }
        while let Some((_, std::cmp::Reverse(index))) = heap.pop() {
            let Some((parts, depth)) = pending.remove(&index) else {
                continue;
            };
            if pieces.nodes.len().saturating_add(parts.len()) > PIECE_BUDGET {
                break;
            }
            let mut kids = Vec::with_capacity(parts.len());
            for (patch, u, v, b) in parts {
                let slot = pieces.nodes.len();
                kids.push(slot);
                if depth + 1 < PIECE_DEPTH
                    && let Some((gain, next)) = best_split(&patch, u, v, b)
                {
                    heap.push((Gain(gain), std::cmp::Reverse(slot)));
                    pending.insert(slot, (next, depth + 1));
                }
                pieces.nodes.push(PieceNode {
                    u,
                    v,
                    bounds: b,
                    children: None,
                });
            }
            if let Some(node) = pieces.nodes.get_mut(index) {
                node.children = Some(kids);
            }
        }
        pieces.grid = pieces.root_grid();
        // Children come after their parents: from the last node back, each
        // parent takes the largest of its children's bounds where that is
        // tighter than its own.
        for index in (0..pieces.nodes.len()).rev() {
            let Some(kids) = pieces.nodes.get(index).and_then(|n| n.children.clone()) else {
                continue;
            };
            let mut most: Option<Bounds> = None;
            for k in kids {
                let b = pieces.nodes.get(k)?.bounds;
                most = Some(most.map_or(b, |m| m.max(&b)));
            }
            if let (Some(most), Some(node)) = (most, pieces.nodes.get_mut(index)) {
                node.bounds = node.bounds.min(&most);
            }
        }
        Some(pieces)
    }

    /// Bounds on the derivatives over the part `u × v` of the parameters:
    /// the largest of the bounds of the pieces that meet it, each taken as
    /// small as the tree has it. `None` where the part reaches outside the
    /// surface's domain or the query looks at too many pieces.
    fn over(&self, u: (f64, f64), v: (f64, f64)) -> Option<Bounds> {
        if !(u.0 >= self.u.0 && u.1 <= self.u.1 && v.0 >= self.v.0 && v.1 <= self.v.1) {
            return None;
        }
        let mut most: Option<Bounds> = None;
        // Every root counts as looked at, as when each is taken in turn and
        // those that do not meet the part are passed over: so the part
        // looks at too many pieces exactly when it did before.
        let (mut stack, mut looks) = match &self.grid {
            Some((us, vs)) => {
                let meeting = |spans: &[(f64, f64)], (lo, hi): (f64, f64)| {
                    let first = spans.partition_point(|s| s.1 < lo);
                    let end = spans.partition_point(|s| s.0 <= hi);
                    first..end.max(first)
                };
                let (is, js) = (meeting(us, u), meeting(vs, v));
                let mut stack = Vec::with_capacity(is.len().saturating_mul(js.len()));
                for i in is {
                    for j in js.clone() {
                        stack.extend(self.roots.get(i * vs.len() + j));
                    }
                }
                let passed_over = self.roots.len().saturating_sub(stack.len());
                (stack, passed_over)
            }
            None => (self.roots.clone(), 0),
        };
        if looks > PIECE_LOOKS {
            return None;
        }
        while let Some(index) = stack.pop() {
            looks += 1;
            if looks > PIECE_LOOKS {
                return None;
            }
            let node = self.nodes.get(index)?;
            if node.u.1 < u.0 || node.u.0 > u.1 || node.v.1 < v.0 || node.v.0 > v.1 {
                continue;
            }
            let within = node.u.0 >= u.0 && node.u.1 <= u.1 && node.v.0 >= v.0 && node.v.1 <= v.1;
            match &node.children {
                Some(kids) if !within => stack.extend(kids.iter().copied()),
                _ => most = Some(most.map_or(node.bounds, |m| m.max(&node.bounds))),
            }
        }
        most
    }

    /// The roots' grid ([`Pieces::grid`]), if they form one: every root
    /// over the span of its row along u and of its column along v, each
    /// row and column a span of its own in increasing order.
    fn root_grid(&self) -> Option<RootGrid> {
        let span = |r: &usize| self.nodes.get(*r).map(|n| (n.u, n.v));
        let first_u = span(self.roots.first()?)?.0;
        let vs: Vec<(f64, f64)> = self
            .roots
            .iter()
            .map_while(|r| span(r).filter(|(u, _)| *u == first_u).map(|(_, v)| v))
            .collect();
        if vs.is_empty() || !self.roots.len().is_multiple_of(vs.len()) {
            return None;
        }
        let us: Vec<(f64, f64)> = self
            .roots
            .iter()
            .step_by(vs.len())
            .map(|r| span(r).map(|(u, _)| u))
            .collect::<Option<_>>()?;
        let increasing = |spans: &[(f64, f64)]| {
            spans.iter().all(|s| s.0 < s.1)
                && spans.windows(2).all(|w| matches!(w, [a, b] if a.1 <= b.0))
        };
        let fits = self.roots.iter().enumerate().all(|(k, r)| {
            span(r)
                == Some((
                    us.get(k / vs.len())
                        .copied()
                        .unwrap_or((f64::NAN, f64::NAN)),
                    vs.get(k % vs.len())
                        .copied()
                        .unwrap_or((f64::NAN, f64::NAN)),
                ))
        });
        (fits && increasing(&us) && increasing(&vs)).then_some((us, vs))
    }
}

/// A surface with the rectangle of its parameters that covers everything of
/// it inside a box.
pub(super) struct Chart {
    /// The borders moved in short of a cusp ([`short_of_poles`]), where
    /// a curve that ends is drawn on to the point.
    pub cusps: Vec<Cusp>,
    pub surface: Surface,
    pub u: (f64, f64),
    pub v: (f64, f64),
    pub periodic_u: bool,
    pub periodic_v: bool,
    pub bounds: Bounds,
    /// The bounds piece by piece, for a freeform surface ([`Pieces`]).
    pub pieces: Option<Arc<Pieces>>,
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

    /// The bounds on the derivatives over the part of the chart `u × v`:
    /// [`Chart::bounds`], and next to a cusp ([`Cusp`]) the bounds on the
    /// derivatives along its border to the cusp's order there (they
    /// vanish as the width does: `crate::pole::Narrowing`), taken at the
    /// far side of the part from the pole. Without them the cells there
    /// would have to be as small across as the width is along, all around
    /// the border.
    ///
    /// For a freeform surface the bounds are those of the pieces under
    /// the part ([`Pieces::over`]) where they are tighter.
    pub(super) fn bounds_in(&self, u: (f64, f64), v: (f64, f64)) -> Bounds {
        let mut b = match self.pieces.as_ref().and_then(|p| p.over(u, v)) {
            Some(local) => self.bounds.min(&local),
            None => self.bounds,
        };
        for cusp in &self.cusps {
            let n = &cusp.narrowing;
            let (lo, hi) = if cusp.pole.across_v() { v } else { u };
            let far = (lo - cusp.from).abs().max((hi - cusp.from).abs()) / n.span;
            if !(far <= 1.0) {
                continue;
            }
            let k = n.order as i32;
            let (speed, bend, twist) = (
                n.speed * far.ppowi(k),
                n.bend * far.ppowi(k),
                n.twist * far.ppowi(k - 1),
            );
            if cusp.pole.across_v() {
                b.lu = b.lu.min(speed);
                b.muu = b.muu.min(bend);
            } else {
                b.lv = b.lv.min(speed);
                b.mvv = b.mvv.min(bend);
            }
            b.muv = b.muv.min(twist);
        }
        b
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
            let away = x.phypot(y);
            if away > half * 1.001 + margin && away > 0.0 {
                let spread = ((half + margin) / away).min(1.0).pasin() * 1.001 + 1e-9;
                let mid = y.patan2(x);
                ((mid - spread, mid + spread), false)
            } else {
                ((0.0, TAU), true)
            }
        };
        let chart = match surface {
            Surface::Plane(p) => {
                let f = *p.frame();
                Chart {
                    cusps: Vec::new(),
                    pieces: None,
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
                    cusps: Vec::new(),
                    pieces: None,
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
                let slope = c.half_angle().ptan();
                let short = 0.25 * tol.linear * c.half_angle().pcos().abs() / slope.abs();
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
                    cusps: Vec::new(),
                    pieces: None,
                    surface: surface.clone(),
                    u,
                    v: (v0, v1),
                    periodic_u,
                    periodic_v: false,
                    bounds: Bounds {
                        lu: widest,
                        lv: 1.0 / c.half_angle().pcos().abs(),
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
                    cusps: Vec::new(),
                    pieces: None,
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
                    cusps: Vec::new(),
                    pieces: None,
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
                    cusps: Vec::new(),
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
                    pieces: Pieces::shared(surface),
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
                    cusps: Vec::new(),
                    surface: surface.clone(),
                    u,
                    v,
                    periodic_u,
                    periodic_v,
                    bounds: nurbs_bounds(s, surface, u, v, count_u, count_v),
                    pieces: Pieces::shared(surface),
                }
            }
        };
        let chart = short_of_poles(chart, tol, other);
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

/// A border of a chart moved in short of a cusp ([`short_of_poles`]).
#[derive(Clone, Copy, Debug)]
pub(super) struct Cusp {
    pub pole: Pole,
    /// The parameter of the moved border.
    pub at: f64,
    /// The parameter of the pole's own border.
    pub from: f64,
    pub narrowing: crate::pole::Narrowing,
}

/// The share of the curvature times the tolerance below which the marcher
/// takes the gradient of the distance for zero (`Marcher::touches`: a
/// point is a touch where `|∇f| ≤ TOUCH · tol · |H|`).
pub(super) const TOUCH: f64 = 4.0;

/// How much longer than the iso-curve the bound of `pole::narrowing` may
/// be: at most four times (held by
/// `pole.rs` › `a_cusp_narrows_to_the_second_order_within_its_bounds`; 1.8
/// on the needle).
const WIDTH_SLACK: f64 = 4.0;

/// How wide, in tolerances, the surface may be (by the bound) where the
/// chart stops short of a cusp: see [`short_of_poles`]. Next to the cusp
/// the curvature across is that of a circle of radius `ρ`, and a surface
/// that crosses there at the angle `φ` has `|∇f| = sin φ` against a
/// Hessian of `1 / ρ`: the cells tell it from a touch only where `ρ >
/// TOUCH · tol / sin φ`, an iso-curve (a whole turn) of `2π` times that.
/// With the bound up to [`WIDTH_SLACK`] loose, the border is where the
/// bound is that times `2π · TOUCH · tol` for crossings at a right angle
/// (about 100 tolerances); a crossing at the border shallower than about
/// a quarter of that slack allows is refused as a touch. (On a face that
/// turns a fraction of the way round, the iso-curve is that fraction of
/// the circle, and the radius is larger by its inverse.)
pub(super) const CUSP_WIDTH: f64 = WIDTH_SLACK * std::f64::consts::TAU * TOUCH;

/// The chart of a freeform surface stopped just short of its poles, as a
/// cone's is of its apex: at a border that is one point the chart
/// collapses (every parameter along the border is the same point, and a
/// surface through that point is zero all along it). The border is moved
/// in by a quarter of the linear tolerance over the bound on the speed
/// across it, so that everything left out is within a quarter of the
/// tolerance of the pole (a point moves no further than its speed allows),
/// and a curve that runs into the pole ends within that of it.
///
/// At a cusp (a pole the surface narrows to as the square of the distance
/// or faster, [`crate::pole::narrowing`]) that is not enough: next to the
/// border the derivatives along it vanish to that order, a surface that
/// holds the line the meridians leave along is within a hair of the chart
/// all along the border and its derivatives too, and the cells take that
/// for a contact along a curve. There the border is moved in further, to
/// where every iso-curve along it is at most [`CUSP_WIDTH`] tolerances
/// long (`β = (width / c)^(1/order)` of the first span). What is left out
/// lies within that width of any one meridian of it; a curve that ends on
/// that border is drawn on to the point along the intersection
/// (`Marcher::on_into_cusp`), not left to the vertex rule. This is done only
/// where the other surface passes within the tolerance of the point and
/// holds the strip all along ([`holds_the_strip`]); elsewhere the chart
/// stops as at any other pole.
fn short_of_poles(mut chart: Chart, tol: &Tolerance, other: &Field) -> Chart {
    for pole in poles(&chart.surface) {
        // Only where the other surface passes through the point: a curve
        // ends there only then, and a surface that crosses the axis near
        // the point but not through it is cut where it is (the cells
        // judge it as before).
        let through = other
            .sample(pole.point)
            .is_some_and(|s| s.dist <= tol.linear);
        let narrowing =
            crate::pole::narrowing(&chart.surface, &pole).filter(|n| through && n.order >= 2);
        let speed = if pole.across_v() {
            chart.bounds.lv
        } else {
            chart.bounds.lu
        };
        let mut short = 0.25 * tol.linear / speed;
        let mut cusp = None;
        if let Some(n) = narrowing {
            let wide = n.within(CUSP_WIDTH * tol.linear);
            if wide > short && holds_the_strip(&chart, &pole, &n, other, tol, wide) {
                short = wide;
                cusp = Some(n);
            }
        }
        let range = if pole.across_v() {
            &mut chart.v
        } else {
            &mut chart.u
        };
        // A surface no larger than the tolerance keeps its chart: nothing
        // of it would be left.
        if !(short > 0.0 && short < 0.25 * (range.1 - range.0)) {
            continue;
        }
        let from = match pole.border {
            crate::pole::Border::UStart | crate::pole::Border::VStart => {
                let from = range.0;
                range.0 += short;
                from
            }
            crate::pole::Border::UEnd | crate::pole::Border::VEnd => {
                let from = range.1;
                range.1 -= short;
                from
            }
        };
        if let Some(n) = cusp {
            let at = match pole.border {
                crate::pole::Border::UStart | crate::pole::Border::VStart => range.0,
                crate::pole::Border::UEnd | crate::pole::Border::VEnd => range.1,
            };
            chart.cusps.push(Cusp {
                pole,
                at,
                from,
                narrowing: n,
            });
        }
    }
    chart
}

/// Whether the other surface holds the strip a cusp's chart would leave
/// out (`wide` across from the pole): within the strip's own width of it
/// all along. Every point of the iso-curve at the share `β` is within
/// `r(β) = ½ · width · β^k` of the meridian `A` through the middle of the
/// border (`pole::Narrowing`), and the other surface's distance `F` is
/// 1-Lipschitz, so where `|F(A(β))| ≤ r(β) + tol` all the way from the
/// tolerance around the point to the border, the other surface runs
/// along the strip as the axis does (a plane, a block's face or a
/// cylinder through the axis), and the curves in the strip are the ones
/// that cross its border, which are drawn on to the point
/// (`Marcher::on_into_cusp`, which refuses one that turns back). Where it
/// leaves the strip somewhere (a ball through the point that swallows
/// the tip, a cone about the axis, a plane through the point at an angle
/// to the axis), the strip is not left out: the chart stops at the pole
/// as for any other (`tol / (4 L)`). (The other way round, `|F| > r +
/// tol` all along, cannot hold for a surface through the point: next to
/// the point `|F|` is at most the distance to it.) `A` is walked by
/// halving stretches across: a stretch is settled by its middle and how
/// far `A` can move on it (the speed across), down to a stretch of an
/// eighth of the tolerance, where the middle decides. And the other
/// surface must cross every iso-curve of the strip the same number of
/// times ([`same_crossings`]): within the width of the meridian it may
/// still cross the axis inside the strip and make a loop there. At most
/// [`STRIP_LOOKS`] samples, past which the strip is not left out.
fn holds_the_strip(
    chart: &Chart,
    pole: &Pole,
    n: &crate::pole::Narrowing,
    other: &Field,
    tol: &Tolerance,
    wide: f64,
) -> bool {
    let (along, across, speed) = if pole.across_v() {
        (chart.u, chart.v, chart.bounds.lv)
    } else {
        (chart.v, chart.u, chart.bounds.lu)
    };
    let (from, toward): (f64, f64) = match pole.border {
        crate::pole::Border::UStart | crate::pole::Border::VStart => (across.0, 1.0),
        crate::pole::Border::UEnd | crate::pole::Border::VEnd => (across.1, -1.0),
    };
    let middle = 0.5 * (along.0 + along.1);
    let at = |off: f64| {
        let x = from + toward * off;
        if pole.across_v() {
            chart.surface.eval(middle, x)
        } else {
            chart.surface.eval(x, middle)
        }
    };
    let width = |off: f64| 0.5 * n.width * (off / n.span).ppowi(n.order as i32);
    let start = tol.linear / speed;
    if !(start < wide && speed > 0.0) {
        return false;
    }
    let mut stack = vec![(start, wide)];
    let mut looks = 0_usize;
    while let Some((a, b)) = stack.pop() {
        looks += 1;
        if looks > STRIP_LOOKS {
            return false;
        }
        let mid = 0.5 * (a + b);
        let Some(sample) = other.sample(at(mid)) else {
            return false;
        };
        let reach = speed * 0.5 * (b - a);
        let allowed = width(b) + tol.linear;
        if sample.dist + reach <= allowed {
            continue;
        }
        if sample.dist > width(mid) + tol.linear || reach <= 0.125 * tol.linear {
            if sample.dist > width(mid) + tol.linear {
                return false;
            }
            continue;
        }
        stack.push((mid, b));
        stack.push((a, mid));
    }
    same_crossings(chart, pole, n, other, tol, (start, wide), looks)
}

/// Whether the other surface crosses every iso-curve along the border the
/// same number of times from the border down to `start` (both as
/// distances across from the pole): then no piece of the intersection
/// begins or ends inside the strip (a closed loop wholly inside it, of a
/// ball or a cylinder through the point that crosses the axis again
/// within the strip, changes the number where it begins and where it
/// ends), and every piece in it crosses the border, where the marcher
/// finds it. The iso-curves are looked at a tolerance apart in space
/// across (the speed across) and along each at places half a tolerance
/// apart by the bound on its length, so a piece is missed only if it is
/// thinner than that; the sign of the distance is read where it is above
/// its rounding. At most [`STRIP_LOOKS`] samples in all, past which the
/// strip is not left out.
fn same_crossings(
    chart: &Chart,
    pole: &Pole,
    n: &crate::pole::Narrowing,
    other: &Field,
    tol: &Tolerance,
    (start, wide): (f64, f64),
    mut looks: usize,
) -> bool {
    let (along, across, speed, closed) = if pole.across_v() {
        (chart.u, chart.v, chart.bounds.lv, chart.periodic_u)
    } else {
        (chart.v, chart.u, chart.bounds.lu, chart.periodic_v)
    };
    let (from, toward): (f64, f64) = match pole.border {
        crate::pole::Border::UStart | crate::pole::Border::VStart => (across.0, 1.0),
        crate::pole::Border::UEnd | crate::pole::Border::VEnd => (across.1, -1.0),
    };
    if !(speed > 0.0) {
        return false;
    }
    let step = tol.linear / speed;
    let levels = ((wide - start) / step).ceil();
    if !(levels.is_finite() && levels >= 0.0) || levels > STRIP_LOOKS as f64 {
        return false;
    }
    let levels = levels as usize;
    let rounding = 64.0 * f64::EPSILON * (pole.point.length() + 1e6 * tol.linear);
    let mut expected: Option<usize> = None;
    for level in 0..=levels {
        let off = (wide - step * level as f64).max(start);
        let x = from + toward * off;
        let width = n.width * (off / n.span).ppowi(n.order as i32);
        let places = (2.0 * width / tol.linear).ceil().clamp(8.0, 1e6) as usize;
        looks = looks.saturating_add(places + 1);
        if looks > STRIP_LOOKS {
            return false;
        }
        let mut crossings = 0_usize;
        let mut last: Option<bool> = None;
        let mut first: Option<bool> = None;
        for i in 0..=places {
            let a = along.0 + (along.1 - along.0) * i as f64 / places as f64;
            let p = if pole.across_v() {
                chart.surface.eval(a, x)
            } else {
                chart.surface.eval(x, a)
            };
            let Some(sample) = other.sample(p) else {
                return false;
            };
            if !(sample.signed.abs() > rounding) {
                continue;
            }
            let side = sample.signed > 0.0;
            if last.is_some_and(|l| l != side) {
                crossings += 1;
            }
            last = Some(side);
            first = first.or(Some(side));
        }
        // An iso-curve that closes on itself crosses where it closes too.
        if closed && first.is_some() && first != last {
            crossings += 1;
        }
        match expected {
            None => expected = Some(crossings),
            Some(e) if e != crossings => return false,
            Some(_) => {}
        }
    }
    true
}

/// Samples of the other surface [`holds_the_strip`] takes at most.
const STRIP_LOOKS: usize = 2_000_000;

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
    let Ok(spans) = s.bezier_spans_shared() else {
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
    for span in spans.iter() {
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
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::BezierPatch;

    /// A rational NURBS surface of degrees (5, 3) with uneven knots and
    /// weights far apart in places: many Bézier spans, and pieces split
    /// under them.
    fn many_spans() -> Surface {
        let (nu, nv, du, dv) = (14, 11, 5, 3);
        let knots = |d: usize, n: usize| {
            let mut k = vec![0.0; d + 1];
            let inner = n - d - 1;
            for i in 1..=inner {
                let t = i as f64 / (inner + 1) as f64;
                k.push(t + 0.2 * (t * 7.0).psin() / (inner + 1) as f64);
            }
            k.extend(std::iter::repeat_n(1.0, d + 1));
            k
        };
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for i in 0..nu {
            for j in 0..nv {
                let (x, y) = (i as f64, j as f64);
                ctrl.push(Vec3::new(x, y, (0.7 * x).psin() + 0.3 * (1.1 * y).pcos()));
                weights.push(if (i + 2 * j) % 7 == 0 {
                    30.0
                } else {
                    1.0 + 0.1 * x
                });
            }
        }
        Surface::Nurbs(
            crate::NurbsSurface::new(du, dv, knots(du, nu), knots(dv, nv), nu, nv, ctrl, weights)
                .unwrap(),
        )
    }

    /// `Pieces::over` with the roots found on their grid gives what it
    /// gives looking at every root, for parts of every size and place
    /// (inside, on span borders, reaching outside the domain, empty).
    #[test]
    fn pieces_over_their_grid_give_what_every_root_gives() {
        let pieces = Pieces::new(&many_spans()).unwrap();
        assert!(pieces.grid.is_some(), "the spans are a grid");
        assert!(pieces.roots.len() > 40, "{} roots", pieces.roots.len());
        let plain = Pieces {
            nodes: pieces
                .nodes
                .iter()
                .map(|n| PieceNode {
                    u: n.u,
                    v: n.v,
                    bounds: n.bounds,
                    children: n.children.clone(),
                })
                .collect(),
            roots: pieces.roots.clone(),
            u: pieces.u,
            v: pieces.v,
            grid: None,
        };
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut unit = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut found = 0;
        for n in 0..4000 {
            let (a, b, c, d) = (unit(), unit(), unit(), unit());
            let scale = [1.0, 0.1, 0.01, 1e-4][n % 4];
            let u = (a, (a + scale * b).min(if n % 9 == 0 { 1.2 } else { 1.0 }));
            let v = (c, (c + scale * d).min(1.0));
            let (got, want) = (pieces.over(u, v), plain.over(u, v));
            assert_eq!(got.is_some(), want.is_some(), "part {u:?} x {v:?}");
            if let (Some(g), Some(w)) = (got, want) {
                found += 1;
                for (x, y) in [
                    (g.lu, w.lu),
                    (g.lv, w.lv),
                    (g.muu, w.muu),
                    (g.muv, w.muv),
                    (g.mvv, w.mvv),
                ] {
                    assert_eq!(x.to_bits(), y.to_bits(), "part {u:?} x {v:?}");
                }
            }
        }
        assert!(found > 3000, "{found}");
    }

    /// A wavy bicubic patch; with `pole`, its first row is one point, so
    /// that a row of the projector's grid is one node many times over.
    fn patch(pole: bool, k: f64) -> Surface {
        let mut ctrl = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let (x, y) = (f64::from(i), f64::from(j));
                let z = 0.3 * (1.7 * x + 0.9 * y).psin() + 0.1 * x * y;
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
