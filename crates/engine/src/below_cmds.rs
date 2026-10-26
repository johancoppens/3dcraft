//! What lies on one side of a plane (`measure.below`, `measure.section`,
//! `measure.levelFor`, and the assertion measures `volumeBelow`,
//! `centroidBelow`, `sectionArea`, `sectionCentroid`, `sectionInertia`,
//! `levelFor`): the volume and centroid of the part of closed bodies below
//! a plane, the area, centroid and second moments of their section by it,
//! and the offset at which the volume below is a given volume. The
//! geometry is `craft3d_kernel::below` (exact on the B-rep); the design
//! note is docs/design/sections-and-levels.md.

use crate::commands::{Run, length_unit, spec, vec3};
use crate::params::Params;
use crate::verify_cmds::{
    Ctx, Dim, Outcome, all_bodies, direction, nonempty, plane, subject, whole_bodies,
};
use crate::{CommandError, CommandSpec, Session};
use craft3d_kernel::below::{BelowPlane, CutPlane, Level, below, level_for};
use craft3d_math::Vec3;
use craft3d_topo::Body;
use serde_json::{Map, Value};

/// Most bodies cut together.
const MAX_BODIES: usize = 200;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("measure.below", "Volume Below a Plane", r#"{"of": subject = all bodies, "plane": "xy|yz|zx" or datum plane (name or id) or a flat face by name or {point: [expr, expr, expr], normal: [number, number, number]} = "xy", "offset": length expr = 0, "xAxis": direction = the world axis most square to the normal}"#,
        "The part of closed bodies on one side of a plane, exactly: the plane is given as measure.symmetry takes it (a base plane, a datum plane, a flat face by its persistent name with its outward normal, or a point and a normal) and moved by offset along its normal, and below is the side the normal points away from. Returns volume (the document's unit cubed), centroid (null when nothing is below), surface (the area of the bodies' surface below the plane), and section, the section of the bodies by the plane, as measure.section gives it. The part below is the boolean intersection with a half-space and is integrated on the exact B-rep; the section's moments are Green's theorem along the curves in which the plane cuts the bodies. A body wholly below gives all of itself and no section; wholly above, nothing. Bodies that overlap are counted once each. offset in the result is how far the plane was moved. Refused: a sheet (no inside), a normal of no length, nothing found below where a vertex of a body lies below the plane. Read-only.", doc: true, names: ["of"], measure_below),
    spec!("measure.section", "Section by a Plane", r#"{"of": subject = all bodies, "plane": "xy|yz|zx" or datum plane (name or id) or a flat face by name or {point: [expr, expr, expr], normal: [number, number, number]} = "xy", "offset": length expr = 0, "xAxis": direction = the world axis most square to the normal}"#,
        "The section of closed bodies by a plane (plane, moved by offset along its normal, as measure.below takes them), exactly: area, centroid, and the second moments about axes through the centroid, ix = ∫y² dA about the axis along xAxis (square to the normal; by default the world axis most square to it, made square), iy = ∫x² dA about the axis along y = normal × xAxis, ixy = ∫xy dA, and the principal moments (major, minor, and majorAxis, the direction of the axis about which the moment is largest), in the document's unit to the fourth power; xAxis and yAxis say the axes used. From Green's theorem along the curves in which the plane cuts the bodies (each edge summed by two Gauss–Legendre rules until they agree); a cross-section of a beam, the cut of a body at a level. Read-only.", doc: true, names: ["of"], measure_section),
    spec!("measure.levelFor", "Level for a Volume", r#"{"of": subject = all bodies, "plane": "xy|yz|zx" or datum plane (name or id) or a flat face by name or {point: [expr, expr, expr], normal: [number, number, number]} = "xy", "volume": volume expr, "xAxis": direction = the world axis most square to the normal}"#,
        "The offset along its normal of plane (as measure.below takes it; only its direction and the point it is measured from count) at which the volume of the bodies below it is volume (the fill level of a tank for a volume of liquid; the plane at which a body floating with a known volume under it lies): found by the kernel by Newton's method on the section's area within a bracket, an offset the boolean refuses stepped around by halving, at most 64 steps (not converging is an error, and so is a volume not strictly between 0 and the bodies' whole volume). Returns offset, steps, residual (how far the volume below is from the one asked for) and everything measure.below gives at that offset. Read-only.", doc: true, names: ["of"], measure_level_for),
];

/// The assertion measures and their dimensions: the one list
/// `assert_cmds` reads them from.
pub(crate) const MEASURES: [(&str, Dim); 6] = [
    ("volumeBelow", Dim::Length(3)),
    ("centroidBelow", Dim::Length(1)),
    ("sectionArea", Dim::Length(2)),
    ("sectionCentroid", Dim::Length(1)),
    ("sectionInertia", Dim::Length(4)),
    ("levelFor", Dim::Length(1)),
];

/// The dimension of assertion measure `name`, when it is one of these.
pub(crate) fn dim_of(name: &str) -> Option<Dim> {
    MEASURES.iter().find(|(m, _)| *m == name).map(|(_, d)| *d)
}

