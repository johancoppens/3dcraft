//! A helix, and the solid a closed profile sweeps as it is carried along
//! one: a coil spring, the groove of a thread.
//!
//! **The helix** winds about an axis at a constant radius, advancing one
//! pitch along the axis per turn: right-handed (counter-clockwise seen
//! from the tip of the axis as it advances) or left-handed. It is not a
//! rational curve (a rational curve is algebraic and meets a plane in
//! finitely many points; the whole helix meets a plane through its axis
//! infinitely often, and a piece of it cannot lie on an algebraic curve
//! without the whole helix doing so), so no NURBS is exact. Where a
//! curve is needed ([`Helix::curve`]) it is a polynomial B-spline that
//! follows the helix in parts of at most a quarter turn, each a
//! polynomial through the helix at the ends of the part and at the
//! Chebyshev points between them. The way along the axis is linear in
//! the angle, and a polynomial reproduces it exactly; the circle is what
//! is approximated, so the distance to the helix is the radius times the
//! distance of the same polynomial to the unit circle, which is measured
//! once per plan ([`circle_error`]).
//!
//! **How the profile is carried.** The profile moves by the screw motion
//! of the helix: it turns about the axis and advances along it, one pitch
//! per turn, as a rigid body. This is the frame of the helix that keeps
//! one axis pointing at the helix's axis (for a helix the Frenet frame
//! does exactly that, and is carried by the same motion), not a
//! rotation-minimising frame: a rotation-minimising frame turns against
//! the Frenet frame by the helix's torsion times its length, so a profile
//! drawn in a plane through the axis (the tooth of a thread) would not
//! stay in such a plane, and the swept thread would not be the same
//! after one turn. Under the screw motion every turn is the first one
//! moved along the axis by the pitch. The profile can lie in any plane
//! through the start of the helix that the motion crosses: a plane
//! through the axis (a thread), or one square to the helix (a spring's
//! wire).
//!
//! **Faces.** Every curve of the profile sweeps one face per half turn
//! (a whole circle of the profile is four quarters first), numbered as
//! pieces in the order they are made; the ends are the profile at the
//! start and where the motion leaves it, both flat. A line of the profile
//! parallel to the axis sweeps a cylinder, exactly (not along a run-out,
//! where it moves away from the axis). Everything else is a
//! B-spline surface through the profile placed at stations along each
//! half turn, as the sweep along a spline does ([`crate::sweep`]): of
//! degree 5, 7 or 9 along the helix, in two or more parts per half turn,
//! raised until every surface lies within half the linear tolerance of
//! the true sweep. The deviation is measured, at three places between
//! each two stations and nine across the profile, against the point the
//! screw motion puts there; it is [`Swept::deviation`].
//!
//! **Run-out.** A [`RunOut`] carries the profile on past the end of the
//! helix while it moves away from the axis (or towards it), by a rise
//! that grows with the square of the angle, so that it leaves the helix
//! tangentially: the groove of a thread lifting out of the material, out
//! of a shank's surface or into a tapped hole.
//!
//! **What is refused, before anything is built:** a profile that reaches
//! the axis or round it, a profile whose plane the motion runs along
//! somewhere (the sweep would fold), a pitch shorter than the profile is
//! long along the axis as the motion sees it (one turn would run into
//! the next), a helix that does not start on the profile's plane. A
//! sweep that passes through itself in another way is for the caller's
//! self-intersection check, as for [`crate::sweep`].

use crate::extrude::{Origin, Role, cap_sources, each_curve_once};
use crate::loft::{Band, skin};
use crate::profile::Profile;
use crate::sweep::{
    Builder, Element, Form, Map2, Motion, P2, Station, Swept, against_normal, coedge, curve_at,
    edge_ends, part_band, quarters, refuse, stations_of,
};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::{Cylinder, NurbsCurve, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, EdgeId, FaceId};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

/// Most turns a helix has.
pub const MAX_TURNS: f64 = 10_000.0;

