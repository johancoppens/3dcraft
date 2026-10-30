//! What an assembly's solve says, measured (docs/design/assemblies.md ›
//! *Measures*): the value of a mate's free motion and an instance's pose,
//! as the commands `measure.joint` and `measure.pose`, and as the measures
//! `jointAngle`, `jointDistance`, `position` and `rotation` that
//! assertions and outputs (`variable.measure`) take, so the solved values
//! of free motions are readable in formulas through outputs.
//!
//! Angles are radians and lengths the assembly's length unit, as the
//! solve has them. A mate that takes no part, an instance that is not
//! placed or a solve that failed is an error that says why, never a value
//! from somewhere else.

use crate::asm::{self, InstanceState, MateState, Placed};
use crate::commands::{Run, length_unit, spec};
use crate::params::Params;
use crate::verify_cmds::{Ctx, Dim, Outcome, only};
use crate::{CommandError, CommandSpec, Session};
use craft3d_math::{Transform, Vec3};
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("measure.joint", "Joint Value", r#"{"mate": string}"#,
        "The solved value of each free motion of a mate of the active assembly, by its name (angle in degrees, distance, x and y in the assembly's length unit), whether it is driven, and its kind. Read-only. A mate that takes no part, or an assembly that is not solved, is an error that says why. The measures jointAngle and jointDistance give the same values to assertions and outputs (variable.measure).", doc: true, measure_joint),
    spec!("measure.pose", "Instance Pose", r#"{"instance": string} or {"point": "Instance/name"}"#,
        "Where an instance of the active assembly is: the position of its origin (the assembly's length unit), its rotation as a rotation vector in degrees (as instance.add takes it), the angle turned and its axis, and its x, y and z axes in the assembly; or, with point, where the origin of the frame a mate would take from that geometry is (a datum, a vertex, an edge's centre, a face's frame: Instance/Feature:name). Read-only. The measures position and rotation give the same values to assertions and outputs.", doc: true, measure_pose),
];

/// The placement of the assembly a measure is taken on.
fn placed_of<'a>(ctx: &Ctx<'a>, measure: &str) -> Result<&'a Arc<Placed>, String> {
    ctx.placed
        .ok_or_else(|| format!("{measure} measures an assembly; the document is a part"))
}

/// The mate `name` as solved, or why there is no value.
fn mate<'p>(placed: &'p Placed, name: &str) -> Result<&'p MateState, String> {
    let st = placed
        .mates
        .iter()
        .find(|m| m.name == name)
        .ok_or_else(|| {
            format!(
                "the assembly has no mate `{}`",
                name.chars().take(80).collect::<String>()
            )
        })?;
    if let Some(why) = &st.error {
        return Err(format!("mate `{name}` takes no part: {why}"));
    }
    if let Some(why) = &placed.solve_error {
        return Err(format!("the assembly is not solved: {why}"));
    }
    Ok(st)
}

/// The free motion `which` of a mate (`None`: its angle for `angle`,
/// else its first length motion) and its value.
fn motion_value(
    placed: &Placed,
    name: &str,
    angle: bool,
    which: Option<&str>,
) -> Result<(&'static str, f64), String> {
    let st = mate(placed, name)?;
    let free = st.kind.free();
    let at = match (angle, which) {
        (true, _) => free.iter().position(|m| *m == "angle"),
        (false, Some(which)) => free.iter().position(|m| *m == which && *m != "angle"),
        (false, None) => free.iter().position(|m| *m != "angle"),
    };
    let Some(at) = at else {
        let wanted = match (angle, which) {
            (true, _) => "an angle".to_string(),
            (false, Some(w)) => format!(
                "a length motion `{}`",
                w.chars().take(40).collect::<String>()
            ),
            (false, None) => "a length motion".to_string(),
        };
        return Err(format!(
            "mate `{name}` ({}) has no {wanted}; its free motions: {}",
            st.kind.name(),
            if free.is_empty() {
                "none".to_string()
            } else {
                free.join(", ")
            }
        ));
    };
    let motion = free.get(at).copied().unwrap_or("angle");
    let value = st
        .values
        .get(at)
        .copied()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("mate `{name}` has no solved {motion}"))?;
    Ok((motion, value))
}

