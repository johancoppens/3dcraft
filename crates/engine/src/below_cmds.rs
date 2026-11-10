//! What lies on one side of a plane (`measure.below`, `measure.section`,
//! `measure.levelFor`, and the assertion measures `volumeBelow`,
//! `centroidBelow`, `sectionArea`, `sectionCentroid`, `sectionInertia`,
//! `sectionTorsion`, `sectionShearCentre`, `sectionExtent`, `levelFor`):
//! the volume and centroid of the part of closed bodies below a plane, the
//! area, centroid, second moments, torsion constant, shear centre and
//! extent along an axis of their section by it, and the offset at which
//! the volume below is a given volume. The geometry is
//! `craft3d_kernel::below` (exact on the B-rep; the torsion by finite
//! elements, `craft3d_kernel::below::torsion`); the design note is
//! docs/design/sections-and-levels.md.

use crate::commands::{Run, length_unit, spec, vec3};
use crate::params::Params;
use crate::verify_cmds::{
    Ctx, Dim, Outcome, all_bodies, direction, nonempty, plane, subject, whole_bodies,
};
use crate::{CommandError, CommandSpec, Session};
use craft3d_kernel::below::torsion::{
    MAX_ELEMENTS, RegionMesh, Torsion, TorsionOptions, section_torsion,
};
use craft3d_kernel::below::{
    BelowPlane, CutError, CutPlane, Equilibrium, Level, Stability, below, floating, level_for,
    section_extent,
};
use craft3d_math::Portable;
use craft3d_math::Vec3;
use craft3d_topo::Body;
use serde_json::{Map, Value};

