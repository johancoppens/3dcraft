//! Surfaces: analytic kinds stay analytic, everything else is Bézier or NURBS.
//!
//! Every analytic surface is placed by a [`Frame`] and parametrised so that
//! `∂u × ∂v` points away from the axis or centre (outward).

use crate::curve::wrap_angle;
use crate::{BezierPatch, GeomError, NurbsSurface};
use craft3d_math::{Bits, Frame, Vec3};
use std::f64::consts::{FRAC_PI_2, TAU};
use std::sync::{Arc, OnceLock};

/// Steps each way of the grid a numeric projection starts from.
const PROJECT_GRID: usize = 16;
/// Samples a numeric projection starts from, at most.
const PROJECT_STARTS: usize = 6;
/// Steps of one descent of a numeric projection, at most.
const PROJECT_STEPS: usize = 80;

/// The samples of a Bézier or NURBS surface a numeric projection starts
/// from ([`Surface::project_grid`]), and what they tell about the surface:
/// whether it closes on itself in u and in v, and which of its border rows
/// are one point ([u start, u end, v start, v end]).
#[derive(Debug)]
pub(crate) struct ProjectGrid {
    points: [[Vec3; PROJECT_GRID + 1]; PROJECT_GRID + 1],
    /// How far the samples reach from the first.
    extent: f64,
    closed_u: bool,
    closed_v: bool,
    collapsed: [bool; 4],
    /// The projections found so far, by the bits of the point projected
    /// ([`Surface::project`] is a function of the surface and the point):
    /// a check of a body against itself after every feature, a mesh and a
    /// mass integral project the points of the boundaries of the faces
    /// that did not change again and again.
    memo: ProjectMemo,
}

/// How many projections one surface remembers at most, and all surfaces
/// together (about 80 bytes each); a surface that would pass either lets
/// go of all of its own first.
const PROJECTIONS_PER_SURFACE: usize = 1 << 15;
#[cfg(not(target_arch = "wasm32"))]
const PROJECTIONS: usize = 1 << 20;
#[cfg(target_arch = "wasm32")]
const PROJECTIONS: usize = 1 << 16;
/// How many projections all surfaces remember now.
static PROJECTIONS_KEPT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The projections a surface remembers ([`ProjectGrid::memo`]).
#[derive(Debug, Default)]
struct ProjectMemo(std::sync::Mutex<std::collections::BTreeMap<[u64; 3], (f64, f64)>>);

impl ProjectMemo {
    fn get(&self, key: &[u64; 3]) -> Option<(f64, f64)> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .copied()
    }

    fn put(&self, key: [u64; 3], found: (f64, f64)) {
        use std::sync::atomic::Ordering::Relaxed;
        let mut map = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if map.contains_key(&key) {
            return;
        }
        let full = map.len() >= PROJECTIONS_PER_SURFACE
            || PROJECTIONS_KEPT.load(Relaxed).saturating_add(1) > PROJECTIONS;
        if full {
            PROJECTIONS_KEPT.fetch_sub(map.len(), Relaxed);
            map.clear();
            // Still full with this surface's own let go: the others hold
            // the room, and this one remembers nothing.
            if PROJECTIONS_KEPT.load(Relaxed).saturating_add(1) > PROJECTIONS {
                return;
            }
        }
        map.insert(key, found);
        PROJECTIONS_KEPT.fetch_add(1, Relaxed);
    }
}

impl Drop for ProjectMemo {
    fn drop(&mut self) {
        let held = self
            .0
            .get_mut()
            .map_or_else(|poisoned| poisoned.into_inner().len(), |map| map.len());
        PROJECTIONS_KEPT.fetch_sub(held, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Where a Bézier or NURBS surface keeps its [`ProjectGrid`] once made:
/// derived from the surface's numbers alone, so not part of its value
/// (always equal, and left out of its debug text).
#[derive(Clone, Default)]
pub(crate) struct GridCache(pub(crate) OnceLock<Arc<ProjectGrid>>);

fn positive(value: f64, message: &'static str) -> Result<f64, GeomError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(GeomError::InvalidParameter(message))
    }
}

/// The xy plane of a frame: `(u, v) ↦ origin + u x + v y`, normal `z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    frame: Frame,
}

impl Plane {
    pub fn new(frame: Frame) -> Self {
        Self { frame }
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }
}

/// A cylinder about the frame's z axis: `(r cos u, r sin u, v)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cylinder {
    frame: Frame,
    radius: f64,
}

impl Cylinder {
    pub fn new(frame: Frame, radius: f64) -> Result<Self, GeomError> {
        Ok(Self {
            frame,
            radius: positive(radius, "radius must be positive and finite")?,
        })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }
}

/// A cone about the frame's z axis: at height `v` the radius is
/// `radius + v tan(half_angle)`. `v` is limited to where that is not
/// negative, so the surface ends at its apex.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cone {
    frame: Frame,
    radius: f64,
    half_angle: f64,
}

impl Cone {
    /// `radius ≥ 0` is the radius at `v = 0`; `half_angle` is the angle
    /// between axis and surface, in `(-π/2, π/2)` and not zero (that would
    /// be a cylinder). Positive widens along +z.
    pub fn new(frame: Frame, radius: f64, half_angle: f64) -> Result<Self, GeomError> {
        if !(radius.is_finite() && radius >= 0.0) {
            return Err(GeomError::InvalidParameter(
                "cone radius must be finite and not negative",
            ));
        }
        if !(half_angle.is_finite() && half_angle.abs() < FRAC_PI_2 && half_angle != 0.0) {
            return Err(GeomError::InvalidParameter(
                "cone half-angle must be non-zero and strictly between -90° and 90°",
            ));
        }
        Ok(Self {
            frame,
            radius,
            half_angle,
        })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }

    pub fn half_angle(&self) -> f64 {
        self.half_angle
    }

    /// The `v` of the apex.
    pub fn apex_v(&self) -> f64 {
        -self.radius / self.half_angle.tan()
    }

    pub(crate) fn radius_at(&self, v: f64) -> f64 {
        self.radius + v * self.half_angle.tan()
    }
}

/// A sphere about the frame's origin: longitude `u`, latitude `v` in
/// `[-π/2, π/2]`: `r (cos v cos u, cos v sin u, sin v)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    frame: Frame,
    radius: f64,
}

impl Sphere {
    pub fn new(frame: Frame, radius: f64) -> Result<Self, GeomError> {
        Ok(Self {
            frame,
            radius: positive(radius, "radius must be positive and finite")?,
        })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }
}

/// A ring torus about the frame's z axis:
/// `((R + r cos v) cos u, (R + r cos v) sin u, r sin v)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Torus {
    frame: Frame,
    major: f64,
    minor: f64,
}

impl Torus {
    /// `major > minor > 0`: the tube must not reach the axis.
    pub fn new(frame: Frame, major: f64, minor: f64) -> Result<Self, GeomError> {
        positive(minor, "torus radii must be positive and finite")?;
        positive(major, "torus radii must be positive and finite")?;
        if major <= minor {
            return Err(GeomError::InvalidParameter(
                "torus major radius must be larger than its minor radius",
            ));
        }
        Ok(Self {
            frame,
            major,
            minor,
        })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// `(major, minor)`.
    pub fn radii(&self) -> (f64, f64) {
        (self.major, self.minor)
    }
}

/// Any surface the kernel knows. No code may assume a particular kind
/// (AGENTS.md rule 3).
#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    Plane(Plane),
    Cylinder(Cylinder),
    Cone(Cone),
    Sphere(Sphere),
    Torus(Torus),
    Bezier(BezierPatch),
    Nurbs(NurbsSurface),
}

