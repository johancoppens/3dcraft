//! The poles of a freeform surface: borders of its parameter range that
//! are one point in space.
//!
//! A Bézier or NURBS surface may have a whole border in one point: the
//! pole of a dome swept about an axis, the apex of a ruled patch, the top
//! of the teapot's lid. The parametrisation stalls there (the derivative
//! along the border is zero, the normal `∂u × ∂v` has no length), but the
//! surface itself is as good a surface as anywhere when it closes up
//! smoothly, and a cone-like point when it does not. What reads the
//! surface through its parameters asks here what it cannot ask there.
//!
//! A border is a pole when its control points are all one point, which is
//! what a border that is one point comes to (the basis functions of a
//! border are independent). One point within [`SAME`] of the size of the
//! control net: control points that differ by rounding are one.

use crate::Surface;
use craft3d_math::{Bbox3, Vec3};

/// Control points this share of the size of the control net apart are one.
const SAME: f64 = 1e-9;
/// How far from a pole, as a share of the parameter range across it, the
/// surface is looked at for what holds at the pole in the limit.
const NUDGE: f64 = 1e-6;
/// Within this share of the parameter range of a pole's border a point of
/// the surface is the pole: nearer than that the derivative along the
/// border is rounding.
const AT: f64 = 1e-9;
/// Places along a border at which the meridians are compared before the
/// best is narrowed down.
const LOOKS: u32 = 32;

/// A border of the parameter range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Border {
    UStart,
    UEnd,
    VStart,
    VEnd,
}

/// A border of a freeform surface that is one point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pole {
    pub border: Border,
    pub point: Vec3,
}

/// The poles of a Bézier or NURBS surface, in the order of [`Border`];
/// none for the other kinds, whose poles and apexes are known by their
/// formulas.
pub fn poles(surface: &Surface) -> Vec<Pole> {
    let (points, count_v): (&[Vec3], usize) = match surface {
        Surface::Bezier(patch) => (patch.control_points(), patch.degree_v() + 1),
        Surface::Nurbs(s) => (s.control_points(), s.counts().1),
        _ => return Vec::new(),
    };
    if count_v == 0 || !points.len().is_multiple_of(count_v) || points.len() < 2 * count_v {
        return Vec::new();
    }
    let ((u0, u1), (v0, v1)) = surface.domain();
    if ![u0, u1, v0, v1].iter().all(|x| x.is_finite()) {
        return Vec::new();
    }
    let count_u = points.len() / count_v;
    let same = SAME * Bbox3::from_points(points.iter().copied()).diagonal();
    let one_point = |border: &mut dyn Iterator<Item = usize>| -> Option<Vec3> {
        let first = *points.get(border.next()?)?;
        for index in border {
            let apart = points.get(index)?.distance(first);
            // A NaN is no point.
            if apart.is_nan() || apart > same {
                return None;
            }
        }
        first.is_finite().then_some(first)
    };
    let mut poles = Vec::new();
    let mut look = |border: Border, indices: &mut dyn Iterator<Item = usize>| {
        if let Some(point) = one_point(indices) {
            poles.push(Pole { border, point });
        }
    };
    look(Border::UStart, &mut (0..count_v));
    look(
        Border::UEnd,
        &mut (0..count_v).map(|j| (count_u - 1) * count_v + j),
    );
    look(Border::VStart, &mut (0..count_u).map(|i| i * count_v));
    look(
        Border::VEnd,
        &mut (0..count_u).map(|i| i * count_v + count_v - 1),
    );
    poles
}

impl Pole {
    /// Whether the border is one of constant v (the surface does not move
    /// with u along it).
    pub fn across_v(&self) -> bool {
        matches!(self.border, Border::VStart | Border::VEnd)
    }

    /// The range of the parameter that runs along the border, the
    /// parameter the border is at, and the range of the parameter across.
    fn ranges(&self, surface: &Surface) -> ((f64, f64), f64, f64) {
        let ((u0, u1), (v0, v1)) = surface.domain();
        match self.border {
            Border::UStart => ((v0, v1), u0, u1 - u0),
            Border::UEnd => ((v0, v1), u1, u0 - u1),
            Border::VStart => ((u0, u1), v0, v1 - v0),
            Border::VEnd => ((u0, u1), v1, v0 - v1),
        }
    }

    /// The parameter the border is at.
    pub fn at(&self, surface: &Surface) -> f64 {
        self.ranges(surface).1
    }

    /// The parameters of the place `along` the border and `off` it, as a
    /// share of the range across.
    fn place(&self, surface: &Surface, along: f64, off: f64) -> (f64, f64) {
        let (_, at, across) = self.ranges(surface);
        if self.across_v() {
            (along, at + across * off)
        } else {
            (at + across * off, along)
        }
    }

