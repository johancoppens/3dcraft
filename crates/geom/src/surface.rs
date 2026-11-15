//! Surfaces: analytic kinds stay analytic, everything else is Bézier or NURBS.
//!
//! Every analytic surface is placed by a [`Frame`] and parametrised so that
//! `∂u × ∂v` points away from the axis or centre (outward).

use crate::curve::wrap_angle;
use crate::{BezierPatch, GeomError, NurbsSurface, RationalBezierPatch};
use craft3d_math::Portable;
use craft3d_math::{Bits, Frame, Vec3};
use std::f64::consts::{FRAC_PI_2, TAU};
use std::sync::{Arc, OnceLock};

/// Steps each way of the grid a numeric projection starts from.
const PROJECT_GRID: usize = 16;
/// Samples a numeric projection starts from, at most.
const PROJECT_STARTS: usize = 6;
/// Steps of one descent of a numeric projection, at most.
const PROJECT_STEPS: usize = 80;
/// Bézier pieces the search for a foot that a descent ended short of
/// looks at, at most ([`Surface::nearest_by_pieces`]).
const PIECE_LOOKS: usize = 1024;
/// Walks that search takes inside pieces, at most, and the steps of each.
const PIECE_WALKS: usize = 64;
const PIECE_STEPS: usize = 16;
/// How many times that search splits a piece, at most.
const PIECE_DEPTH: usize = 48;

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

/// How many projections all surfaces remember together, at most, in
/// [`SHARDS`] parts by the surface (so that threads that project onto
/// different surfaces rarely wait for each other). Each part lets its
/// oldest go first, whichever of its surfaces it is of: a surface keeps at
/// most a part's share (a sixteenth of the whole), and the ones left idle
/// make room for those in use. About 56 bytes each ([`Shard`]).
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
const PROJECTIONS: usize = 1 << 20;
#[cfg(all(target_arch = "wasm32", not(test)))]
const PROJECTIONS: usize = 1 << 16;
/// In the unit tests, room enough that the tests running beside each other
/// do not push out what one of them looks for.
#[cfg(test)]
const PROJECTIONS: usize = 1 << 14;
const SHARDS: usize = 16;

/// A projection remembered: the surface's grid ([`ProjectMemo::id`]) and
/// the bits of the point.
type ProjectionKey = (u64, [u64; 3]);

/// A projection remembered: the parameters found, and the work finding
/// them took (`crate::allowance`), which a hit charges again, so that a
/// projection counts the same remembered or not.
type Projected = ((f64, f64), u64);

/// No entry, in [`Shard::table`].
const EMPTY: u32 = u32::MAX;

/// One part of the projections remembered: a ring of the projections (each
/// point's bits held once, there) in the order they came, the oldest
/// overwritten when it is full, and a table of places in the ring by the
/// point (open addressing, a slot after another where one is taken; a
/// place let go of moves the ones after it back, so that no search stops
/// short). The hash only decides where a place is looked for: what is
/// found is compared in full.
struct Shard {
    ring: Vec<(ProjectionKey, Projected)>,
    /// The ring's next place to write.
    next: usize,
    table: Vec<u32>,
    /// How many projections the ring holds at most.
    room: usize,
}

impl Shard {
    const fn new(room: usize) -> Shard {
        Shard {
            ring: Vec::new(),
            next: 0,
            table: Vec::new(),
            room,
        }
    }

    /// Where `key` is looked for first in the table (`mask` its size less
    /// one).
    fn home(key: &ProjectionKey, mask: usize) -> usize {
        let (id, [x, y, z]) = *key;
        let mut h = id.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for w in [x, y, z] {
            h = (h ^ w).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            h ^= h >> 31;
        }
        usize::try_from(h).unwrap_or(0) & mask
    }

    /// The table's slot that holds `key`, if any.
    fn slot_of(&self, key: &ProjectionKey) -> Option<usize> {
        let mask = self.table.len().checked_sub(1)?;
        let mut at = Shard::home(key, mask);
        for _ in 0..self.table.len() {
            let place = *self.table.get(at)?;
            if place == EMPTY {
                return None;
            }
            if self
                .ring
                .get(place as usize)
                .is_some_and(|(held, _)| held == key)
            {
                return Some(at);
            }
            at = (at + 1) & mask;
        }
        None
    }

    fn get(&self, key: &ProjectionKey) -> Option<Projected> {
        let slot = self.slot_of(key)?;
        let place = *self.table.get(slot)?;
        self.ring.get(place as usize).map(|(_, found)| *found)
    }

    /// Lets go of the table's slot `slot`, moving back the slots after it
    /// whose search passes over it.
    fn vacate(&mut self, mut slot: usize) {
        let Some(mask) = self.table.len().checked_sub(1) else {
            return;
        };
        let mut next = (slot + 1) & mask;
        for _ in 0..self.table.len() {
            let Some(&place) = self.table.get(next) else {
                return;
            };
            if place == EMPTY {
                break;
            }
            let home = self
                .ring
                .get(place as usize)
                .map_or(next, |(key, _)| Shard::home(key, mask));
            // Whether the search from `home` to `next` passes `slot`.
            let passes = if slot <= next {
                home <= slot || home > next
            } else {
                home <= slot && home > next
            };
            if passes {
                if let Some(cell) = self.table.get_mut(slot) {
                    *cell = place;
                }
                slot = next;
            }
            next = (next + 1) & mask;
        }
        if let Some(cell) = self.table.get_mut(slot) {
            *cell = EMPTY;
        }
    }

