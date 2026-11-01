//! Spindle and horn tori: a `TOROIDAL_SURFACE` whose major radius is no
//! larger than its minor radius, and `DEGENERATE_TOROIDAL_SURFACE`
//! (ISO 10303-42).
//!
//! Turned about its axis, the tube's circle then crosses the axis (or, for
//! a horn torus, touches it), and the surface passes through itself: it is
//! two sheets that meet only at the two points where the circle crosses the
//! axis (the cusps). The **apple** is what the arc of the circle on the
//! centre's side of the axis sweeps (the circle's points with
//! `R + r cos v ≥ 0`); the **lemon** what the arc beyond the axis sweeps,
//! which is the same as what its mirror image on the centre's side sweeps:
//! the arc of the circle of radius `r` about the point `−R` of the frame's
//! x axis, where it lies on the centre's side (`|w| ≤ acos(R / r)`).
//!
//! A face lies on one sheet. `craft3d-geom`'s torus is a ring torus (the
//! tube clear of the axis), so a face on a sheet is read as the exact
//! rational B-spline surface of revolution of that sheet's arc alone (as
//! `kernel::revolve` makes an arc whose circle reaches the axis,
//! `docs/design/sweeps.md`): its ends on the axis are poles, and the
//! surface does not pass through itself, so nothing downstream (meshing,
//! measuring, booleans) meets the self-intersecting part. Which sheet a face
//! of a `TOROIDAL_SURFACE` lies on is measured on its edges;
//! `DEGENERATE_TOROIDAL_SURFACE` names it (`select_outer`), and edges on
//! the other one are refused.
//!
//! **The file's normal.** ISO 10303-42 defines the normal of a
//! `toroidal_surface` explicitly, `N(u, v) = cos v (cos u x + sin u y) +
//! sin v z`: away from the tube's centre circle at the same `u`, on both
//! sheets. That is `∂u × ∂v` of `σ(u, v) = C + (R + r cos v)(cos u x +
//! sin u y) + r sin v z` where `R + r cos v > 0` (the apple) and its
//! opposite where it is negative (the lemon): so on the lemon the normal
//! points out of the lemon, not into it ([`Normal::Iso`]). A
//! `SURFACE_OF_REVOLUTION` has no such definition: its normal is `∂u × ∂v`
//! of its own parametrisation, which turns over between the two sheets
//! ([`Normal::Swept`]). An offset moves each sheet along the file's
//! normal: the apple and the lemon may then lie on tori of different minor
//! radii, so each sheet keeps its own.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]
use crate::geometry::revolved;
use crate::swept::Swept as Read;
use craft3d_geom::{NurbsCurve, Surface, Torus};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};

/// One sheet of a spindle torus: the minor radius of the torus it lies on
/// and whether the file's normal points away from the tube's centre
/// circle (`outward`) or towards it. The tube's centre for a point of the
/// apple is the point of the centre circle on the same side of the axis,
/// for a point of the lemon the one on the opposite side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sheet {
    pub minor: f64,
    pub outward: bool,
}

/// A toroidal surface whose sheets are not yet chosen: about `frame`'s z
/// axis, the tube's centre circle of radius `major` in its xy plane.
/// `apple` with a minor radius below `major` is a whole ring torus (an
/// offset can shrink an apple into one while its lemon remains); a lemon
/// whose minor radius is no larger than `major` is empty. `select` keeps
/// only the apple (`Some(true)`) or the lemon (`Some(false)`); the other
/// sheet is still known, so that edges on it are refused by name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spindle {
    pub frame: Frame,
    pub major: f64,
    pub apple: Sheet,
    pub lemon: Sheet,
    pub select: Option<bool>,
}

/// Every edge point within this share of the larger residual of the
/// other sheet's circle votes for its sheet.
const VOTE: f64 = 0.25;

/// Which normal the file gives a toroidal surface whose tube reaches its
/// axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normal {
    /// ISO 10303-42's `N` of a `toroidal_surface`: away from the tube's
    /// centre circle on both sheets.
    Iso,
    /// `∂u × ∂v` of a surface of revolution of the circle, which runs the
    /// way of the standard torus's `v` unless `flip`: away from the tube's
    /// centre on the apple, towards it on the lemon (turned over by
    /// `flip`).
    Swept { flip: bool },
}

impl Spindle {
    /// A `TOROIDAL_SURFACE` or `DEGENERATE_TOROIDAL_SURFACE` (normal
    /// [`Normal::Iso`]) or a circle turned across its axis
    /// ([`Normal::Swept`]) with `major ≤ minor` (both positive and
    /// finite); `select` keeps only the apple (`Some(true)`) or the lemon
    /// (`Some(false)`).
    pub fn toroidal(
        frame: Frame,
        major: f64,
        minor: f64,
        select: Option<bool>,
        normal: Normal,
    ) -> Result<Self, String> {
        let (apple_out, lemon_out) = match normal {
            Normal::Iso => (true, true),
            Normal::Swept { flip } => (!flip, flip),
        };
        if !(major > 0.0 && minor > 0.0 && major.is_finite() && minor.is_finite()) {
            return Err(format!(
                "torus radii must be positive and finite (major {major:e}, minor {minor:e})"
            ));
        }
        let spindle = Self {
            frame,
            major,
            apple: Sheet {
                minor,
                outward: apple_out,
            },
            lemon: Sheet {
                minor,
                outward: lemon_out,
            },
            select,
        };
        if spindle.apple().is_none() && spindle.lemon().is_none() {
            return Err(format!(
                "the inner part (lemon) of a horn torus (major radius {major:e} equal to its minor radius) is empty"
            ));
        }
        Ok(spindle)
    }

