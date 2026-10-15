//! A colour and a material per body: `body.appearance` and
//! `body.material`, and what `document.inspect` and `document.measure`
//! say of them (the mass and the inertia tensor of a body with a
//! density).
//!
//! The body is given by the feature that makes it; the entry is kept in
//! the document beside the history (`Document::bodies`) and reaches the
//! bodies made from that body by the rule of `appearance`.

use crate::appearance::{body_appearance, body_material};
use crate::commands::{Run, edit, fail, feature_ref, spec};
use crate::mass::mass_and_inertia_of;
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Appearance, Document, FeatureId, FeatureKind, LengthUnit, Material};
use craft3d_regen::Regen;
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("body.appearance", "Body Colour", r##"{"body": id or name, "color": "#rrggbb" or [r, g, b], "metallic": number, "roughness": number, "clear": boolean = false}"##,
        "Gives the body of a feature a colour (sRGB: \"#c8102e\" or [200, 16, 46], each 0 to 255), and optionally metallic and roughness (0 to 1, kept for exporters; the viewport shades from the colour). The body keeps it through every regeneration and edit; bodies made from it keep it too: a boolean's result has its target's colour; the faces a tool cut take the target's colour too, while the faces a union or an intersection takes from a coloured tool keep the tool's; set a colour on the later feature to colour its whole body. view.render, view.replay, the viewport and 3MF exports show it. clear takes the colour away (the theme's again).", doc: true, body_appearance_cmd),
    spec!("body.material", "Body Material", r#"{"body": id or name, "name": string, "density": number (kg/m³) or "7.85 g/cm3", "clear": boolean = false}"#,
        "Gives the body of a feature a material: a name and a density, a number in kg/m³ or a string with a unit (kg/m3, g/cm3, g/mm3, lb/in3, lb/ft3). document.inspect and document.measure then give the body's mass (kg) and its inertia tensor about the centroid (kg times the document's length unit squared), computed exactly from the B-rep; body.material itself measures nothing and says which bodies have the material. Bodies made from it keep it, as with body.appearance (a boolean's result has its target's). clear takes it away.", doc: true, body_material_cmd),
];

/// The feature of `key`, which must be one that can make a body.
fn body_ref(doc: &Document, p: &Params, command: &str) -> Result<FeatureId, CommandError> {
    let id = feature_ref(doc, p, command, "body")?;
    let kind = doc.feature(id).map(|f| &f.kind);
    if let Some(kind) = kind
        && (kind.is_datum() || matches!(kind, FeatureKind::Sketch(_)))
    {
        return fail(format!(
            "{command}: {id} is a {}, which makes no body; give the feature that makes the body",
            kind.type_name()
        ));
    }
    Ok(id)
}

/// A colour from `#rrggbb`, `#rgb` or `[r, g, b]` (0 to 255).
fn color_param(command: &str, value: &Value) -> Result<[u8; 3], CommandError> {
    let bad = || {
        CommandError::new(format!(
            "{command}: `color` must be \"#rrggbb\" or [r, g, b] with whole numbers from 0 to 255, got {value}"
        ))
    };
    match value {
        Value::String(text) => {
            let hex = text.trim().strip_prefix('#').ok_or_else(bad)?;
            let digits: Vec<u8> = hex
                .chars()
                .map(|c| c.to_digit(16).and_then(|d| u8::try_from(d).ok()))
                .collect::<Option<Vec<u8>>>()
                .ok_or_else(bad)?;
            match digits.as_slice() {
                [r, g, b] => Ok([r * 17, g * 17, b * 17]),
                [r1, r0, g1, g0, b1, b0] => Ok([r1 * 16 + r0, g1 * 16 + g0, b1 * 16 + b0]),
                _ => Err(bad()),
            }
        }
        Value::Array(items) if items.len() == 3 => {
            let mut out = [0_u8; 3];
            for (slot, item) in out.iter_mut().zip(items) {
                *slot = item
                    .as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(bad)?;
            }
            Ok(out)
        }
        _ => Err(bad()),
    }
}

/// A factor from 0 to 1.
fn factor(p: &Params, key: &str) -> Result<Option<f64>, CommandError> {
    match p.number(key)? {
        Some(x) if !(0.0..=1.0).contains(&x) => fail(format!(
            "{}: `{key}` must be between 0 and 1, got {x}",
            p.command()
        )),
        other => Ok(other),
    }
}

