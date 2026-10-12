//! TEMPORARY: stands in for `craft3d_kernel::revolve::revolve` and
//! `craft3d_kernel::boolean::boolean` until they are merged from their own
//! branches. Delete this file then.

pub(crate) fn boolean(
    _target: &craft3d_topo::Body,
    _tool: &craft3d_topo::Body,
    _op: craft3d_doc::BooleanOp,
    _tolerance: &craft3d_math::Tolerance,
) -> Result<(craft3d_topo::Body, Vec<(u64, usize)>), String> {
    Err("booleans are not built yet".to_string())
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