    fn put(&mut self, key: ProjectionKey, found: Projected) {
        if self.room == 0 || self.slot_of(&key).is_some() {
            return;
        }
        if self.table.is_empty() {
            let size = (2 * self.room).next_power_of_two();
            self.table = vec![EMPTY; size];
        }
        let place = self.next;
        if place < self.ring.len() {
            // The oldest goes.
            let oldest = self.ring.get(place).map(|(held, _)| *held);
            if let Some(slot) = oldest.and_then(|held| self.slot_of(&held)) {
                self.vacate(slot);
            }
            if let Some(cell) = self.ring.get_mut(place) {
                *cell = (key, found);
            }
        } else {
            self.ring.push((key, found));
        }
        self.next = (place + 1) % self.room;
        let Some(mask) = self.table.len().checked_sub(1) else {
            return;
        };
        let mut at = Shard::home(&key, mask);
        for _ in 0..self.table.len() {
            match self.table.get_mut(at) {
                Some(cell) if *cell == EMPTY => {
                    *cell = u32::try_from(place).unwrap_or(EMPTY);
                    return;
                }
                _ => at = (at + 1) & mask,
            }
        }
    }

    /// How many projections it holds, and the bytes its ring and table take.
    fn size(&self) -> (usize, usize) {
        (
            self.ring.len(),
            self.ring.capacity() * std::mem::size_of::<(ProjectionKey, Projected)>()
                + self.table.capacity() * std::mem::size_of::<u32>(),
        )
    }
}

static PROJECTIONS_KEPT: [std::sync::Mutex<Shard>; SHARDS] =
    [const { std::sync::Mutex::new(Shard::new(PROJECTIONS / SHARDS)) }; SHARDS];

/// How many projections all surfaces remember, and about how many bytes
/// they take.
pub fn kept_projections() -> (usize, usize) {
    PROJECTIONS_KEPT
        .iter()
        .fold((0, 0), |(count, bytes), shard| {
            let (c, b) = shard
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .size();
            (count + c, bytes + b)
        })
}

/// Lets go of every projection remembered (each is found again, the same,
/// when asked for): what a long run of unrelated work (`document.sample`'s
/// points) left behind does not stay until it is pushed out.
pub fn forget_projections() {
    for shard in &PROJECTIONS_KEPT {
        let mut shard = shard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let room = shard.room;
        *shard = Shard::new(room);
    }
}

/// The next grid's number: every grid has its own, never used again, so
/// that what is remembered of a grid let go is never taken for another's.
static GRIDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    /// Projections this thread found remembered (for the tests).
    static HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The projections a surface remembers ([`ProjectGrid::memo`]): its number
/// in the projections all surfaces remember.
#[derive(Debug)]
struct ProjectMemo {
    id: u64,
}

impl Default for ProjectMemo {
    fn default() -> Self {
        ProjectMemo {
            id: GRIDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }
}

impl ProjectMemo {
    fn shard(&self) -> std::sync::MutexGuard<'static, Shard> {
        let at = usize::try_from(self.id % SHARDS as u64).unwrap_or(0);
        PROJECTIONS_KEPT
            .get(at)
            .unwrap_or(&PROJECTIONS_KEPT[0])
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn get(&self, key: &[u64; 3]) -> Option<(f64, f64)> {
        let found = self.shard().get(&(self.id, *key));
        #[cfg(test)]
        if found.is_some() {
            HITS.with(|hits| hits.set(hits.get() + 1));
        }
        let (found, units) = found?;
        crate::allowance::charge(units);
        Some(found)
    }

    fn put(&self, key: [u64; 3], found: (f64, f64), units: u64) {
        self.shard().put((self.id, key), (found, units));
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
        -self.radius / self.half_angle.ptan()
    }