/// How a span of the helix is followed, tried in this order: in how many
/// parts per turn, and of what degree each part is.
const PLANS: [(usize, usize); 8] = [
    (4, 5),
    (4, 7),
    (4, 9),
    (8, 7),
    (8, 9),
    (16, 9),
    (32, 9),
    (64, 9),
];

/// Most faces one sweep along a helix makes.
const MAX_FACES: usize = crate::sweep::MAX_SWEEP_FACES;

/// A helix about an axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Helix {
    /// The point of the axis level with the start.
    pub origin: Vec3,
    /// The unit direction of the axis, the way the helix advances.
    pub axis: Vec3,
    /// The unit direction from `origin` to the start, square to `axis`.
    pub start: Vec3,
    pub radius: f64,
    /// How far the helix advances along the axis per turn.
    pub pitch: f64,
    pub turns: f64,
    /// Clockwise seen from the tip of the axis, instead of
    /// counter-clockwise.
    pub left_handed: bool,
}

/// A helix as a curve: a polynomial B-spline, and how far it was found to
/// lie from the helix at most.
#[derive(Clone, Debug, PartialEq)]
pub struct HelixCurve {
    pub curve: NurbsCurve,
    pub deviation: f64,
    /// Parts per turn and degree of each part.
    pub parts_per_turn: usize,
    pub degree: usize,
}

/// Carrying the profile on past the end of the helix while it moves away
/// from the axis (or towards it, by a negative rise: the groove of an
/// internal thread running out into the hole).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunOut {
    /// How far round the run-out goes, in radians.
    pub angle: f64,
    /// How far the profile has moved away from the axis at its end;
    /// negative towards it.
    pub rise: f64,
}

fn invalid(name: &'static str, value: f64) -> KernelError {
    KernelError::InvalidSize { name, value }
}

impl Helix {
    /// A helix with `axis` and `start` made unit and square to each other.
    pub fn new(
        origin: Vec3,
        axis: Vec3,
        start: Vec3,
        radius: f64,
        pitch: f64,
        turns: f64,
        left_handed: bool,
    ) -> Result<Helix, KernelError> {
        if !origin.is_finite() {
            return Err(invalid("helix origin", f64::NAN));
        }
        let axis = axis.normalized().ok_or(invalid("helix axis", 0.0))?;
        let start = (start - axis * start.dot(axis))
            .normalized()
            .ok_or(invalid("helix start direction", 0.0))?;
        if !(radius.is_finite() && radius > 0.0) {
            return Err(invalid("helix radius", radius));
        }
        if !(pitch.is_finite() && pitch > 0.0) {
            return Err(invalid("helix pitch", pitch));
        }
        if !(turns.is_finite() && turns > 0.0 && turns <= MAX_TURNS) {
            return Err(invalid("helix turns", turns));
        }
        Ok(Helix {
            origin,
            axis,
            start,
            radius,
            pitch,
            turns,
            left_handed,
        })
    }

    /// +1 for a right-handed helix, -1 for a left-handed one.
    pub fn sense(&self) -> f64 {
        if self.left_handed { -1.0 } else { 1.0 }
    }

    /// How far the helix advances along its axis per radian.
    pub fn advance(&self) -> f64 {
        self.pitch / TAU
    }

    /// The angle the helix goes round, in radians.
    pub fn angle(&self) -> f64 {
        TAU * self.turns
    }

    pub fn height(&self) -> f64 {
        self.pitch * self.turns
    }

    /// The length of the helix: `turns · √((2πr)² + p²)`.
    pub fn length(&self) -> f64 {
        self.turns * (TAU * self.radius).hypot(self.pitch)
    }

    /// `v` turned about the axis the way the helix goes, through `angle`.
    pub fn turned(&self, v: Vec3, angle: f64) -> Vec3 {
        let (s, c) = (self.sense() * angle).sin_cos();
        let a = self.axis;
        v * c + a.cross(v) * s + a * (a.dot(v) * (1.0 - c))
    }