    /// The apple, if it is kept and not empty.
    pub fn apple(&self) -> Option<Sheet> {
        let s = self.apple;
        (self.select != Some(false) && s.minor > 0.0 && s.minor.is_finite()).then_some(s)
    }

    /// The lemon, if it is kept and not empty.
    pub fn lemon(&self) -> Option<Sheet> {
        let s = self.lemon;
        (self.select != Some(true) && s.minor.is_finite() && lemon_exists(self.major, s.minor))
            .then_some(s)
    }

    /// The `OFFSET_SURFACE` of it by `distance` along the file's normal:
    /// each sheet's minor radius grows by the distance where its normal
    /// points away from the tube's centre, shrinks where it points
    /// towards it. A sheet that shrinks to nothing (an apple to a minor
    /// radius of zero, a lemon to one no larger than the major radius) is
    /// gone; a ring torus where only an apple of a smaller minor radius
    /// than the major remains.
    pub fn offset(&self, distance: f64) -> Result<Read, String> {
        let moved = |s: Sheet| Sheet {
            minor: s.minor + if s.outward { distance } else { -distance },
            outward: s.outward,
        };
        Self {
            apple: moved(self.apple),
            lemon: moved(self.lemon),
            ..*self
        }
        .read()
    }

    /// A ring torus as itself; a spindle as the read whose sheet a face
    /// chooses.
    pub fn read(self) -> Result<Read, String> {
        match (self.apple(), self.lemon()) {
            (None, None) => Err(
                "the offset goes through the torus's axis and leaves no surface; it is not read"
                    .into(),
            ),
            (Some(s), None) if s.minor < self.major => Ok(Read::Done(
                Surface::Torus(
                    Torus::new(self.frame, self.major, s.minor).map_err(|e| e.to_string())?,
                ),
                !s.outward,
            )),
            _ => Ok(Read::Spindle(self)),
        }
    }

    /// Why there is no apple (`outer`) or lemon: not selected, or empty.
    fn missing(&self, outer: bool) -> String {
        let (part, sheet) = if outer {
            ("outer part (apple)", self.apple)
        } else {
            ("inner part (lemon)", self.lemon)
        };
        if self.select == Some(!outer) {
            format!(
                "the surface is only its {} (select_outer {})",
                if outer {
                    "inner part (lemon)"
                } else {
                    "outer part (apple)"
                },
                if outer { ".F." } else { ".T." }
            )
        } else if outer {
            format!(
                "its {part} is empty: its minor radius {:e} is not positive (an offset through the axis)",
                sheet.minor
            )
        } else {
            format!(
                "its {part} is empty: its minor radius {:e} is no larger than the major radius {:e} (a horn torus, or an offset that leaves none)",
                sheet.minor, self.major
            )
        }
    }

    /// Which sheet the points (along a face's edges) lie on: `true` the
    /// apple. A point votes for a sheet when it lies on that sheet's
    /// circle and further than `tolerance` and four times as far from the
    /// other's; points near the axis, where both sheets meet, do not
    /// count. Points on both sheets, or none that tells, are refused
    /// (unless there is only one sheet).
    pub fn sheet_of(&self, points: &[Vec3], tolerance: f64) -> Result<bool, String> {
        let only = match (self.apple(), self.lemon()) {
            (Some(_), None) => Some(true),
            (None, Some(_)) => Some(false),
            _ => None,
        };
        // Both circles vote, kept or not: a point on a sheet that is not
        // kept is refused, not taken for the other.
        let (mut apples, mut lemons) = (0usize, 0usize);
        for &p in points {
            let (x, y, h) = self.frame.local(p);
            let d = (x * x + y * y).sqrt();
            let off = |centre: f64, s: Sheet| {
                if s.minor > 0.0 {
                    (((d - centre) * (d - centre) + h * h).sqrt() - s.minor).abs()
                } else {
                    f64::INFINITY
                }
            };
            let (a, l) = (off(self.major, self.apple), off(-self.major, self.lemon));
            if !(a.is_finite() || l.is_finite()) {
                continue;
            }
            if a < VOTE * l && l > tolerance {
                apples += 1;
            } else if l < VOTE * a && a > tolerance {
                lemons += 1;
            }
        }
        match (apples > 0, lemons > 0, only) {
            (true, true, _) => Err(format!(
                "the face's edges lie on both the outer part (apple, {apples} points) and the inner part (lemon, {lemons} points) of the self-intersecting torus; a face lies on one"
            )),
            (true, false, Some(false)) => Err(format!(
                "the face's edges lie on the outer part (apple) of the torus, but {}",
                self.missing(true)
            )),
            (false, true, Some(true)) => Err(format!(
                "the face's edges lie on the inner part (lemon) of the torus, but {}",
                self.missing(false)
            )),
            (true, false, _) => Ok(true),
            (false, true, _) => Ok(false),
            (false, false, Some(only)) => Ok(only),
            (false, false, None) => Err(
                "cannot tell from the face's edges whether it lies on the outer part (apple) or the inner part (lemon) of the self-intersecting torus"
                    .into(),
            ),
        }
    }

