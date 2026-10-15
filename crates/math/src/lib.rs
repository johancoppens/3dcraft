//! Foundation maths for 3DCraft: `f64` vectors, bounding boxes, frames,
//! transforms, tolerances and quadrature.
//!
//! Kernel maths is `f64` throughout (AGENTS.md rule 3). Operations that can
//! fail on degenerate input (normalising a zero vector) return `Option`.

mod bits;
mod frame;
pub mod quadrature;
mod tolerance;
mod transform;

pub use bits::{Bits, bits_of};
pub use frame::Frame;
pub use tolerance::Tolerance;
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

    /// Unit vector in the same direction; `None` for a zero-length or
    /// non-finite vector.
    pub fn normalized(self) -> Option<Vec3> {
        let len = self.length();
        if len.is_finite() && len > 0.0 {
            Some(self * (1.0 / len))
        } else {
            None
        }
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

    #[test]
    fn normalizing_degenerate_vectors_is_none() {
        assert!(Vec3::ZERO.normalized().is_none());
        assert!(Vec3::new(f64::NAN, 0.0, 0.0).normalized().is_none());
        assert!(Vec3::new(f64::INFINITY, 0.0, 0.0).normalized().is_none());
        let n = Vec3::new(0.0, 0.0, -2.5).normalized().unwrap();
        assert_eq!(n, Vec3::new(0.0, 0.0, -1.0));
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