    /// `p` carried by the screw motion of the helix through `angle`.
    pub fn moved(&self, p: Vec3, angle: f64) -> Vec3 {
        self.origin + self.turned(p - self.origin, angle) + self.axis * (self.advance() * angle)
    }

    /// The point of the helix at `angle` from its start.
    pub fn point(&self, angle: f64) -> Vec3 {
        self.moved(self.origin + self.start * self.radius, angle)
    }

    /// The velocity of the point `p` under the screw motion, per radian.
    pub fn velocity(&self, p: Vec3) -> Vec3 {
        self.axis.cross(p - self.origin) * self.sense() + self.axis * self.advance()
    }

    /// The point `p` in the helix's own terms: its distance from the axis,
    /// its angle about the axis from the start (the way the helix goes,
    /// in `(-π, π]`) and its height along the axis.
    pub fn cylindrical(&self, p: Vec3) -> (f64, f64, f64) {
        let d = p - self.origin;
        let height = d.dot(self.axis);
        let across = d - self.axis * height;
        let x = across.dot(self.start);
        let y = across.dot(self.axis.cross(self.start)) * self.sense();
        (across.length(), y.atan2(x), height)
    }

    /// The helix as a polynomial B-spline within `tol` of it: parts of at
    /// most a quarter turn, raised until the radius times the error of
    /// the plan on the unit circle is within a quarter of `tol`; the
    /// deviation is then measured along the whole curve.
    pub fn curve(&self, tol: f64) -> Result<HelixCurve, KernelError> {
        if !(tol.is_finite() && tol > 0.0) {
            return Err(invalid("tolerance", tol));
        }
        let (per_turn, degree) = plan_for(self.radius, tol / 4.0)?;
        let parts = parts_for(self.turns, per_turn)?;
        let total = self.angle();
        let within = stations_of(degree);
        let mut angles = Vec::with_capacity(parts * degree + 1);
        for part in 0..parts {
            let from = total * part as f64 / parts as f64;
            let to = total * (part + 1) as f64 / parts as f64;
            for (step, share) in within.iter().enumerate() {
                if step == 0 && part > 0 {
                    continue;
                }
                angles.push(if step == degree {
                    to
                } else {
                    from + (to - from) * share
                });
            }
        }
        let band = part_band(parts, degree, &within)?;
        let through: Vec<Vec3> = angles.iter().map(|a| self.point(*a)).collect();
        let mut control = band.points(&through);
        if let (Some(first), Some(p)) = (control.first_mut(), through.first()) {
            *first = *p;
        }
        if let (Some(last), Some(p)) = (control.last_mut(), through.last()) {
            *last = *p;
        }
        let curve = NurbsCurve::polynomial(degree, band.knots.clone(), control)?;
        // Measured at five places in every part against the point of the
        // helix at the same share of the part's angle.
        let mut deviation: f64 = 0.0;
        for part in 0..parts {
            for share in [0.1, 0.3, 0.5, 0.7, 0.9] {
                let v = part as f64 + share;
                let angle = total * v / parts as f64;
                deviation = deviation.max(curve.eval(v).distance(self.point(angle)));
            }
        }
        if !deviation.is_finite() {
            return Err(invalid("helix deviation", deviation));
        }
        Ok(HelixCurve {
            curve,
            deviation,
            parts_per_turn: per_turn,
            degree,
        })
    }
}

/// In how many parts `turns` turns are followed at `per_turn` parts per
/// turn: at least one.
fn parts_for(turns: f64, per_turn: usize) -> Result<usize, KernelError> {
    let parts = (turns * per_turn as f64 - 1e-9).ceil().max(1.0);
    if !(parts.is_finite() && parts <= MAX_FACES as f64) {
        return Err(KernelError::TooManyCurves {
            count: MAX_FACES.saturating_add(1),
            limit: MAX_FACES,
        });
    }
    Ok(parts as usize)
}

