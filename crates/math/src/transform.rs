//! Affine transforms of model space.

use crate::Vec3;

/// `p ↦ M p + t`, with `M` stored as its three rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    rows: [Vec3; 3],
    translation: Vec3,
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        rows: [Vec3::X, Vec3::Y, Vec3::Z],
        translation: Vec3::ZERO,
    };

    pub fn translation(t: Vec3) -> Transform {
        Transform {
            translation: t,
            ..Transform::IDENTITY
        }
    }

    /// Uniform scaling about the origin.
    pub fn scaling(factor: f64) -> Transform {
        Transform {
            rows: [Vec3::X * factor, Vec3::Y * factor, Vec3::Z * factor],
            translation: Vec3::ZERO,
        }
    }

    /// Rotation by `angle` radians about the axis through the origin along
    /// `axis`, counter-clockwise seen from the axis tip (Rodrigues' formula).
    /// `None` for a zero axis or a non-finite angle.
    pub fn rotation(axis: Vec3, angle: f64) -> Option<Transform> {
        let k = axis.normalized()?;
        if !angle.is_finite() {
            return None;
        }
        let (s, c) = angle.sin_cos();
        let v = 1.0 - c;
        Some(Transform {
            rows: [
                Vec3::new(
                    c + k.x * k.x * v,
                    k.x * k.y * v - k.z * s,
                    k.x * k.z * v + k.y * s,
                ),
                Vec3::new(
                    k.y * k.x * v + k.z * s,
                    c + k.y * k.y * v,
                    k.y * k.z * v - k.x * s,
                ),
                Vec3::new(
                    k.z * k.x * v - k.y * s,
                    k.z * k.y * v + k.x * s,
                    c + k.z * k.z * v,
                ),
            ],
            translation: Vec3::ZERO,
        })
    }

    /// Maps a position.
    pub fn point(&self, p: Vec3) -> Vec3 {
        self.vector(p) + self.translation
    }

    /// Maps a direction or displacement (no translation).
    pub fn vector(&self, v: Vec3) -> Vec3 {
        let [a, b, c] = self.rows;
        Vec3::new(a.dot(v), b.dot(v), c.dot(v))
    }

    fn columns(&self) -> [Vec3; 3] {
        let [a, b, c] = self.rows;
        [
            Vec3::new(a.x, b.x, c.x),
            Vec3::new(a.y, b.y, c.y),
            Vec3::new(a.z, b.z, c.z),
        ]
    }

    /// The transform that applies `self` first and `next` after it.
    pub fn then(&self, next: &Transform) -> Transform {
        let [cx, cy, cz] = self.columns();
        let row = |r: Vec3| Vec3::new(r.dot(cx), r.dot(cy), r.dot(cz));
        let [a, b, c] = next.rows;
        Transform {
            rows: [row(a), row(b), row(c)],
            translation: next.point(self.translation),
        }
    }

    /// Volume scale factor; negative for a mirror.
    pub fn determinant(&self) -> f64 {
        let [a, b, c] = self.rows;
        a.dot(b.cross(c))
    }

    /// `None` when the transform flattens space (determinant zero or not
    /// finite).
    pub fn inverse(&self) -> Option<Transform> {
        let det = self.determinant();
        if !det.is_finite() || det == 0.0 {
            return None;
        }
        let [a, b, c] = self.rows;
        // Columns of the inverse are the cross products of the rows over det.
        let (i, j, k) = (
            b.cross(c) * (1.0 / det),
            c.cross(a) * (1.0 / det),
            a.cross(b) * (1.0 / det),
        );
        let rows = [
            Vec3::new(i.x, j.x, k.x),
            Vec3::new(i.y, j.y, k.y),
            Vec3::new(i.z, j.z, k.z),
        ];
        let linear = Transform {
            rows,
            translation: Vec3::ZERO,
        };
        let translation = -linear.vector(self.translation);
        (translation.is_finite() && rows.iter().all(|r| r.is_finite()))
            .then_some(Transform { rows, translation })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-13
    }

    #[test]
    fn quarter_turns_map_the_axes() {
        let rz = Transform::rotation(Vec3::Z * 7.0, FRAC_PI_2).unwrap();
        assert!(close(rz.vector(Vec3::X), Vec3::Y));
        assert!(close(rz.vector(Vec3::Y), -Vec3::X));
        assert!(close(rz.vector(Vec3::Z), Vec3::Z));
        let rx = Transform::rotation(Vec3::X, FRAC_PI_2).unwrap();
        assert!(close(rx.vector(Vec3::Y), Vec3::Z));
        assert!((rz.determinant() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn rotation_keeps_the_axis_and_all_lengths() {
        let axis = Vec3::new(1.0, -2.0, 0.5);
        let r = Transform::rotation(axis, 1.234).unwrap();
        assert!(close(r.vector(axis), axis));
        let v = Vec3::new(0.3, 0.7, -1.9);
        assert!((r.vector(v).length() - v.length()).abs() < 1e-14);
        // Angle between v's and its image's components perpendicular to the axis.
        let k = axis.normalized().unwrap();
        let perp = |w: Vec3| (w - k * w.dot(k)).normalized().unwrap();
        assert!((perp(v).dot(perp(r.vector(v))).acos() - 1.234).abs() < 1e-13);
    }

    #[test]
    fn composition_applies_left_then_right() {
        let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
        let r = Transform::rotation(Vec3::Z, FRAC_PI_2).unwrap();
        // Translate, then rotate: (0,0,0) -> (1,0,0) -> (0,1,0).
        assert!(close(t.then(&r).point(Vec3::ZERO), Vec3::Y));
        // Rotate, then translate: (0,0,0) -> (0,0,0) -> (1,0,0).
        assert!(close(r.then(&t).point(Vec3::ZERO), Vec3::X));
        let p = Vec3::new(0.2, -4.0, 9.0);
        let s = Transform::scaling(2.5);
        assert!(close(
            t.then(&r).then(&s).point(p),
            s.point(r.point(t.point(p)))
        ));
    }

    #[test]
    fn inverse_undoes_the_transform() {
        let m = Transform::translation(Vec3::new(3.0, -1.0, 2.0))
            .then(&Transform::rotation(Vec3::new(1.0, 1.0, 1.0), 0.8).unwrap())
            .then(&Transform::scaling(0.25))
            .then(&Transform::translation(Vec3::new(-7.0, 0.5, 0.0)));
        let inv = m.inverse().unwrap();
        let p = Vec3::new(1.5, 2.5, -3.5);
        assert!(close(inv.point(m.point(p)), p));
        assert!(close(m.point(inv.point(p)), p));
        assert!((m.determinant() - 0.25_f64.powi(3)).abs() < 1e-15);
        assert_eq!(Transform::IDENTITY.inverse(), Some(Transform::IDENTITY));
    }

    #[test]
    fn degenerate_transforms_are_rejected() {
        assert!(Transform::rotation(Vec3::ZERO, 1.0).is_none());
        assert!(Transform::rotation(Vec3::Z, f64::NAN).is_none());
        assert!(Transform::scaling(0.0).inverse().is_none());
        assert!(Transform::scaling(f64::INFINITY).inverse().is_none());
        assert!(
            Transform::translation(Vec3::new(f64::NAN, 0.0, 0.0))
                .inverse()
                .is_none()
        );
    }
}
