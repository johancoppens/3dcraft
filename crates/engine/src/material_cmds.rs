//! Layer 3's materials: the layers beyond metallic and roughness that
//! `body.appearance` and `face.appearance` take (anisotropy, a clear
//! coat, transmission, a subsurface wrap; docs/design/rendering.md,
//! *Layer 3: materials*), and the small library of presets as data
//! (`material_presets.json`): `material.presets` lists them,
//! `material.preset` gives one to a body (its appearance and its
//! material), with any of its numbers changed.

use crate::appearance::{Values, body_appearance};
use crate::appearance_cmds::{appearance_json, body_ref, color_param, described_material, factor};
use crate::commands::{Run, edit, fail, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{AnisotropyDirection, Appearance, BodyAppearance, Material, Optics};
use serde_json::{Map, Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!(
        "material.presets",
        "Material Presets",
        "{}",
        "Lists the material presets: steel, aluminium-brushed, abs, glass, car-paint, wood, and those with a solid texture: oak, walnut, beech, marble, granite, carbon and cast-iron; each with its colour, metallic, roughness, its layers (anisotropy, clear coat, transmission, subsurface, pattern) and its material (name and density in kg/m³), as material.preset gives them (a pattern's scale, axis and origin are fitted to the body then). Reads nothing of the document.",
        doc: false,
        material_presets_cmd
    ),
    spec!(
        "material.preset",
        "Material Preset",
        r##"{"body": id or name, "preset": "steel" | "aluminium-brushed" | "abs" | "glass" | "car-paint" | "wood" | "oak" | "walnut" | "beech" | "marble" | "granite" | "carbon" | "cast-iron", "pattern": {fields of body.appearance's pattern, each replacing the preset's} or null, "color": "#rrggbb" or [r, g, b], "metallic": number, "roughness": number, "anisotropy": number, "anisotropyAxis": [x, y, z] or "parametric", "clearcoat": number, "clearcoatRoughness": number, "transmission": number, "ior": number, "thinWalled": boolean, "absorption": "#rrggbb" or [r, g, b], "absorptionDistance": number (mm), "subsurface": number, "subsurfaceColor": "#rrggbb" or [r, g, b]}"##,
        "Gives the body of a feature a preset of the material library (material.presets lists them): its appearance (colour, metallic, roughness and the layers the studio look shades: anisotropy for brushed metal, a clear coat for car paint, transmission for glass, a subsurface wrap for wax-like plastics and wood) and its material (name and density, so document.measure weighs it). Any number given beside the preset changes it (the presets are data; their numbers are a starting point): {\"preset\": \"car-paint\", \"color\": \"#1f4fa0\"} is a blue car paint. The presets with a solid texture (oak, walnut, beech, marble, granite, carbon, cast-iron) fit it to the body: its scale, axis (the box's longest side; wood's tilted 0.06 towards the thinnest side) and origin (wood's pith beside the thinnest side) from the body's box, and the colour is the texture's mean; a `pattern` object beside the preset changes its fields one by one ({\"preset\": \"oak\", \"pattern\": {\"scale\": 3}}), null takes it away. One undo step; body.appearance and body.material change either half afterwards.",
        doc: true,
        material_preset_cmd
    ),
];

/// The parameters of the layers, as `body.appearance`, `face.appearance`
/// and `material.preset` take them.
pub(crate) const OPTICS_KEYS: [&str; 12] = [
    "anisotropy",
    "anisotropyAxis",
    "clearcoat",
    "clearcoatRoughness",
    "transmission",
    "ior",
    "thinWalled",
    "absorption",
    "absorptionDistance",
    "subsurface",
    "subsurfaceColor",
    "pattern",
];

/// The layers `p` gives, starting from `base`: each key present replaces
/// its number. Refused when one is out of its range (anisotropy −1 to 1,
/// the others 0 to 1, ior 1 to 3, absorptionDistance a positive number of
/// millimetres, anisotropyAxis a direction or `"parametric"`).
pub(crate) fn optics_param(p: &Params, base: Optics) -> Result<Optics, CommandError> {
    let id = p.command();
    let mut o = base;
    if let Some(a) = p.number("anisotropy")? {
        o.anisotropy = a;
    }
    if let Some(value) = p.get("anisotropyAxis") {
        // "parametric", as the appearance is said back: the face's own
        // direction again.
        if value.as_str() == Some("parametric") {
            o.direction = AnisotropyDirection::Parametric;
        } else {
            let axis = value
                .as_array()
                .filter(|a| a.len() == 3)
                .and_then(|a| {
                    let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
                    <[f64; 3]>::try_from(v).ok()
                })
                .ok_or_else(|| {
                    CommandError::new(format!(
                        "{id}: `anisotropyAxis` must be three numbers [x, y, z] or \"parametric\", got {value}"
                    ))
                })?;
            o.direction = AnisotropyDirection::Axis(axis);
        }
    }
    for (key, slot) in [
        ("clearcoat", &mut o.clearcoat),
        ("clearcoatRoughness", &mut o.clearcoat_roughness),
        ("transmission", &mut o.transmission),
        ("subsurface", &mut o.subsurface),
    ] {
        if let Some(v) = factor(p, key)? {
            *slot = v;
        }
    }
    if let Some(ior) = p.number("ior")? {
        o.ior = ior;
    }
    if let Some(thin) = p.bool("thinWalled")? {
        o.thin_walled = thin;
    }
    if let Some(value) = p.get("absorption") {
        o.absorption = Some(color_param(id, value)?);
    }
    if let Some(d) = p.number("absorptionDistance")? {
        o.absorption_distance = d;
    }
    if let Some(value) = p.get("subsurfaceColor") {
        o.subsurface_color = Some(color_param(id, value)?);
    }
    o.check()
        .map_err(|why| CommandError::new(format!("{id}: {why}")))?;
    Ok(o)
}

/// The layers that are not at their default, by their parameter names,
/// into `map`.
pub(crate) fn optics_into(map: &mut Map<String, Value>, o: &Optics) {
    let d = Optics::default();
    let hex = |[r, g, b]: [u8; 3]| format!("#{r:02x}{g:02x}{b:02x}");
    if o.anisotropy != d.anisotropy {
        map.insert("anisotropy".into(), o.anisotropy.into());
        map.insert(
            "anisotropyAxis".into(),
            match o.direction {
                AnisotropyDirection::Parametric => Value::from("parametric"),
                AnisotropyDirection::Axis(axis) => json!(axis),
            },
        );
    }
    for (key, v, dv) in [
        ("clearcoat", o.clearcoat, d.clearcoat),
        (
            "clearcoatRoughness",
            o.clearcoat_roughness,
            d.clearcoat_roughness,
        ),
        ("transmission", o.transmission, d.transmission),
        ("subsurface", o.subsurface, d.subsurface),
    ] {
        if v != dv {
            map.insert(key.into(), v.into());
        }
    }
    if o.transmission != 0.0 || o.ior != d.ior {
        map.insert("ior".into(), o.ior.into());
    }
    if o.thin_walled {
        map.insert("thinWalled".into(), true.into());
    }
    if let Some(c) = o.absorption {
        map.insert("absorption".into(), hex(c).into());
        map.insert("absorptionDistance".into(), o.absorption_distance.into());
    }
    if let Some(c) = o.subsurface_color {
        map.insert("subsurfaceColor".into(), hex(c).into());
    }
    if let Some(t) = &o.pattern {
        map.insert("pattern".into(), crate::solid_texture::pattern_json(t));
    }
}

/// The library, as data.
const PRESETS: &str = include_str!("material_presets.json");

/// The keys of a preset entry.
const PRESET_KEYS: [&str; 17] = [
    "preset",
    "label",
    "color",
    "metallic",
    "roughness",
    "material",
    "anisotropy",
    "anisotropyAxis",
    "clearcoat",
    "clearcoatRoughness",
    "transmission",
    "ior",
    "thinWalled",
    "absorption",
    "absorptionDistance",
    "subsurface",
    "pattern",
];

/// The presets' entries, in their order.
fn presets() -> Result<Vec<Map<String, Value>>, CommandError> {
    let value: Value = serde_json::from_str(PRESETS)
        .map_err(|e| CommandError::new(format!("the material presets do not read: {e}")))?;
    value
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e.as_object().cloned())
                .collect()
        })
        .ok_or_else(|| CommandError::new("the material presets are not a list"))
}