/// Most bodies cut together.
const MAX_BODIES: usize = 200;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("measure.below", "Volume Below a Plane", r#"{"of": subject = all bodies, "plane": "xy|yz|zx" or datum plane (name or id) or a flat face by name or {point: [expr, expr, expr], normal: [number, number, number]} = "xy", "offset": length expr = 0, "xAxis": direction = the world axis most square to the normal}"#,
        "The part of closed bodies on one side of a plane, exactly: the plane is given as measure.symmetry takes it (a base plane, a datum plane, a flat face by its persistent name with its outward normal, or a point and a normal) and moved by offset along its normal, and below is the side the normal points away from. Returns volume (the document's unit cubed), centroid (null when nothing is below), surface (the area of the bodies' surface below the plane), and section, the section of the bodies by the plane, as measure.section gives it without its torsion. The part below is the boolean intersection with a half-space and is integrated on the exact B-rep; the section's moments are Green's theorem along the curves in which the plane cuts the bodies. A body wholly below gives all of itself and no section; wholly above, nothing. Bodies that overlap are counted once each. offset in the result is how far the plane was moved. Refused: a sheet (no inside), a normal of no length, nothing found below where a vertex of a body lies below the plane. Read-only.", doc: true, names: ["of"], measure_below),
    spec!("measure.section", "Section by a Plane", r#"{"of": subject = all bodies, "plane": "xy|yz|zx" or datum plane (name or id) or a flat face by name or {point: [expr, expr, expr], normal: [number, number, number]} = "xy", "offset": length expr = 0, "xAxis": direction = the world axis most square to the normal, "torsion": boolean = true, "torsionTolerance": number = 1e-4, "maxElements": integer = 60000}"#,
        "The section of closed bodies by a plane (plane, moved by offset along its normal, as measure.below takes them), exactly: area, centroid, and the second moments about axes through the centroid, ix = ∫y² dA about the axis along xAxis (square to the normal; by default the world axis most square to it, made square), iy = ∫x² dA about the axis along y = normal × xAxis, ixy = ∫xy dA, and the principal moments (major, minor, and majorAxis, the direction of the axis about which the moment is largest), in the document's unit to the fourth power; xAxis and yAxis say the axes used. From Green's theorem along the curves in which the plane cuts the bodies (each edge summed by two Gauss–Legendre rules until they agree); a cross-section of a beam, the cut of a body at a level. With torsion (default true), also the St Venant torsion of the section, for any section with any number of holes: torsion, the torsion constant J (the document's unit to the fourth power), shearCentre, the shear centre (Trefftz's, independent of Poisson's ratio) as a point, and shearOffset, it from the centroid along xAxis and yAxis, and warping, the warping constant Cw about it (the unit to the sixth power); torsionSolve says how they were found: converged, change (of J between the last two rounds), elements, rounds and tolerance. Solved by finite elements (quadratic triangles whose boundary nodes lie on the exact curves) for the warping function, refined adaptively until J and the shear centre change by less than torsionTolerance (relative, 1e-9 to 0.1) from one round to the next, at most maxElements elements (up to 200000; 60000 in the browser); a solve that reaches the cap gives what it reached with converged false, never a number that claims more. A torsion that cannot be solved for at all (section faces that overlap or touch, a cap below the first mesh) leaves torsion, shearCentre, shearOffset and warping null with torsionSolve.error saying why, unless torsion: true was given, which makes it the command's error. Read-only.", doc: true, names: ["of"], measure_section),
    spec!("measure.levelFor", "Level for a Volume", r#"{"of": subject = all bodies, "plane": "xy|yz|zx" or datum plane (name or id) or a flat face by name or {point: [expr, expr, expr], normal: [number, number, number]} = "xy", "volume": volume expr, "xAxis": direction = the world axis most square to the normal}"#,
        "The offset along its normal of plane (as measure.below takes it; only its direction and the point it is measured from count) at which the volume of the bodies below it is volume (the fill level of a tank for a volume of liquid; the plane at which a body floating with a known volume under it lies): found by the kernel by Newton's method on the section's area within a bracket, an offset the boolean refuses stepped around by other points of the bracket, at most 64 steps (not converging is an error, and so is a volume not strictly between 0 and the bodies' whole volume). Returns offset, steps, residual (how far the volume below is from the one asked for) and everything measure.below gives at that offset. Read-only.", doc: true, names: ["of"], measure_level_for),
    spec!("measure.floating", "Floating Equilibrium", r#"{"of": subject = all bodies, "plane": "xy|yz|zx" or datum plane (name or id) or a flat face by name or {point: [expr, expr, expr], normal: [number, number, number]} = "xy", "volume": volume expr, "mass": number, "density": number, "centerOfGravity": [expr, expr, expr], "heel": angle expr = 0, "trim": angle expr = 0, "xAxis": direction = the world axis most square to the normal}"#,
        "The free-floating equilibrium of closed bodies: the plane, an offset and a tilt of its normal, below which the bodies have volume (or mass / density, the mass in any unit and the density in that unit per cubic metre, so kg and kg/m³) and whose centroid below (the centre of buoyancy) lies on the plane's normal through centerOfGravity. plane is the upright waterplane (as measure.below takes it, its normal up; xAxis its x axis, the length); heel (about xAxis) and trim (about y = normal × xAxis) tilt it for the start. Found by the kernel by Newton's method over the tilt, each step a measure.levelFor of the volume, with the exact derivative from the section's moments (the metacentric heights), at most 40 steps of at most 0.25 rad; not converging is an error. Returns equilibria, each with offset (along its normal from the plane's point), normal, heel, trim and tilt (degrees, from plane), centroid, arm (how far the centre of gravity is off the normal through the centroid), height (of the centre of gravity above the centroid along the normal), principal (the two metacentric heights gm, each with the axis it turns about and stable, unstable or neutral: within floor of zero), stable, iterations, settled and everything measure.below gives there. The first is the equilibrium reached from the start (the nearest one); where it is unstable, those the bodies fall to from it follow (one each way along each unstable direction: an upright box with its centre of gravity too high finds its two angles of loll); other equilibria are not searched. floats is the index of the first stable one (null when none is); neutral says whether the first turns freely in some direction (a sphere about its centre: any tilt is an equilibrium, to second order); unfinished lists the searches from an unstable equilibrium that ended without one. Read-only.", doc: true, names: ["of"], measure_floating),
];

