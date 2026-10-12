//! Inside or outside on any analytic surface, as a winding number.
//!
//! A face is a region of its surface bounded by closed loops. On a plane
//! the winding number of the loops about a point says whether the point is
//! in the region, with no special cases. The other surfaces are not planes,
//! but each is mapped onto one by a *chart*, computed from the points in
//! space rather than from the surface parameters, so that neither a seam
//! nor a pole is a special place:
//!
//! | Surface | Chart | What is missing from the plane |
//! |---|---|---|
//! | plane | its own coordinates | nothing |
//! | cylinder | angle and `exp(height / radius)` as polar coordinates (conformal) | the two ends, which no bounded face reaches |
//! | cone | dropped along its axis onto the plane across it | nothing: the apex is an ordinary point |
//! | sphere | stereographic projection from a point chosen clear of the loops | that point, which a face may contain |
//! | torus | cut open along a circle that the loops do not cross, then one angle and the other as radius | the cut circle, which a face may contain |
//!
//! A loop that goes around a cylinder's axis, a cone's apex or a torus'
//! tube simply goes around the origin of its chart.
//!
//! The charts are never evaluated as coordinates of a far point. What the
//! winding number needs is the direction from the image of `q` to the image
//! of `p`, up to a positive factor, and that has a bounded form on a
//! cylinder (no exponential) and on a sphere (no division), even where the
//! images themselves run off to infinity.
//!
//! A face on a sphere or a torus may contain the missing point or circle.
//! Its loops then wind about the points *outside* it (see
//! `Outline::infinite`).

use super::Reason;
use super::hits::speed_bound;
use super::outline::Seg;
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Frame, Vec3};
use std::f64::consts::{PI, TAU};

pub(super) const TOO_CLOSE: Reason =
    "a point too close to a face boundary to tell inside from outside (at the edge of tolerance)";
const FREEFORM: Reason = "a freeform (Bézier or NURBS) face; booleans handle planes, cylinders, cones, spheres and tori so far";
const NO_POLE: Reason =
    "a face on a sphere whose boundary comes close to every point tried as a centre of projection";
const NO_CUT: Reason = "a face on a torus whose boundary crosses every circle around its axis and every circle around its tube";

/// The most evaluations the winding sum spends on one curve piece.
const MAX_TURN_STEPS: usize = 1 << 16;
/// Samples per boundary piece when looking for a circle of a torus that the
/// boundary does not cross.
const CUT_SAMPLES: u32 = 16;

#[derive(Clone, Debug)]
pub(super) enum Chart {
    Plane(Frame),
    Cylinder {
        frame: Frame,
        radius: f64,
    },
    Cone(Frame),
    Sphere {
        center: Vec3,
        radius: f64,
        /// The centre of projection, as a unit vector from the centre, and
        /// two axes across it.
        pole: Vec3,
        x: Vec3,
        y: Vec3,
    },
    /// A torus cut along the circle around its tube at angle `cut` about
    /// the axis: the angle around the tube is the chart's angle.
    Tube {
        frame: Frame,
        major: f64,
        minor: f64,
        cut: f64,
    },
    /// A torus cut along the circle around its axis at angle `cut` around
    /// the tube: the angle about the axis is the chart's angle.
    Ring {
        frame: Frame,
        major: f64,
        minor: f64,
        cut: f64,
    },
}

/// What the loops are wound about.
#[derive(Clone, Copy, Debug)]
pub(super) enum About {
    /// A point of the surface.
    Point(Vec3),
    /// The inner end of a cut torus: the origin of its chart.
    InnerEnd,
}

/// A point to wind about, prepared.
struct Target {
    /// Chart coordinates of the point, where the chart has them.
    at: (f64, f64),
    /// The point itself (cylinder: in the frame; sphere: as a unit vector
    /// less the pole).
    q: Vec3,
    /// How far the direction moves per unit of distance of the point it
    /// looks at.
    lipschitz: f64,
}

fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

/// The angle from direction `a` to direction `b`, in `(-π, π]`.
fn angle(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 * b.1 - a.1 * b.0).atan2(a.0 * b.0 + a.1 * b.1)
}

/// The angles about the axis and around the tube of a point of a torus.
fn torus_angles(frame: &Frame, major: f64, p: Vec3) -> (f64, f64) {
    let (x, y, z) = frame.local(p);
    (y.atan2(x), z.atan2(x.hypot(y) - major))
}

