//! TEMPORARY: stands in for `Body::transformed`, `craft3d_topo::merge`,
//! `Transform::mirror` and `Transform::rotation_about` until they are
//! merged from their own branch. Delete this file then.

use craft3d_math::{Transform, Vec3};
use craft3d_topo::Body;

pub(crate) trait BodyExt {
    fn transformed(&self, t: &Transform) -> Result<Body, String>;
}

impl BodyExt for Body {
    fn transformed(&self, t: &Transform) -> Result<Body, String> {
        if *t == Transform::IDENTITY {
            Ok(self.clone())
        } else {
            Err("moving a body is not built yet".to_string())
        }
    }
}

pub(crate) fn merge(_bodies: &[&Body]) -> Result<(Body, ()), String> {
    Err("joining bodies is not built yet".to_string())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn revolve(
    _profiles: &[craft3d_kernel::profile::Profile],
    _plane: &craft3d_math::Frame,
    _axis_point: [f64; 2],
    _axis_direction: [f64; 2],
    _from: f64,
    _to: f64,
    _tolerance: &craft3d_math::Tolerance,
) -> Result<craft3d_kernel::extrude::OpResult, String> {
    Err("revolving is not built yet".to_string())
}

pub(crate) trait TransformExt: Sized {
    fn mirror(point: Vec3, normal: Vec3) -> Option<Self>;
    fn rotation_about(point: Vec3, axis: Vec3, angle: f64) -> Option<Self>;
}

impl TransformExt for Transform {
    fn mirror(_point: Vec3, _normal: Vec3) -> Option<Transform> {
        None
    }

    fn rotation_about(point: Vec3, axis: Vec3, angle: f64) -> Option<Transform> {
        Some(
            Transform::translation(point * -1.0)
                .then(&Transform::rotation(axis, angle)?)
                .then(&Transform::translation(point)),
        )
    }
}