/// kg/m³ per unit of density that can follow the number.
const DENSITY_UNITS: [(&str, f64); 8] = [
    ("kg/m3", 1.0),
    ("kg/m^3", 1.0),
    ("g/cm3", 1000.0),
    ("g/cm^3", 1000.0),
    ("g/mm3", 1e6),
    ("g/mm^3", 1e6),
    ("lb/in3", 27_679.904_710_203_12),
    ("lb/ft3", 16.018_463_373_960_14),
];

/// A density in kg/m³: a plain number, or a number with a unit.
fn density_param(command: &str, value: &Value) -> Result<f64, CommandError> {
    let bad = |why: &str| {
        CommandError::new(format!(
            "{command}: `density` {why}; give a number in kg/m³ (7850) or one with a unit (\"7.85 g/cm3\"; units: kg/m3, g/cm3, g/mm3, lb/in3, lb/ft3), got {value}"
        ))
    };
    let density = match value {
        Value::Number(n) => n.as_f64().ok_or_else(|| bad("is not a number"))?,
        Value::String(text) => {
            let text = text.trim();
            let (number, per) = match DENSITY_UNITS.iter().find(|(unit, _)| text.ends_with(unit)) {
                Some((unit, per)) => (text.get(..text.len() - unit.len()).unwrap_or(""), *per),
                None => (text, 1.0),
            };
            number
                .trim()
                .parse::<f64>()
                .map_err(|_| bad("is not a number with a known unit"))?
                * per
        }
        _ => return Err(bad("is neither a number nor a string")),
    };
    if !(density.is_finite() && density > 0.0) {
        return Err(bad("must be positive"));
    }
    Ok(density)
}

fn appearance_json(appearance: &Appearance) -> Value {
    let mut out = json!({ "color": appearance.hex() });
    if let Some(map) = out.as_object_mut() {
        if let Some(m) = appearance.metallic {
            map.insert("metallic".into(), m.into());
        }
        if let Some(r) = appearance.roughness {
            map.insert("roughness".into(), r.into());
        }
    }
    out
}

fn body_appearance_cmd(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "body.appearance";
    let p = Params::new(
        ID,
        params,
        &["body", "color", "metallic", "roughness", "clear"],
    )?;
    let doc = session.document()?.history.current();
    let id = body_ref(doc, &p, ID)?;
    let clear = p.bool("clear")?.unwrap_or(false);
    let appearance = if clear {
        if let Some(key) = ["color", "metallic", "roughness"]
            .into_iter()
            .find(|key| p.has(key))
        {
            return fail(format!(
                "{ID}: `clear` takes the colour away; leave out `{key}`"
            ));
        }
        None
    } else {
        let color = color_param(ID, p.get("color").ok_or_else(|| p.missing("color"))?)?;
        Some(Appearance {
            color,
            metallic: factor(&p, "metallic")?,
            roughness: factor(&p, "roughness")?,
        })
    };
    let label = format!("{} colour of {id}", if clear { "Clear" } else { "Set" });
    edit(session, ID, &label, |doc| {
        doc.set_appearance(id, appearance)
    })?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    Ok(json!({
        "body": id.to_string(),
        "appearance": appearance.as_ref().map_or(Value::Null, appearance_json),
        "bodies": shown_bodies(doc, &regen, |doc, regen, at| {
            body_appearance(doc, regen, at).map(|a| appearance_json(&a))
        }),
    }))
}