    pub(crate) fn radius_at(&self, v: f64) -> f64 {
        self.radius + v * self.half_angle.ptan()
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

    /// Whether it is freeform (a Bézier patch or a NURBS surface) rather
    /// than one of the analytic surfaces, whose extremes are known in
    /// closed form.
    pub fn is_freeform(&self) -> bool {
        match self {
            Surface::Plane(_)
            | Surface::Cylinder(_)
            | Surface::Cone(_)
            | Surface::Sphere(_)
            | Surface::Torus(_) => false,
            Surface::Bezier(_) | Surface::Nurbs(_) => true,
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
        let turn = || u.psin_cos();
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
                let (r, slope) = (c.radius_at(v), c.half_angle.ptan());
                (
                    c.frame.point(r * cu, r * su, v),
                    c.frame.vector(-r * su, r * cu, 0.0),
                    c.frame.vector(slope * cu, slope * su, 1.0),
                )
            }
            Surface::Sphere(s) => {
                let (su, cu) = turn();
                let (sv, cv) = v.psin_cos();
                let r = s.radius;
                (
                    s.frame.point(r * cv * cu, r * cv * su, r * sv),
                    s.frame.vector(-r * cv * su, r * cv * cu, 0.0),
                    s.frame.vector(-r * sv * cu, -r * sv * su, r * cv),
                )
            }
            Surface::Torus(t) => {
                let (su, cu) = turn();
                let (sv, cv) = v.psin_cos();
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
        let turn = || u.psin_cos();
        match self {
            Surface::Plane(p) => Some(p.frame.z()),
            Surface::Cylinder(c) => {
                let (su, cu) = turn();
                Some(c.frame.vector(cu, su, 0.0))
            }
            Surface::Cone(c) => {
                // At the apex, up to the rounding of `radius + v tan α`.
                let slope = c.half_angle.ptan();
                if c.radius_at(v) <= 1e-12 * (c.radius + (v * slope).abs()) {
                    return None;
                }
                let (su, cu) = turn();
                c.frame.vector(cu, su, -c.half_angle.ptan()).normalized()
            }
            Surface::Sphere(s) => {
                let (su, cu) = turn();
                let (sv, cv) = v.psin_cos();
                Some(s.frame.vector(cv * cu, cv * su, sv))
            }
            Surface::Torus(t) => {
                let (su, cu) = turn();
                let (sv, cv) = v.psin_cos();
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
                wrap_angle(y.patan2(x))
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
                let slope = s.half_angle.ptan();
                let v = ((x.phypot(y) - s.radius) * slope + z) / (slope * slope + 1.0);
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
                let rho = x.phypot(y);
                (
                    angle(y, x),
                    if rho == 0.0 && z == 0.0 {
                        0.0
                    } else {
                        z.patan2(rho)
                    },
                )
            }
            Surface::Torus(s) => {
                let (x, y, z) = s.frame.local(p);
                let off = x.phypot(y) - s.major;
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
    /// This walk does not go on where it ends short, as
    /// [`Surface::project`] does: its callers read a walk that did not
    /// settle as an error and fall back on `project`, or check what they
    /// got against what they need, and they were built on its answers as
    /// they are.
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
    /// **Where the walks end short.** The answer is looked at once more:
    /// where its foot could still come nearer to `p`, by sliding along the
    /// tangent plane, by more than the coordinates resolve there
    /// ([`Surface::slides`]: a spline whose weights make its
    /// parametrisation race, or whose tangents are nearly parallel, where
    /// the damped steps crawl and run out), a second walk goes on from it
    /// ([`Surface::settle`]), and where that does not settle either, a
    /// search over the surface's Bézier pieces
    /// ([`Surface::nearest_by_pieces`]). What they find replaces the answer
    /// only where it is nearer by more than that resolution, or as near
    /// and settled where the answer was not. Where the answer is not short
    /// so, or the surface has a pole or a border squeezed nearly into one
    /// ([`Surface::goes_on`]), nothing of this is done: the answer is
    /// what the walks found, bit for bit.
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
        // Remembered from every projection onto the surface before, with
        // what finding it took, which a hit charges again; nothing is
        // remembered while not counting (`crate::allowance::counting`).
        if let Some(found) = grid.memo.get(&key) {
            return Ok(found);
        }
        let (found, units) =
            crate::allowance::measured(|| self.project_from_grid(p, &grid, ((u0, u1), (v0, v1))));
        let found = found?;
        if crate::allowance::counting() {
            grid.memo.put(key, found, units);
        }
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
        // The surface and its tangents at the best foot (the walk's last).
        let mut best_at = None;
        if let Some(&(_, _, i, j)) = starts.first() {
            let (found, u, v, _, there) =
                self.descend(p, at(i, j), (closed_u, closed_v), collapsed);
            if found <= 16.0 * f64::EPSILON * (p.length() + extent) {
                return Ok((u, v));
            }
            best = Some((found, u, v));
            best_at = Some(there);
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
            let (found, u, v, _, there) = self.descend(
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
                best_at = Some(there);
            }
        }
        // Never further than the nearest sample (a pole, when nothing
        // was started from; a start that could not improve).
        let (d, i, j) = nearest_sample;
        let (Some((found, u, v)), Some(there)) = (best, best_at) else {
            return Ok(at(i, j));
        };
        if !at_most(found, d) {
            return Ok(at(i, j));
        }
        // Where the walks ended short, the foot found by going on (not
        // beside a pole: see `goes_on`).
        let domain = ((u0, u1), (v0, v1));
        if self.slides(p, (u, v), there, (closed_u, closed_v), domain) && self.goes_on() {
            #[cfg(test)]
            FINISHED.with(|n| n.set(n.get() + 1));
            let short = Foot {
                distance: found,
                u,
                v,
                at: there,
                settled: false,
            };
            let foot = self.finish(p, short, (closed_u, closed_v), collapsed);
            return Ok((foot.u, foot.v));
        }
        Ok((u, v))
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

    /// How far the foot at `(u, v)` of `p` (where the surface and its
    /// tangents are `at`) would still slide towards `p` along the tangent
    /// plane, within the directions `domain` leaves free: the length of
    /// what `p − S` has along the tangent plane (along the free tangent
    /// alone where a border holds the other; nothing at a corner that
    /// holds both). Zero at a nearest point, to rounding.
    ///
    /// The tangent plane's share is what is left of `p − S` off the unit
    /// normal from the cross product of the tangents, which keeps its
    /// direction where the tangents are nearly parallel (where the normal
    /// equations, which square the angle between them, lose it); where
    /// the tangents give no normal, the longer of the shares along each.
    fn sliding(
        &self,
        p: Vec3,
        (u, v): (f64, f64),
        (s, su, sv): (Vec3, Vec3, Vec3),
        closed: (bool, bool),
        ((u0, u1), (v0, v1)): ((f64, f64), (f64, f64)),
    ) -> f64 {
        let r = p - s;
        let (a, c) = (su.dot(su), sv.dot(sv));
        let (ru, rv) = (r.dot(su), r.dot(sv));
        let held = |x: f64, (low, high): (f64, f64), pull: f64, closed: bool| {
            !closed && ((x <= low && pull < 0.0) || (x >= high && pull > 0.0))
        };
        let hold_u = held(u, (u0, u1), ru, closed.0) || !above_zero(a);
        let hold_v = held(v, (v0, v1), rv, closed.1) || !above_zero(c);
        let along = |pull: f64, square: f64| (pull * pull / square).sqrt();
        match (hold_u, hold_v) {
            (true, true) => 0.0,
            (true, false) => along(rv, c),
            (false, true) => along(ru, a),
            (false, false) => {
                let single = along(ru, a).max(along(rv, c));
                match su.cross(sv).normalized() {
                    Some(n) => (r - n * r.dot(n)).length().max(single),
                    None => single,
                }
            }
        }
    }

    /// What the coordinates can tell apart at the foot `(u, v)` of `p`
    /// (where the surface and its tangents are `at`): 64 units in the last
    /// place of the largest coordinate of `p` and the foot, and how far
    /// the surface moves for four units in the last place of the
    /// parameters there (of the larger of a parameter and its domain's
    /// ends). Where a spline races (a light weight between heavy ones),
    /// the next parameter that can be written is a visible way off, and
    /// no foot is nearer than that.
    fn resolution(&self, p: Vec3, (u, v): (f64, f64), (s, su, sv): (Vec3, Vec3, Vec3)) -> f64 {
        let size = [p.x, p.y, p.z, s.x, s.y, s.z]
            .into_iter()
            .fold(0.0_f64, |most, c| most.max(c.abs()));
        let ((u0, u1), (v0, v1)) = self.domain();
        let reach = |x: f64, low: f64, high: f64| x.abs().max(low.abs()).max(high.abs());
        64.0 * f64::EPSILON * size
            + 4.0 * f64::EPSILON * (su.length() * reach(u, u0, u1) + sv.length() * reach(v, v0, v1))
    }

    /// Whether the foot at `(u, v)` still slides by more than the
    /// coordinates resolve there: a walk that stopped there ended short.
    /// Not a number slides.
    fn slides(
        &self,
        p: Vec3,
        (u, v): (f64, f64),
        at: (Vec3, Vec3, Vec3),
        closed: (bool, bool),
        domain: ((f64, f64), (f64, f64)),
    ) -> bool {
        !at_most(
            self.sliding(p, (u, v), at, closed, domain),
            self.resolution(p, (u, v), at),
        )
    }

    /// Whether the foot could come nearer to `p`, by sliding along the
    /// tangent plane, by more than the coordinates resolve at it: sliding
    /// by `s` brings a foot `d` from `p` nearer by `d − √(d² − s²)` at most
    /// (`s²/2d` where `s` is small). A foot far from its point whose slide
    /// is the rounding of a wild sheet's own evaluation cannot; nor one
    /// placed a little off along the plane, which a walk sets right
    /// ([`Surface::settle`]) where a search over the whole surface
    /// ([`Surface::nearest_by_pieces`]) has nothing to find. Not a number
    /// can.
    fn could_come_nearer(&self, p: Vec3, foot: &Foot, closed: (bool, bool)) -> bool {
        let slide = self.sliding(p, (foot.u, foot.v), foot.at, closed, self.domain());
        !at_most(
            Self::gain(foot.distance, slide),
            self.resolution(p, (foot.u, foot.v), foot.at),
        )
    }

    /// How much nearer a foot `distance` from its point can come by
    /// sliding `slide` along the tangent plane: `d − √(d² − s²)`, written
    /// `s² / (d + √(d² − s²))` so that a small slide does not cancel.
    fn gain(distance: f64, slide: f64) -> f64 {
        // On the point, or square to the tangent plane: nothing to gain.
        if slide == 0.0 {
            return 0.0;
        }
        let square = (distance * distance - slide * slide).max(0.0);
        slide * slide / (distance + square.sqrt())
    }

    /// Whether a search that ended short on this surface goes on: not on a
    /// surface with a pole or a border squeezed nearly into one
    /// ([`crate::pole::squeezed`]: a border row of control points that
    /// reaches less than a ten-thousandth as far as the row next to it,
    /// where [`crate::pole::poles`] takes a billionth of the net). Beside such a border
    /// the descent finds the points of the rails on other meridians, and
    /// the measure and the tessellator read those feet as they are, or mend
    /// them along the borders where the surface has a pole
    /// (`pole::BorderFeet`); the tessellator cannot yet mesh a face whose
    /// rails' feet are found on the border at its pole (the fan at a pole
    /// is owed). Going on there moved what was built right: at true poles
    /// the meshes and measures of `pole_measure`'s tips and blades (the
    /// trial Gate of 2026-11-15), at poles squeezed open by 2e-8 and 1e-7
    /// of the scale mesh triangle counts and the reasons measures are
    /// refused (the review of 2026-11-15).
    fn goes_on(&self) -> bool {
        !crate::pole::squeezed(self)
    }

    /// Of two feet of `p`, whether `a` is the better: nearer by more than
    /// the coordinates resolve at `b`, or as near as that and settled
    /// where `b` is not (far from the origin a foot that still slides can
    /// read as near as the true one, the distance being flat at a foot).
    fn better(&self, p: Vec3, a: &Foot, b: &Foot) -> bool {
        let margin = self.resolution(p, (b.u, b.v), b.at);
        a.distance < b.distance - margin
            || (a.settled && !b.settled && a.distance <= b.distance + margin)
    }

    /// The foot of `p` where the walks of [`Surface::project_numeric`]
    /// ended `short` (its foot still slides): a second walk from there
    /// ([`Surface::settle`]), and where that does not settle, a search
    /// over the Bézier pieces of the surface
    /// ([`Surface::nearest_by_pieces`]). The better of what they find and
    /// `short` itself ([`Surface::better`]).
    fn finish(&self, p: Vec3, short: Foot, closed: (bool, bool), collapsed: [bool; 4]) -> Foot {
        let mut best = short;
        let walk = self.settle(
            p,
            (short.u, short.v),
            closed,
            collapsed,
            self.domain(),
            PROJECT_STEPS,
        );
        if self.better(p, &walk, &best) {
            best = walk;
        }
        if !best.settled && self.could_come_nearer(p, &best, closed) {
            let found = self.nearest_by_pieces(p, best, closed, collapsed);
            if self.better(p, &found, &best) {
                best = found;
            }
        }
        best
    }

    /// A walk from `start` to a foot of `p` within `domain` that does not
    /// slide: where [`Surface::descend`] ended short.
    ///
    /// Each step is the one of Gauss–Newton, solved in the tangent plane's
    /// own axes (`J = Q R`, `e1` along `∂u`, `e2 = n × e1` with the unit
    /// normal from the cross product), so that where the tangents are
    /// nearly parallel the step across them is kept: the normal equations
    /// of [`Surface::descend`] square the sine between the tangents, and
    /// with it the step across them, away (a sheet sheared a millionfold
    /// leaves it a millionth of a millionth of what it is, and its damping,
    /// however light, leaves it less). Where a step does not help, the
    /// next is shorter along the same direction, which is downhill (the
    /// Gauss–Newton direction is, wherever the tangents span a plane);
    /// where a border holds one parameter, or the tangents give no
    /// normal, the step is the one of [`Surface::descend`]'s equations,
    /// damped. A step helps as it does there: nearer by more than
    /// rounding, or as near with less slope.
    ///
    /// Settled as soon as sliding on could not bring the foot nearer by
    /// more than the coordinates resolve ([`Surface::slides`]): a stopping
    /// rule in the units of the model, where `descend`'s is in those of
    /// the parameters, and one that a slide which is the rounding of the
    /// surface's own evaluation, far from its point, meets at once. Not
    /// settled where no step helps however short, the steps run out or a
    /// number is not one.
    fn settle(
        &self,
        p: Vec3,
        start: (f64, f64),
        closed: (bool, bool),
        collapsed: [bool; 4],
        ((u0, u1), (v0, v1)): ((f64, f64), (f64, f64)),
        steps: usize,
    ) -> Foot {
        let domain = ((u0, u1), (v0, v1));
        let cache = std::cell::RefCell::new(crate::DerivativeCache::default());
        let eval = |u: f64, v: f64| match self {
            Surface::Nurbs(s) => s.derivatives_cached(u, v, &mut cache.borrow_mut()),
            _ => self.derivatives(u, v),
        };
        let place = |x: f64, (low, high): (f64, f64), closed: bool| {
            if closed {
                let wrapped = low + (x - low).rem_euclid(high - low);
                if wrapped.is_finite() { wrapped } else { low }
            } else {
                x.clamp(low, high)
            }
        };
        let held = |x: f64, (low, high): (f64, f64), pull: f64, closed: bool| {
            !closed && ((x <= low && pull < 0.0) || (x >= high && pull > 0.0))
        };
        // The slope is how far the foot slides: along the tangent plane,
        // not along each tangent, which where they are nearly parallel
        // both stand nearly square to what is left to slide.
        let look = |u: f64, v: f64| {
            let at = eval(u, v);
            let (s, su, sv) = at;
            let r = p - s;
            let (ru, rv) = (r.dot(su), r.dot(sv));
            let hold = (
                held(u, (u0, u1), ru, closed.0),
                held(v, (v0, v1), rv, closed.1),
            );
            let slope = self.sliding(p, (u, v), at, closed, domain);
            (r, (ru, rv), hold, slope, at)
        };
        let (mut u, mut v) = start;
        let mut here = look(u, v);
        let mut lambda = 1e-3;
        let mut settled = false;
        let mut stalled = 0_u32;
        for _ in 0..steps {
            let (r, (ru, rv), (hold_u, hold_v), slope, at) = here;
            let (_, su, sv) = at;
            let distance = r.length();
            if distance.is_nan() {
                break;
            }
            if distance <= 0.0
                || (hold_u && hold_v)
                || at_most(slope, self.resolution(p, (u, v), at))
            {
                settled = true;
                break;
            }
            if lambda > 1e12 {
                break;
            }
            let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
            let (hold_u, hold_v) = (hold_u || !above_zero(a), hold_v || !above_zero(c));
            // Gauss–Newton in the tangent plane's axes, shortened by the
            // damping; `None` where it does not apply.
            let across = (|| {
                if hold_u || hold_v {
                    return None;
                }
                let n = su.cross(sv).normalized()?;
                let e1 = su.normalized()?;
                let e2 = n.cross(e1);
                let (r11, r12, r22) = (su.length(), sv.dot(e1), sv.dot(e2));
                let dv = r.dot(e2) / r22;
                let du = (r.dot(e1) - r12 * dv) / r11;
                let shorter = 1.0 + lambda;
                (du.is_finite() && dv.is_finite()).then_some((du / shorter, dv / shorter))
            })();
            let (a, c) = (a * (1.0 + lambda), c * (1.0 + lambda));
            let (mut du, mut dv) = match across {
                Some(step) => step,
                None if hold_u && hold_v => {
                    settled = true;
                    break;
                }
                None if hold_u => (0.0, rv / c),
                None if hold_v => (ru / a, 0.0),
                None => {
                    let det = a * c - b * b;
                    ((c * ru - b * rv) / det, (a * rv - b * ru) / det)
                }
            };
            // A step that would leave the domain stops at the border (half
            // way to one that is a pole), as in `descend`.
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
                break;
            }
            let next = (
                place(u + du, (u0, u1), closed.0),
                place(v + dv, (v0, v1), closed.1),
            );
            let there = look(next.0, next.1);
            let reached = there.0.length();
            let rounding = 16.0 * f64::EPSILON * (p.length() + distance);
            let level = reached <= distance + rounding;
            if reached < distance - rounding || (level && there.3 < slope) {
                // A step of Gauss–Newton takes the slide at least to half
                // where the plane is a model of the surface; two in a row
                // that do not are walking in the rounding of the surface's
                // own evaluation (a slide that is noise, far from its
                // point), which no step can lessen: the walk ends there.
                stalled = if there.3 > 0.5 * slope {
                    stalled + 1
                } else {
                    0
                };
                (u, v, here) = (next.0, next.1, there);
                lambda = (lambda / 8.0).max(1e-14);
                if stalled >= 2 {
                    break;
                }
            } else {
                lambda *= 8.0;
            }
        }
        Foot {
            distance: here.0.length(),
            u,
            v,
            at: here.4,
            settled,
        }
    }

    /// The foot of `p` by a search over the Bézier pieces of a freeform
    /// surface, for where the walks from the grid ended short: best first
    /// by how near a piece can come (the distance to a box round its
    /// control points in the axes of its own corners, which the piece lies
    /// in for positive weights), splitting a piece until it is even enough
    /// to walk on (its weights within a factor of four, its net within a
    /// quarter of its size of the bilinear map of its corners), then
    /// walking inside it ([`Surface::settle`], held to the piece). A piece
    /// whose walk does not settle is split further. A piece of a spline
    /// is split where its weights balance (for end weights `w₀`, `wₙ` along
    /// a direction, the parameter `β / (1 + β)` with `β = (w₀ / wₙ)^(1/n)`,
    /// the middle of the Möbius reparametrisation that evens them, *The
    /// NURBS Book* §6.1): where a weight is light, the middle of the
    /// parameter is far from the middle of the piece in space, and
    /// halving the parameter left pieces as large in space as before.
    ///
    /// Ends at the first walk that settles inside its piece with a foot
    /// better than `best` ([`Surface::better`]), when no piece left can
    /// come nearer than the best foot by more than the coordinates
    /// resolve, or when [`PIECE_LOOKS`] pieces or [`PIECE_WALKS`] walks are
    /// spent. Gives the best foot found, `best` where none is better.
    fn nearest_by_pieces(
        &self,
        p: Vec3,
        mut best: Foot,
        closed: (bool, bool),
        collapsed: [bool; 4],
    ) -> Foot {
        // The spans as the surface keeps them (`bezier_spans_shared`),
        // looked at in place: only the pieces split from them are made.
        let shared;
        let whole;
        let pieces: Vec<Span<'_>> = match self {
            Surface::Nurbs(s) => match s.bezier_spans_shared() {
                Ok(spans) => {
                    shared = spans;
                    shared
                        .iter()
                        .map(|span| (&span.patch, span.u, span.v))
                        .collect()
                }
                Err(_) => return best,
            },
            Surface::Bezier(b) => {
                whole = RationalBezierPatch::from_bezier(b);
                vec![(&whole, (0.0, 1.0), (0.0, 1.0))]
            }
            _ => return best,
        };
        let full = self.domain();
        let ((du0, du1), (dv0, dv1)) = full;
        let mut heap = std::collections::BinaryHeap::new();
        let mut made = 0_usize;
        for (patch, u, v) in pieces {
            // A pass over its control net (`crate::allowance`).
            crate::allowance::charge(1);
            let (gap, size) = piece_bound(patch, p);
            heap.push(Piece {
                gap,
                made,
                patch: std::borrow::Cow::Borrowed(patch),
                u,
                v,
                depth: 0,
                size,
            });
            made += 1;
        }
        // The nearest corner of a piece seen (corners are points of the
        // surface, at parameters known): what no piece left may beat.
        let mut corner: Option<(f64, f64, f64)> = None;
        let (mut looks, mut walks) = (0_usize, 0_usize);
        while let Some(piece) = heap.pop() {
            let margin = self.resolution(p, (best.u, best.v), best.at);
            let least = corner.map_or(best.distance, |(d, _, _)| d.min(best.distance));
            if !below(piece.gap, least - margin) || looks >= PIECE_LOOKS {
                break;
            }
            looks += 1;
            // A look is counted (`crate::allowance`), and a bound in force
            // that is reached ends the search with what it has.
            if crate::allowance::pass() {
                break;
            }
            let (m, n) = piece.patch.degrees();
            for (i, j, u, v) in [
                (0, 0, piece.u.0, piece.v.0),
                (m, 0, piece.u.1, piece.v.0),
                (0, n, piece.u.0, piece.v.1),
                (m, n, piece.u.1, piece.v.1),
            ] {
                if let Some(q) = piece.patch.control_point(i * (n + 1) + j) {
                    let d = q.distance(p);
                    if corner.is_none_or(|(least, _, _)| d < least) {
                        corner = Some((d, u, v));
                    }
                }
            }
            if walks < PIECE_WALKS && piece_is_even(&piece.patch, piece.size) {
                walks += 1;
                let start = (0.5 * (piece.u.0 + piece.u.1), 0.5 * (piece.v.0 + piece.v.1));
                // The domain's poles where the piece reaches them; no
                // seam inside a piece.
                let [c0, c1, c2, c3] = collapsed;
                let poles = [
                    c0 && piece.u.0 <= du0,
                    c1 && piece.u.1 >= du1,
                    c2 && piece.v.0 <= dv0,
                    c3 && piece.v.1 >= dv1,
                ];
                let mut walk = self.settle(
                    p,
                    start,
                    (false, false),
                    poles,
                    (piece.u, piece.v),
                    PIECE_STEPS,
                );
                // Held at a side of the piece inside the domain is not
                // settled on the surface.
                walk.settled =
                    walk.settled && !self.slides(p, (walk.u, walk.v), walk.at, closed, full);
                if self.better(p, &walk, &best) {
                    best = walk;
                    if walk.settled {
                        break;
                    }
                }
            }
            if piece.depth >= PIECE_DEPTH {
                continue;
            }
            let (su, sv) = (
                piece_balance(&piece.patch, true),
                piece_balance(&piece.patch, false),
            );
            let um = piece.u.0 + su * (piece.u.1 - piece.u.0);
            let vm = piece.v.0 + sv * (piece.v.1 - piece.v.0);
            let (left, right) = piece.patch.split_u(su);
            for (half, u) in [(left, (piece.u.0, um)), (right, (um, piece.u.1))] {
                let (low, high) = half.split_v(sv);
                for (quarter, v) in [(low, (piece.v.0, vm)), (high, (vm, piece.v.1))] {
                    crate::allowance::charge(1);
                    let (gap, size) = piece_bound(&quarter, p);
                    heap.push(Piece {
                        gap,
                        made,
                        patch: std::borrow::Cow::Owned(quarter),
                        u,
                        v,
                        depth: piece.depth + 1,
                        size,
                    });
                    made += 1;
                }
            }
        }
        // A corner nearer than every walk's foot.
        if let Some((d, u, v)) = corner {
            let at = self.derivatives(u, v);
            let foot = Foot {
                distance: d,
                u,
                v,
                at,
                settled: false,
            };
            if self.better(p, &foot, &best) {
                best = foot;
            }
        }
        best
    }
}

/// A Bézier piece of a freeform surface, with its parameters there.
type Span<'a> = (&'a RationalBezierPatch, (f64, f64), (f64, f64));

/// `x <= bound`: false where either is not a number.
fn at_most(x: f64, bound: f64) -> bool {
    x <= bound
}

/// `x < bound`: false where either is not a number.
fn below(x: f64, bound: f64) -> bool {
    x < bound
}

/// `x > 0`: false for a number that is not one.
fn above_zero(x: f64) -> bool {
    x > 0.0
}

/// A foot of a point on a freeform surface: its distance, its parameters,
/// the surface and its tangents there, and whether it no longer slides
/// ([`Surface::slides`]).
#[derive(Clone, Copy, Debug)]
struct Foot {
    distance: f64,
    u: f64,
    v: f64,
    at: (Vec3, Vec3, Vec3),
    settled: bool,
}

/// A piece of [`Surface::nearest_by_pieces`]: how near it can come to the
/// point, the order it was made in (the tie-break), the patch, its
/// parameters on the surface, how often it was split and its size.
struct Piece<'a> {
    gap: f64,
    made: usize,
    /// A span of the surface as it keeps them, or a piece split from one.
    patch: std::borrow::Cow<'a, RationalBezierPatch>,
    u: (f64, f64),
    v: (f64, f64),
    depth: usize,
    size: f64,
}

impl PartialEq for Piece<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for Piece<'_> {}

impl PartialOrd for Piece<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Piece<'_> {
    /// The nearest first (a max-heap pops the greatest), then the first
    /// made.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .gap
            .total_cmp(&self.gap)
            .then(other.made.cmp(&self.made))
    }
}