/// The middle of the widest stretch of the circle that none of the
/// stretches `(start, length)` covers, and its width.
fn widest_gap(mut covered: Vec<(f64, f64)>) -> Option<(f64, f64)> {
    if covered.iter().any(|&(_, length)| length >= TAU) {
        return None;
    }
    if covered.is_empty() {
        return Some((PI, TAU));
    }
    covered.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Walk once around, starting where the first stretch starts.
    let first = covered.first().map_or(0.0, |c| c.0);
    let mut reach = first;
    let mut best: Option<(f64, f64)> = None;
    let mut consider = |from: f64, to: f64| {
        let width = to - from;
        if width > 0.0 && best.is_none_or(|(_, widest)| width > widest) {
            best = Some((wrap(from + 0.5 * width), width));
        }
    };
    for &(start, length) in &covered {
        consider(reach, start);
        reach = reach.max(start + length);
    }
    consider(reach, first + TAU);
    best
}

impl Chart {
    /// A chart of `surface` for telling the inside of `loops`.
    pub(super) fn new(surface: &Surface, loops: &[Vec<Seg>]) -> Result<Chart, Reason> {
        Ok(match surface {
            Surface::Plane(s) => Chart::Plane(*s.frame()),
            Surface::Cylinder(s) => Chart::Cylinder {
                frame: *s.frame(),
                radius: s.radius(),
            },
            Surface::Cone(s) => Chart::Cone(*s.frame()),
            Surface::Sphere(s) => {
                let frame = s.frame();
                let (center, radius) = (frame.origin(), s.radius());
                // The candidate furthest from the boundary: the axes, then
                // the diagonals.
                let mut best: Option<(f64, Vec3)> = None;
                let signs = [-1.0, 1.0];
                let mut candidates = Vec::with_capacity(14);
                for s in signs {
                    candidates.push(frame.z() * s);
                    candidates.push(frame.x() * s);
                    candidates.push(frame.y() * s);
                }
                for a in signs {
                    for b in signs {
                        for c in signs {
                            candidates.push(frame.vector(a, b, c) * (1.0 / 3.0_f64.sqrt()));
                        }
                    }
                }
                for pole in candidates {
                    let clear = super::outline::clearance(loops, center + pole * radius);
                    if best.is_none_or(|(most, _)| clear > most) {
                        best = Some((clear, pole));
                    }
                }
                let Some((clear, pole)) = best else {
                    return Err(NO_POLE);
                };
                if clear.is_nan() || clear < 1e-3 * radius {
                    return Err(NO_POLE);
                }
                let across = Frame::from_z(center, pole).ok_or(NO_POLE)?;
                Chart::Sphere {
                    center,
                    radius,
                    pole,
                    x: across.x(),
                    y: across.y(),
                }
            }
            Surface::Torus(s) => {
                let frame = *s.frame();
                let (major, minor) = s.radii();
                // Per boundary piece, the stretch of each angle it covers:
                // followed from sample to sample, and widened by as much
                // as the curve can stray between two samples.
                let mut about_axis = Vec::new();
                let mut around_tube = Vec::new();
                for seg in loops.iter().flatten() {
                    let step = (seg.range.1 - seg.range.0) / f64::from(CUT_SAMPLES);
                    // Half a step of travel, as an angle: at most a quarter
                    // turn's worth of arc per chord.
                    let stray = 0.25 * PI * speed_bound(&seg.curve) * step;
                    let margins = (stray / (major - minor), stray / minor);
                    let (start_u, start_v) =
                        torus_angles(&frame, major, seg.curve.eval(seg.range.0));
                    let (mut u, mut v) = (0.0_f64, 0.0_f64);
                    let (mut lo, mut hi) = ((0.0_f64, 0.0_f64), (0.0_f64, 0.0_f64));
                    let mut previous = (start_u, start_v);
                    for k in 1..=CUT_SAMPLES {
                        let p = seg.curve.eval(seg.range.0 + step * f64::from(k));
                        let here = torus_angles(&frame, major, p);
                        u += (here.0 - previous.0 + PI).rem_euclid(TAU) - PI;
                        v += (here.1 - previous.1 + PI).rem_euclid(TAU) - PI;
                        lo = (lo.0.min(u), lo.1.min(v));
                        hi = (hi.0.max(u), hi.1.max(v));
                        previous = here;
                    }
                    if ![u, v, margins.0, margins.1].iter().all(|x| x.is_finite()) {
                        return Err(NO_CUT);
                    }
                    about_axis.push((
                        wrap(start_u + lo.0 - margins.0),
                        hi.0 - lo.0 + 2.0 * margins.0,
                    ));
                    around_tube.push((
                        wrap(start_v + lo.1 - margins.1),
                        hi.1 - lo.1 + 2.0 * margins.1,
                    ));
                }
                const NARROWEST: f64 = 1e-6;
                match (widest_gap(about_axis), widest_gap(around_tube)) {
                    (Some((cut, width)), _) if width > NARROWEST => Chart::Tube {
                        frame,
                        major,
                        minor,
                        cut,
                    },
                    (_, Some((cut, width))) if width > NARROWEST => Chart::Ring {
                        frame,
                        major,
                        minor,
                        cut,
                    },
                    _ => return Err(NO_CUT),
                }
            }
            Surface::Bezier(_) | Surface::Nurbs(_) => return Err(FREEFORM),
        })
    }