/// How far the polynomial of `degree` through the unit circle at the
/// ends and the Chebyshev points of an arc of a `per_turn`-th of a turn
/// lies from that arc, at most (sampled at 256 places). The same for
/// every arc of that size, so it is worked out once per plan.
pub fn circle_error(per_turn: usize, degree: usize) -> Result<f64, KernelError> {
    let span = TAU / per_turn as f64;
    let within = stations_of(degree);
    let band = part_band(1, degree, &within)?;
    let at = |a: f64| Vec3::new(a.cos(), a.sin(), 0.0);
    let through: Vec<Vec3> = within.iter().map(|s| at(span * s)).collect();
    let curve = NurbsCurve::polynomial(degree, band.knots.clone(), band.points(&through))?;
    let mut worst: f64 = 0.0;
    for i in 0..=256 {
        let v = f64::from(i) / 256.0;
        worst = worst.max(curve.eval(v).distance(at(span * v)));
    }
    Ok(worst)
}

/// The cheapest plan that keeps a point `reach` from the axis within
/// `allowed` of the helix it follows.
fn plan_for(reach: f64, allowed: f64) -> Result<(usize, usize), KernelError> {
    let mut last = f64::INFINITY;
    for (per_turn, degree) in PLANS {
        last = reach * circle_error(per_turn, degree)?;
        if last <= allowed {
            return Ok((per_turn, degree));
        }
    }
    Err(refuse(
        &[],
        format!(
            "a helix of radius {reach} cannot be followed within {allowed}: the finest plan is still {last} off; give the document a coarser tolerance"
        ),
    ))
}

/// Where the profile is at an angle of the motion.
struct Screw<'a> {
    helix: &'a Helix,
    home: Station,
    /// The direction away from the axis the run-out moves the profile
    /// along, at the start.
    outward: Vec3,
    /// Where the helix ends (radians), and the run-out after it.
    end: f64,
    run_out: Option<RunOut>,
}

impl Screw<'_> {
    fn rise(&self, angle: f64) -> f64 {
        match self.run_out {
            Some(run) if angle > self.end && run.angle > 0.0 => {
                let share = ((angle - self.end) / run.angle).clamp(0.0, 1.0);
                run.rise * share * share
            }
            _ => 0.0,
        }
    }

    fn station(&self, angle: f64) -> Station {
        let h = self.helix;
        Station {
            origin: h.moved(self.home.origin, angle)
                + h.turned(self.outward, angle) * self.rise(angle),
            u: h.turned(self.home.u, angle),
            b: h.turned(self.home.b, angle),
            square: true,
        }
    }
}

/// A run of the motion that becomes one face per curve of the profile.
struct Leg {
    from: f64,
    to: f64,
}

/// The stations of one leg and the band through them.
struct Followed {
    stations: Vec<Station>,
    band: Band,
    deviation: f64,
}

/// The stations of `leg` from `first` (the station at its start, to the
/// bit) at `per_turn` parts per turn of `degree`, and how far the faces
/// through them lie from the true sweep.
fn follow(
    screw: &Screw,
    leg: &Leg,
    first: &Station,
    loops: &[Vec<Element>],
    (per_turn, degree): (usize, usize),
) -> Result<Followed, KernelError> {
    let span = leg.to - leg.from;
    let parts = parts_for(span / TAU, per_turn)?;
    let within = stations_of(degree);
    let at = |v: f64| leg.from + span * v / parts as f64;
    let mut stations = Vec::with_capacity(parts * degree + 1);
    for part in 0..parts {
        for (step, share) in within.iter().enumerate() {
            if step == 0 && part > 0 {
                continue;
            }
            stations.push(if part == 0 && step == 0 {
                *first
            } else if part + 1 == parts && step == degree {
                screw.station(leg.to)
            } else {
                screw.station(at(part as f64 + share))
            });
        }
    }
    let band = part_band(parts, degree, &within)?;
    let mut worst: f64 = 0.0;
    for element in loops.iter().flatten() {
        let column = stations
            .iter()
            .map(|s| curve_at(element, s, edge_ends(element, s)))
            .collect::<Result<Vec<_>, _>>()?;
        let surface = skin(&column, &band)?;
        let own = column.first().ok_or_else(internal)?;
        let ((u0, u1), _) = surface.domain();
        const ACROSS: u32 = 8;
        for part in 0..parts {
            for step in 0..degree {
                let (Some(&s0), Some(&s1)) = (within.get(step), within.get(step + 1)) else {
                    return Err(internal());
                };
                for share in [0.25, 0.5, 0.75] {
                    let v = part as f64 + s0 + (s1 - s0) * share;
                    let exact = screw.station(at(v));
                    for i in 0..=ACROSS {
                        let u = u0 + (u1 - u0) * f64::from(i) / f64::from(ACROSS);
                        let made = surface.eval(u, v);
                        let meant = exact.at(first.local(own.eval(u)));
                        worst = worst.max(made.distance(meant));
                    }
                }
            }
        }
    }
    if !worst.is_finite() {
        return Err(internal());
    }
    Ok(Followed {
        stations,
        band,
        deviation: worst,
    })
}