/// A distance from `p` no larger than that of any point of `patch`, and
/// the size of the box it is read from: the box round the control points
/// along the axes of the patch's corners (`e1` along the sides of `u`,
/// `e2` the sides of `v` square to it, `e3` across), which holds the patch
/// where its weights are positive (a convex hull), and lies close round a
/// piece that is nearly flat whichever way it is turned. The world's axes
/// where the corners give none.
fn piece_bound(patch: &RationalBezierPatch, p: Vec3) -> (f64, f64) {
    let (m, n) = patch.degrees();
    let at = |i: usize, j: usize| patch.control_point(i * (n + 1) + j);
    let (Some(c00), Some(c10), Some(c01), Some(c11)) = (at(0, 0), at(m, 0), at(0, n), at(m, n))
    else {
        return (0.0, f64::INFINITY);
    };
    let axes = (|| {
        let e1 = ((c10 - c00) + (c11 - c01)).normalized()?;
        let side = (c01 - c00) + (c11 - c10);
        let e2 = (side - e1 * side.dot(e1)).normalized()?;
        Some([e1, e2, e1.cross(e2)])
    })()
    .unwrap_or([
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ]);
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for q in patch.points() {
        let d = q - c00;
        for ((axis, lo), hi) in axes.iter().zip(&mut low).zip(&mut high) {
            let x = d.dot(*axis);
            *lo = lo.min(x);
            *hi = hi.max(x);
        }
    }
    let d = p - c00;
    let (mut gap, mut size) = (0.0, 0.0);
    for ((axis, lo), hi) in axes.iter().zip(low).zip(high) {
        let x = d.dot(*axis);
        let outside = (lo - x).max(x - hi).max(0.0);
        gap += outside * outside;
        size += (hi - lo) * (hi - lo);
    }
    // Not a number reads as near, so the piece is looked at.
    let gap = gap.sqrt();
    (if gap.is_nan() { 0.0 } else { gap }, size.sqrt())
}