impl Surface {
    /// A short name for messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Surface::Plane(_) => "plane",
            Surface::Cylinder(_) => "cylinder",
            Surface::Cone(_) => "cone",
            Surface::Sphere(_) => "sphere",
            Surface::Torus(_) => "torus",
            Surface::Bezier(_) => "bezier patch",
            Surface::Nurbs(_) => "nurbs surface",
        }
    }

    /// `((u start, u end), (v start, v end))`; infinite where the surface
    /// is unbounded, one period where it closes on itself.
    pub fn domain(&self) -> ((f64, f64), (f64, f64)) {
        let all = (f64::NEG_INFINITY, f64::INFINITY);
        let turn = (0.0, TAU);
        match self {
            Surface::Plane(_) => (all, all),
            Surface::Cylinder(_) => (turn, all),
            Surface::Cone(c) => {
                let apex = c.apex_v();
                (
                    turn,
                    if c.half_angle > 0.0 {
                        (apex, f64::INFINITY)
                    } else {
                        (f64::NEG_INFINITY, apex)
                    },
                )
            }
            Surface::Sphere(_) => (turn, (-FRAC_PI_2, FRAC_PI_2)),
            Surface::Torus(_) => (turn, turn),
            Surface::Bezier(_) => ((0.0, 1.0), (0.0, 1.0)),
            Surface::Nurbs(s) => s.domain(),
        }
    }

    /// The period in u, for surfaces closed around their axis.
    pub fn period_u(&self) -> Option<f64> {
        match self {
            Surface::Cylinder(_) | Surface::Cone(_) | Surface::Sphere(_) | Surface::Torus(_) => {
                Some(TAU)
            }
            Surface::Plane(_) | Surface::Bezier(_) | Surface::Nurbs(_) => None,
        }
    }

    /// The period in v (only the torus).
    pub fn period_v(&self) -> Option<f64> {
        matches!(self, Surface::Torus(_)).then_some(TAU)
    }

    pub fn eval(&self, u: f64, v: f64) -> Vec3 {
        // A freeform surface's point alone: the same bits as the point of
        // its derivatives (the same sums in the same order), without them.
        match self {
            Surface::Bezier(b) => b.eval(u, v),
            Surface::Nurbs(s) => s.eval(u, v),
            _ => self.derivatives(u, v).0,
        }
    }

    /// `(point, ∂/∂u, ∂/∂v)` at `(u, v)`.
    pub fn derivatives(&self, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
        // The angle's sine and cosine only for the kinds that turn about an
        // axis (a freeform surface is evaluated in its own parameters).
        let turn = || u.sin_cos();
        match self {
            Surface::Plane(p) => (p.frame.point(u, v, 0.0), p.frame.x(), p.frame.y()),
            Surface::Cylinder(c) => {
                let (su, cu) = turn();
                let r = c.radius;
                (
                    c.frame.point(r * cu, r * su, v),
                    c.frame.vector(-r * su, r * cu, 0.0),
                    c.frame.z(),
                )
            }
            Surface::Cone(c) => {
                let (su, cu) = turn();
                let (r, slope) = (c.radius_at(v), c.half_angle.tan());
                (
                    c.frame.point(r * cu, r * su, v),
                    c.frame.vector(-r * su, r * cu, 0.0),
                    c.frame.vector(slope * cu, slope * su, 1.0),
                )
            }
            Surface::Sphere(s) => {
                let (su, cu) = turn();
                let (sv, cv) = v.sin_cos();
                let r = s.radius;
                (
                    s.frame.point(r * cv * cu, r * cv * su, r * sv),
                    s.frame.vector(-r * cv * su, r * cv * cu, 0.0),
                    s.frame.vector(-r * sv * cu, -r * sv * su, r * cv),
                )
            }
            Surface::Torus(t) => {
                let (su, cu) = turn();
                let (sv, cv) = v.sin_cos();
                let ring = t.major + t.minor * cv;
                (
                    t.frame.point(ring * cu, ring * su, t.minor * sv),
                    t.frame.vector(-ring * su, ring * cu, 0.0),
                    t.frame
                        .vector(-t.minor * sv * cu, -t.minor * sv * su, t.minor * cv),
                )
            }
            Surface::Bezier(b) => b.derivatives(u, v),
            Surface::Nurbs(s) => s.derivatives(u, v),
        }
    }

    /// Unit normal `∂u × ∂v` at `(u, v)`. Analytic kinds use their closed
    /// form, which also holds at the poles of a sphere; `None` at a cone's
    /// apex and wherever a freeform surface has no normal.
    pub fn normal(&self, u: f64, v: f64) -> Option<Vec3> {
        let turn = || u.sin_cos();
        match self {
            Surface::Plane(p) => Some(p.frame.z()),
            Surface::Cylinder(c) => {
                let (su, cu) = turn();
                Some(c.frame.vector(cu, su, 0.0))
            }
            Surface::Cone(c) => {
                // At the apex, up to the rounding of `radius + v tan α`.
                let slope = c.half_angle.tan();
                if c.radius_at(v) <= 1e-12 * (c.radius + (v * slope).abs()) {
                    return None;
                }
                let (su, cu) = turn();
                c.frame.vector(cu, su, -c.half_angle.tan()).normalized()
            }
            Surface::Sphere(s) => {
                let (su, cu) = turn();
                let (sv, cv) = v.sin_cos();
                Some(s.frame.vector(cv * cu, cv * su, sv))
            }
            Surface::Torus(t) => {
                let (su, cu) = turn();
                let (sv, cv) = v.sin_cos();
                Some(t.frame.vector(cv * cu, cv * su, sv))
            }
            Surface::Bezier(b) => b.normal(u, v),
            Surface::Nurbs(s) => {
                let (_, du, dv) = s.derivatives(u, v);
                du.cross(dv).normalized()
            }
        }
    }

    /// The parameters of the point on the surface closest to `p`.
    ///
    /// Closed form for the analytic kinds; where the closest point is not
    /// unique (on a cylinder's axis, at a sphere's centre) the free
    /// parameter is 0. For Bézier and NURBS surfaces a search from the
    /// samples of a grid that are nearer than their neighbours, which
    /// finds a nearest point and, unless the surface folds more finely
    /// than the grid resolves or `p` is far from it, the nearest: see
    /// `project_numeric` for what is promised. On a surface that closes
    /// on itself a point at the seam may come back at either end of the
    /// parameter range; a point at a pole with any value of the parameter
    /// that runs around it.
    pub fn project(&self, p: Vec3) -> Result<(f64, f64), GeomError> {
        if !p.is_finite() {
            return Err(GeomError::InvalidParameter(
                "cannot project a non-finite point",
            ));
        }
        let angle = |y: f64, x: f64| {
            if x == 0.0 && y == 0.0 {
                0.0
            } else {
                wrap_angle(y.atan2(x))
            }
        };
        Ok(match self {
            Surface::Plane(s) => {
                let (x, y, _) = s.frame.local(p);
                (x, y)
            }
            Surface::Cylinder(s) => {
                let (x, y, z) = s.frame.local(p);
                (angle(y, x), z)
            }
            Surface::Cone(s) => {
                let (x, y, z) = s.frame.local(p);
                // In the half-plane (ρ, z) the cone is the ray from the
                // apex along (tan α, 1); project onto it, stopping at the apex.
                let slope = s.half_angle.tan();
                let v = ((x.hypot(y) - s.radius) * slope + z) / (slope * slope + 1.0);
                let apex = s.apex_v();
                (
                    angle(y, x),
                    if s.half_angle > 0.0 {
                        v.max(apex)
                    } else {
                        v.min(apex)
                    },
                )
            }
            Surface::Sphere(s) => {
                let (x, y, z) = s.frame.local(p);
                let rho = x.hypot(y);
                (
                    angle(y, x),
                    if rho == 0.0 && z == 0.0 {
                        0.0
                    } else {
                        z.atan2(rho)
                    },
                )
            }
            Surface::Torus(s) => {
                let (x, y, z) = s.frame.local(p);
                let off = x.hypot(y) - s.major;
                (angle(y, x), angle(z, off))
            }
            Surface::Bezier(_) | Surface::Nurbs(_) => self.project_numeric(p)?,
        })
    }

    /// A nearest point of a Bézier or NURBS surface found from `start`
    /// alone: the descent of [`Surface::project`] without the search over
    /// the whole surface that gives it its start. For a point known to be
    /// near the surface's point at `start` (the next of a run of points
    /// along a curve on the surface, started from where the last one
    /// landed) this costs a few evaluations where `project` costs
    /// hundreds. For the analytic kinds it is `project`, which has the
    /// answer in closed form.
    ///
    /// What comes back is where the walk from `start` settled: a point
    /// where the distance to `p` stops decreasing, to rounding, in every
    /// direction the walk can take within the domain. That is the nearest
    /// point only when `start` is close to it; from elsewhere the walk
    /// may settle on another dip of the distance, and the caller that
    /// needs the nearest point checks what it got (the distance, and that
    /// `p − S` is square to the surface) and falls back on `project`. The
    /// domain is taken as open, with no seam and no pole: a point across
    /// the seam of a surface that closes on itself is found at the seam,
    /// and one past a collapsed border at it, where the walk is held by
    /// the border and `p − S` is not square to the surface.
    ///
    /// A walk that does not settle within its steps, or meets a number
    /// that is not one, is [`GeomError::NoConvergence`]. A `start` outside
    /// the domain begins at its border; one that is not finite is
    /// [`GeomError::InvalidParameter`], as is a `p` that is not.
    pub fn project_from(&self, p: Vec3, start: (f64, f64)) -> Result<(f64, f64), GeomError> {
        self.project_from_with_derivatives(p, start)
            .map(|(at, _)| at)
    }

    /// [`Surface::project_from`], and [`Surface::derivatives`] at the
    /// parameters found (the walk worked them out at its last step: the
    /// same bits as asking for them again).
    #[allow(clippy::type_complexity)]
    pub fn project_from_with_derivatives(
        &self,
        p: Vec3,
        start: (f64, f64),
    ) -> Result<((f64, f64), (Vec3, Vec3, Vec3)), GeomError> {
        if !p.is_finite() {
            return Err(GeomError::InvalidParameter(
                "cannot project a non-finite point",
            ));
        }
        if !(start.0.is_finite() && start.1.is_finite()) {
            return Err(GeomError::InvalidParameter(
                "cannot start a projection from non-finite parameters",
            ));
        }
        if !matches!(self, Surface::Bezier(_) | Surface::Nurbs(_)) {
            let (u, v) = self.project(p)?;
            return Ok(((u, v), self.derivatives(u, v)));
        }
        let ((u0, u1), (v0, v1)) = self.domain();
        if ![u0, u1, v0, v1].iter().all(|x| x.is_finite()) {
            return Err(GeomError::Unsupported(
                "numeric projection onto an unbounded surface",
            ));
        }
        let start = (start.0.clamp(u0, u1), start.1.clamp(v0, v1));
        let (_, u, v, settled, at) = self.descend(p, start, (false, false), [false; 4]);
        if !(settled && u.is_finite() && v.is_finite()) {
            return Err(GeomError::NoConvergence(
                "the walk from the start of a projection did not settle",
            ));
        }
        Ok(((u, v), at))
    }

    /// The nearest point of a Bézier or NURBS surface, numerically.
    ///
    /// The surface is sampled on a grid of [`PROJECT_GRID`] steps each
    /// way. Every sample that is nearer to `p` than its neighbours is a
    /// place to start from, and around the nearest of them the surface is
    /// sampled four times as closely for more ([`PROJECT_STARTS`] in all
    /// at most, the nearest first). From each a damped Gauss–Newton
    /// iteration (Levenberg–Marquardt) walks downhill within the domain.
    /// The nearest of what they find is the answer, and it is never
    /// further from `p` than the nearest sample. When the walk from the
    /// nearest sample ends on `p` itself, that is the answer and nothing
    /// else is tried.
    ///
    /// What the grid finds out about the surface first:
    ///
    /// - **Closed.** Where the first and the last row of samples are the
    ///   same points, the surface closes on itself in that direction (a
    ///   can, a torus). The iteration then runs across the seam, and a
    ///   point just short of the seam comes back on its own side.
    /// - **Collapsed.** Where a whole border row is one point (a pole, the
    ///   middle of the teapot's lid) no sample of it says which way to go,
    ///   so the row is not started from; its neighbours are. The damping
    ///   keeps the step defined there, where the tangent plane is not.
    /// - **Borders.** At a border of an open surface the iteration drops
    ///   the direction that points out of the domain and goes on along
    ///   the border.
    ///
    /// **What is promised.** The result is a local nearest point (the
    /// distance does not decrease in any direction within the domain), to
    /// rounding when `p` is on the surface and to about 1e-12 of the
    /// domain otherwise. It is *the* nearest point when the nearest
    /// point's surroundings, out to where the distance to `p` stops
    /// growing, are wider than a cell of the grid (a quarter of a cell
    /// next to the nearest sample): always for a point closer to the
    /// surface than the smallest radius the surface curves with, on a
    /// surface whose features are larger than a sixteenth of its domain.
    /// Further off, or on a surface that folds more finely, another local
    /// nearest point may be returned.
    fn project_numeric(&self, p: Vec3) -> Result<(f64, f64), GeomError> {
        let ((u0, u1), (v0, v1)) = self.domain();
        if ![u0, u1, v0, v1].iter().all(|x| x.is_finite()) {
            return Err(GeomError::Unsupported(
                "numeric projection onto an unbounded surface",
            ));
        }
        let grid = self.project_grid((u0, u1), (v0, v1));
        let key = [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
        if let Some(found) = grid.memo.get(&key) {
            return Ok(found);
        }
        let found = self.project_from_grid(p, &grid, ((u0, u1), (v0, v1)))?;
        grid.memo.put(key, found);
        Ok(found)
    }

    /// [`Surface::project_numeric`], from the grid of samples.
    fn project_from_grid(
        &self,
        p: Vec3,
        grid: &ProjectGrid,
        ((u0, u1), (v0, v1)): ((f64, f64), (f64, f64)),
    ) -> Result<(f64, f64), GeomError> {
        const N: usize = PROJECT_GRID;
        let at = |i: usize, j: usize| {
            (
                u0 + (u1 - u0) * i as f64 / N as f64,
                v0 + (v1 - v0) * j as f64 / N as f64,
            )
        };
        let ProjectGrid {
            points,
            extent,
            closed_u,
            closed_v,
            collapsed,
            memo: _,
        } = grid;
        let (extent, closed_u, closed_v, collapsed) = (*extent, *closed_u, *closed_v, *collapsed);
        let skipped = |i: usize, j: usize| {
            (i == 0 && collapsed[0])
                || (i == N && (collapsed[1] || closed_u))
                || (j == 0 && collapsed[2])
                || (j == N && (collapsed[3] || closed_v))
        };
        let distance = |i: usize, j: usize| {
            points
                .get(i)
                .and_then(|row| row.get(j))
                .map_or(f64::INFINITY, |point| point.distance(p))
        };
        // The neighbour of a sample one step along, across the seam of a
        // closed direction; `None` beyond the border of an open one.
        let step = |index: usize, by: isize, closed: bool| -> Option<usize> {
            let moved = index as isize + by;
            if closed {
                Some(moved.rem_euclid(N as isize) as usize)
            } else {
                usize::try_from(moved).ok().filter(|&moved| moved <= N)
            }
        };

        // The samples nearer than their neighbours, the nearest first.
        let mut nearest_sample = (f64::INFINITY, 0, 0);
        let mut starts: Vec<(f64, f64, usize, usize)> = Vec::new();
        for i in 0..=N {
            for j in 0..=N {
                let d = distance(i, j);
                if d < nearest_sample.0 {
                    nearest_sample = (d, i, j);
                }
                if skipped(i, j) || !d.is_finite() {
                    continue;
                }
                let mut lowest = true;
                let mut reach = 0.0_f64;
                for (di, dj) in [
                    (-1, -1),
                    (-1, 0),
                    (-1, 1),
                    (0, -1),
                    (0, 1),
                    (1, -1),
                    (1, 0),
                    (1, 1),
                ] {
                    let (Some(ni), Some(nj)) = (step(i, di, closed_u), step(j, dj, closed_v))
                    else {
                        continue;
                    };
                    if let (Some(&here), Some(&there)) = (
                        points.get(i).and_then(|row| row.get(j)),
                        points.get(ni).and_then(|row| row.get(nj)),
                    ) {
                        reach = reach.max(here.distance(there));
                    }
                    if skipped(ni, nj) {
                        continue;
                    }
                    // Of two equally near neighbours the first counts.
                    let other = distance(ni, nj);
                    if other < d || (other == d && (ni, nj) < (i, j)) {
                        lowest = false;
                    }
                }
                if lowest {
                    starts.push((d, reach, i, j));
                }
            }
        }
        starts.sort_by(|a, b| a.0.total_cmp(&b.0).then((a.2, a.3).cmp(&(b.2, b.3))));

        // From the nearest of them first. A point of the surface is found
        // at once, and then there is nothing nearer to look for: the
        // common case, of points that are on the surface to begin with.
        let mut best: Option<(f64, f64, f64)> = None;
        if let Some(&(_, _, i, j)) = starts.first() {
            let (found, u, v, _, _) = self.descend(p, at(i, j), (closed_u, closed_v), collapsed);
            if found <= 16.0 * f64::EPSILON * (p.length() + extent) {
                return Ok((u, v));
            }
            best = Some((found, u, v));
        }
        // Around it the surface is then looked at four times as closely:
        // two nearest points a cell apart (the end of a tube and its side
        // just before the end) are one start to the grid.
        const FINE: usize = 4;
        let mut places: Vec<(f64, f64, f64, f64)> = Vec::new();
        if let Some(&(_, reach, i, j)) = starts.first() {
            let (cu, cv) = at(i, j);
            let (hu, hv) = ((u1 - u0) / (N * FINE) as f64, (v1 - v0) / (N * FINE) as f64);
            let mut fine = [[f64::INFINITY; 2 * FINE + 1]; 2 * FINE + 1];
            let spot = |a: usize, b: usize| {
                let u = cu + hu * (a as f64 - FINE as f64);
                let v = cv + hv * (b as f64 - FINE as f64);
                let inside =
                    (closed_u || (u0..=u1).contains(&u)) && (closed_v || (v0..=v1).contains(&v));
                // Not on a border that is one point: see above.
                let half = 0.5 * hu.min(hv);
                let on_a_pole = (collapsed[0] && u - u0 < half)
                    || (collapsed[1] && u1 - u < half)
                    || (collapsed[2] && v - v0 < half)
                    || (collapsed[3] && v1 - v < half);
                (inside && !on_a_pole).then_some((u, v))
            };
            for (a, row) in fine.iter_mut().enumerate() {
                for (b, d) in row.iter_mut().enumerate() {
                    if let Some((u, v)) = spot(a, b) {
                        *d = self.eval(u, v).distance(p);
                    }
                }
            }
            let at_fine = |a: usize, b: usize| {
                fine.get(a)
                    .and_then(|row| row.get(b))
                    .copied()
                    .unwrap_or(f64::INFINITY)
            };
            for a in 0..=2 * FINE {
                for b in 0..=2 * FINE {
                    let d = at_fine(a, b);
                    let Some((u, v)) = spot(a, b).filter(|_| d.is_finite()) else {
                        continue;
                    };
                    let lowest = (0..3).all(|da| {
                        (0..3).all(|db| {
                            let (Some(na), Some(nb)) =
                                ((a + da).checked_sub(1), (b + db).checked_sub(1))
                            else {
                                return true;
                            };
                            let other = at_fine(na, nb);
                            !(other < d || (other == d && (na, nb) < (a, b)))
                        })
                    });
                    if lowest {
                        places.push((d, reach / FINE as f64, u, v));
                    }
                }
            }
            places.sort_by(|x, y| {
                x.0.total_cmp(&y.0)
                    .then(x.2.total_cmp(&y.2))
                    .then(x.3.total_cmp(&y.3))
            });
        }
        // Then the other samples that are nearer than their neighbours.
        places.extend(starts.iter().skip(1).map(|&(d, reach, i, j)| {
            let (u, v) = at(i, j);
            (d, reach, u, v)
        }));

        for &(d, reach, u, v) in places.iter().take(PROJECT_STARTS) {
            // What is found from a sample is within its cells: it cannot
            // be nearer than the sample by more than they reach.
            if best.is_some_and(|(found, _, _)| d - reach > found) {
                continue;
            }
            let (found, u, v, _, _) = self.descend(
                p,
                (
                    if closed_u {
                        u0 + (u - u0).rem_euclid(u1 - u0)
                    } else {
                        u
                    },
                    if closed_v {
                        v0 + (v - v0).rem_euclid(v1 - v0)
                    } else {
                        v
                    },
                ),
                (closed_u, closed_v),
                collapsed,
            );
            if best.is_none_or(|(least, _, _)| found < least) {
                best = Some((found, u, v));
            }
        }
        // Never further than the nearest sample (a pole, when nothing
        // was started from; a start that could not improve).
        let (d, i, j) = nearest_sample;
        Ok(match best {
            Some((found, u, v)) if found <= d => (u, v),
            _ => at(i, j),
        })
    }

    /// The samples [`Surface::project_numeric`] starts from, of a Bézier
    /// or NURBS surface: a function of the surface alone, made the first
    /// time a point is projected onto it and kept with it (cloned with it)
    /// from then on. A tessellation or a mass integral projects every
    /// point of a face's boundary onto the face's surface, each from a
    /// grid of 289 evaluations; with the grid kept that is a look-up, and
    /// every projection the same, bit for bit.
    fn project_grid(&self, (u0, u1): (f64, f64), (v0, v1): (f64, f64)) -> Arc<ProjectGrid> {
        // Made uncounted (`crate::allowance::uncounted`): kept with the
        // surface and cloned with it, so whether it is made here depends
        // on what was done with the surface before.
        let made = || {
            Arc::new(crate::allowance::uncounted(|| {
                self.sample_grid((u0, u1), (v0, v1))
            }))
        };
        match self {
            Surface::Nurbs(s) => Arc::clone(s.project_grid.0.get_or_init(made)),
            Surface::Bezier(b) => Arc::clone(b.project_grid.0.get_or_init(made)),
            _ => made(),
        }
    }

    /// [`Surface::project_grid`], made.
    fn sample_grid(&self, (u0, u1): (f64, f64), (v0, v1): (f64, f64)) -> ProjectGrid {
        const N: usize = PROJECT_GRID;
        let at = |i: usize, j: usize| {
            (
                u0 + (u1 - u0) * i as f64 / N as f64,
                v0 + (v1 - v0) * j as f64 / N as f64,
            )
        };
        let mut points = [[Vec3::ZERO; N + 1]; N + 1];
        let mut extent = 0.0_f64;
        for (i, row) in points.iter_mut().enumerate() {
            for (j, point) in row.iter_mut().enumerate() {
                let (u, v) = at(i, j);
                *point = self.eval(u, v);
            }
        }
        let first = points.first().and_then(|row| row.first()).copied();
        let first = first.unwrap_or(Vec3::ZERO);
        for row in &points {
            for point in row {
                let reach = point.distance(first);
                if reach.is_finite() {
                    extent = extent.max(reach);
                }
            }
        }
        // The same point, as far as a sample can tell.
        let same = |a: Vec3, b: Vec3| a.distance(b) <= 1e-9 * extent;
        let row = |i: usize| points.get(i).map_or(&[][..], |row| row.as_slice());
        let column = |j: usize| points.iter().filter_map(move |row| row.get(j).copied());
        let closed_u = row(0).iter().zip(row(N)).all(|(&a, &b)| same(a, b));
        let closed_v = column(0).zip(column(N)).all(|(a, b)| same(a, b));
        let one_point = |points: &mut dyn Iterator<Item = Vec3>| {
            let Some(first) = points.next() else {
                return false;
            };
            for point in points {
                if !same(point, first) {
                    return false;
                }
            }
            true
        };
        // Border rows that are a single point: [u start, u end, v start, v end].
        let collapsed = [
            one_point(&mut row(0).iter().copied()),
            one_point(&mut row(N).iter().copied()),
            one_point(&mut column(0)),
            one_point(&mut column(N)),
        ];
        ProjectGrid {
            points,
            extent,
            closed_u,
            closed_v,
            collapsed,
            memo: ProjectMemo::default(),
        }
    }

    /// Walks downhill from `start` to a nearest point of `p` within the
    /// domain of a freeform surface: `(distance, u, v, settled)`, where
    /// `settled` says the walk ended where it found nothing more to gain
    /// (on `p` itself, at a step too short to move the parameters, with
    /// no step that helped however short, or held at a corner of the
    /// domain) and not because it ran out of steps or met a number that
    /// is not one.
    ///
    /// Levenberg–Marquardt (K. Levenberg 1944, D. Marquardt 1963) on the
    /// residual `S(u, v) − p`: each step solves
    /// `(JᵀJ + λ diag JᵀJ) δ = Jᵀ r`, and `λ` follows whether the step
    /// helped. With `λ` small this is Gauss–Newton, which lands on a
    /// point of the surface in a few steps; with `λ` large it is a short
    /// step down the gradient, defined also where the tangent plane is
    /// not.
    ///
    /// Gauss–Newton takes the surface for its tangent plane. For a point
    /// off the surface each step then falls short by the share the
    /// point's distance is of the radius the surface curves with, and
    /// near a tight curve it crawls. When steps are seen to fall short
    /// the curving of the surface is added to the equations (Newton's
    /// method), from differences of the tangents, wherever that leaves
    /// the distance a bowl.
    ///
    /// A step helps when it brings the surface point nearer by more than
    /// rounding. Next to the nearest point the distance is flat (it
    /// changes with the square of the step) and stops telling steps apart
    /// at the square root of the rounding; there a step helps when the
    /// distance stays what it is to rounding and the slope `Jᵀ r` gets
    /// smaller, which is what pins the parameters down to the rounding
    /// itself.
    fn descend(
        &self,
        p: Vec3,
        start: (f64, f64),
        closed: (bool, bool),
        collapsed: [bool; 4],
    ) -> (f64, f64, f64, bool, (Vec3, Vec3, Vec3)) {
        let ((u0, u1), (v0, v1)) = self.domain();
        // The walk evaluates the surface at three values of each parameter
        // around a step: the blend factors of a spline kept for them.
        let cache = std::cell::RefCell::new(crate::DerivativeCache::default());
        let eval = |u: f64, v: f64| match self {
            Surface::Nurbs(s) => s.derivatives_cached(u, v, &mut cache.borrow_mut()),
            _ => self.derivatives(u, v),
        };
        // A parameter after a step: around a closed direction, or stopped
        // at the border of an open one.
        let place = |x: f64, (low, high): (f64, f64), closed: bool| {
            if closed {
                let wrapped = low + (x - low).rem_euclid(high - low);
                if wrapped.is_finite() { wrapped } else { low }
            } else {
                x.clamp(low, high)
            }
        };
        // At a border, a direction that leads out of the domain is held
        // and the other one followed alone.
        let held = |x: f64, (low, high): (f64, f64), pull: f64, closed: bool| {
            !closed && ((x <= low && pull < 0.0) || (x >= high && pull > 0.0))
        };
        // Everything about a place: the distance, the normal equations,
        // which directions are held, and the slope along the free ones as
        // a length.
        let look = |u: f64, v: f64| {
            let (s, su, sv) = eval(u, v);
            let r = p - s;
            let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
            let (ru, rv) = (r.dot(su), r.dot(sv));
            let hold = (
                held(u, (u0, u1), ru, closed.0),
                held(v, (v0, v1), rv, closed.1),
            );
            let along = |pull: f64, square: f64, hold: bool| {
                if hold || square <= 0.0 {
                    0.0
                } else {
                    pull.abs() / square.sqrt()
                }
            };
            let slope = along(ru, a, hold.0).max(along(rv, c, hold.1));
            (r, (a, b, c), (ru, rv), hold, slope, (s, su, sv))
        };
        // How the tangents change from `(u, v)`, by differences of them a
        // little way off each way (or one way, at a border): what the
        // curving of the surface adds to the normal equations, `r · S''`.
        let bend = |u: f64, v: f64, r: Vec3| {
            let span = |x: f64, (low, high): (f64, f64), closed: bool| {
                let h = 1e-5 * (high - low);
                if closed {
                    (x - h, x + h)
                } else {
                    ((x - h).max(low), (x + h).min(high))
                }
            };
            let (ua, ub) = span(u, (u0, u1), closed.0);
            let (va, vb) = span(v, (v0, v1), closed.1);
            let (_, su_a, sv_a) = eval(ua, v);
            let (_, su_b, sv_b) = eval(ub, v);
            let (_, _, sv_c) = eval(u, va);
            let (_, _, sv_d) = eval(u, vb);
            (
                r.dot(su_b - su_a) / (ub - ua),
                r.dot(sv_b - sv_a) / (ub - ua),
                r.dot(sv_d - sv_c) / (vb - va),
            )
        };
        let (mut u, mut v) = start;
        let mut here = look(u, v);
        let mut lambda = 1e-3;
        // Gauss–Newton leaves the curving of the surface out. That costs
        // nothing for a point on the surface, and for a point off it
        // makes every step fall short by the share its distance is of the
        // radius the surface curves with. Once steps are seen to fall
        // short, the curving is put in (Newton's method).
        let mut curved = false;
        let mut settled = false;
        for _ in 0..PROJECT_STEPS {
            let (r, (a, b, c), (ru, rv), (hold_u, hold_v), slope, _) = here;
            let distance = r.length();
            // Not a number: nothing to follow.
            if distance.is_nan() {
                break;
            }
            // On the surface, no step that helps however short, or held
            // at a corner: nowhere to go.
            if distance <= 0.0 || lambda > 1e12 || (hold_u && hold_v) {
                settled = true;
                break;
            }
            let (mut a, mut b, mut c) = (a, b, c);
            if curved {
                let (ruu, ruv, rvv) = bend(u, v, r);
                let (na, nb, nc) = (a - ruu, b - ruv, c - rvv);
                // Only where the distance is a bowl, along what is free to
                // move: elsewhere Newton's step leads uphill. (Along a
                // border the distance still falls towards the outside,
                // and is a bowl only along the border.)
                let bowl = match (hold_u, hold_v) {
                    (true, _) => nc > 0.0,
                    (_, true) => na > 0.0,
                    _ => na > 0.0 && nc > 0.0 && na * nc - nb * nb > 0.0,
                };
                if bowl {
                    (a, b, c) = (na, nb, nc);
                }
            }
            // Each parameter is damped in its own measure (Marquardt's
            // scaling): one the surface barely moves with, around a pole,
            // is then as free as the other. One it does not move with at
            // all has nothing to say.
            let (hold_u, hold_v) = (hold_u || a <= 0.0, hold_v || c <= 0.0);
            if hold_u && hold_v {
                settled = true;
                break;
            }
            let (a, c) = (a * (1.0 + lambda), c * (1.0 + lambda));
            let (mut du, mut dv) = if hold_u {
                (0.0, rv / c)
            } else if hold_v {
                (ru / a, 0.0)
            } else {
                let det = a * c - b * b;
                ((c * ru - b * rv) / det, (a * rv - b * ru) / det)
            };
            // A step that would leave the domain stops at the border, and
            // the other parameter makes the best of that. At a border
            // that is one point it stops half way: on a pole there may be
            // no tangent left to go on by, and a point next to a pole is
            // reached from next to it.
            let stop = |x: f64, step: f64, (low, high): (f64, f64), poles: (bool, bool)| {
                let (border, pole) = if step < 0.0 {
                    (low, poles.0)
                } else {
                    (high, poles.1)
                };
                if pole { 0.5 * (border - x) } else { border - x }
            };
            let [pole_u0, pole_u1, pole_v0, pole_v1] = collapsed;
            let leaves_u = !closed.0
                && !hold_u
                && (!(u0..=u1).contains(&(u + du))
                    || (pole_u0 && u + du <= u0)
                    || (pole_u1 && u + du >= u1));
            if leaves_u {
                du = stop(u, du, (u0, u1), (pole_u0, pole_u1));
                if !hold_v {
                    dv = (rv - b * du) / c;
                }
            }
            let leaves_v = !closed.1
                && !hold_v
                && (!(v0..=v1).contains(&(v + dv))
                    || (pole_v0 && v + dv <= v0)
                    || (pole_v1 && v + dv >= v1));
            if leaves_v {
                dv = stop(v, dv, (v0, v1), (pole_v0, pole_v1));
                if !hold_u && !leaves_u && (u0..=u1).contains(&(u + (ru - b * dv) / a)) {
                    du = (ru - b * dv) / a;
                }
            }
            if !(du.is_finite() && dv.is_finite()) {
                // No tangent at all here, or a number that is not one:
                // nothing to follow.
                break;
            }
            let next = (
                place(u + du, (u0, u1), closed.0),
                place(v + dv, (v0, v1), closed.1),
            );
            let moved = du.abs() / (u1 - u0) + dv.abs() / (v1 - v0);
            let there = look(next.0, next.1);
            let reached = there.0.length();
            // What rounding makes of a distance: the difference of two
            // positions, each good to its own last place.
            let rounding = 16.0 * f64::EPSILON * (p.length() + distance);
            let level = reached <= distance + rounding;
            if reached < distance - rounding || (level && there.4 < slope) {
                // A step of Gauss–Newton on a point of the surface takes
                // the slope down to its square; one that leaves a quarter
                // of it is falling short.
                curved |= there.4 > 0.25 * slope;
                (u, v, here) = (next.0, next.1, there);
                lambda = (lambda / 8.0).max(1e-14);
            } else {
                lambda *= 8.0;
            }
            // A step this small is the rounding of the parameters.
            if moved <= 4.0 * f64::EPSILON {
                settled = true;
                break;
            }
        }
        (here.0.length(), u, v, settled, here.5)
    }
}