    /// True for the charts that leave out a place a face may contain.
    pub(super) fn is_closed(&self) -> bool {
        matches!(
            self,
            Chart::Sphere { .. } | Chart::Tube { .. } | Chart::Ring { .. }
        )
    }

    /// True for the charts whose origin is not a point of the surface but
    /// one end of a cut torus.
    pub(super) fn has_inner_end(&self) -> bool {
        matches!(self, Chart::Tube { .. } | Chart::Ring { .. })
    }

    /// Chart coordinates, for the charts that have them.
    fn coordinates(&self, p: Vec3) -> (f64, f64) {
        match self {
            Chart::Plane(frame) | Chart::Cone(frame) => {
                let (x, y, _) = frame.local(p);
                (x, y)
            }
            Chart::Tube {
                frame, major, cut, ..
            } => {
                let (u, v) = torus_angles(frame, *major, p);
                let reach = 1.0 + wrap(u - cut) / TAU;
                (reach * v.cos(), reach * v.sin())
            }
            Chart::Ring {
                frame, major, cut, ..
            } => {
                let (u, v) = torus_angles(frame, *major, p);
                let reach = 1.0 + wrap(v - cut) / TAU;
                (reach * u.cos(), reach * u.sin())
            }
            Chart::Cylinder { .. } | Chart::Sphere { .. } => (0.0, 0.0),
        }
    }

    fn target(&self, about: About) -> Option<Target> {
        let q = match about {
            About::Point(q) => q,
            About::InnerEnd => {
                let lipschitz = match self {
                    Chart::Tube { major, minor, .. } => 1.0 / (TAU * (major - minor)) + 2.0 / minor,
                    Chart::Ring { major, minor, .. } => 1.0 / (TAU * minor) + 2.0 / (major - minor),
                    _ => return None,
                };
                return Some(Target {
                    at: (0.0, 0.0),
                    q: Vec3::ZERO,
                    lipschitz,
                });
            }
        };
        Some(match self {
            Chart::Plane(_) | Chart::Cone(_) => Target {
                at: self.coordinates(q),
                q,
                lipschitz: 1.0,
            },
            Chart::Cylinder { frame, radius } => {
                let (x, y, z) = frame.local(q);
                let reach = x.hypot(y);
                if !(reach.is_finite() && reach > 0.0) {
                    return None;
                }
                Target {
                    at: (0.0, 0.0),
                    q: Vec3::new(x / reach, y / reach, z),
                    lipschitz: 2.0 / radius,
                }
            }
            Chart::Sphere {
                center,
                radius,
                pole,
                ..
            } => {
                let unit = (q - *center).normalized()?;
                let from_pole = unit - *pole;
                let far = from_pole.length();
                Target {
                    at: (0.0, 0.0),
                    q: from_pole,
                    lipschitz: far * (far + 4.0) / radius,
                }
            }
            Chart::Tube { major, minor, .. } => Target {
                at: self.coordinates(q),
                q,
                lipschitz: 1.0 / (TAU * (major - minor)) + 2.0 / minor,
            },
            Chart::Ring { major, minor, .. } => Target {
                at: self.coordinates(q),
                q,
                lipschitz: 1.0 / (TAU * minor) + 2.0 / (major - minor),
            },
        })
    }

    /// The direction from the image of the target to the image of `p`, up
    /// to a positive factor; zero only when they are the same point.
    fn direction(&self, target: &Target, p: Vec3) -> (f64, f64) {
        match self {
            Chart::Plane(_) | Chart::Cone(_) | Chart::Tube { .. } | Chart::Ring { .. } => {
                let (x, y) = self.coordinates(p);
                (x - target.at.0, y - target.at.1)
            }
            Chart::Cylinder { frame, radius } => {
                // With w = exp((height + i radius angle) / radius) the
                // difference of two images is, up to a positive factor and
                // a fixed turn, (E c - 1, E s) / (E + 1): E the ratio of
                // their distances from the origin, (c, s) the cosine and
                // sine of the angle between them. In terms of
                // T = (E - 1) / (E + 1) = tanh(Δheight / 2 radius) nothing
                // overflows.
                let (x, y, z) = frame.local(p);
                let reach = x.hypot(y);
                if !(reach.is_finite() && reach > 0.0) {
                    return (0.0, 0.0);
                }
                let (ux, uy) = (x / reach, y / reach);
                let c = ux * target.q.x + uy * target.q.y;
                let s = target.q.x * uy - target.q.y * ux;
                let t = (0.5 * (z - target.q.z) / radius).tanh();
                (0.5 * (t * (1.0 + c) - (1.0 - c)), 0.5 * (t + 1.0) * s)
            }
            Chart::Sphere {
                center, pole, x, y, ..
            } => {
                // Stereographic projection from the pole N is an inversion
                // about it; the difference of the images of p and q is,
                // times |p - N|² |q - N|², the vector below, which lies
                // across N.
                let Some(unit) = (p - *center).normalized() else {
                    return (0.0, 0.0);
                };
                let from_pole = unit - *pole;
                let v =
                    from_pole * target.q.length_squared() - target.q * from_pole.length_squared();
                (v.dot(*x), v.dot(*y))
            }
        }
    }