/// The parameters of each assertion measure.
pub(crate) fn keys_of(measure: &str) -> &'static [&'static str] {
    match measure {
        "volumeBelow" | "sectionArea" => &["of", "plane", "offset", "xAxis"],
        "centroidBelow" | "sectionCentroid" => &["of", "plane", "offset", "xAxis", "axis"],
        "sectionInertia" => &["of", "plane", "offset", "xAxis", "about"],
        "levelFor" => &["of", "plane", "volume", "xAxis"],
        _ => &[],
    }
}

/// The keys of these measures whose values are expressions.
pub(crate) const EXPRS: [&str; 2] = ["offset", "volume"];

fn given<'v>(p: &'v Map<String, Value>, key: &str) -> Option<&'v Value> {
    p.get(key).filter(|v| !v.is_null())
}

/// The bodies named by `of` (default all), whole.
fn bodies_named(
    ctx: &Ctx<'_>,
    p: &Map<String, Value>,
) -> Result<Vec<(craft3d_doc::FeatureId, std::sync::Arc<Body>)>, String> {
    let found = match given(p, "of") {
        None => all_bodies(ctx),
        Some(v) => nonempty(subject(ctx, v, &["body"])?, "of")?,
    };
    let bodies = whole_bodies(ctx, &found, "a cut by a plane")?;
    if bodies.is_empty() {
        return Err("there are no bodies to cut".into());
    }
    if bodies.len() > MAX_BODIES {
        return Err(format!(
            "{} bodies; a cut by a plane takes at most {MAX_BODIES}",
            bodies.len()
        ));
    }
    Ok(bodies)
}

/// The cutting plane: `plane` (default xy, as measure.symmetry takes it)
/// with its normal, and `xAxis`, by default the world axis least parallel
/// to the normal (x, y, z in that order where two are as square to it),
/// made square to it; and the offset of the given plane along the normal
/// from the origin (`point · normal`).
fn cut_plane(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<(CutPlane, f64), String> {
    let (point, normal) = match given(p, "plane") {
        None => (Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0)),
        Some(v) => plane(ctx, Some(v))?,
    };
    let normal = normal
        .normalized()
        .ok_or("the plane's normal has no length")?;
    let x_axis = match given(p, "xAxis") {
        Some(v) => direction(Some(v), "xAxis")?,
        None => {
            let axes = [
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ];
            let mut best = Vec3::new(1.0, 0.0, 0.0);
            let mut least = f64::INFINITY;
            for axis in axes {
                let along = axis.dot(normal).abs();
                if along < least {
                    least = along;
                    best = axis;
                }
            }
            (best - normal * best.dot(normal))
                .normalized()
                .ok_or("the plane's normal gives no plane")?
        }
    };
    let side = CutPlane::new(normal, x_axis).map_err(|e| e.to_string())?;
    let base = point.dot(side.normal);
    if !base.is_finite() {
        return Err("the plane's point is not finite".into());
    }
    Ok((side, base))
}

fn num(v: f64) -> Value {
    if v.is_finite() { v.into() } else { Value::Null }
}

/// The section as the commands give it.
fn section_json(found: &BelowPlane, plane: &CutPlane) -> Map<String, Value> {
    let s = found.section;
    let mut out = Map::new();
    out.insert("area".into(), num(s.area));
    out.insert("centroid".into(), s.centroid.map_or(Value::Null, vec3));
    out.insert("ix".into(), num(s.i_x));
    out.insert("iy".into(), num(s.i_y));
    out.insert("ixy".into(), num(s.product));
    match s.principal(plane) {
        Some((major, minor, axis)) => {
            out.insert("major".into(), num(major));
            out.insert("minor".into(), num(minor));
            out.insert("majorAxis".into(), vec3(axis));
        }
        None => {
            out.insert("major".into(), Value::Null);
            out.insert("minor".into(), Value::Null);
            out.insert("majorAxis".into(), Value::Null);
        }
    }
    out.insert("xAxis".into(), vec3(plane.x_axis));
    out.insert("yAxis".into(), vec3(plane.y_axis()));
    out
}

fn below_json(found: &BelowPlane, plane: &CutPlane) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("offset".into(), num(found.offset));
    out.insert("normal".into(), vec3(plane.normal));
    out.insert("volume".into(), num(found.volume));
    out.insert("centroid".into(), found.centroid.map_or(Value::Null, vec3));
    out.insert("surface".into(), num(found.surface));
    out.insert("section".into(), Value::Object(section_json(found, plane)));
    out
}

/// What the three commands and the assertion measures measure: below the
/// plane at `offset`, or at the offset for `volume`.
#[derive(Clone, Copy)]
enum Asked {
    At,
    For,
}