fn internal() -> KernelError {
    refuse(
        &[],
        "the sweep along a helix lost track of a curve; this is a bug",
    )
}

fn point_text(p: Vec3) -> String {
    format!("({}, {}, {})", p.x, p.y, p.z)
}

/// Points of the element at the home station, to check the profile
/// against the axis: its ends and points between them.
fn samples(element: &Element, home: &Station) -> Result<Vec<Vec3>, KernelError> {
    const COUNT: u32 = 32;
    let curve = curve_at(element, home, edge_ends(element, home))?;
    let (t0, t1) = curve.domain();
    Ok((0..=COUNT)
        .map(|i| curve.eval(t0 + (t1 - t0) * f64::from(i) / f64::from(COUNT)))
        .collect())
}

/// Sweeps `profile`, which lies in `profile_plane`, along `helix`, and
/// on along `run_out` when one is given.
///
/// See the module documentation for how the profile is carried, what the
/// faces are and what is refused. The profile may have holes; they are
/// swept with it. The body is one solid shell that has passed
/// `craft3d_topo::check` with `tolerance`. Faces are named as the sweep
/// along a path names them, with the profile's curve as the only source
/// of a side (`side(e3)`), numbered by half turn: [`Swept::faces`].
pub fn sweep_helix(
    profile: &Profile,
    profile_plane: &Frame,
    helix: &Helix,
    run_out: Option<RunOut>,
    tolerance: &Tolerance,
) -> Result<Swept, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(invalid("tolerance", tol));
    }
    if profile.outer.segments.is_empty() {
        return Err(KernelError::NoProfile);
    }
    if let Some(run) = run_out
        && !(run.angle.is_finite() && run.angle > 0.0 && run.angle <= TAU && run.rise.is_finite())
    {
        return Err(invalid("run-out", run.angle));
    }
    each_curve_once(std::slice::from_ref(profile))?;
    let center = profile
        .centroid()
        .map(|c| profile_plane.point(c[0], c[1], 0.0))
        .filter(|c| c.is_finite())
        .ok_or(KernelError::InvalidProfile {
            reason: "the profile encloses no area",
        })?;

    // The helix starts on the profile's plane.
    let begin = helix.point(0.0);
    let off = (begin - profile_plane.origin()).dot(profile_plane.z());
    if off.is_nan() || off.abs() > tol {
        return Err(refuse(
            &[],
            format!(
                "the helix starts at {} and the plane of the profile is {} from it; draw the profile on a plane through the start of the helix",
                point_text(begin),
                off.abs()
            ),
        ));
    }

    // The profile faces the way the motion carries it.
    let facing = profile_plane.z().dot(helix.velocity(center));
    let mirrored = facing < 0.0;
    let home = Station {
        origin: profile_plane.origin(),
        u: profile_plane.x(),
        b: if mirrored {
            -profile_plane.y()
        } else {
            profile_plane.y()
        },
        square: true,
    };
    let map = Map2 {
        origin: [0.0, 0.0],
        x: [1.0, 0.0],
        y: if mirrored { [0.0, -1.0] } else { [0.0, 1.0] },
        mirrored,
    };
    let mut loops: Vec<Vec<Element>> = Vec::with_capacity(1 + profile.holes.len());
    for path in std::iter::once(&profile.outer).chain(&profile.holes) {
        let mut elements = path
            .segments
            .iter()
            .map(|s| map.element(s, tol))
            .collect::<Result<Vec<_>, _>>()?;
        match elements.as_slice() {
            [] => {
                return Err(KernelError::InvalidProfile {
                    reason: "a loop without segments",
                });
            }
            [only] if !only.whole() => {
                return Err(KernelError::InvalidProfile {
                    reason: "a loop of one segment that is not a circle is not closed",
                });
            }
            [_] => {}
            several if several.iter().any(Element::whole) => {
                return Err(KernelError::InvalidProfile {
                    reason: "a circle is a loop on its own",
                });
            }
            _ => {}
        }
        if mirrored {
            elements = elements.iter().rev().map(Element::flipped).collect();
        }
        // No surface along the helix closes on itself.
        elements = elements.iter().flat_map(quarters).collect();
        for element in &mut elements {
            element.flat = false;
        }
        loops.push(elements);
    }

    // The profile against the axis: it stays clear of it and on one side
    // of it, its plane is crossed by the motion everywhere, and it is
    // shorter along the axis, as the motion sees it, than the pitch.
    let (mut reach, mut low, mut high) = (0.0_f64, f64::INFINITY, f64::NEG_INFINITY);
    let mut near = f64::INFINITY;
    let forward = home.forward()?;
    for element in loops.first().into_iter().flatten() {
        for p in samples(element, &home)? {
            let (rho, angle, height) = helix.cylindrical(p);
            if !(rho.is_finite() && rho > tol) {
                return Err(refuse(
                    &[element.source],
                    format!(
                        "the profile reaches the axis of the helix at {} on e{}; a profile swept along a helix has to stay clear of its axis",
                        point_text(p),
                        element.source
                    ),
                ));
            }
            if angle.abs() >= PI / 2.0 {
                return Err(refuse(
                    &[element.source],
                    format!(
                        "the profile reaches round the axis of the helix (to {} on e{}); it has to lie to one side of the axis, where the helix starts",
                        point_text(p),
                        element.source
                    ),
                ));
            }
            let speed = helix.velocity(p);
            let across = forward.dot(speed);
            if across.is_nan() || across <= 1e-6 * speed.length() {
                return Err(refuse(
                    &[element.source],
                    format!(
                        "the helix carries the profile along its own plane at {} on e{}: the sweep would fold over itself; draw the profile on a plane through the axis or square to the helix",
                        point_text(p),
                        element.source
                    ),
                ));
            }
            let level = height - helix.advance() * angle;
            reach = reach.max(rho);
            near = near.min(rho);
            (low, high) = (low.min(level), high.max(level));
        }
    }
    let long = high - low;
    if !long.is_finite() || long >= helix.pitch - tol {
        return Err(refuse(
            &[],
            format!(
                "the pitch {} is not longer than the profile is along the axis ({}): one turn would run into the next; make the pitch longer or the profile shorter",
                helix.pitch,
                high - low
            ),
        ));
    }
    if let Some(run) = run_out {
        reach += run.rise.max(0.0);
        // A run-out towards the axis stays clear of it.
        if near + run.rise.min(0.0) <= tol {
            return Err(refuse(
                &[],
                format!(
                    "the run-out moves the profile {} towards the axis of the helix, and the profile comes within {near} of it; a profile swept along a helix has to stay clear of its axis",
                    -run.rise
                ),
            ));
        }
    }

    let radial = center - helix.origin;
    let outward = (radial - helix.axis * radial.dot(helix.axis))
        .normalized()
        .ok_or_else(internal)?;
    let end = helix.angle();
    let screw = Screw {
        helix,
        home,
        outward,
        end,
        run_out,
    };
    // Half turns, then the run-out in half turns.
    let mut legs = Vec::new();
    let mut push = |from: f64, to: f64| {
        let halves = ((to - from) / PI - 1e-9).ceil().max(1.0) as usize;
        for k in 0..halves {
            let a = from + PI * k as f64;
            let b = if k + 1 == halves {
                to
            } else {
                from + PI * (k + 1) as f64
            };
            legs.push(Leg { from: a, to: b });
        }
    };
    push(0.0, end);
    if let Some(run) = run_out {
        push(end, end + run.angle);
    }
    let sides: usize = loops.iter().map(Vec::len).sum();
    if sides.saturating_mul(legs.len()) > MAX_FACES {
        return Err(KernelError::TooManyCurves {
            count: sides.saturating_mul(legs.len()),
            limit: MAX_FACES,
        });
    }
    // The plan the helix itself would need at the profile's reach; a
    // leg whose faces are still too far off tries the finer ones.
    let first_plan = match plan_for(reach, tol / 4.0) {
        Ok(plan) => PLANS.iter().position(|p| *p == plan).unwrap_or(0),
        Err(_) => PLANS.len().saturating_sub(1),
    };

    let mut b = Builder {
        body: Body::new(BodyKind::Solid),
        faces: Vec::new(),
        tol,
    };
    let first_ring = b.ring(&loops, &home, true)?;
    let mut ring = first_ring.clone();
    let mut station = home;
    let mut shell: Vec<FaceId> = Vec::new();
    let (mut deviation, mut stations, mut length) = (0.0_f64, 1_usize, helix.length());
    // The start of the helix in the axes of the home station.
    let along = {
        let d = begin - home.origin;
        (d.dot(home.u), d.dot(home.b))
    };
    for leg in &legs {
        let mut followed = None;
        let mut reached = f64::INFINITY;
        for plan in PLANS.iter().skip(first_plan) {
            let made = follow(&screw, leg, &station, &loops, *plan)?;
            reached = made.deviation;
            if made.deviation <= tol / 2.0 {
                followed = Some(made);
                break;
            }
        }
        let Some(followed) = followed else {
            return Err(refuse(
                &[],
                format!(
                    "the sweep along the helix cannot be brought within the tolerance: its surfaces are still {reached} from the true sweep, and half the tolerance ({}) is allowed; give the document a coarser tolerance",
                    tol / 2.0
                ),
            ));
        };
        deviation = deviation.max(followed.deviation);
        stations += followed.stations.len().saturating_sub(1);
        // Along the run-out, the way the start of the helix goes, by its
        // chords through the stations.
        if leg.from >= end {
            let points: Vec<Vec3> = followed
                .stations
                .iter()
                .map(|s| s.origin + s.u * along.0 + s.b * along.1)
                .collect();
            length += points
                .iter()
                .zip(points.iter().skip(1))
                .map(|(p, q)| p.distance(*q))
                .sum::<f64>();
        }
        let next = *followed.stations.last().ok_or_else(internal)?;
        let next_ring = b.ring(&loops, &next, true)?;
        let motion = Motion::Follow {
            stations: followed.stations,
            band: followed.band,
        };
        for (l, elements) in loops.iter().enumerate() {
            let (Some(from), Some(to), Some(low_edges), Some(high_edges)) = (
                ring.vertices.get(l),
                next_ring.vertices.get(l),
                ring.edges.get(l),
                next_ring.edges.get(l),
            ) else {
                return Err(internal());
            };
            let mut swept: Vec<EdgeId> = Vec::with_capacity(elements.len());
            for (element, (&here, &there)) in elements.iter().zip(from.iter().zip(to)) {
                swept.push(b.rail(&motion, element.start, here, there)?.0);
            }
            for (j, element) in elements.iter().enumerate() {
                let (
                    Some(&Some((bottom, bottom_against))),
                    Some(&Some((top, top_against))),
                    Some(&here),
                    Some(&there),
                ) = (
                    low_edges.get(j),
                    high_edges.get(j),
                    swept.get(j),
                    swept.get((j + 1) % elements.len()),
                )
                else {
                    return Err(internal());
                };
                let around = vec![vec![
                    coedge(bottom, bottom_against),
                    coedge(there, false),
                    coedge(top, !top_against),
                    coedge(here, true),
                ]];
                let rigid = screw.rise(leg.to) == 0.0;
                let cylinder = if rigid {
                    along_axis(element, &station, helix, tol)?
                } else {
                    None
                };
                let (surface, reversed) = match cylinder {
                    Some(surface) => surface,
                    None => b
                        .surface(&motion, element, &station, &next, 0)
                        .map_err(|error| match error {
                            KernelError::Sweep { .. } => refuse(
                                &[element.source],
                                format!(
                                    "the face e{} of the profile sweeps along the helix folds onto itself",
                                    element.source
                                ),
                            ),
                            other => other,
                        })?,
                };
                shell.push(b.face(surface, reversed, around, Role::Side, vec![element.source]));
            }
        }
        station = next;
        ring = next_ring;
    }

    let sources = cap_sources(&profile.outer);
    for (role, at, ring) in [
        (Role::StartCap, &home, &first_ring),
        (Role::EndCap, &station, &ring),
    ] {
        let forward = at.forward()?;
        let start = role == Role::StartCap;
        let normal = if start { -forward } else { forward };
        let frame = Frame::new(at.origin, normal, at.u).ok_or_else(internal)?;
        let mut around = Vec::with_capacity(ring.edges.len());
        for edges in &ring.edges {
            let edges = edges
                .iter()
                .map(|e| e.ok_or_else(internal))
                .collect::<Result<Vec<_>, _>>()?;
            around.push(if start {
                edges
                    .iter()
                    .rev()
                    .map(|&(edge, against)| coedge(edge, !against))
                    .collect()
            } else {
                edges
                    .iter()
                    .map(|&(edge, against)| coedge(edge, against))
                    .collect()
            });
        }
        shell.push(b.face(
            Surface::Plane(craft3d_geom::Plane::new(frame)),
            false,
            around,
            role,
            sources.clone(),
        ));
    }
    b.body.add_shell(shell);

    let mut seen: BTreeMap<(Role, Vec<u64>), u32> = BTreeMap::new();
    for origin in &mut b.faces {
        let number = seen
            .entry((origin.role, origin.sources.clone()))
            .or_insert(0);
        origin.piece = *number;
        *number = number.saturating_add(1);
    }
    let faces: Vec<Origin> = b.faces;
    let mut defects = craft3d_topo::check_remembered(&b.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "sweep along a helix",
            defects,
        });
    }
    Ok(Swept {
        body: b.body,
        faces,
        closed: false,
        length,
        deviation,
        stations,
    })
}