/// The assertion measures and their dimensions: the one list
/// `assert_cmds` reads them from.
pub(crate) const MEASURES: [(&str, Dim); 15] = [
    ("volumeBelow", Dim::Length(3)),
    ("centroidBelow", Dim::Length(1)),
    ("sectionArea", Dim::Length(2)),
    ("sectionCentroid", Dim::Length(1)),
    ("sectionInertia", Dim::Length(4)),
    ("sectionTorsion", Dim::Length(4)),
    ("sectionShearCentre", Dim::Length(1)),
    ("sectionExtent", Dim::Length(1)),
    ("levelFor", Dim::Length(1)),
    ("floatingOffset", Dim::Length(1)),
    ("floatingHeel", Dim::Of(craft3d_expr::Dimension::ANGLE)),
    ("floatingTrim", Dim::Of(craft3d_expr::Dimension::ANGLE)),
    ("floatingGM", Dim::Length(1)),
    ("floatingGML", Dim::Length(1)),
    ("floatingStable", Dim::Flag),
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
        "sectionTorsion" => &["of", "plane", "offset", "xAxis"],
        "sectionShearCentre" => &["of", "plane", "offset", "xAxis", "axis"],
        "sectionInertia" => &["of", "plane", "offset", "xAxis", "about"],
        "sectionExtent" => &["of", "plane", "offset", "axis"],
        "levelFor" => &["of", "plane", "volume", "xAxis"],
        "floatingOffset" | "floatingHeel" | "floatingTrim" | "floatingGM" | "floatingGML"
        | "floatingStable" => &FLOATING_KEYS,
        _ => &[],
    }
}

/// The keys of these measures whose values are expressions (a list of
/// them for `centerOfGravity`).
pub(crate) const EXPRS: [&str; 7] = [
    "offset",
    "volume",
    "mass",
    "density",
    "centerOfGravity",
    "heel",
    "trim",
];

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
/// plane at `offset`, or at the offset for `volume`; or below the plane at
/// `offset` with the torsion of the section.
#[derive(Clone, Copy)]
enum Asked {
    At,
    For,
    Twist(TorsionOptions),
}

/// The tessellator's triangulation of a region, as the kernel's torsion
/// takes it.
fn mesher(
    points: &[(f64, f64)],
    segments: &[(usize, usize)],
    interior: &[(f64, f64)],
    clearance: &[f64],
) -> Result<RegionMesh, String> {
    craft3d_tess::triangulate_region(points, segments, interior, clearance)
        .map(|m| RegionMesh {
            points: m.points,
            triangles: m.triangles,
            boundary: m.boundary,
            interior: m.interior,
        })
        .map_err(|e| e.to_string())
}

fn measured(
    ctx: &Ctx<'_>,
    p: &Map<String, Value>,
    asked: Asked,
    allowed: &[&str],
) -> Result<(BelowPlane, CutPlane, Option<Level>), String> {
    measured_twisted(ctx, p, asked, allowed).map(|(found, plane, level, _)| (found, plane, level))
}

type Measured = (
    BelowPlane,
    CutPlane,
    Option<Level>,
    Option<Result<Torsion, CutError>>,
);