    /// The surface of a face on the apple (`outer`) or the lemon, and
    /// whether the file's normal is opposite to its own.
    pub fn surface(&self, outer: bool) -> Result<(Surface, bool), String> {
        let (major, frame) = (self.major, self.frame);
        let missing = || format!("the surface has no such part: {}", self.missing(outer));
        let sheet = if outer { self.apple() } else { self.lemon() }.ok_or_else(missing)?;
        let r = sheet.minor;
        if outer && r < major {
            let torus = Torus::new(frame, major, r).map_err(|e| e.to_string())?;
            return Ok((Surface::Torus(torus), !sheet.outward));
        }
        // The meridian arc in the frame's xz half-plane on the side of x,
        // about the point `centre` of the x axis, from angle `-reach` to
        // `reach` (measured from x towards z).
        let (centre, reach) = if outer {
            (major, (-major / r).clamp(-1.0, 1.0).pacos())
        } else {
            (-major, (major / r).clamp(-1.0, 1.0).pacos())
        };
        if !(reach > 0.0) {
            return Err(missing());
        }
        let meridian = Frame::new(frame.point(centre, 0.0, 0.0), -frame.y(), frame.x())
            .ok_or("the torus's frame is degenerate")?;
        let arc = craft3d_geom::network::arc(&meridian, r, -reach, 2.0 * reach)
            .map_err(|e| e.to_string())?;
        // Its ends lie on the axis: put them there exactly, so that the
        // surface has poles rather than rings of rounding's radius.
        let mut points = arc.control_points().to_vec();
        let (o, z) = (frame.origin(), frame.z());
        for end in [0, points.len().saturating_sub(1)] {
            if let Some(p) = points.get_mut(end) {
                *p = o + z * (*p - o).dot(z);
            }
        }
        let arc = NurbsCurve::new(
            arc.degree(),
            arc.knots().to_vec(),
            points,
            arc.weights().to_vec(),
        )
        .map_err(|e| e.to_string())?;
        let nurbs = revolved(&arc, o, z).map_err(|e| e.to_string())?;
        let surface = Surface::Nurbs(nurbs);
        // Its normal against the file's, in the middle of the arc (away
        // from the poles), a quarter of the way round.
        let ((u0, u1), (v0, v1)) = surface.domain();
        let (u, v) = (u0 + 0.25 * (u1 - u0), 0.5 * (v0 + v1));
        let p = surface.eval(u, v);
        let ours = surface
            .normal(u, v)
            .ok_or("the surface of the torus's part has no normal in its middle")?;
        let radial = (p - o - z * (p - o).dot(z))
            .normalized()
            .ok_or("the middle of the torus's part lies on its axis")?;
        let away = p - (o + radial * centre);
        let file = if sheet.outward { away } else { -away };
        let dot = ours.dot(file);
        if !(dot.abs() > 1e-9 * file.length()) {
            return Err("cannot tell which way the torus's normal points".into());
        }
        Ok((surface, dot < 0.0))
    }
}

/// Whether a torus of these radii has a lemon: its tube crosses the axis
/// (more than by rounding).
fn lemon_exists(major: f64, minor: f64) -> bool {
    minor > major * (1.0 + 1e-12)
}

/// The file's normal of a surface of revolution of a circle in a plane
/// through the axis, against the standard's torus with the same circle at
/// `u = 0`: whether it is turned over (`flip` of [`Spindle::toroidal`]).
/// `point` is on the circle, `normal` the file's normal there; `None`
/// where the point is too near the axis or the normal too small to tell.
pub fn flipped(frame: &Frame, major: f64, point: Vec3, normal: Vec3) -> Option<bool> {
    let (x, _, h) = frame.local(point);
    // `σ`'s normal is `(R + r cos v)` times the direction away from the
    // tube's centre (at `u = 0`, the frame's x axis at `major`); `x` is
    // `R + r cos v`.
    let away = point - frame.point(major, 0.0, 0.0);
    let size = (major * major + h * h + x * x).sqrt();
    if !(x.abs() > 1e-6 * size) {
        return None;
    }
    let ours = away * x.signum();
    let dot = ours.dot(normal);
    (dot.abs() > 1e-9 * ours.length() * normal.length()).then_some(dot < 0.0)
}