/// Whether a piece is even enough to walk on: its weights within a factor
/// of four of each other, and its control points within a quarter of
/// `size` of the bilinear map of its corners.
fn piece_is_even(patch: &RationalBezierPatch, size: f64) -> bool {
    let Some((light, heavy)) = patch.weight_range() else {
        return false;
    };
    if !at_most(heavy, 4.0 * light) {
        return false;
    }
    let (m, n) = patch.degrees();
    let at = |i: usize, j: usize| patch.control_point(i * (n + 1) + j);
    let (Some(c00), Some(c10), Some(c01), Some(c11)) = (at(0, 0), at(m, 0), at(0, n), at(m, n))
    else {
        return false;
    };
    for i in 0..=m {
        for j in 0..=n {
            let (s, t) = (i as f64 / m.max(1) as f64, j as f64 / n.max(1) as f64);
            let bilinear = c00 * ((1.0 - s) * (1.0 - t))
                + c10 * (s * (1.0 - t))
                + c01 * ((1.0 - s) * t)
                + c11 * (s * t);
            if !at(i, j).is_some_and(|q| q.distance(bilinear) <= 0.25 * size) {
                return false;
            }
        }
    }
    true
}

/// Where to split a piece along u (or v): where its weights balance, the
/// parameter `β / (1 + β)` with `β = (w₀ / wₙ)^(1/n)` from the geometric
/// mean over its rows of the ratio of the weights at the two ends (see
/// [`Surface::nearest_by_pieces`]); the middle for even weights, and never
/// nearer an end than a sixteenth.
fn piece_balance(patch: &RationalBezierPatch, along_u: bool) -> f64 {
    let weights = patch.weights();
    let (m, n) = patch.degrees();
    let (rows, degree) = if along_u { (n + 1, m) } else { (m + 1, n) };
    let mut log_ratio = 0.0;
    for k in 0..rows {
        let (first, last) = if along_u {
            (weights.get(k), weights.get(m * (n + 1) + k))
        } else {
            (weights.get(k * (n + 1)), weights.get(k * (n + 1) + n))
        };
        let (Some(&a), Some(&b)) = (first, last) else {
            return 0.5;
        };
        log_ratio += (a / b).pln();
    }
    let beta = (log_ratio / (rows as f64 * degree.max(1) as f64)).pexp();
    let t = beta / (1.0 + beta);
    if t.is_finite() {
        t.clamp(1.0 / 16.0, 15.0 / 16.0)
    } else {
        0.5
    }
}