/// The instance `name` as placed, or why it is not.
fn instance<'p>(placed: &'p Placed, name: &str) -> Result<&'p InstanceState, String> {
    let st = placed.instance(name).ok_or_else(|| {
        format!(
            "the assembly has no instance `{}`",
            name.chars().take(80).collect::<String>()
        )
    })?;
    if let Some(why) = &st.error {
        return Err(format!("instance `{name}` is not placed: {why}"));
    }
    Ok(st)
}

/// The rotation vector of a pose (radians).
fn rotation_of(pose: &Transform) -> Vec3 {
    craft3d_assembly::Mat3::from_cols(
        pose.vector(Vec3::X),
        pose.vector(Vec3::Y),
        pose.vector(Vec3::Z),
    )
    .log()
}

fn component(v: Vec3, axis: &str) -> Result<f64, String> {
    match axis {
        "x" => Ok(v.x),
        "y" => Ok(v.y),
        "z" => Ok(v.z),
        other => Err(format!(
            "`axis` is x, y or z, got `{}`",
            other.chars().take(20).collect::<String>()
        )),
    }
}

fn text<'p>(p: &'p Map<String, Value>, key: &str) -> Result<Option<&'p str>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(format!("`{key}` is a name")),
    }
}

/// Where `p` says: an instance's origin or the origin of a frame through
/// one (`point`).
fn located(placed: &Arc<Placed>, p: &Map<String, Value>) -> Result<Vec3, String> {
    match (text(p, "instance")?, text(p, "point")?) {
        (Some(name), None) => Ok(instance(placed, name)?.pose.point(Vec3::ZERO)),
        (None, Some(point)) => {
            if let Some(why) = &placed.solve_error {
                return Err(format!("the assembly is not solved: {why}"));
            }
            asm::point_in(placed, point).map_err(|e| format!("`point`: {e}"))
        }
        (Some(_), Some(_)) => Err("give `instance` or `point`, not both".to_string()),
        (None, None) => Err(
            "`instance` (an instance's name) or `point` (Instance/Feature:name) is required"
                .to_string(),
        ),
    }
}

/// The measures `jointAngle`, `jointDistance`, `position` and `rotation`.
pub(crate) fn take(ctx: &Ctx<'_>, name: &str, p: &Map<String, Value>) -> Result<Outcome, String> {
    let placed = placed_of(ctx, name)?;
    match name {
        "jointAngle" | "jointDistance" => {
            let angle = name == "jointAngle";
            if angle {
                only(p, &["mate"])?;
            } else {
                only(p, &["mate", "motion"])?;
            }
            let mate = text(p, "mate")?.ok_or("`mate` is required: the mate's name")?;
            let (motion, value) = motion_value(placed, mate, angle, text(p, "motion")?)?;
            let dim = if angle {
                Dim::Of(craft3d_expr::Dimension::ANGLE)
            } else {
                Dim::Length(1)
            };
            let mut out = Outcome::number(dim, value);
            out.detail.insert("mate".into(), mate.into());
            out.detail.insert("motion".into(), motion.into());
            Ok(out)
        }
        "position" => {
            only(p, &["instance", "point", "axis"])?;
            let axis = text(p, "axis")?.ok_or("`axis` is required: x, y or z")?;
            let at = located(placed, p)?;
            let mut out = Outcome::number(Dim::Length(1), component(at, axis)?);
            out.detail
                .insert("position".into(), json!([at.x, at.y, at.z]));
            Ok(out)
        }
        "rotation" => {
            only(p, &["instance", "axis"])?;
            let name = text(p, "instance")?.ok_or("`instance` is required: an instance's name")?;
            let r = rotation_of(&instance(placed, name)?.pose);
            let value = match text(p, "axis")? {
                Some(axis) => component(r, axis)?,
                None => r.length(),
            };
            let mut out = Outcome::number(Dim::Of(craft3d_expr::Dimension::ANGLE), value);
            out.detail
                .insert("rotationVector".into(), json!([r.x, r.y, r.z]));
            Ok(out)
        }
        other => Err(format!("unknown measure `{other}`")),
    }
}