/// The cylinder a line of the profile parallel to the axis sweeps, and
/// whether the outside is against its normal; `None` for every other
/// curve.
fn along_axis(
    element: &Element,
    station: &Station,
    helix: &Helix,
    tol: f64,
) -> Result<Option<(Surface, bool)>, KernelError> {
    if !matches!(element.form, Form::Line) {
        return Ok(None);
    }
    let (p, q) = (station.at(element.start), station.at(element.end));
    if (q - p).cross(helix.axis).length() > tol / 8.0 {
        return Ok(None);
    }
    let middle: P2 = [
        0.5 * (element.start[0] + element.end[0]),
        0.5 * (element.start[1] + element.end[1]),
    ];
    let at = station.at(middle);
    let (rho, _, _) = helix.cylindrical(at);
    // Not along the helix's own frame: the frame of the station turns
    // with the motion, the cylinder is fixed.
    let frame = Frame::new(helix.origin, helix.axis, helix.start).ok_or_else(internal)?;
    let surface = Surface::Cylinder(Cylinder::new(frame, rho)?);
    let along = [
        element.end[0] - element.start[0],
        element.end[1] - element.start[1],
    ];
    let outward = station.vector(along).cross(helix.velocity(at));
    let reversed = against_normal(&surface, at, outward)?;
    Ok(Some((surface, reversed)))
}