    /// The angle the direction from the target to a point moving along the
    /// piece (in the curve's own direction) turns through.
    fn turn(&self, seg: &Seg, target: &Target) -> Result<f64, Reason> {
        let (from, to) = (self.direction(target, seg.a), self.direction(target, seg.b));
        // A straight piece of a plane, a ruling of a cylinder and a ruling
        // of a cone are straight in their charts: they turn by less than
        // half a turn about any point not on them.
        let straight = matches!(
            (&seg.curve, self),
            (
                Curve3::Line(_),
                Chart::Plane(_) | Chart::Cylinder { .. } | Chart::Cone(_)
            )
        );
        if straight {
            return Ok(angle(from, to));
        }
        // A stretch that stays within a disc that does not contain the
        // target turns by less than half a turn, which is what `angle`
        // assumes. Halve the stretches until they do.
        let pace = speed_bound(&seg.curve) * target.lipschitz;
        if !(pace.is_finite() && pace > 0.0) {
            return Err(TOO_CLOSE);
        }
        let mut total = 0.0;
        let mut stack = vec![(seg.range.0, from, seg.range.1, to)];
        let mut steps = 0_usize;
        while let Some((t0, p0, t1, p1)) = stack.pop() {
            steps += 1;
            if steps > MAX_TURN_STEPS {
                return Err(TOO_CLOSE);
            }
            if p0.0.hypot(p0.1) > pace * (t1 - t0) {
                total += angle(p0, p1);
            } else {
                let mid = 0.5 * (t0 + t1);
                if !(mid > t0 && mid < t1) {
                    return Err(TOO_CLOSE);
                }
                let pm = self.direction(target, seg.curve.eval(mid));
                stack.push((mid, pm, t1, p1));
                stack.push((t0, p0, mid, pm));
            }
        }
        Ok(total)
    }

    /// How many times the loops go around `about`, counter-clockwise in
    /// the chart.
    pub(super) fn winding<'a>(
        &self,
        loops: impl IntoIterator<Item = &'a [Seg]>,
        about: About,
    ) -> Result<i32, Reason> {
        let target = self.target(about).ok_or(TOO_CLOSE)?;
        if let (Chart::Sphere { .. }, About::Point(_)) = (self, about) {
            // At the centre of projection: at infinity in the chart, where
            // nothing winds.
            if target.q.length() < 1e-9 {
                return Ok(0);
            }
        }
        let mut total = 0.0;
        for seg in loops.into_iter().flatten() {
            total += seg.sign() * self.turn(seg, &target)?;
        }
        let turns = total / TAU;
        let whole = turns.round();
        if !(whole.is_finite() && (turns - whole).abs() < 0.05 && whole.abs() < 1e6) {
            return Err(TOO_CLOSE);
        }
        Ok(whole as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_widest_gap_between_stretches_of_a_circle() {
        assert_eq!(widest_gap(Vec::new()), Some((PI, TAU)));
        assert_eq!(widest_gap(vec![(1.0, TAU)]), None);
        let (middle, width) = widest_gap(vec![(1.0, 1.0), (4.0, 0.5)]).unwrap();
        // From 4.5 across the seam to 1: wider than from 2 to 4.
        assert!((width - (TAU - 3.5)).abs() < 1e-12);
        assert!((middle - wrap(4.5 + 0.5 * (TAU - 3.5))).abs() < 1e-12);
        // A stretch across the seam; overlapping stretches.
        let (middle, width) = widest_gap(vec![(6.0, 1.0), (0.5, 1.0), (1.0, 2.0)]).unwrap();
        assert!((width - 3.0).abs() < 1e-12, "{width}");
        assert!((middle - 4.5).abs() < 1e-12, "{middle}");
        // Covered between them.
        assert_eq!(widest_gap(vec![(0.0, 4.0), (3.0, 4.0)]), None);
    }
}