/// The appearance and the material of a preset entry (with any changes
/// merged into it), read as `command`'s parameters are.
fn preset_parts(
    command: &'static str,
    entry: &Map<String, Value>,
) -> Result<(Appearance, Material), CommandError> {
    let mut keys: Vec<&str> = PRESET_KEYS.to_vec();
    keys.extend(["subsurfaceColor", "body"]);
    let value = Value::Object(entry.clone());
    let p = Params::new(command, &value, &keys)?;
    let mut optics = optics_param(&p, Optics::default())?;
    // A preset's pattern is fitted to the body later (its scale, axis and
    // origin from the body's box, `fit`); here its numbers are checked.
    let pattern = match p.get("pattern") {
        Some(v) => crate::solid_texture::pattern_param(command, v, None)?,
        None => None,
    };
    optics.pattern = pattern.map(|(t, _)| t);
    let color = match (p.get("color"), &optics.pattern) {
        (Some(v), _) => color_param(command, v)?,
        (None, Some(t)) => crate::solid_texture::mean_color(t),
        (None, None) => return Err(p.missing("color")),
    };
    let appearance = Appearance {
        color,
        metallic: factor(&p, "metallic")?,
        roughness: factor(&p, "roughness")?,
        optics,
    };
    let material = entry
        .get("material")
        .and_then(|m| {
            Some(Material::new(
                m.get("name")?.as_str()?,
                m.get("density")?.as_f64()?,
            ))
        })
        .ok_or_else(|| {
            CommandError::new(format!(
                "{command}: the preset's material needs a name and a density"
            ))
        })?;
    material
        .check()
        .map_err(|why| CommandError::new(format!("{command}: {why}")))?;
    Ok((appearance, material))
}

