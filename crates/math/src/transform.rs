//! Affine transforms of model space.

use crate::portable::Portable;
use crate::{Frame, Vec3};

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
        let (s, c) = angle.psin_cos();
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

    /// Reflection in the plane through `point` with normal `normal`.
    /// `None` for a zero or non-finite normal, or a non-finite point.
    pub fn mirror(point: Vec3, normal: Vec3) -> Option<Transform> {
        let n = normal.normalized()?;
        if !point.is_finite() {
            return None;
        }
        // Householder: v ↦ v − 2 n (n·v).
        let row = |axis: Vec3, component: f64| axis - n * (2.0 * component);
        let linear = Transform {
            rows: [row(Vec3::X, n.x), row(Vec3::Y, n.y), row(Vec3::Z, n.z)],
            translation: Vec3::ZERO,
        };
        linear.about(point)
    }

    /// The rigid motion that takes the world frame to `frame`: local
    /// coordinates `(a, b, c)` of the frame to the point
    /// `origin + a x + b y + c z`. Its inverse takes points into the
    /// frame's coordinates, so `from_frame(f).inverse().then(&from_frame(g))`
    /// carries `f` onto `g`.
    pub fn from_frame(frame: &Frame) -> Transform {
        let (x, y, z) = (frame.x(), frame.y(), frame.z());
        Transform {
            rows: [
                Vec3::new(x.x, y.x, z.x),
                Vec3::new(x.y, y.y, z.y),
                Vec3::new(x.z, y.z, z.z),
            ],
            translation: frame.origin(),
        }
    }

    /// Rotation by `angle` radians about the line through `point` along
    /// `axis`, counter-clockwise seen from the axis tip. `None` for a zero
    /// axis or anything not finite.
    pub fn rotation_about(point: Vec3, axis: Vec3, angle: f64) -> Option<Transform> {
        if !point.is_finite() {
            return None;
        }
        Transform::rotation(axis, angle)?.about(point)
    }

    /// The linear part of `self` applied about `point` instead of the
    /// origin: `p ↦ point + M (p − point)`.
    fn about(&self, point: Vec3) -> Option<Transform> {
        let translation = point - self.vector(point);
        translation.is_finite().then_some(Transform {
            rows: self.rows,
            translation,
        })
    }

    /// True when the transform preserves lengths within `epsilon`
    /// (relative): the linear part is orthonormal (every entry of
    /// `M Mᵀ − I` is at most `epsilon` in size, so the determinant is +1 or
    /// −1 within about `3 epsilon`), and everything is finite. A rigid
    /// motion has determinant +1; −1 means the transform also mirrors.
    /// False for a negative or non-finite `epsilon`.
    pub fn is_isometry(&self, epsilon: f64) -> bool {
        // Written so that NaN anywhere fails the test.
        if !(epsilon >= 0.0 && epsilon.is_finite() && self.translation.is_finite()) {
            return false;
        }
        let [a, b, c] = self.rows;
        [a.dot(a) - 1.0, b.dot(b) - 1.0, c.dot(c) - 1.0]
            .into_iter()
            .chain([a.dot(b), b.dot(c), c.dot(a)])
            .all(|deviation| deviation.abs() <= epsilon)
    }

    /// The image of a frame under an isometry, as a right-handed frame.
    ///
    /// For a rigid motion all three axes are the images of the frame's
    /// axes. A mirror would turn the frame left-handed, so there the x and
    /// y axes are the images and the z axis is the *opposite* of the image
    /// of z: local coordinates `(a, b, c)` of the frame map to
    /// `(a, b, −c)` of the result.
    ///
    /// The axes are re-orthonormalised (z first, then x), so the result
    /// depends only on the frame's axes and the transform: frames with
    /// bit-identical axes get bit-identical axes. `None` when the image is
    /// degenerate or not finite; meaningless for a transform that is not
    /// an isometry.
    pub fn frame(&self, frame: &Frame) -> Option<Frame> {
        let z = self.vector(frame.z());
        let z = if self.determinant() < 0.0 { -z } else { z };
        Frame::new(self.point(frame.origin()), z, self.vector(frame.x()))
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
    use crate::portable::Portable;
    use std::f64::consts::FRAC_PI_2;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-13
    }

    #[test]
    fn a_frame_as_a_transform_carries_one_frame_onto_another() {
        let f = Frame::new(
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap();
        let g = Frame::new(Vec3::new(-5.0, 0.5, 9.0), Vec3::Y, Vec3::Z).unwrap();
        let t = Transform::from_frame(&f);
        assert!(close(
            t.point(Vec3::new(1.0, 2.0, 3.0)),
            f.point(1.0, 2.0, 3.0)
        ));
        assert!(t.is_isometry(1e-12) && (t.determinant() - 1.0).abs() < 1e-12);
        let carry = t.inverse().unwrap().then(&Transform::from_frame(&g));
        let p = f.point(0.7, -1.1, 2.5);
        assert!(close(carry.point(p), g.point(0.7, -1.1, 2.5)));
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
        assert!((perp(v).dot(perp(r.vector(v))).pacos() - 1.234).abs() < 1e-13);
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
        assert!((m.determinant() - 0.25_f64.ppowi(3)).abs() < 1e-15);
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

    #[test]
    fn mirror_reflects_in_its_plane() {
        let (point, normal) = (Vec3::new(1.0, -2.0, 3.0), Vec3::new(2.0, 1.0, -0.5));
        let m = Transform::mirror(point, normal).unwrap();
        assert!((m.determinant() + 1.0).abs() < 1e-15);
        assert!(m.is_isometry(1e-15));
        let n = normal.normalized().unwrap();
        // Points of the plane stay; the normal direction is negated.
        let in_plane = point + n.cross(Vec3::X) * 3.0 + n.cross(Vec3::Y) * -2.0;
        assert!(close(m.point(in_plane), in_plane));
        assert!(close(m.point(point + n * 2.5), point - n * 2.5));
        assert!(close(m.vector(n), -n));
        // An involution.
        let p = Vec3::new(0.3, 9.0, -4.0);
        assert!(close(m.point(m.point(p)), p));
        assert!(close(m.then(&m).point(p), p));
        // The plain coordinate mirrors are exact.
        let mx = Transform::mirror(Vec3::ZERO, Vec3::X).unwrap();
        assert_eq!(
            mx.point(Vec3::new(1.0, 2.0, 3.0)),
            Vec3::new(-1.0, 2.0, 3.0)
        );
        assert!(Transform::mirror(point, Vec3::ZERO).is_none());
        assert!(Transform::mirror(point, Vec3::new(f64::NAN, 0.0, 1.0)).is_none());
        assert!(Transform::mirror(Vec3::new(f64::INFINITY, 0.0, 0.0), Vec3::X).is_none());
    }

    #[test]
    fn rotation_about_a_line_keeps_the_line() {
        let (point, axis) = (Vec3::new(4.0, 0.0, -1.0), Vec3::new(0.0, 0.0, 2.0));
        let r = Transform::rotation_about(point, axis, FRAC_PI_2).unwrap();
        assert!(close(r.point(point), point));
        assert!(close(r.point(point + axis * 3.0), point + axis * 3.0));
        // (5, 0, 0) is one unit along +x from the line: it goes to +y.
        assert!(close(
            r.point(Vec3::new(5.0, 0.0, 0.0)),
            Vec3::new(4.0, 1.0, 0.0)
        ));
        assert!((r.determinant() - 1.0).abs() < 1e-15);
        // Angles about one line add up.
        let (a, b) = (0.7, -1.9);
        let ab = Transform::rotation_about(point, axis, a)
            .unwrap()
            .then(&Transform::rotation_about(point, axis, b).unwrap());
        let sum = Transform::rotation_about(point, axis, a + b).unwrap();
        let p = Vec3::new(1.5, 2.5, -3.5);
        assert!(close(ab.point(p), sum.point(p)));
        assert!(Transform::rotation_about(point, Vec3::ZERO, 1.0).is_none());
        assert!(Transform::rotation_about(point, axis, f64::INFINITY).is_none());
        assert!(Transform::rotation_about(Vec3::new(f64::NAN, 0.0, 0.0), axis, 1.0).is_none());
    }

    #[test]
    fn isometries_are_recognised() {
        let rigid = Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.9)
            .unwrap()
            .then(&Transform::translation(Vec3::new(1e6, -3.0, 0.5)));
        assert!(rigid.is_isometry(1e-14));
        assert!(Transform::IDENTITY.is_isometry(0.0));
        let mirrored = rigid.then(&Transform::mirror(Vec3::X, Vec3::new(1.0, 1.0, 0.0)).unwrap());
        assert!(mirrored.is_isometry(1e-14) && mirrored.determinant() < 0.0);
        assert!(!Transform::scaling(1.001).is_isometry(1e-6));
        assert!(Transform::scaling(1.0 + 1e-9).is_isometry(1e-6));
        assert!(
            !Transform::scaling(-1.0)
                .then(&Transform::scaling(2.0))
                .is_isometry(1e-6)
        );
        assert!(Transform::scaling(-1.0).is_isometry(0.0), "a point mirror");
        // A shear has determinant 1 and still changes lengths.
        let shear = Transform {
            rows: [Vec3::new(1.0, 0.5, 0.0), Vec3::Y, Vec3::Z],
            translation: Vec3::ZERO,
        };
        assert!((shear.determinant() - 1.0).abs() < 1e-15);
        assert!(!shear.is_isometry(1e-6));
        assert!(!Transform::scaling(0.0).is_isometry(1e-6));
        for bad in [f64::NAN, f64::INFINITY] {
            assert!(!Transform::scaling(bad).is_isometry(1e-6));
            assert!(!Transform::translation(Vec3::new(0.0, bad, 0.0)).is_isometry(1e-6));
            assert!(!rigid.is_isometry(bad));
        }
        assert!(!rigid.is_isometry(-1.0));
    }

    #[test]
    fn frames_stay_right_handed_and_a_mirror_negates_local_z() {
        let frame = Frame::new(
            Vec3::new(1.0, -2.0, 0.5),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap();
        let rigid =
            Transform::rotation_about(Vec3::new(3.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 1.0), 2.1)
                .unwrap();
        let mirror =
            Transform::mirror(Vec3::new(0.0, 5.0, 0.0), Vec3::new(1.0, -3.0, 2.0)).unwrap();
        for (t, sign) in [(rigid, 1.0), (mirror, -1.0), (rigid.then(&mirror), -1.0)] {
            let image = t.frame(&frame).unwrap();
            assert!(image.x().cross(image.y()).distance(image.z()) < 1e-15);
            for (a, b, c) in [(1.0, 0.0, 0.0), (0.5, -2.0, 0.0), (0.3, 0.7, -1.1)] {
                assert!(close(
                    t.point(frame.point(a, b, c)),
                    image.point(a, b, sign * c)
                ));
            }
            // Only the axes decide the axes: another origin, the same bits.
            let moved = frame.at(Vec3::new(-7.0, 0.1, 22.0)).unwrap();
            let moved_image = t.frame(&moved).unwrap();
            assert_eq!(
                (moved_image.x(), moved_image.y(), moved_image.z()),
                (image.x(), image.y(), image.z())
            );
        }
        assert!(Transform::scaling(f64::NAN).frame(&frame).is_none());
        assert!(Transform::scaling(0.0).frame(&frame).is_none());
    }
}
