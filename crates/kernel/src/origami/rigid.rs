//! Rigid motions of the sheet's plane coordinates: `x ↦ M x + t`, with
//! `M` a rotation.

use craft3d_math::Vec3;

/// A rigid motion in the coordinates of the sheet's plane (x and y in the
/// plane, z along its normal).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rigid {
    /// The rows of the rotation.
    pub rows: [Vec3; 3],
    pub translation: Vec3,
}

impl Rigid {
    pub const IDENTITY: Rigid = Rigid {
        rows: [Vec3::X, Vec3::Y, Vec3::Z],
        translation: Vec3::ZERO,
    };

    /// The rotation by `angle` about the line through `p` along the unit
    /// vector `d`, counter-clockwise seen from the tip of `d` (Rodrigues).
    pub fn about(p: Vec3, d: Vec3, angle: f64) -> Rigid {
        let (s, c) = angle.sin_cos();
        let v = 1.0 - c;
        let rows = [
            Vec3::new(
                c + d.x * d.x * v,
                d.x * d.y * v - d.z * s,
                d.x * d.z * v + d.y * s,
            ),
            Vec3::new(
                d.y * d.x * v + d.z * s,
                c + d.y * d.y * v,
                d.y * d.z * v - d.x * s,
            ),
            Vec3::new(
                d.z * d.x * v - d.y * s,
                d.z * d.y * v + d.x * s,
                c + d.z * d.z * v,
            ),
        ];
        let m = Rigid {
            rows,
            translation: Vec3::ZERO,
        };
        Rigid {
            rows,
            translation: p - m.vector(p),
        }
    }

    pub fn vector(&self, v: Vec3) -> Vec3 {
        let [a, b, c] = self.rows;
        Vec3::new(a.dot(v), b.dot(v), c.dot(v))
    }

    pub fn point(&self, p: Vec3) -> Vec3 {
        self.vector(p) + self.translation
    }

    /// `self` after `first`: the motion that applies `first`, then `self`.
    pub fn after(&self, first: &Rigid) -> Rigid {
        let col = |k: usize| {
            let [a, b, c] = first.rows;
            match k {
                0 => Vec3::new(a.x, b.x, c.x),
                1 => Vec3::new(a.y, b.y, c.y),
                _ => Vec3::new(a.z, b.z, c.z),
            }
        };
        let row = |r: Vec3| Vec3::new(r.dot(col(0)), r.dot(col(1)), r.dot(col(2)));
        let [a, b, c] = self.rows;
        Rigid {
            rows: [row(a), row(b), row(c)],
            translation: self.point(first.translation),
        }
    }

    /// The inverse motion.
    pub fn inverse(&self) -> Rigid {
        let [a, b, c] = self.rows;
        let rows = [
            Vec3::new(a.x, b.x, c.x),
            Vec3::new(a.y, b.y, c.y),
            Vec3::new(a.z, b.z, c.z),
        ];
        let m = Rigid {
            rows,
            translation: Vec3::ZERO,
        };
        Rigid {
            rows,
            translation: -m.vector(self.translation),
        }
    }

    pub fn is_finite(&self) -> bool {
        self.rows.iter().all(|r| r.is_finite()) && self.translation.is_finite()
    }
}

pub(crate) fn lift(p: [f64; 2]) -> Vec3 {
    Vec3::new(p[0], p[1], 0.0)
}