    /// How far `(u, v)` is from the pole's border, as a share of the
    /// parameter range across.
    pub fn share_off(&self, surface: &Surface, u: f64, v: f64) -> f64 {
        let (_, at, across) = self.ranges(surface);
        let here = if self.across_v() { v } else { u };
        ((here - at) / across).abs()
    }

    /// Whether the point of the surface at `(u, v)` is this pole: on its
    /// border, to rounding.
    pub fn holds(&self, surface: &Surface, u: f64, v: f64) -> bool {
        let (_, at, across) = self.ranges(surface);
        let here = if self.across_v() { v } else { u };
        // Not `>`: a NaN is nowhere.
        (here - at).abs() <= AT * across.abs()
    }

    /// The unit normal `∂u × ∂v` of the surface at the pole as it is
    /// reached along the meridian at `along` (the parameter that runs
    /// along the border): the limit, taken a little way off the pole.
    /// Where the surface closes up smoothly it is the same along every
    /// meridian; at a cone-like point it is the normal of the cone along
    /// that ruling. `None` where the surface has no normal nearby either.
    ///
    /// At an end of the border that is the border of another pole (a pole
    /// at a corner of the parameters) every place off this border near
    /// that end is next to that pole too, and both derivatives are small
    /// there: the place is kept as far from that end as from this border.
    pub fn normal_along(&self, surface: &Surface, along: f64) -> Option<Vec3> {
        let ((low, high), _, _) = self.ranges(surface);
        let (mut low_pole, mut high_pole) = (false, false);
        for other in poles(surface) {
            if other.across_v() == self.across_v() {
                continue;
            }
            match other.border {
                Border::UStart | Border::VStart => low_pole = true,
                Border::UEnd | Border::VEnd => high_pole = true,
            }
        }
        [NUDGE, 1e-4, 1e-2].into_iter().find_map(|off| {
            let step = off * (high - low);
            let mut along = along;
            if low_pole {
                along = along.max(low + step);
            }
            if high_pole {
                along = along.min(high - step);
            }
            let (u, v) = self.place(surface, along, off);
            let (_, su, sv) = surface.derivatives(u, v);
            let cross = su.cross(sv);
            // A cross product that short against its factors is rounding.
            (cross.length() > 1e-9 * su.length() * sv.length())
                .then(|| cross.normalized())
                .flatten()
        })
    }

    /// The unit normal of the surface for a point `p` whose nearest point
    /// of the surface is the pole: the limit normal along the meridian
    /// that leaves the pole nearest to the direction of `p`.
    ///
    /// The side of the surface `p` is on is the sign of `(p − pole) ·
    /// normal`. Why this meridian: a point a little further along whose
    /// foot is still an ordinary point of the surface near the pole has
    /// that foot on the meridian nearest to its direction, and is on the
    /// side that foot's normal says; the sign cannot change on the way to
    /// a point whose foot is the pole, because it would have to pass
    /// through zero, which takes a point straight behind that meridian
    /// (in its tangent plane and across the border), and for such a point
    /// that meridian is the farthest in direction, not the nearest. Where
    /// the surface closes up smoothly every meridian gives the same
    /// normal.
    ///
    /// Where the pole is at a corner of the parameters (two adjacent
    /// borders are this one point), the meridians leave it from both
    /// borders, and the nearest of all is taken.
    pub fn normal_towards(&self, surface: &Surface, p: Vec3) -> Option<Vec3> {
        let (along, score) = self.best_meridian(surface, p);
        let mut pick = (score, *self, along);
        for other in poles(surface) {
            // A border across the other parameter that is a pole too
            // shares a corner with this one, and so is this point.
            if other.across_v() == self.across_v() {
                continue;
            }
            let (along, score) = other.best_meridian(surface, p);
            if score > pick.0 {
                pick = (score, other, along);
            }
        }
        pick.1.normal_along(surface, pick.2)
    }

    /// The parameter along the border of the meridian that leaves the
    /// pole nearest to the direction of `p` (the middle of the border for
    /// `p` at the pole).
    pub fn meridian_towards(&self, surface: &Surface, p: Vec3) -> f64 {
        self.best_meridian(surface, p).0
    }

    /// [`Self::meridian_towards`], and how well that meridian points
    /// towards `p` (the cosine of the angle between them; minus infinity
    /// when nothing points anywhere).
    fn best_meridian(&self, surface: &Surface, p: Vec3) -> (f64, f64) {
        let ((low, high), _, _) = self.ranges(surface);
        let from = p - self.point;
        let middle = 0.5 * (low + high);
        if !(from.length() > 0.0 && from.is_finite()) {
            return (middle, f64::NEG_INFINITY);
        }
        // How well the meridian at `along` points towards `p`.
        let towards = |along: f64| {
            let (u, v) = self.place(surface, along, NUDGE);
            (surface.eval(u, v) - self.point)
                .normalized()
                .map_or(f64::NEG_INFINITY, |leaving| leaving.dot(from))
        };
        let step = (high - low) / f64::from(LOOKS);
        let mut best = (f64::NEG_INFINITY, middle);
        for k in 0..=LOOKS {
            let along = low + step * f64::from(k);
            let value = towards(along);
            if value > best.0 {
                best = (value, along);
            }
        }
        // Narrowed between the neighbours of the best place.
        let (mut a, mut b) = ((best.1 - step).max(low), (best.1 + step).min(high));
        for _ in 0..48 {
            let (m1, m2) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
            if !(m1 > a && m2 < b && m1 < m2) {
                break;
            }
            if towards(m1) >= towards(m2) {
                b = m2;
            } else {
                a = m1;
            }
        }
        let narrowed = 0.5 * (a + b);
        let value = towards(narrowed);
        let (along, value) = if value >= best.0 {
            (narrowed, value)
        } else {
            (best.1, best.0)
        };
        (along, value / from.length())
    }