impl Bits for Plane {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Plane { frame } = self;
        frame.push_bits(out);
    }
}

impl Bits for Cylinder {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Cylinder { frame, radius } = self;
        frame.push_bits(out);
        radius.push_bits(out);
    }
}

impl Bits for Cone {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Cone {
            frame,
            radius,
            half_angle,
        } = self;
        frame.push_bits(out);
        radius.push_bits(out);
        half_angle.push_bits(out);
    }
}

impl Bits for Sphere {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Sphere { frame, radius } = self;
        frame.push_bits(out);
        radius.push_bits(out);
    }
}

impl Bits for Torus {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Torus {
            frame,
            major,
            minor,
        } = self;
        frame.push_bits(out);
        major.push_bits(out);
        minor.push_bits(out);
    }
}

impl Bits for Surface {
    fn push_bits(&self, out: &mut Vec<u64>) {
        match self {
            Surface::Plane(s) => {
                out.push(0);
                s.push_bits(out);
            }
            Surface::Cylinder(s) => {
                out.push(1);
                s.push_bits(out);
            }
            Surface::Cone(s) => {
                out.push(2);
                s.push_bits(out);
            }
            Surface::Sphere(s) => {
                out.push(3);
                s.push_bits(out);
            }
            Surface::Torus(s) => {
                out.push(4);
                s.push_bits(out);
            }
            Surface::Bezier(s) => {
                out.push(5);
                s.push_bits(out);
            }
            Surface::Nurbs(s) => {
                out.push(6);
                s.push_bits(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(scale: f64) -> Frame {
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap()
    }

    fn bumpy(scale: f64) -> BezierPatch {
        let mut net = [[Vec3::ZERO; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            for (j, p) in row.iter_mut().enumerate() {
                let (x, y) = (i as f64, j as f64 * 1.5);
                *p = Vec3::new(x, y, ((i * 7 + j * 3) % 5) as f64 * 0.3 - 0.2 * x) * scale;
            }
        }
        BezierPatch::bicubic(net).unwrap()
    }

    /// A surface remembers the projections it made, and lets go of them
    /// when full: whatever it holds, every projection is the one a surface
    /// that never kept anything makes, bit for bit, with more points than
    /// it keeps asked twice over.
    #[test]
    fn remembered_projections_are_the_ones_made_afresh() {
        let make = || Surface::Nurbs(NurbsSurface::from_bezier(&bumpy(1.0)).unwrap());
        let kept = make();
        let count = PROJECTIONS_PER_SURFACE + 500;
        let point = |k: usize| {
            let t = k as f64;
            Vec3::new(
                (t * 0.0137).sin() * 2.0 + 1.5,
                (t * 0.0261).cos() * 3.0 + 2.0,
                (t * 0.0093).sin() * 1.5,
            )
        };
        let fresh: Vec<(f64, f64)> = (0..count)
            .step_by(97)
            .map(|k| make().project(point(k)).unwrap())
            .collect();
        for _ in 0..2 {
            for k in 0..count {
                let _ = kept.project(point(k));
            }
            for (k, want) in (0..count).step_by(97).zip(&fresh) {
                let got = kept.project(point(k)).unwrap();
                assert_eq!(
                    (got.0.to_bits(), got.1.to_bits()),
                    (want.0.to_bits(), want.1.to_bits())
                );
            }
        }
    }

    /// The derivatives a walk from a start comes back with are those of the
    /// surface at the parameters it found, bit for bit.
    #[test]
    fn a_walk_gives_the_derivatives_where_it_ends() {
        for scale in [1e-3, 1.0, 1e6] {
            let mut all = surfaces(scale);
            all.push(Surface::Nurbs(
                NurbsSurface::from_bezier(&bumpy(scale)).unwrap(),
            ));
            for surface in all {
                for k in 0..30 {
                    let t = f64::from(k);
                    let p = Vec3::new(
                        (t * 0.37).sin() * 2.0 + 1.5,
                        (t * 0.61).cos() * 3.0,
                        (t * 0.23).sin(),
                    ) * scale;
                    let start = (0.1 + 0.02 * t, 0.9 - 0.025 * t);
                    let Ok(((u, v), at)) = surface.project_from_with_derivatives(p, start) else {
                        continue;
                    };
                    let want = surface.derivatives(u, v);
                    let bits = |(a, b, c): (Vec3, Vec3, Vec3)| {
                        [a, b, c].map(|q| [q.x.to_bits(), q.y.to_bits(), q.z.to_bits()])
                    };
                    assert_eq!(bits(at), bits(want), "{}", surface.kind());
                    assert_eq!(surface.project_from(p, start).ok(), Some((u, v)));
                }
            }
        }
    }

    /// The samples a projection starts from are kept with the surface
    /// once made: every later projection, and one onto a clone, is the
    /// projection onto a surface that never kept them, bit for bit.
    #[test]
    fn a_kept_projection_grid_projects_as_a_fresh_one() {
        for scale in [1e-3, 1.0, 1e6] {
            let nurbs = NurbsSurface::from_bezier(&bumpy(scale)).unwrap();
            let bezier = || Surface::Bezier(bumpy(scale));
            let spline = || Surface::Nurbs(NurbsSurface::from_bezier(&bumpy(scale)).unwrap());
            let makers: [&dyn Fn() -> Surface; 2] = [&bezier, &spline];
            for make in makers {
                let kept = make();
                for k in 0..40 {
                    let t = f64::from(k);
                    let p = Vec3::new(
                        (t * 0.37).sin() * 2.0 + 1.5,
                        (t * 0.61).cos() * 3.0 + 2.0,
                        (t * 0.23).sin() * 1.5,
                    ) * scale;
                    let fresh = make().project(p).unwrap();
                    let again = kept.project(p).unwrap();
                    let cloned = kept.clone().project(p).unwrap();
                    for (a, b) in [(fresh, again), (fresh, cloned)] {
                        assert_eq!(a.0.to_bits(), b.0.to_bits());
                        assert_eq!(a.1.to_bits(), b.1.to_bits());
                    }
                }
            }
            // Kept or not, a surface is its numbers.
            let used = Surface::Nurbs(nurbs.clone());
            let _ = used.project(Vec3::ZERO);
            assert_eq!(used, Surface::Nurbs(nurbs.clone()));
            assert_eq!(format!("{used:?}"), format!("{:?}", Surface::Nurbs(nurbs)));
        }
    }

    fn surfaces(scale: f64) -> Vec<Surface> {
        let f = frame(scale);
        vec![
            Surface::Plane(Plane::new(f)),
            Surface::Cylinder(Cylinder::new(f, 2.0 * scale).unwrap()),
            Surface::Cone(Cone::new(f, 1.5 * scale, 0.4).unwrap()),
            Surface::Cone(Cone::new(f, 1.5 * scale, -0.7).unwrap()),
            Surface::Sphere(Sphere::new(f, 3.0 * scale).unwrap()),
            Surface::Torus(Torus::new(f, 4.0 * scale, 1.0 * scale).unwrap()),
            Surface::Bezier(bumpy(scale)),
            Surface::Nurbs(NurbsSurface::from_bezier(&bumpy(scale)).unwrap()),
        ]
    }

    /// Interior sample parameters of a surface, away from poles and apexes.
    fn samples(s: &Surface, scale: f64) -> Vec<(f64, f64)> {
        let ((u0, u1), (v0, v1)) = s.domain();
        let span = |a: f64, b: f64| match (a.is_finite(), b.is_finite()) {
            (true, true) => (a, b),
            (true, false) => (a, a + 4.0 * scale),
            (false, true) => (b - 4.0 * scale, b),
            (false, false) => (-3.0 * scale, 3.0 * scale),
        };
        let ((u0, u1), (v0, v1)) = (span(u0, u1), span(v0, v1));
        let mut out = Vec::new();
        for i in 1..8 {
            for j in 1..8 {
                out.push((
                    u0 + (u1 - u0) * f64::from(i) / 8.0,
                    v0 + (v1 - v0) * f64::from(j) / 8.0,
                ));
            }
        }
        out
    }

    #[test]
    fn points_satisfy_the_implicit_equations() {
        for scale in [1e-3, 1.0, 1e6] {
            let f = frame(scale);
            for s in surfaces(scale) {
                for (u, v) in samples(&s, scale) {
                    let (x, y, z) = f.local(s.eval(u, v));
                    let rho = x.hypot(y);
                    let residual = match &s {
                        Surface::Plane(_) => z,
                        Surface::Cylinder(c) => rho - c.radius(),
                        Surface::Cone(c) => rho - (c.radius() + z * c.half_angle().tan()),
                        Surface::Sphere(sp) => (rho * rho + z * z).sqrt() - sp.radius(),
                        Surface::Torus(t) => {
                            ((rho - t.radii().0).powi(2) + z * z).sqrt() - t.radii().1
                        }
                        Surface::Bezier(_) | Surface::Nurbs(_) => 0.0,
                    };
                    assert!(
                        residual.abs() <= 1e-12 * scale,
                        "{} at ({u}, {v}), scale {scale}",
                        s.kind()
                    );
                }
            }
        }
    }

    #[test]
    fn derivatives_match_central_differences_and_normals_are_outward() {
        for scale in [1e-3, 1.0, 1e6] {
            let f = frame(scale);
            for s in surfaces(scale) {
                for (u, v) in samples(&s, scale) {
                    let (p, du, dv) = s.derivatives(u, v);
                    assert_eq!(p, s.eval(u, v));
                    let analytic = !matches!(s, Surface::Bezier(_) | Surface::Nurbs(_));
                    // Angles and Bézier parameters are O(1); lengths scale.
                    let hu = if matches!(s, Surface::Plane(_)) {
                        1e-6 * scale
                    } else {
                        1e-6
                    };
                    let hv = if matches!(
                        s,
                        Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_)
                    ) {
                        1e-6 * scale
                    } else {
                        1e-6
                    };
                    let fd_u = (s.eval(u + hu, v) - s.eval(u - hu, v)) * (0.5 / hu);
                    let fd_v = (s.eval(u, v + hv) - s.eval(u, v - hv)) * (0.5 / hv);
                    assert!(
                        du.distance(fd_u) <= 1e-6 * du.length().max(1e-300),
                        "{} du at ({u}, {v})",
                        s.kind()
                    );
                    assert!(
                        dv.distance(fd_v) <= 1e-6 * dv.length().max(1e-300),
                        "{} dv at ({u}, {v})",
                        s.kind()
                    );

                    let n = s.normal(u, v).unwrap();
                    assert!((n.length() - 1.0).abs() < 1e-14);
                    assert!(
                        n.dot(du).abs() <= 1e-12 * du.length()
                            && n.dot(dv).abs() <= 1e-12 * dv.length()
                    );
                    assert!(
                        n.dot(du.cross(dv)) > 0.0,
                        "{}: normal must be du × dv",
                        s.kind()
                    );
                    if analytic && !matches!(s, Surface::Plane(_)) {
                        // Away from the axis (for the torus: from the tube's centre circle).
                        let (x, y, z) = f.local(p);
                        let from = match &s {
                            Surface::Sphere(_) => f.origin(),
                            Surface::Torus(t) => {
                                let k = t.radii().0 / x.hypot(y);
                                f.point(x * k, y * k, 0.0)
                            }
                            _ => f.point(0.0, 0.0, z),
                        };
                        assert!(
                            n.dot(p - from) > 0.0,
                            "{}: normal must point outward",
                            s.kind()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn projecting_a_point_of_the_surface_returns_its_parameters() {
        for scale in [1e-3, 1.0, 1e6] {
            for s in surfaces(scale) {
                let ((u0, u1), (v0, v1)) = s.domain();
                for (u, v) in samples(&s, scale) {
                    let p = s.eval(u, v);
                    let (pu, pv) = s.project(p).unwrap();
                    assert!(
                        s.eval(pu, pv).distance(p) <= 1e-9 * scale,
                        "{} at ({u}, {v}), scale {scale}: got ({pu}, {pv})",
                        s.kind()
                    );
                    let size = |a: f64, b: f64| if (b - a).is_finite() { b - a } else { scale };
                    assert!(
                        (pu - u).abs() <= 1e-7 * size(u0, u1),
                        "{} u at ({u}, {v}): {pu}",
                        s.kind()
                    );
                    assert!(
                        (pv - v).abs() <= 1e-7 * size(v0, v1),
                        "{} v at ({u}, {v}): {pv}",
                        s.kind()
                    );
                }
            }
        }
    }

    #[test]
    fn the_foot_of_an_off_surface_point_lies_along_the_normal() {
        for s in surfaces(1.0) {
            for (u, v) in samples(&s, 1.0).into_iter().step_by(5) {
                let (foot, n) = (s.eval(u, v), s.normal(u, v).unwrap());
                // Lift the point off the surface by less than any radius of curvature.
                for lift in [0.3, -0.2] {
                    let p = foot + n * lift;
                    let (pu, pv) = s.project(p).unwrap();
                    let found = s.eval(pu, pv);
                    assert!(
                        found.distance(foot) <= 1e-7,
                        "{} at ({u}, {v}) lift {lift}: {found:?}",
                        s.kind()
                    );
                }
            }
        }
    }

    #[test]
    fn poles_apexes_and_axes_are_handled() {
        let f = Frame::WORLD;
        let sphere = Surface::Sphere(Sphere::new(f, 2.0).unwrap());
        // Closed-form normal at the pole, where ∂u vanishes.
        assert!(sphere.derivatives(1.0, FRAC_PI_2).1.length() < 1e-15);
        assert!(sphere.normal(1.0, FRAC_PI_2).unwrap().distance(Vec3::Z) < 1e-15);
        assert_eq!(sphere.project(Vec3::ZERO).unwrap(), (0.0, 0.0));
        assert_eq!(
            sphere.project(Vec3::new(0.0, 0.0, 5.0)).unwrap(),
            (0.0, FRAC_PI_2)
        );

        let cone = Cone::new(f, 1.0, 0.5_f64.atan()).unwrap();
        assert!((cone.apex_v() + 2.0).abs() < 1e-14);
        let cone = Surface::Cone(cone);
        assert!(cone.normal(0.3, -2.0).is_none(), "no normal at the apex");
        assert!(cone.eval(0.3, -2.0).distance(Vec3::new(0.0, 0.0, -2.0)) < 1e-14);
        // Points beyond the apex project onto the apex, not the other nappe.
        let (_, v) = cone.project(Vec3::new(0.1, 0.0, -9.0)).unwrap();
        assert!((v + 2.0).abs() < 1e-14);
        assert!((cone.domain().1.0 + 2.0).abs() < 1e-14);

        let cylinder = Surface::Cylinder(Cylinder::new(f, 1.0).unwrap());
        assert_eq!(
            cylinder.project(Vec3::new(0.0, 0.0, 3.0)).unwrap(),
            (0.0, 3.0)
        );
        let (u, _) = cylinder.project(Vec3::new(1.0, -1e-300, 0.0)).unwrap();
        assert!((0.0..TAU).contains(&u));
        for s in surfaces(1.0) {
            assert!(
                s.project(Vec3::new(0.0, f64::NAN, 0.0)).is_err(),
                "{}",
                s.kind()
            );
        }
    }

    #[test]
    fn invalid_dimensions_are_rejected() {
        let f = Frame::WORLD;
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(Cylinder::new(f, bad).is_err());
            assert!(Sphere::new(f, bad).is_err());
            assert!(Torus::new(f, 5.0, bad).is_err());
        }
        assert!(Torus::new(f, 1.0, 1.0).is_err(), "tube touches the axis");
        assert!(Torus::new(f, 1.0, 2.0).is_err());
        assert!(Cone::new(f, -1.0, 0.3).is_err());
        assert!(Cone::new(f, 1.0, 0.0).is_err(), "that is a cylinder");
        assert!(Cone::new(f, 1.0, FRAC_PI_2).is_err());
        assert!(Cone::new(f, 1.0, f64::NAN).is_err());
        assert!(Cone::new(f, 0.0, 0.3).is_ok(), "apex at the origin");
    }
}
