//! Foundation maths for 3DCraft: `f64` vectors, bounding boxes, frames,
//! transforms, tolerances and quadrature.
//!
//! Kernel maths is `f64` throughout (AGENTS.md rule 3). Operations that can
//! fail on degenerate input (normalising a zero vector) return `Option`.

mod bits;
mod bug;
mod frame;
pub mod pool;
mod portable;
pub mod quadrature;
mod threads;
mod tolerance;
mod transform;

pub use bits::{Bits, bits_of};
pub use bug::BUG;
pub use frame::Frame;
pub use portable::{Portable, fingerprint as portable_fingerprint};
pub use threads::available_threads;
pub use tolerance::{DEFAULT_LOOSEST, Tolerance};
pub use transform::Transform;

use std::ops::{Add, AddAssign, Mul, Neg, Sub};

/// A point or a direction in model space.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }

    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }

    pub fn distance(self, o: Vec3) -> f64 {
        (self - o).length()
    }

    /// The angle between `self` and `o`, in radians from 0 (the same way)
    /// to π (opposite ways), read from its sine and its cosine together:
    /// `atan2(|self × o|, self · o)`. Accurate to rounding over the whole
    /// range, near 0 and π too, where the cosine alone is not: `cos θ` is
    /// `1 − θ²/2`, which rounds to 1 for every `θ` below about 1.05e-8, so
    /// `acos` of a dot, or a dot compared with the cosine of a small angle
    /// such as an angular tolerance of 1e-9, cannot tell those angles
    /// apart (docs/design/plates.md › *Corners*, 2026-11-14). Neither
    /// vector need be a unit vector (the angle does not depend on their
    /// lengths); for a zero vector it means nothing (0 or π). The same
    /// operations in the same order as the copies written out before it,
    /// so it gives their bits.
    pub fn angle_to(self, o: Vec3) -> f64 {
        self.cross(o).length().patan2(self.dot(o))
    }

    /// Unit vector in the same direction; `None` for a zero-length or
    /// non-finite vector (or one whose squared length overflows).
    ///
    /// Where the squared length falls below the normal range (a vector
    /// shorter than about 1.5e-154, such as the normal of a triangle 1e-81
    /// across), its square root loses precision and the vector it divides
    /// comes out too long (by up to a fifth: J-018's fourth review, a proof
    /// that took a unit direction to be at most 1 + 8 ε long). Such a
    /// vector is divided by its largest part first, so its square lies
    /// between 1 and 3; every other vector is normalised as it always was,
    /// to the bit.
    pub fn normalized(self) -> Option<Vec3> {
        let len2 = self.length_squared();
        if len2 >= f64::MIN_POSITIVE {
            let len = len2.sqrt();
            return if len.is_finite() {
                Some(self * (1.0 / len))
            } else {
                None
            };
        }
        let big = self.x.abs().max(self.y.abs()).max(self.z.abs());
        if !self.is_finite() || big.is_nan() || big <= 0.0 {
            return None;
        }
        let scaled = Vec3::new(self.x / big, self.y / big, self.z / big);
        Some(scaled * (1.0 / scaled.length()))
    }

    pub fn lerp(self, o: Vec3, t: f64) -> Vec3 {
        self * (1.0 - t) + o * t
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn min(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    pub fn max(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// Axis-aligned bounding box. Empty until the first point is added.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bbox3 {
    min: Vec3,
    max: Vec3,
    empty: bool,
}

impl Default for Bbox3 {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Bbox3 {
    pub const EMPTY: Bbox3 = Bbox3 {
        min: Vec3::ZERO,
        max: Vec3::ZERO,
        empty: true,
    };

    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Self {
        let mut b = Self::EMPTY;
        for p in points {
            b.add(p);
        }
        b
    }

    /// The box with these corners, as [`Self::corners`] gives them, bit
    /// for bit (what a box written out reads back as).
    pub fn from_corners(min: Vec3, max: Vec3) -> Self {
        Bbox3 {
            min,
            max,
            empty: false,
        }
    }

    pub fn add(&mut self, p: Vec3) {
        if self.empty {
            *self = Bbox3 {
                min: p,
                max: p,
                empty: false,
            };
        } else {
            self.min = self.min.min(p);
            self.max = self.max.max(p);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.empty
    }

    /// `(min, max)`, or `None` when no point was added.
    pub fn corners(&self) -> Option<(Vec3, Vec3)> {
        if self.empty {
            None
        } else {
            Some((self.min, self.max))
        }
    }

    pub fn center(&self) -> Option<Vec3> {
        self.corners().map(|(lo, hi)| lo.lerp(hi, 0.5))
    }

    /// Length of the diagonal; 0 for an empty box.
    pub fn diagonal(&self) -> f64 {
        self.corners().map_or(0.0, |(lo, hi)| lo.distance(hi))
    }

    /// True when `p` is inside the box grown by `margin` on every side.
    pub fn contains(&self, p: Vec3, margin: f64) -> bool {
        self.corners().is_some_and(|(lo, hi)| {
            p.x >= lo.x - margin
                && p.x <= hi.x + margin
                && p.y >= lo.y - margin
                && p.y <= hi.y + margin
                && p.z >= lo.z - margin
                && p.z <= hi.z + margin
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_of_axes_is_right_handed() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(Vec3::Y.cross(Vec3::Z), Vec3::X);
        assert_eq!(Vec3::Z.cross(Vec3::X), Vec3::Y);
    }

    #[test]
    fn length_and_dot() {
        let v = Vec3::new(3.0, 4.0, 12.0);
        assert_eq!(v.length(), 13.0);
        assert_eq!(v.dot(Vec3::new(1.0, 1.0, 1.0)), 19.0);
    }

    /// The angle between two vectors is read to rounding near 0 and π,
    /// where the cosine rounds to ±1: at turns of 1e-15 to 1e-6 (the dot 1
    /// in floating point below about 1.05e-8) it is the turn to a few units
    /// in the last place, the same either way round, the same for vectors
    /// of other lengths, and π less the turn for the opposite vector; a
    /// right angle is π/2.
    #[test]
    fn the_angle_between_vectors_is_read_to_rounding_near_0_and_pi() {
        for turn in [1e-15, 3e-12, 1e-9, 5e-9, 1e-8, 2.5e-8, 1e-6] {
            let (s, c) = (turn.psin(), turn.pcos());
            let u = Vec3::new(c, s, 0.0);
            assert!((Vec3::X.angle_to(u) - turn).abs() <= 8.0 * f64::EPSILON * turn);
            assert_eq!(Vec3::X.angle_to(u), u.angle_to(Vec3::X));
            let long = Vec3::new(1e6 * c, 1e6 * s, 0.0);
            assert!((Vec3::X.angle_to(long) - turn).abs() <= 8.0 * f64::EPSILON * turn);
            let opposite = Vec3::X.angle_to(-u);
            assert!((opposite - (std::f64::consts::PI - turn)).abs() <= 8.0 * f64::EPSILON);
            if turn < 1e-8 {
                assert_eq!(Vec3::X.dot(u), 1.0, "the cosine's reading: {turn}");
            }
        }
        let right = Vec3::new(0.0, 0.0, 2.0).angle_to(Vec3::new(3.0, 0.0, 0.0));
        assert!((right - std::f64::consts::FRAC_PI_2).abs() <= f64::EPSILON);
        // The copies it replaces, written out: the same bits.
        let (a, b) = (Vec3::new(0.3, -1.2, 2.5), Vec3::new(0.2, 0.3, 1.0));
        assert_eq!(
            a.angle_to(b).to_bits(),
            a.cross(b).length().patan2(a.dot(b)).to_bits()
        );
    }

    #[test]
    fn normalizing_degenerate_vectors_is_none() {
        assert!(Vec3::ZERO.normalized().is_none());
        assert!(Vec3::new(f64::NAN, 0.0, 0.0).normalized().is_none());
        assert!(Vec3::new(f64::INFINITY, 0.0, 0.0).normalized().is_none());
        let n = Vec3::new(0.0, 0.0, -2.5).normalized().unwrap();
        assert_eq!(n, Vec3::new(0.0, 0.0, -1.0));
    }

    /// J-018's fourth review: a vector whose squared length is below the
    /// normal range (3e-170, 4e-170: squares underflow) is still made a unit
    /// vector, (0.6, 0.8, 0) to rounding, never longer than 1 + 3e-16; one
    /// of 3e-163 and 4e-163 (squares subnormal, the plain square root a
    /// fifth off) likewise. Vectors in the normal range are as before, to
    /// the bit.
    #[test]
    fn tiny_vectors_normalise_to_unit_length() {
        for k in [1e-170, 1e-163, 1e-160, 1e-81] {
            let n = Vec3::new(3.0 * k, 4.0 * k, 0.0).normalized().unwrap();
            assert!(
                (n.x - 0.6).abs() < 1e-15 && (n.y - 0.8).abs() < 1e-15 && n.z == 0.0,
                "{k}: {n:?}"
            );
            assert!(n.length() <= 1.0 + 3e-16, "{k}: {}", n.length());
        }
        for v in [
            Vec3::new(3.0, 4.0, 12.0),
            Vec3::new(1e-150, 2e-151, -3e-150),
            Vec3::new(1e150, 1.0, 0.0),
        ] {
            assert_eq!(v.normalized(), Some(v * (1.0 / v.length())));
        }
    }

    #[test]
    fn bbox_grows_and_reports_empty() {
        let mut b = Bbox3::EMPTY;
        assert!(b.is_empty() && b.corners().is_none() && b.diagonal() == 0.0);
        b.add(Vec3::new(1.0, -2.0, 3.0));
        b.add(Vec3::new(-1.0, 2.0, 0.0));
        let (lo, hi) = b.corners().unwrap();
        assert_eq!(lo, Vec3::new(-1.0, -2.0, 0.0));
        assert_eq!(hi, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(b.center().unwrap(), Vec3::new(0.0, 0.0, 1.5));
        assert!(b.contains(Vec3::new(0.0, 0.0, 1.0), 0.0));
        assert!(!b.contains(Vec3::new(0.0, 0.0, 3.1), 0.0));
        assert!(b.contains(Vec3::new(0.0, 0.0, 3.1), 0.2));
    }
}