fn measured(
    ctx: &Ctx<'_>,
    p: &Map<String, Value>,
    asked: Asked,
    allowed: &[&str],
) -> Result<(BelowPlane, CutPlane, Option<Level>), String> {
    crate::verify_cmds::only(p, allowed)?;
    let bodies = bodies_named(ctx, p)?;
    let (plane, base) = cut_plane(ctx, p)?;
    let plain: Vec<&Body> = bodies.iter().map(|(_, b)| b.as_ref()).collect();
    // The kernel's offset is from the origin; the commands' from the
    // plane given.
    let from_given = |mut found: BelowPlane| {
        found.offset -= base;
        found
    };
    match asked {
        Asked::At => {
            let offset = given(p, "offset")
                .map(|v| ctx.quantity(v, "offset", 1))
                .transpose()?
                .unwrap_or(0.0);
            let found =
                below(&plain, &plane, base + offset, &ctx.tolerance).map_err(|e| e.to_string())?;
            Ok((from_given(found), plane, None))
        }
        Asked::For => {
            let volume = ctx.quantity(
                given(p, "volume").ok_or("`volume` is required: the volume below the plane")?,
                "volume",
                3,
            )?;
            let level =
                level_for(&plain, &plane, volume, &ctx.tolerance).map_err(|e| e.to_string())?;
            Ok((from_given(level.below), plane, Some(level)))
        }
    }
}

const PLANE_KEYS: [&str; 4] = ["of", "plane", "offset", "xAxis"];
const LEVEL_KEYS: [&str; 4] = ["of", "plane", "volume", "xAxis"];

fn run_command(
    session: &mut Session,
    params: &Value,
    id: &'static str,
    keys: &[&str],
    shape: fn(&BelowPlane, &CutPlane, Option<&Level>) -> Map<String, Value>,
    asked: Asked,
) -> Run {
    Params::new(id, params, keys)?;
    let map = match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let err = |e: String| CommandError::new(format!("{id}: {e}"));
    let ctx = Ctx::new(doc, &regen).map_err(err)?;
    let (found, plane, level) = measured(&ctx, &map, asked, keys).map_err(err)?;
    let mut out = shape(&found, &plane, level.as_ref());
    out.insert("lengthUnit".into(), length_unit(doc).into());
    Ok(Value::Object(out))
}

fn measure_below(session: &mut Session, params: &Value) -> Run {
    run_command(
        session,
        params,
        "measure.below",
        &PLANE_KEYS,
        |found, plane, _| below_json(found, plane),
        Asked::At,
    )
}

fn measure_section(session: &mut Session, params: &Value) -> Run {
    run_command(
        session,
        params,
        "measure.section",
        &PLANE_KEYS,
        |found, plane, _| {
            let mut out = section_json(found, plane);
            out.insert("offset".into(), num(found.offset));
            out.insert("normal".into(), vec3(plane.normal));
            out
        },
        Asked::At,
    )
}

fn measure_level_for(session: &mut Session, params: &Value) -> Run {
    run_command(
        session,
        params,
        "measure.levelFor",
        &LEVEL_KEYS,
        |found, plane, level| {
            let mut out = below_json(found, plane);
            if let Some(level) = level {
                out.insert("steps".into(), level.steps.into());
                out.insert("residual".into(), num(level.residual));
            }
            out
        },
        Asked::For,
    )
}

/// A coordinate of a point along `axis` (x, y or z).
fn coordinate(p: &Map<String, Value>, point: Option<Vec3>, what: &str) -> Result<f64, String> {
    let point = point.ok_or_else(|| format!("there is no {what}: nothing is there"))?;
    match p.get("axis").and_then(Value::as_str) {
        Some("x") => Ok(point.x),
        Some("y") => Ok(point.y),
        Some("z") => Ok(point.z),
        _ => Err("`axis` must be x, y or z".into()),
    }
}

/// The assertion measure `name`.
pub(crate) fn measure(
    ctx: &Ctx<'_>,
    name: &str,
    p: &Map<String, Value>,
) -> Result<Outcome, String> {
    let Some((_, dim)) = MEASURES.iter().find(|(m, _)| *m == name) else {
        return Err(format!("unknown measure `{name}`"));
    };
    let asked = if name == "levelFor" {
        Asked::For
    } else {
        Asked::At
    };
    let (found, plane, _) = measured(ctx, p, asked, keys_of(name))?;
    let s = found.section;
    let value = match name {
        "volumeBelow" => found.volume,
        "centroidBelow" => coordinate(p, found.centroid, "centroid below the plane")?,
        "sectionArea" => s.area,
        "sectionCentroid" => coordinate(p, s.centroid, "section")?,
        "sectionInertia" => {
            let principal = s.principal(&plane);
            match p.get("about").and_then(Value::as_str) {
                Some("x") => s.i_x,
                Some("y") => s.i_y,
                Some("xy") => s.product,
                Some("major") => principal
                    .map(|(m, _, _)| m)
                    .ok_or("the section has no area")?,
                Some("minor") => principal
                    .map(|(_, m, _)| m)
                    .ok_or("the section has no area")?,
                _ => return Err("`about` must be x, y, xy, major or minor".into()),
            }
        }
        _ => found.offset,
    };
    if !value.is_finite() {
        return Err(format!("{name} is not a finite number"));
    }
    let mut out = Outcome::number(*dim, value);
    out.detail = below_json(&found, &plane);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_measure_has_its_keys() {
        for (name, _) in MEASURES {
            assert!(!keys_of(name).is_empty(), "{name}");
        }
    }
}