fn measured_twisted(
    ctx: &Ctx<'_>,
    p: &Map<String, Value>,
    asked: Asked,
    allowed: &[&str],
) -> Result<Measured, String> {
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
    let offset = || -> Result<f64, String> {
        Ok(given(p, "offset")
            .map(|v| ctx.quantity(v, "offset", 1))
            .transpose()?
            .unwrap_or(0.0))
    };
    match asked {
        Asked::At => {
            let found = below(&plain, &plane, base + offset()?, &ctx.tolerance)
                .map_err(|e| e.to_string())?;
            Ok((from_given(found), plane, None, None))
        }
        Asked::Twist(options) => {
            let (found, torsion) = section_torsion(
                &plain,
                &plane,
                base + offset()?,
                &ctx.tolerance,
                &options,
                &mesher,
            )
            .map_err(|e| e.to_string())?;
            Ok((from_given(found), plane, None, torsion))
        }
        Asked::For => {
            let volume = ctx.quantity(
                given(p, "volume").ok_or("`volume` is required: the volume below the plane")?,
                "volume",
                3,
            )?;
            let level =
                level_for(&plain, &plane, volume, &ctx.tolerance).map_err(|e| e.to_string())?;
            Ok((from_given(level.below), plane, Some(level), None))
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
    let (doc, placed) = crate::verify_cmds::measured_document(open);
    let doc = &*doc;
    let err = |e: String| CommandError::new(format!("{id}: {e}"));
    let mut ctx = Ctx::new(doc, &regen).map_err(err)?;
    ctx.placed = placed.as_ref();
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

const SECTION_KEYS: [&str; 7] = [
    "of",
    "plane",
    "offset",
    "xAxis",
    "torsion",
    "torsionTolerance",
    "maxElements",
];

/// The torsion of a section as the commands give it.
fn torsion_json(out: &mut Map<String, Value>, t: &Torsion, options: &TorsionOptions) {
    out.insert("torsion".into(), num(t.constant));
    out.insert("shearCentre".into(), vec3(t.shear_centre));
    out.insert(
        "shearOffset".into(),
        Value::Array(vec![num(t.shear_offset.0), num(t.shear_offset.1)]),
    );
    out.insert("warping".into(), num(t.warping));
    let mut solve = Map::new();
    solve.insert("converged".into(), t.converged.into());
    solve.insert("change".into(), num(t.change));
    solve.insert("elements".into(), t.elements.into());
    solve.insert("rounds".into(), t.rounds.into());
    solve.insert("tolerance".into(), num(options.tolerance));
    out.insert("torsionSolve".into(), Value::Object(solve));
}

fn measure_section(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "measure.section";
    let p = Params::new(ID, params, &SECTION_KEYS)?;
    let asked_for = p.bool("torsion")?;
    let twist = asked_for.unwrap_or(true);
    let mut options = TorsionOptions::default();
    if let Some(tolerance) = p.number("torsionTolerance")? {
        if !(tolerance.is_finite() && (1e-9..=0.1).contains(&tolerance)) {
            return Err(CommandError::new(format!(
                "{ID}: `torsionTolerance` must be a number from 1e-9 to 0.1"
            )));
        }
        options.tolerance = tolerance;
    }
    if let Some(cap) = p.get("maxElements") {
        let cap = cap
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_ELEMENTS).contains(n))
            .ok_or_else(|| {
                CommandError::new(format!(
                    "{ID}: `maxElements` must be a whole number from 1 to {MAX_ELEMENTS}"
                ))
            })?;
        options.max_elements = cap;
    }
    let mut map = match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    for key in ["torsion", "torsionTolerance", "maxElements"] {
        map.remove(key);
    }
    let open = session.document_mut()?;
    let regen = open.regen();
    let (doc, placed) = crate::verify_cmds::measured_document(open);
    let doc = &*doc;
    let err = |e: String| CommandError::new(format!("{ID}: {e}"));
    let mut ctx = Ctx::new(doc, &regen).map_err(err)?;
    ctx.placed = placed.as_ref();
    let asked = if twist {
        Asked::Twist(options)
    } else {
        Asked::At
    };
    let (found, plane, _, torsion) =
        measured_twisted(&ctx, &map, asked, &PLANE_KEYS).map_err(err)?;
    let mut out = section_json(&found, &plane);
    out.insert("offset".into(), num(found.offset));
    out.insert("normal".into(), vec3(plane.normal));
    let nulls = |out: &mut Map<String, Value>| {
        for key in ["torsion", "shearCentre", "shearOffset", "warping"] {
            out.insert(key.into(), Value::Null);
        }
    };
    match torsion {
        Some(Ok(t)) => torsion_json(&mut out, &t, &options),
        // Asked for by name, a torsion that cannot be solved for is the
        // command's error; by default the section is still given, with
        // the reason.
        Some(Err(e)) if asked_for == Some(true) => return Err(err(e.to_string())),
        Some(Err(e)) => {
            nulls(&mut out);
            let mut solve = Map::new();
            solve.insert("error".into(), e.to_string().into());
            solve.insert("tolerance".into(), num(options.tolerance));
            out.insert("torsionSolve".into(), Value::Object(solve));
        }
        None if twist => nulls(&mut out),
        None => {}
    }
    out.insert("lengthUnit".into(), length_unit(doc).into());
    Ok(Value::Object(out))
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

const FLOATING_KEYS: [&str; 9] = [
    "of",
    "plane",
    "volume",
    "mass",
    "density",
    "centerOfGravity",
    "heel",
    "trim",
    "xAxis",
];

/// An angle parameter in radians: a plain number is degrees, an
/// expression an angle (or a plain number, degrees).
fn angle(ctx: &Ctx<'_>, p: &Map<String, Value>, key: &str) -> Result<f64, String> {
    let Some(value) = given(p, key) else {
        return Ok(0.0);
    };
    let radians = match value {
        Value::Number(n) => n.as_f64().map(f64::to_radians),
        Value::String(source) => {
            let quantity = ctx.evaluate(source, key)?;
            if quantity.dimension == craft3d_expr::Dimension::ANGLE {
                Some(quantity.value)
            } else if quantity.dimension.is_dimensionless() {
                Some(quantity.value.to_radians())
            } else {
                return Err(format!("`{key}` must be an angle"));
            }
        }
        _ => None,
    };
    radians
        .filter(|a| a.is_finite())
        .ok_or_else(|| format!("`{key}` must be a finite angle"))
}

/// The volume below: `volume`, or `mass` / `density` (the density per
/// cubic metre) in the document's unit cubed.
fn floating_volume(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<f64, String> {
    let positive = |key: &str| -> Result<f64, String> {
        let value = given(p, key).ok_or_else(|| format!("`{key}` is required with mass"))?;
        // A mass in kg (a plain number, or `300 kg`); a density per cubic
        // metre (a plain number, or `1025 kg / (1 m)^3`), whatever the
        // document's length unit.
        let number = if key == "mass" {
            ctx.quantity_of(value, key, craft3d_expr::Dimension::MASS)?
        } else {
            let density = craft3d_expr::Dimension::DENSITY;
            match value {
                Value::String(source) => {
                    let q = ctx.evaluate(source, key)?;
                    if q.dimension.is_dimensionless() {
                        q.value
                    } else if q.dimension == density {
                        q.value / craft3d_expr::Unit::Metre.factor().ppowi(-3)
                    } else {
                        return Err(format!(
                            "`{key}` is {}, but a density is needed (kg/m^3: a number, or `1025 kg / (1 m)^3`)",
                            q.dimension.described()
                        ));
                    }
                }
                other => ctx.quantity(other, key, 0)?,
            }
        };
        if number > 0.0 && number.is_finite() {
            Ok(number)
        } else {
            Err(format!("`{key}` must be a positive number"))
        }
    };
    match (given(p, "volume"), given(p, "mass"), given(p, "density")) {
        (Some(v), None, None) => ctx.quantity(v, "volume", 3),
        (None, Some(_), _) | (None, _, Some(_)) => {
            let cubic_metres = positive("mass")? / positive("density")?;
            let unit = ctx.doc.units.length.as_expr_unit().factor();
            let volume = cubic_metres * (1000.0 / unit).ppowi(3);
            if volume.is_finite() {
                Ok(volume)
            } else {
                Err("mass / density is not a finite volume".into())
            }
        }
        (None, None, None) => {
            Err("`volume` is required (the volume below), or `mass` and `density`".into())
        }
        _ => Err("give `volume` or `mass` and `density`, not both".into()),
    }
}

fn degrees(a: f64) -> Value {
    num(a.to_degrees())
}

fn stability_name(s: Stability) -> &'static str {
    match s {
        Stability::Stable => "stable",
        Stability::Unstable => "unstable",
        Stability::Neutral => "neutral",
    }
}

/// One equilibrium as `measure.floating` gives it; `start` is the
/// upright plane, `base` the point of the plane given.
fn equilibrium_json(e: &Equilibrium, start: &CutPlane, point: Vec3) -> Map<String, Value> {
    let mut below = e.level.below;
    below.offset -= point.dot(e.plane.normal);
    let mut out = below_json(&below, &e.plane);
    let n = e.plane.normal;
    let (x0, y0, z0) = (start.x_axis, start.y_axis(), start.normal);
    out.insert("heel".into(), degrees((-n.dot(y0)).patan2(n.dot(z0))));
    out.insert("trim".into(), degrees(n.dot(x0).clamp(-1.0, 1.0).pasin()));
    out.insert("tilt".into(), degrees(n.dot(z0).clamp(-1.0, 1.0).pacos()));
    out.insert("arm".into(), num(e.arm));
    out.insert("height".into(), num(e.energy));
    out.insert(
        "principal".into(),
        Value::Array(
            e.principal
                .iter()
                .map(|p| {
                    let mut one = Map::new();
                    one.insert("gm".into(), num(p.gm));
                    one.insert("about".into(), vec3(p.about));
                    one.insert("stability".into(), stability_name(p.stability).into());
                    Value::Object(one)
                })
                .collect(),
        ),
    );
    out.insert("stable".into(), e.is_stable().into());
    out.insert("iterations".into(), e.iterations.into());
    out.insert("settled".into(), e.settled.into());
    out
}

/// Why a floating equilibrium was not had: the search did not settle
/// (no equilibrium within its steps, or a level that did not converge),
/// which an assertion takes as not proven, or any other error.
enum FloatFailed {
    Unsettled(String),
    Error(String),
}

impl From<String> for FloatFailed {
    fn from(e: String) -> Self {
        FloatFailed::Error(e)
    }
}

impl From<&str> for FloatFailed {
    fn from(e: &str) -> Self {
        FloatFailed::Error(e.to_string())
    }
}

/// How a floating search's error counts: the search not settling (no
/// equilibrium within its steps or its levels, or a level whose search
/// did not converge) leaves the answer unknown, not proven either way;
/// anything else is an error of the question (no bodies, a volume more
/// than they hold, a sheet).
fn failed(e: CutError) -> FloatFailed {
    let text = e.to_string();
    match e {
        CutError::NoEquilibrium { .. } | CutError::NotConverged { .. } => {
            FloatFailed::Unsettled(text)
        }
        _ => FloatFailed::Error(text),
    }
}

/// A floating equilibrium found, with what it was found for.
struct Floated {
    found: craft3d_kernel::below::Floating,
    /// The upright plane (before `heel` and `trim`).
    upright: CutPlane,
    /// The point of the plane given.
    point: Vec3,
    volume: f64,
    centre: Vec3,
}

/// The floating equilibria of `measure.floating` and of the assertion
/// measures `floating…`: their parameters read in `ctx` (expressions of
/// the variables, and of the outputs where `ctx` has them).
fn solve_floating(ctx: &Ctx<'_>, map: &Map<String, Value>) -> Result<Floated, FloatFailed> {
    let bodies = bodies_named(ctx, map)?;
    let (upright, _) = cut_plane(ctx, map)?;
    let point = match given(map, "plane") {
        None => Vec3::ZERO,
        Some(v) => plane(ctx, Some(v))?.0,
    };
    let volume = floating_volume(ctx, map)?;
    let centre = ctx.point(
        Some(given(map, "centerOfGravity").ok_or("`centerOfGravity` is required: [x, y, z]")?),
        "centerOfGravity",
    )?;
    let (heel, trim) = (angle(ctx, map, "heel")?, angle(ctx, map, "trim")?);
    // The start: the upright plane turned by trim about y, then by
    // heel about x.
    let (x0, y0, z0) = (upright.x_axis, upright.y_axis(), upright.normal);
    let (sh, ch) = heel.psin_cos();
    let (st, ct) = trim.psin_cos();
    let normal = x0 * st - y0 * (ct * sh) + z0 * (ct * ch);
    let x_axis = x0 * ct + y0 * (st * sh) - z0 * (st * ch);
    let start = CutPlane::new(normal, x_axis).map_err(|e| e.to_string())?;
    let plain: Vec<&Body> = bodies.iter().map(|(_, b)| b.as_ref()).collect();
    let found = floating(&plain, &start, volume, centre, &ctx.tolerance).map_err(failed)?;
    Ok(Floated {
        found,
        upright,
        point,
        volume,
        centre,
    })
}

fn measure_floating(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "measure.floating";
    Params::new(ID, params, &FLOATING_KEYS)?;
    let map = match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let (doc, placed) = crate::verify_cmds::measured_document(open);
    let doc = &*doc;
    let err = |e: String| CommandError::new(format!("{ID}: {e}"));
    let mut ctx = Ctx::new(doc, &regen).map_err(err)?;
    ctx.placed = placed.as_ref();
    let Floated {
        found,
        upright,
        point,
        volume,
        centre,
    } = solve_floating(&ctx, &map).map_err(|e| match e {
        FloatFailed::Unsettled(e) | FloatFailed::Error(e) => err(e),
    })?;
    let mut out = Map::new();
    out.insert(
        "equilibria".into(),
        Value::Array(
            found
                .equilibria
                .iter()
                .map(|e| Value::Object(equilibrium_json(e, &upright, point)))
                .collect(),
        ),
    );
    out.insert(
        "floats".into(),
        found
            .equilibria
            .iter()
            .position(Equilibrium::is_stable)
            .map_or(Value::Null, Value::from),
    );
    out.insert(
        "neutral".into(),
        found
            .equilibria
            .first()
            .is_some_and(|e| {
                e.principal
                    .iter()
                    .any(|p| p.stability == Stability::Neutral)
            })
            .into(),
    );
    out.insert(
        "unfinished".into(),
        Value::Array(
            found
                .unfinished
                .iter()
                .map(|e| e.to_string().into())
                .collect(),
        ),
    );
    out.insert("volumeAsked".into(), num(volume));
    out.insert("centerOfGravity".into(), vec3(centre));
    out.insert("levels".into(), found.levels.into());
    out.insert("floor".into(), num(found.floor));
    out.insert(
        "note".into(),
        "the equilibrium reached from the start (the nearest) first; where it is unstable, those the bodies fall to from it; other equilibria are not searched".into(),
    );
    out.insert("lengthUnit".into(), length_unit(doc).into());
    Ok(Value::Object(out))
}

/// The assertion measures `floatingOffset`, `floatingHeel`,
/// `floatingTrim`, `floatingGM`, `floatingGML` and `floatingStable`: of
/// the equilibrium `measure.floating` reaches from the start (its first,
/// the nearest), with the same parameters. The offset along the
/// equilibrium's normal from the plane's point; heel and trim (radians)
/// from the upright plane as `measure.floating` gives them in degrees;
/// GM and GML the metacentric heights for a turn about the equilibrium
/// plane's x axis (`xAxis` turned with it: a heel, for a hull with its
/// length along x) and about its y axis (a trim), `I / V − KG + KB` with
/// I the waterplane's second moment about that axis through its
/// centroid: the diagonal of the stiffness whose eigenvalues are the
/// principal heights, so equal to them where the waterplane is symmetric
/// about x; stable whether it is stable in both principal directions
/// (neutral, a height within the floor of zero, is not proven either
/// way, unless another direction is unstable). A search that does not
/// settle is not proven, with the kernel's reason.
fn floating_measure(ctx: &Ctx<'_>, name: &str, p: &Map<String, Value>) -> Result<Outcome, String> {
    crate::verify_cmds::only(p, &FLOATING_KEYS)?;
    let dim = dim_of(name).ok_or_else(|| format!("unknown measure `{name}`"))?;
    let solved = match solve_floating(ctx, p) {
        Ok(solved) => solved,
        Err(FloatFailed::Unsettled(why)) => {
            return Ok(Outcome::unproven(
                dim,
                format!("no floating equilibrium was settled from the start: {why}"),
            ));
        }
        Err(FloatFailed::Error(e)) => return Err(e),
    };
    let e = solved
        .found
        .equilibria
        .first()
        .ok_or("no floating equilibrium was found")?;
    let detail = equilibrium_json(e, &solved.upright, solved.point);
    let below = e.level.below;
    let section = below.section;
    let v = below.volume;
    let n = e.plane.normal;
    let (x0, y0, z0) = (
        solved.upright.x_axis,
        solved.upright.y_axis(),
        solved.upright.normal,
    );
    let gm_of = |i: f64| {
        if v > 0.0 && v.is_finite() {
            Ok(i / v - e.energy)
        } else {
            Err("the volume below is not positive".to_string())
        }
    };
    let mut out = if name == "floatingStable" {
        // Unstable in some direction is a no, stable in all a yes; a
        // direction whose GM lies within the floor of zero (neutral) is
        // neither for certain: the measurement cannot tell its sign.
        if e.is_stable() || e.is_unstable() {
            Outcome::flag(e.is_stable())
        } else {
            let floor = crate::assert_cmds::shown(solved.found.floor);
            Outcome::unproven(
                Dim::Flag,
                format!(
                    "neutral within measurement: a metacentric height lies within the floor ({floor}) of zero, so whether it turns back or further is not told"
                ),
            )
        }
    } else {
        let value = match name {
            "floatingOffset" => below.offset - solved.point.dot(n),
            "floatingHeel" => (-n.dot(y0)).patan2(n.dot(z0)),
            "floatingTrim" => n.dot(x0).clamp(-1.0, 1.0).pasin(),
            "floatingGM" => gm_of(section.i_x)?,
            "floatingGML" => gm_of(section.i_y)?,
            _ => return Err(format!("unknown measure `{name}`")),
        };
        if !value.is_finite() {
            return Err(format!("{name} is not a finite number"));
        }
        Outcome::number(dim, value)
    };
    out.detail = detail;
    if name == "floatingStable" {
        let words: Vec<String> = e
            .principal
            .iter()
            .map(|p| {
                format!(
                    "{} (GM {})",
                    stability_name(p.stability),
                    crate::assert_cmds::shown(p.gm)
                )
            })
            .collect();
        out.note = Some(words.join(", "));
    }
    Ok(out)
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
    if name == "sectionExtent" {
        return extent_measure(ctx, p);
    }
    if name.starts_with("floating") {
        return floating_measure(ctx, name, p);
    }
    if name == "sectionTorsion" || name == "sectionShearCentre" {
        return torsion_measure(ctx, name, p);
    }
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

/// The assertion measures `sectionTorsion` (J) and `sectionShearCentre`
/// (a coordinate of the shear centre along `axis`), with the default
/// torsion options. A solve that stopped at its element cap is not proven:
/// the number it reached is given, with why.
fn torsion_measure(ctx: &Ctx<'_>, name: &str, p: &Map<String, Value>) -> Result<Outcome, String> {
    let options = TorsionOptions::default();
    let (found, plane, _, torsion) =
        measured_twisted(ctx, p, Asked::Twist(options), keys_of(name))?;
    let t = torsion
        .ok_or("the plane does not cut the bodies: there is no section")?
        .map_err(|e| e.to_string())?;
    let (value, dim) = if name == "sectionTorsion" {
        (t.constant, Dim::Length(4))
    } else {
        (
            coordinate(p, Some(t.shear_centre), "shear centre")?,
            Dim::Length(1),
        )
    };
    if !value.is_finite() {
        return Err(format!("{name} is not a finite number"));
    }
    let mut out = Outcome::number(dim, value);
    out.detail = below_json(&found, &plane);
    let mut solved = Map::new();
    torsion_json(&mut solved, &t, &options);
    out.detail.extend(solved);
    if !t.converged {
        out.unproven = Some(format!(
            "the torsion did not converge within {} elements: J changed by {} in the last round",
            options.max_elements,
            crate::assert_cmds::shown(t.change)
        ));
    }
    Ok(out)
}

/// The assertion measure `sectionExtent`: how far the section of the
/// bodies by the plane (at `offset`) reaches along `axis` (x, y, z or a
/// direction), from where it begins to where it ends; `from` and `to` in
/// the detail are those two coordinates along the axis. A plane that cuts
/// nothing is an error, not a length of zero.
fn extent_measure(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    crate::verify_cmds::only(p, keys_of("sectionExtent"))?;
    let along = direction(
        Some(given(p, "axis").ok_or("`axis` is required: x, y, z or [x, y, z]")?),
        "axis",
    )?;
    let bodies = bodies_named(ctx, p)?;
    let (plane, base) = cut_plane(ctx, p)?;
    let offset = given(p, "offset")
        .map(|v| ctx.quantity(v, "offset", 1))
        .transpose()?
        .unwrap_or(0.0);
    let plain: Vec<&Body> = bodies.iter().map(|(_, b)| b.as_ref()).collect();
    let found = section_extent(&plain, &plane, base + offset, along, &ctx.tolerance)
        .map_err(|e| e.to_string())?
        .ok_or("the plane does not cut the bodies: there is no section")?;
    let value = found.hi - found.lo;
    if !value.is_finite() {
        return Err("sectionExtent is not a finite number".into());
    }
    let mut out = Outcome::number(Dim::Length(1), value);
    out.detail.insert("from".into(), num(found.lo));
    out.detail.insert("to".into(), num(found.hi));
    out.detail.insert("axis".into(), vec3(along));
    out.detail.insert("offset".into(), num(offset));
    out.detail.insert("normal".into(), vec3(plane.normal));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A search that does not settle is not proven; an error of the
    /// question stays an error.
    #[test]
    fn a_search_that_does_not_settle_is_unsettled_not_an_error() {
        let unsettled = [
            CutError::NoEquilibrium {
                steps: 40,
                arm: 1.0,
                normal: Vec3::new(0.0, 0.0, 1.0),
            },
            CutError::NotConverged {
                steps: 64,
                low: 0.0,
                high: 1.0,
                residual: 0.5,
            },
        ];
        for e in unsettled {
            assert!(matches!(failed(e), FloatFailed::Unsettled(_)));
        }
        for e in [
            CutError::NoBodies,
            CutError::OutOfRange {
                volume: 2.0,
                total: 1.0,
            },
            CutError::NotSolid { body: 0 },
        ] {
            assert!(matches!(failed(e), FloatFailed::Error(_)));
        }
    }

    #[test]
    fn every_measure_has_its_keys() {
        for (name, _) in MEASURES {
            assert!(!keys_of(name).is_empty(), "{name}");
        }
    }
}