#[cfg(test)]
thread_local! {
    /// How many searches of this thread ended short and went on
    /// ([`Surface::finish`] and the second walk of `project_from`): the
    /// tests that hold the old path to its bits count it.
    pub(crate) static FINISHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
mod project_sweep;

#[cfg(test)]
mod project_short;

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

    /// The point of a Bézier patch alone (`Surface::eval`) is the point of
    /// its derivatives, bit for bit, inside its square and beyond it, at
    /// several degrees and scales.
    #[test]
    fn the_point_of_a_patch_is_the_point_of_its_derivatives() {
        let raised = |m: usize, n: usize, scale: f64| {
            let ctrl: Vec<Vec3> = (0..(m + 1) * (n + 1))
                .map(|k| {
                    let (i, j) = ((k / (n + 1)) as f64, (k % (n + 1)) as f64);
                    Vec3::new(i, 1.3 * j, ((k * 7) % 5) as f64 * 0.3) * scale
                })
                .collect();
            Surface::Bezier(BezierPatch::new(m, n, ctrl).unwrap())
        };
        for scale in [1e-3, 1.0, 1e6] {
            for surface in [
                Surface::Bezier(bumpy(scale)),
                raised(1, 1, scale),
                raised(2, 5, scale),
                raised(7, 3, scale),
            ] {
                for i in 0..=24 {
                    for j in 0..=24 {
                        let (u, v) = (
                            -0.2 + 1.4 * f64::from(i) / 24.0,
                            -0.2 + 1.4 * f64::from(j) / 24.0,
                        );
                        let (a, b) = (surface.eval(u, v), surface.derivatives(u, v).0);
                        assert_eq!(
                            [a.x.to_bits(), a.y.to_bits(), a.z.to_bits()],
                            [b.x.to_bits(), b.y.to_bits(), b.z.to_bits()],
                            "{u} {v}"
                        );
                    }
                }
            }
        }
    }

    /// Every projection remembered is the one made afresh (from the grid,
    /// past the memo), bit for bit: surfaces project points, then project
    /// them all again and compare each with a fresh one, and the second
    /// round finds them remembered.
    #[test]
    fn remembered_projections_are_the_ones_made_afresh() {
        let make = |k: usize| {
            let scale = 1.0 + 0.01 * k as f64;
            Surface::Nurbs(NurbsSurface::from_bezier(&bumpy(scale)).unwrap())
        };
        let point = |k: usize| {
            let t = k as f64;
            Vec3::new(
                (t * 0.137).psin() * 2.0 + 1.5,
                (t * 0.261).pcos() * 3.0 + 2.0,
                (t * 0.093).psin() * 1.5,
            )
        };
        let surfaces: Vec<Surface> = (0..8).map(make).collect();
        let points = 60;
        let hits_before = HITS.with(std::cell::Cell::get);
        for round in 0..2 {
            for surface in &surfaces {
                let ((u0, u1), (v0, v1)) = surface.domain();
                let grid = surface.project_grid((u0, u1), (v0, v1));
                for k in 0..points {
                    let got = surface.project(point(k)).unwrap();
                    if round == 1 {
                        let fresh = surface
                            .project_from_grid(point(k), &grid, ((u0, u1), (v0, v1)))
                            .unwrap();
                        assert_eq!(
                            (got.0.to_bits(), got.1.to_bits()),
                            (fresh.0.to_bits(), fresh.1.to_bits())
                        );
                    }
                }
            }
        }
        // The second round found what the first left (all of it but what
        // the tests beside this one may have pushed out).
        let hits = HITS.with(std::cell::Cell::get) - hits_before;
        assert!(hits >= surfaces.len() * points / 2, "{hits}");
        assert!(kept_projections().0 > 0);
    }

    /// One part of the projections remembered, alone: it holds at most its
    /// room, lets its oldest go first whichever surface it is of, finds
    /// every projection it holds and none it let go of, and a surface new
    /// to it when it is full still has what it puts.
    #[test]
    fn a_full_part_lets_the_oldest_go_and_finds_the_rest() {
        let room = 64;
        let mut shard = Shard::new(room);
        let key = |id: u64, k: u64| (id, [k, k.wrapping_mul(7), k ^ 0x55]);
        let value = |id: u64, k: u64| ((id as f64, k as f64 * 0.5), k);
        let mut put: Vec<(u64, u64)> = Vec::new();
        let mut seed = 7_u64;
        for step in 0..2000_u64 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let (id, k) = ((seed >> 60) % 5, (seed >> 33) % 300);
            let before = shard.get(&key(id, k));
            shard.put(key(id, k), value(id, k));
            if before.is_none() {
                put.push((id, k));
            }
            assert!(shard.ring.len() <= room);
            // The last `room` new ones are held, every older one is not.
            let held = put.len().min(room);
            let (old, new) = put.split_at(put.len() - held);
            for &(id, k) in new {
                assert_eq!(shard.get(&key(id, k)), Some(value(id, k)), "{step}");
            }
            for &(id, k) in old.iter().rev().take(room) {
                if !new.contains(&(id, k)) {
                    assert_eq!(shard.get(&key(id, k)), None, "{step}");
                }
            }
        }
        // A surface it has never seen, with the part full.
        shard.put(key(99, 1), value(99, 1));
        assert_eq!(shard.get(&key(99, 1)), Some(value(99, 1)));
        assert_eq!(shard.ring.len(), room);
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
                        (t * 0.37).psin() * 2.0 + 1.5,
                        (t * 0.61).pcos() * 3.0,
                        (t * 0.23).psin(),
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
                        (t * 0.37).psin() * 2.0 + 1.5,
                        (t * 0.61).pcos() * 3.0 + 2.0,
                        (t * 0.23).psin() * 1.5,
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
                    let rho = x.phypot(y);
                    let residual = match &s {
                        Surface::Plane(_) => z,
                        Surface::Cylinder(c) => rho - c.radius(),
                        Surface::Cone(c) => rho - (c.radius() + z * c.half_angle().ptan()),
                        Surface::Sphere(sp) => (rho * rho + z * z).sqrt() - sp.radius(),
                        Surface::Torus(t) => {
                            ((rho - t.radii().0).ppowi(2) + z * z).sqrt() - t.radii().1
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
                                let k = t.radii().0 / x.phypot(y);
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

        let cone = Cone::new(f, 1.0, 0.5_f64.patan()).unwrap();
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
