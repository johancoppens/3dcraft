//! Small helpers several operations share.

use craft3d_geom::NurbsCurve;
use craft3d_geom::refine::interpolate;
use craft3d_math::Vec3;

/// `index` as an id, `u32::MAX` (no entity) past what an id holds.
pub(crate) fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// A polynomial cubic (or lower) through `points` at `params`.
pub(crate) fn curve_through(params: &[f64], points: &[Vec3]) -> Option<NurbsCurve> {
    let degree = params.len().checked_sub(1)?.min(3);
    let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
    let (knots, rows) = interpolate(params, degree, &values).ok()?;
    let ctrl: Vec<Vec3> = rows
        .iter()
        .map(|r| match r.as_slice() {
            [x, y, z] => Some(Vec3::new(*x, *y, *z)),
            _ => None,
        })
        .collect::<Option<_>>()?;
    NurbsCurve::polynomial(degree, knots, ctrl).ok()
}