/// What a measure of an assembly depends on beyond the bodies its
/// subjects name, as text for the key of its answer: the solved values
/// and poses it reads. `None` for any other measure.
pub(crate) fn key_text(placed: Option<&Arc<Placed>>, p: &Map<String, Value>) -> Option<String> {
    let measure = p.get("measure").and_then(Value::as_str)?;
    if !matches!(
        measure,
        "jointAngle" | "jointDistance" | "position" | "rotation"
    ) {
        return None;
    }
    let Some(placed) = placed else {
        return Some("no assembly".to_string());
    };
    let solve = format!("{:?}", placed.solve_error);
    Some(match measure {
        "jointAngle" | "jointDistance" => {
            let name = p.get("mate").and_then(Value::as_str).unwrap_or_default();
            match placed.mates.iter().find(|m| m.name == name) {
                Some(st) => format!("{solve}{:?}{:?}{:?}", st.kind, st.error, st.values),
                None => format!("{solve}no mate"),
            }
        }
        _ => {
            let pose = p
                .get("instance")
                .and_then(Value::as_str)
                .and_then(|n| placed.instance(n))
                .map(|st| format!("{:?}{:?}", st.error, st.pose));
            let point = p
                .get("point")
                .and_then(Value::as_str)
                .map(|n| format!("{:?}", asm::point_in(placed, n)));
            format!("{solve}{pose:?}{point:?}")
        }
    })
}

fn active_placed(session: &mut Session, id: &str) -> Result<Arc<Placed>, CommandError> {
    session
        .document_mut()?
        .placed()
        .ok_or_else(|| CommandError::new(format!("{id}: the active document is not an assembly")))
}

fn measure_joint(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "measure.joint";
    let p = Params::new(ID, params, &["mate"])?;
    let name = p.required_str("mate")?.to_string();
    let placed = active_placed(session, ID)?;
    let unit = length_unit(session.document()?.history.current());
    let st = mate(&placed, &name).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let mut values = Map::new();
    for (motion, value) in st.kind.free().iter().zip(&st.values) {
        let shown = if *motion == "angle" {
            value.to_degrees()
        } else {
            *value
        };
        values.insert((*motion).to_string(), shown.into());
    }
    Ok(json!({
        "mate": name,
        "kind": st.kind.name(),
        "driven": st.driven,
        "values": values,
        "angleUnit": "deg",
        "unit": unit,
    }))
}

fn measure_pose(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "measure.pose";
    let p = Params::new(ID, params, &["instance", "point"])?;
    let placed = active_placed(session, ID)?;
    let unit = length_unit(session.document()?.history.current());
    let map: Map<String, Value> = match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let err = |e: String| CommandError::new(format!("{ID}: {e}"));
    let at = located(&placed, &map).map_err(err)?;
    let position = json!([at.x, at.y, at.z]);
    if let Some(point) = p.str("point")? {
        return Ok(json!({ "point": point, "position": position, "unit": unit }));
    }
    let name = p.required_str("instance")?;
    let st = instance(&placed, name).map_err(err)?;
    let r = rotation_of(&st.pose);
    let angle = r.length();
    let axis = r.normalized().map(|a| json!([a.x, a.y, a.z]));
    let deg = r * (180.0 / std::f64::consts::PI);
    let vector = |v: Vec3| json!([v.x, v.y, v.z]);
    Ok(json!({
        "instance": name,
        "position": position,
        "rotation": [deg.x, deg.y, deg.z],
        "angle": angle.to_degrees(),
        "axis": axis,
        "x": vector(st.pose.vector(Vec3::X)),
        "y": vector(st.pose.vector(Vec3::Y)),
        "z": vector(st.pose.vector(Vec3::Z)),
        "unit": unit,
    }))
}
