//! Revolutions in regeneration: the profiles of a solved sketch are turned
//! about a line in the sketch's plane.

use crate::extrude::{Swept, swap_start_and_end, swept};
use crate::{FeatureResult, angle};
use craft3d_doc::{Document, Geometry, Revolve, RevolveAxis};
use craft3d_expr::Evaluation;
use craft3d_kernel::extrude::Origins;
use craft3d_topo::Body;
use std::f64::consts::TAU;

// TEMPORARY, see pending.rs.
use crate::pending::revolve;

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
            let Some(Geometry::Line { start, end }) = line.map(|e| e.geometry) else {
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
    let mut result = revolve(&profiles, &plane, point, direction, from, to, &tolerance)
        .map_err(|e| e.to_string())?;
    // "Start" is the cap in the sketch plane, as for an extrusion.
    if !full && !feature.symmetric && turn < 0.0 {
        swap_start_and_end(&mut result.origins);
    }
    Ok((result.body, result.origins))
}
