//! Revolutions in regeneration: the profiles of a solved sketch are turned
//! about a line in the sketch's plane.

use crate::extrude::{Swept, swap_start_and_end, swept};
use crate::{FeatureResult, angle};
use craft3d_doc::{Document, Geometry, Revolve, RevolveAxis};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::Origins;
use craft3d_topo::Body;
use std::f64::consts::TAU;

use craft3d_kernel::revolve::revolve;

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: &Revolve,
    earlier: &[FeatureResult],
) -> Result<(Body, Origins), String> {
    let Swept {
        name,
        sketch,
        solved,
        profiles,
        plane,
        tolerance,
    } = swept(doc, earlier, feature.sketch, "revolve")?;

    // The axis, in the sketch's own coordinates: a point and a direction.
    let (point, direction) = match feature.axis {
        RevolveAxis::X => ([0.0, 0.0], [1.0, 0.0]),
        RevolveAxis::Y => ([0.0, 0.0], [0.0, 1.0]),
        RevolveAxis::Line { entity } => {
            let line = sketch.entities.iter().find(|e| e.id == entity);
            let Some(&Geometry::Line { start, end }) = line.map(|e| &e.geometry) else {
                return Err(match line {
                    None => format!("axis: sketch `{name}` has no entity {entity}"),
                    Some(_) => format!(
                        "axis: {entity} of sketch `{name}` is not a line; the axis is a line of the sketch, or x or y"
                    ),
                });
            };
            let (Some(a), Some(b)) = (solved.point(start), solved.point(end)) else {
                return Err(format!(
                    "axis: {entity} of sketch `{name}` has no solved position"
                ));
            };
            let direction = [b.0 - a.0, b.1 - a.1];
            if direction[0].hypot(direction[1]) <= tolerance.linear {
                return Err(format!(
                    "axis: {entity} of sketch `{name}` has no length, so it gives no direction"
                ));
            }
            ([a.0, a.1], direction)
        }
        RevolveAxis::Datum { axis } => {
            let line = crate::datum::datum_axis(doc, earlier, axis, "axis")?;
            let made_by = doc.feature(axis).map_or("", |f| f.name.as_str());
            // A revolution turns about a line in the plane of its sketch.
            let (x, y, off) = plane.local(line.origin);
            let (dx, dy, lean) = (
                line.direction.dot(plane.x()),
                line.direction.dot(plane.y()),
                line.direction.dot(plane.z()),
            );
            let lean = lean.clamp(-1.0, 1.0).asin();
            if lean.is_nan() || lean.abs() > tolerance.angular {
                return Err(format!(
                    "axis: `{made_by}` does not lie in the plane of sketch `{name}` (it makes {} deg with it); a revolution turns about a line in the sketch's plane",
                    lean.to_degrees()
                ));
            }
            if off.is_nan() || off.abs() > tolerance.linear {
                return Err(format!(
                    "axis: `{made_by}` does not lie in the plane of sketch `{name}` (it is {} beside it); a revolution turns about a line in the sketch's plane",
                    off.abs()
                ));
            }
            ([x, y], [dx, dy])
        }
    };

    let turn = angle(doc, variables, "angle", &feature.angle)?;
    if !turn.is_finite() || turn.abs() <= tolerance.angular {
        return Err(format!(
            "angle must be an angle other than zero, but `{}` is {} deg",
            feature.angle.source(),
            turn.to_degrees()
        ));
    }
    // A hair over a full turn is a full turn; more is a mistake.
    let size = turn.abs();
    if size > TAU * (1.0 + 1e-9) {
        return Err(format!(
            "angle is at most a full turn (360 deg), but `{}` is {} deg",
            feature.angle.source(),
            turn.to_degrees()
        ));
    }
    let size = size.min(TAU);
    let full = TAU - size <= tolerance.angular;
    let (from, to) = if full {
        (0.0, TAU)
    } else if feature.symmetric {
        (-size / 2.0, size / 2.0)
    } else if turn > 0.0 {
        (0.0, size)
    } else {
        (-size, 0.0)
    };
    // (A spline that ends on the axis sweeps a face whose surface closes
    // into a point there, a pole: measured and cut like any other since
    // 2026-10-14.)
    let mut result = revolve(&profiles, &plane, point, direction, from, to, &tolerance)
        .map_err(|e| match e {
            // Curves are called what the sketch calls them.
            KernelError::CrossesAxis { curves } => {
                let list: Vec<String> = curves
                    .iter()
                    .map(|id| solved.curve_name(*id))
                    .collect();
                format!(
                    "axis: sketch `{name}` lies on both sides of the axis ({} reach across); a profile may touch the axis but not cross it",
                    list.join(", ")
                )
            }
            KernelError::UnsupportedCurve { id, reason } => {
                format!(
                    "sketch `{name}`: {} cannot be revolved about this axis: {reason}",
                    solved.curve_name(id)
                )
            }
            other => other.to_string(),
        })?;
    // "Start" is the cap in the sketch plane, as for an extrusion.
    if !full && !feature.symmetric && turn < 0.0 {
        swap_start_and_end(&mut result.origins);
    }
    Ok((result.body, result.origins))
}