fn preset_json(entry: &Map<String, Value>, appearance: &Appearance, material: &Material) -> Value {
    let mut out = appearance_json(appearance);
    if let Some(map) = out.as_object_mut() {
        if let Some(name) = entry.get("preset") {
            map.insert("preset".into(), name.clone());
        }
        if let Some(label) = entry.get("label") {
            map.insert("label".into(), label.clone());
        }
        map.insert(
            "material".into(),
            json!({ "name": material.name, "density": material.density }),
        );
    }
    out
}

fn material_presets_cmd(_session: &mut Session, params: &Value) -> Run {
    const ID: &str = "material.presets";
    Params::new(ID, params, &[])?;
    let mut out = Vec::new();
    for entry in presets()? {
        let (appearance, material) = preset_parts(ID, &entry)?;
        out.push(preset_json(&entry, &appearance, &material));
    }
    Ok(json!({ "presets": out }))
}

fn material_preset_cmd(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "material.preset";
    let mut keys = vec!["body", "preset", "color", "metallic", "roughness"];
    keys.extend(OPTICS_KEYS);
    let p = Params::new(ID, params, &keys)?;
    let name = p.required_str("preset")?;
    let entries = presets()?;
    let Some(entry) = entries
        .iter()
        .find(|e| e.get("preset").and_then(Value::as_str) == Some(name))
    else {
        let names: Vec<&str> = entries
            .iter()
            .filter_map(|e| e.get("preset").and_then(Value::as_str))
            .collect();
        return fail(format!(
            "{ID}: there is no preset `{name}`; the presets are {}",
            names.join(", ")
        ));
    };
    // The changes given replace the preset's numbers; a pattern's fields
    // replace the preset's pattern's one by one (`null` takes it away).
    let mut merged = entry.clone();
    if let Some(given) = params.as_object() {
        for (key, value) in given {
            if key == "pattern"
                && let (Some(Value::Object(base)), Value::Object(changes)) =
                    (merged.get_mut("pattern"), value)
            {
                for (k, v) in changes {
                    base.insert(k.clone(), v.clone());
                }
            } else if key != "body" && key != "preset" {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    if merged.get("pattern").is_some_and(Value::is_null) {
        merged.remove("pattern");
    }
    let (mut appearance, material) = preset_parts(ID, &merged)?;
    let id = {
        let doc = session.document()?.history.current();
        body_ref(doc, &p, ID)?
    };
    if let Some(Value::Object(_)) = merged.get("pattern") {
        // Fitted to the body: what the preset (and the changes) leave out.
        let scale_given = merged.get("pattern").and_then(|v| v.get("scale")).is_some();
        if let Some(t) = appearance.optics.pattern {
            let open = session.document_mut()?;
            let regen = open.regen();
            let body = regen.feature(id).and_then(|r| r.body.as_deref());
            appearance.optics.pattern =
                Some(crate::solid_texture::resolve(ID, t, scale_given, body)?);
        }
    }
    let body = BodyAppearance::from(appearance);
    let label = format!("Preset {name} on {id}");
    edit(session, ID, &label, |doc| {
        doc.set_body_appearance(id, Some(body.clone()))?;
        doc.set_material(id, Some(material.clone()))
    })?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let values = Values::new(doc);
    Ok(json!({
        "body": id.to_string(),
        "preset": name,
        "appearance": body_appearance(doc, &regen, id).map_or(Value::Null, |a| appearance_json(&a)),
        "material": described_material(&values, &material),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_reads_and_is_in_range() {
        let entries = presets().unwrap();
        let names: Vec<&str> = entries
            .iter()
            .filter_map(|e| e.get("preset").and_then(Value::as_str))
            .collect();
        assert_eq!(
            names,
            [
                "steel",
                "aluminium-brushed",
                "abs",
                "glass",
                "car-paint",
                "wood",
                "oak",
                "walnut",
                "beech",
                "marble",
                "granite",
                "carbon",
                "cast-iron"
            ]
        );
        for entry in &entries {
            let (appearance, material) = preset_parts("material.presets", entry).unwrap();
            appearance.check().unwrap();
            material.check().unwrap();
        }
    }
}