fn body_material_cmd(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "body.material";
    let p = Params::new(ID, params, &["body", "name", "density", "clear"])?;
    let doc = session.document()?.history.current();
    let id = body_ref(doc, &p, ID)?;
    let clear = p.bool("clear")?.unwrap_or(false);
    let material = if clear {
        if let Some(key) = ["name", "density"].into_iter().find(|key| p.has(key)) {
            return fail(format!(
                "{ID}: `clear` takes the material away; leave out `{key}`"
            ));
        }
        None
    } else {
        let name = p.required_str("name")?.to_string();
        let density = density_param(ID, p.get("density").ok_or_else(|| p.missing("density"))?)?;
        Some(Material { name, density })
    };
    let label = format!("{} material of {id}", if clear { "Clear" } else { "Set" });
    edit(session, ID, &label, |doc| {
        doc.set_material(id, material.clone())
    })?;
    // Which bodies have it, and from where; not their mass, which is
    // measured when document.inspect or document.measure asks for it
    // (exactly, from the B-rep: seconds on a body of spline faces).
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let bodies: Vec<Value> = regen
        .bodies()
        .filter_map(|(at, _)| {
            let described = material_json(doc, &regen, at)?;
            Some(json!({ "feature": at.to_string(), "material": described }))
        })
        .collect();
    Ok(json!({
        "body": id.to_string(),
        "material": material.as_ref().map_or(Value::Null, |m| json!({ "name": m.name, "density": m.density })),
        "bodies": bodies,
    }))
}

/// The bodies of the document for which `what` says something, with it.
fn shown_bodies(
    doc: &Document,
    regen: &Regen,
    what: impl Fn(&Document, &Regen, FeatureId) -> Option<Value>,
) -> Vec<Value> {
    regen
        .bodies()
        .filter_map(|(at, _)| {
            let said = what(doc, regen, at)?;
            Some(json!({ "feature": at.to_string(), "appearance": said }))
        })
        .collect()
}

/// Metres per document length unit.
fn metres_per_unit(unit: LengthUnit) -> f64 {
    match unit {
        LengthUnit::Micrometre => 1e-6,
        LengthUnit::Millimetre => 1e-3,
        LengthUnit::Centimetre => 1e-2,
        LengthUnit::Metre => 1.0,
        LengthUnit::Inch => 0.0254,
        LengthUnit::Foot => 0.3048,
    }
}

/// The material of the body of `at`: its name and density, and the
/// feature it has them from when that is another.
fn material_json(doc: &Document, regen: &Regen, at: FeatureId) -> Option<Value> {
    let (from, material) = body_material(doc, regen, at)?;
    let mut described = json!({ "name": material.name, "density": material.density });
    if let (Some(map), true) = (described.as_object_mut(), from != at) {
        map.insert("from".into(), from.to_string().into());
    }
    Some(described)
}

/// Measures the body of `at` for [`physical_fields`] ahead of its mass
/// properties: with a material its mass and inertia are measured at once,
/// in about the time of the inertia alone, which a measure of its mass
/// properties first would not leave room for.
pub(crate) fn measure_ahead(doc: &Document, regen: &Regen, at: FeatureId, body: &Arc<Body>) {
    if body_material(doc, regen, at).is_some() {
        let _measured = mass_and_inertia_of(body);
    }
}

/// Adds to a body's entry of `document.inspect` or `document.measure`
/// its appearance and material (and from which feature it has them,
/// when that is another), and with a material its mass (kg) and inertia
/// tensor about the centroid (kg · unit²).
pub(crate) fn physical_fields(
    entry: &mut Map<String, Value>,
    doc: &Document,
    regen: &Regen,
    at: FeatureId,
    body: &Arc<Body>,
    unit: &str,
) {
    if let Some(appearance) = body_appearance(doc, regen, at) {
        entry.insert("appearance".into(), appearance_json(&appearance));
    }
    let (Some((_, material)), Some(described)) =
        (body_material(doc, regen, at), material_json(doc, regen, at))
    else {
        return;
    };
    entry.insert("material".into(), described);
    let m = metres_per_unit(doc.units.length);
    let per_volume = material.density * m * m * m;
    let (mass, inertia) = mass_and_inertia_of(body);
    match mass {
        Ok(mass) => {
            entry.insert("mass".into(), (mass.volume * per_volume).into());
            entry.insert("massUnit".into(), "kg".into());
        }
        Err(e) => {
            entry.insert("massError".into(), e.to_string().into());
            return;
        }
    }
    match inertia {
        Ok(inertia) => {
            let tensor: Vec<Vec<f64>> = inertia
                .tensor
                .iter()
                .map(|row| row.iter().map(|x| x * per_volume).collect())
                .collect();
            entry.insert("inertia".into(), json!(tensor));
            entry.insert("inertiaUnit".into(), format!("kg {unit}^2").into());
        }
        Err(e) => {
            entry.insert("inertiaError".into(), e.to_string().into());
        }
    }
}
