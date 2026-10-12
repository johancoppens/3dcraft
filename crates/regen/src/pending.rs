//! TEMPORARY: stands in for `craft3d_kernel::revolve::revolve` until it is
//! merged from its own branch. Delete this file then.

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
