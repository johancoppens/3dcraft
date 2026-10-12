//! Right-handed orthonormal coordinate frames: the placement of analytic
//! curves and surfaces.

use crate::Vec3;

/// An origin and three mutually perpendicular unit axes with `x × y = z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    origin: Vec3,
    x: Vec3,
    y: Vec3,
    z: Vec3,
}

impl Frame {
    /// The world frame.
    pub const WORLD: Frame = Frame {
        origin: Vec3::ZERO,
        x: Vec3::X,
        y: Vec3::Y,
        z: Vec3::Z,
    };

    /// A frame at `origin` whose z axis is along `z` and whose x axis is
    /// `x_hint` made perpendicular to it. `None` when `z` is zero or
    /// `x_hint` is parallel to `z`, or anything is not finite.
    pub fn new(origin: Vec3, z: Vec3, x_hint: Vec3) -> Option<Frame> {
        let z = z.normalized()?;
        let x = (x_hint - z * x_hint.dot(z)).normalized()?;
        // Reject hints so close to z that x is rounding noise.
        if x_hint.cross(z).length() <= 1e-12 * x_hint.length() {
            return None;
        }
        origin.is_finite().then_some(Frame {
            origin,
            x,
            y: z.cross(x),
            z,
        })
    }

    /// A frame at `origin` with the given z axis and an x axis chosen
    /// deterministically: the world axis least aligned with z, made
    /// perpendicular to it.
    pub fn from_z(origin: Vec3, z: Vec3) -> Option<Frame> {
        let unit = z.normalized()?;
        let hint = if unit.x.abs() <= unit.y.abs() && unit.x.abs() <= unit.z.abs() {
            Vec3::X
        } else if unit.y.abs() <= unit.z.abs() {
            Vec3::Y
        } else {
            Vec3::Z
        };
        Frame::new(origin, z, hint)
    }

    /// The same axes, bit for bit, at another origin. `None` when the
    /// origin is not finite. (Rebuilding a frame with [`Frame::new`] from
    /// its own axes re-orthonormalises them and may change the last bit,
    /// which is enough to make two parallel circles disagree about where
    /// their angle 0 is.)
    pub fn at(&self, origin: Vec3) -> Option<Frame> {
        origin.is_finite().then_some(Frame { origin, ..*self })
    }

    /// The frame seen from the other side: the same x axis, y and z
    /// negated, at `origin`. Still right-handed.
    pub fn reversed_at(&self, origin: Vec3) -> Option<Frame> {
        origin.is_finite().then_some(Frame {
            origin,
            x: self.x,
            y: -self.y,
            z: -self.z,
        })
    }

    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    pub fn x(&self) -> Vec3 {
        self.x
    }

    pub fn y(&self) -> Vec3 {
        self.y
    }

    pub fn z(&self) -> Vec3 {
        self.z
    }

    /// The world position of local coordinates `(a, b, c)`.
    pub fn point(&self, a: f64, b: f64, c: f64) -> Vec3 {
        self.origin + self.vector(a, b, c)
    }

    /// The world direction of local components `(a, b, c)`.
    pub fn vector(&self, a: f64, b: f64, c: f64) -> Vec3 {
        self.x * a + self.y * b + self.z * c
    }

    /// The local coordinates of world position `p`.
    pub fn local(&self, p: Vec3) -> (f64, f64, f64) {
        let d = p - self.origin;
        (d.dot(self.x), d.dot(self.y), d.dot(self.z))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_are_orthonormal_and_right_handed() {
        for (z, hint) in [
            (Vec3::new(1.0, 2.0, 3.0), Vec3::new(-2.0, 0.5, 0.1)),
            (Vec3::Z, Vec3::new(5.0, 5.0, 5.0)),
            (Vec3::new(0.0, -1e-8, 0.0), Vec3::X * 1e8),
        ] {
            let f = Frame::new(Vec3::new(1.0, -1.0, 2.0), z, hint).unwrap();
            for (a, b) in [(f.x(), f.y()), (f.y(), f.z()), (f.z(), f.x())] {
                assert!(a.dot(b).abs() < 1e-15);
            }
            for axis in [f.x(), f.y(), f.z()] {
                assert!((axis.length() - 1.0).abs() < 1e-15);
            }
            assert!(f.x().cross(f.y()).distance(f.z()) < 1e-15);
            assert!(f.z().distance(z.normalized().unwrap()) < 1e-15);
            assert!(f.x().dot(hint) > 0.0);
        }
    }

    #[test]
    fn local_and_world_coordinates_round_trip() {
        let f = Frame::new(Vec3::new(3.0, 4.0, 5.0), Vec3::new(1.0, 1.0, 0.0), Vec3::Z).unwrap();
        let p = f.point(0.5, -2.0, 7.0);
        let (a, b, c) = f.local(p);
        assert!((a - 0.5).abs() < 1e-14 && (b + 2.0).abs() < 1e-14 && (c - 7.0).abs() < 1e-14);
        assert_eq!(Frame::WORLD.point(1.0, 2.0, 3.0), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn moving_and_reversing_keep_the_axes_exactly() {
        let f = Frame::from_z(Vec3::new(0.0, 0.0, 20.0), Vec3::new(1.0, 0.0, 0.4)).unwrap();
        let moved = f.at(f.point(0.0, 0.0, 40.0)).unwrap();
        assert_eq!((moved.x(), moved.y(), moved.z()), (f.x(), f.y(), f.z()));
        assert_eq!(moved.origin(), f.point(0.0, 0.0, 40.0));
        let back = f.reversed_at(f.origin()).unwrap();
        assert_eq!((back.x(), back.y(), back.z()), (f.x(), -f.y(), -f.z()));
        assert!(
            back.x().cross(back.y()).distance(back.z()) < 1e-15,
            "still right-handed"
        );
        assert!(f.at(Vec3::new(f64::NAN, 0.0, 0.0)).is_none());
        assert!(f.reversed_at(Vec3::new(0.0, f64::INFINITY, 0.0)).is_none());
    }

    #[test]
    fn degenerate_input_gives_no_frame() {
        assert!(Frame::new(Vec3::ZERO, Vec3::ZERO, Vec3::X).is_none());
        assert!(Frame::new(Vec3::ZERO, Vec3::Z, Vec3::Z * -3.0).is_none());
        assert!(Frame::new(Vec3::ZERO, Vec3::Z, Vec3::new(1e-14, 0.0, 1.0)).is_none());
        assert!(Frame::new(Vec3::new(f64::NAN, 0.0, 0.0), Vec3::Z, Vec3::X).is_none());
        assert!(Frame::from_z(Vec3::ZERO, Vec3::ZERO).is_none());
        for z in [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(-0.2, 9.0, 0.3),
        ] {
            assert!(Frame::from_z(Vec3::ZERO, z).is_some());
        }
    }
}