    /// The parameters `(u, v)` of the place on the meridian at `along`
    /// that is `off` of the parameter range across from the pole.
    pub fn off_pole(&self, surface: &Surface, along: f64, off: f64) -> (f64, f64) {
        self.place(surface, along, off)
    }
}

/// The pole of `surface` that its point at `(u, v)` is, when it is one.
pub fn pole_at(surface: &Surface, u: f64, v: f64) -> Option<Pole> {
    // Only a point on a border of the range can be: nothing is looked up
    // for any other.
    let ((u0, u1), (v0, v1)) = surface.domain();
    let on_border = (u - u0).abs() <= AT * (u1 - u0)
        || (u1 - u).abs() <= AT * (u1 - u0)
        || (v - v0).abs() <= AT * (v1 - v0)
        || (v1 - v).abs() <= AT * (v1 - v0);
    if !on_border {
        return None;
    }
    poles(surface)
        .into_iter()
        .find(|pole| pole.holds(surface, u, v))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::{BezierPatch, NurbsSurface};

    /// A cone of half-angle `alpha` about z with its apex at the origin,
    /// a quarter turn of it: u around the axis, v from the apex out.
    fn quarter_cone(alpha: f64) -> Surface {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let (r, h) = (alpha.sin(), alpha.cos());
        let ring = [(1.0, 0.0, 1.0), (1.0, 1.0, s), (0.0, 1.0, 1.0)];
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for (x, y, w) in ring {
            for reach in [0.0, 2.0] {
                ctrl.push(Vec3::new(x * r * reach, y * r * reach, h * reach));
                weights.push(w);
            }
        }
        Surface::Nurbs(
            NurbsSurface::new(
                2,
                1,
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                3,
                2,
                ctrl,
                weights,
            )
            .unwrap(),
        )
    }

    #[test]
    fn a_collapsed_border_is_a_pole_and_no_other_is() {
        let cone = quarter_cone(0.5);
        let found = poles(&cone);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].border, Border::VStart);
        assert_eq!(found[0].point, Vec3::ZERO);
        assert!(pole_at(&cone, 0.3, 0.0).is_some());
        assert!(pole_at(&cone, 0.3, 1.0).is_none());
        assert!(pole_at(&cone, 0.3, 0.5).is_none());
        // A flat patch has none; a plane neither.
        let flat = Surface::Bezier(
            BezierPatch::new(
                1,
                1,
                vec![Vec3::ZERO, Vec3::Y, Vec3::X, Vec3::new(1.0, 1.0, 0.0)],
            )
            .unwrap(),
        );
        assert!(poles(&flat).is_empty());
    }

    #[test]
    fn the_normal_at_an_apex_is_that_of_the_ruling_nearest_to_the_point() {
        for alpha in [0.2_f64, 0.5, 1.2] {
            let cone = quarter_cone(alpha);
            let pole = poles(&cone)[0];
            // Along a ruling the normal is the cone's there.
            for along in [0.0, 0.3, 1.0] {
                let n = pole.normal_along(&cone, along).unwrap();
                let (u, v) = (along, 0.5);
                let (_, su, sv) = cone.derivatives(u, v);
                assert!(n.distance(su.cross(sv).normalized().unwrap()) < 1e-6);
            }
            // Points behind the apex, nearest to it: outside the cone,
            // which is where its normal `∂u × ∂v` points (away from the
            // axis and back).
            for (x, y) in [(0.3, 0.1), (0.1, 0.4), (0.2, 0.2)] {
                let ruling = Vec3::new(x, y, 0.0).normalized().unwrap() * alpha.sin()
                    + Vec3::Z * alpha.cos();
                // Square to a ruling and outwards, then pushed back.
                let out = (Vec3::new(x, y, 0.0).normalized().unwrap() * alpha.cos()
                    - Vec3::Z * alpha.sin())
                    * 0.3
                    - ruling * 0.2;
                let n = pole.normal_towards(&cone, out).unwrap();
                assert!(out.dot(n) > 0.0, "{alpha} {x} {y}");
            }
        }
    }
}
