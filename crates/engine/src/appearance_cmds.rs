//! A colour and a material per body, a colour per face:
//! `body.appearance`, `body.material` and `face.appearance`, and what
//! `document.inspect` and `document.measure` say of them (the mass and
//! the inertia tensor of a body with a density).
//!
//! The body is given by the feature that makes it, a face by that and its
//! persistent name there; the entries are kept in the document beside the
//! history (`Document::bodies`, `Document::faces`) and reach the bodies
//! and faces made from them by the rule of `appearance`.

use crate::appearance::{
    Values, body_appearance, body_appearance_in, body_material, face_appearances, lost_faces,
};
use crate::commands::{Run, edit, fail, feature_ref, spec};
use crate::mass::mass_and_inertia_of;
use crate::params::Params;
use crate::selection::{MAX_SELECTION, Resolver, SelectionKind, feature_named};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    Appearance, BodyAppearance, BodyColor, Document, Expr, FeatureId, FeatureKind, LengthUnit,
    Material, PersistentName,
};
use craft3d_regen::Regen;
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("body.appearance", "Body Colour", r##"{"body": id or name, "color": "#rrggbb" or [r, g, b] (each a number or an expression), "metallic": number, "roughness": number, "anisotropy": number, "anisotropyAxis": [x, y, z] or "parametric", "clearcoat": number, "clearcoatRoughness": number, "transmission": number, "ior": number, "thinWalled": boolean, "absorption": "#rrggbb" or [r, g, b], "absorptionDistance": number (mm), "subsurface": number, "subsurfaceColor": "#rrggbb" or [r, g, b], "pattern": {kind: "wood|marble|granite|speckle|noise|brushed|carbon", colors: [2 to 4 colours] = the kind's, scale: number = fitted to the body, axis: [x, y, z] = fitted, origin: [x, y, z] = fitted, seed: integer = 0, distortion: 0 to 8, detail: octaves 1 to 6 = 4, grain: 0 to 1, rays: 0 to 1, spot: 0 to 1 = 0.5, wrap: boolean = true, alongAxis: boolean, roughness: [at the first colour, at the last] = the appearance's} or null, "clear": boolean = false}"##,
        "Gives the body of a feature a colour (sRGB: \"#c8102e\" or [200, 16, 46], each 0 to 255; in [r, g, b] each may also be an expression of the variables that gives a plain number from 0 to 255, rounded, such as [\"woods[wood].red\", \"if(dark, 40, 90)\", 60], and the colour then follows them: a Jig's choice recolours the body; refused when they do not evaluate now, and a colour whose expressions stop evaluating draws the body without it, which document.check and document.inspect (colorError) say), and optionally metallic and roughness (0 to 1), and the layers the studio look shades beyond them (docs/guide: Materials): anisotropy (−1 to 1; the highlight stretched along the face's own u direction, around the axis of a turned face, or along anisotropyAxis; \"parametric\" there is the face's own direction again, as the appearance is said back), clearcoat and clearcoatRoughness (0 to 1: a clear layer over the colour, car paint), transmission (0 to 1: glass and clear plastic, with ior 1 to 3, default 1.5, thinWalled for a sheet, absorption the colour light takes after absorptionDistance mm inside, default 10), subsurface (0 to 1, a wrap of the light past the shadow's edge for wax, skin or wood, tinted by subsurfaceColor), and pattern, a solid texture the studio look evaluates at each point of the body in the model's space, so that a cut shows it running through (docs/guide: Materials): its kind, a ramp of 2 to 4 colours (defaults per kind), its scale (the size of a ring, vein, cell or tow in model units), axis and origin (left out: fitted to the body's box, the axis along its longest side), seed, distortion (ring wobble, marble's turbulence, 0 to 8), detail (octaves, 1 to 6), grain and rays (wood's streaks and medullary rays, 0 to 1), spot (speckle's dot, 0 to 1), wrap (carbon: around the axis for a tube, else across it for a plate), alongAxis (the anisotropy along the axis) and roughness at the ramp's ends; without a color the appearance takes the texture's mean colour, which the plain look and the OBJ, 3MF and STEP exports show. material.preset gives a ready set. The body keeps it through every regeneration and edit; bodies made from it keep it too: a boolean's result has its target's colour; the faces a tool cut take the target's colour too, while the faces a union or an intersection takes from a coloured tool keep the tool's; set a colour on the later feature to colour its whole body. Given to a feature that is switched off (suppressed, or inactive by its condition), it colours the body that feature passes on when that body has no colour of its own; a colour of a feature that is on whose expressions stop evaluating gives the body no colour (not another feature's). view.render, view.replay, the viewport and 3MF exports show it. clear takes the colour away (the theme's again).", doc: true, body_appearance_cmd),
    spec!("body.material", "Body Material", r#"{"body": id or name, "name": string, "density": number (kg/m³), "7.85 g/cm3" or an expression ("sheets[material].density g/cm3"), "clear": boolean = false}"#,
        "Gives the body of a feature a material: a name and a density, a number in kg/m³ or a string with a unit (kg/m3, g/cm3, g/mm3, lb/in3, lb/ft3), or an expression of the variables that gives a plain number, in kg/m³ or in the unit after it (\"sheets[material].density g/cm3\", \"if(pla, 1240, 700)\"): the density then follows them (a Jig's choice of material changes the mass); refused when it does not evaluate now to a positive number, and a density that stops evaluating gives no mass, which document.check and document.inspect (densityError) say. document.inspect and document.measure then give the body's mass (kg) and its inertia tensor about the centroid (kg times the document's length unit squared), computed exactly from the B-rep; body.material itself measures nothing and says which bodies have the material. Bodies made from it keep it, as with body.appearance (a boolean's result has its target's). Given to a feature that is switched off (suppressed, or inactive by its condition), it weighs the body that feature passes on when that body has no material of its own. clear takes it away.", doc: true, body_material_cmd),
    spec!("face.appearance", "Face Colour", r##"{"faces": [persistent name], "body": id or name = the body that has the face, "selection": boolean = false, "color": "#rrggbb" or [r, g, b], "metallic": number, "roughness": number, "anisotropy": number, "anisotropyAxis": [x, y, z] or "parametric", "clearcoat": number, "clearcoatRoughness": number, "transmission": number, "ior": number, "thinWalled": boolean, "absorption": "#rrggbb" or [r, g, b], "absorptionDistance": number (mm), "subsurface": number, "subsurfaceColor": "#rrggbb" or [r, g, b], "pattern": {kind: "wood|marble|granite|speckle|noise|brushed|carbon", colors: [2 to 4 colours] = the kind's, scale: number = fitted to the body, axis: [x, y, z] = fitted, origin: [x, y, z] = fitted, seed: integer = 0, distortion: 0 to 8, detail: octaves 1 to 6 = 4, grain: 0 to 1, rays: 0 to 1, spot: 0 to 1 = 0.5, wrap: boolean = true, alongAxis: boolean, roughness: [at the first colour, at the last] = the appearance's} or null, "clear": boolean = false}"##,
        "Gives faces a colour of their own (sRGB: \"#2f6fd6\" or [47, 111, 214]), and optionally metallic and roughness (0 to 1) and the layers of body.appearance (anisotropy, clear coat, transmission, subsurface, pattern; a pattern's defaults are fitted to the box of the first face's body, and without a color the faces take its mean colour). The faces are given by their persistent names (as document.inspect, select.get or ui.pick give them; body says whose body when the name alone does not), or selection: true takes the faces of the current selection (select.get): select a face, then colour it. The colour is kept in the document by the body's feature and the face's name there, so it survives every regeneration and every upstream edit that keeps the face; the faces later features make of it (the copies of a pattern, a transform or a boolean, the pieces of a face cut in two) take it too, by the rule of body.appearance (the wall a tool cut has the target's colour). It goes before the body's colour at the same feature; a colour set with body.appearance on a strictly later feature's body goes before it (that body is then all in that colour, as for any earlier colour), and a face colour on that later body before that again. A face that several faces were merged into (aliases) has the colour of its own name, else of its aliases in their order. A face an edit takes away keeps its entry, colours nothing and is listed under lostFaces here and in document.inspect (undo brings the face and its colour back); it never moves to another face. clear takes the colour away (also a lost face's, by its name and body). The viewport, view.render, view.replay and the 3MF and STEP exports show it. One undo step.", doc: true, names: ["faces"], face_appearance_cmd),
];

/// The feature of `key`, which must be one that can make a body.
pub(crate) fn body_ref(
    doc: &Document,
    p: &Params,
    command: &str,
) -> Result<FeatureId, CommandError> {
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
pub(crate) fn color_param(command: &str, value: &Value) -> Result<[u8; 3], CommandError> {
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

/// A body's colour: as [`color_param`] gives it, or `[r, g, b]` with
/// expressions of the variables among them (each a plain number from 0
/// to 255), which the colour then follows.
fn body_color_param(command: &str, value: &Value) -> Result<BodyColor, CommandError> {
    let Value::Array(items) = value else {
        return color_param(command, value).map(BodyColor::Fixed);
    };
    if !items.iter().any(Value::is_string) {
        return color_param(command, value).map(BodyColor::Fixed);
    }
    let bad = || {
        CommandError::new(format!(
            "{command}: `color` with expressions must be [r, g, b], each a number from 0 to 255 or an expression that gives one (\"woods[wood].red\"), got {value}"
        ))
    };
    let mut exprs = Vec::with_capacity(3);
    for item in items {
        match item {
            Value::String(text) if !text.trim().is_empty() => exprs.push(Expr::new(text.trim())),
            Value::Number(n) => exprs.push(Expr::new(n.to_string())),
            _ => return Err(bad()),
        }
    }
    let channels: [Expr; 3] = exprs.try_into().map_err(|_| bad())?;
    Ok(BodyColor::Exprs(Box::new(channels)))
}

/// A factor from 0 to 1.
pub(crate) fn factor(p: &Params, key: &str) -> Result<Option<f64>, CommandError> {
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

/// A density: a plain number in kg/m³, a number with a unit, or an
/// expression of the variables (a plain number, in kg/m³ or in the unit
/// that follows it), as a [`Material`] keeps it: its density, and the
/// expression with the density as the kg/m³ per unit of its value.
fn density_param(command: &str, value: &Value) -> Result<(f64, Option<Expr>), CommandError> {
    let bad = |why: &str| {
        CommandError::new(format!(
            "{command}: `density` {why}; give a number in kg/m³ (7850), one with a unit (\"7.85 g/cm3\"; units: kg/m3, g/cm3, g/mm3, lb/in3, lb/ft3) or an expression of the variables that gives a plain number, with or without such a unit (\"sheets[material].density\", \"if(pla, 1.24, 0.7) g/cm3\"), got {value}"
        ))
    };
    let (density, expr) = match value {
        Value::Number(n) => (n.as_f64().ok_or_else(|| bad("is not a number"))?, None),
        Value::String(text) => {
            let text = text.trim();
            let (number, per) = match DENSITY_UNITS.iter().find(|(unit, _)| text.ends_with(unit)) {
                Some((unit, per)) => (text.get(..text.len() - unit.len()).unwrap_or(""), *per),
                None => (text, 1.0),
            };
            let number = number.trim();
            if number.is_empty() {
                return Err(bad("has no number before its unit"));
            }
            match number.parse::<f64>() {
                Ok(n) => (n * per, None),
                Err(_) => (per, Some(Expr::new(number))),
            }
        }
        _ => return Err(bad("is neither a number nor a string")),
    };
    if !(density.is_finite() && density > 0.0) {
        return Err(bad("must be positive"));
    }
    Ok((density, expr))
}

/// A body's appearance as given: as [`appearance_json`] with its colour
/// now, and with expressions also those (`colorExprs`), and the reason
/// instead of the colour when they do not evaluate.
fn body_appearance_json(values: &Values, appearance: &BodyAppearance) -> Value {
    let values = match appearance.color {
        BodyColor::Exprs(_) => values.get(),
        BodyColor::Fixed(_) => None,
    };
    let mut out = match appearance.resolve(values) {
        Ok(resolved) => appearance_json(&resolved),
        Err(why) => json!({ "colorError": why }),
    };
    if let (Some(map), BodyColor::Exprs(exprs)) = (out.as_object_mut(), &appearance.color) {
        let sources: Vec<&str> = exprs.iter().map(Expr::source).collect();
        map.insert("colorExprs".into(), json!(sources));
    }
    out
}

pub(crate) fn appearance_json(appearance: &Appearance) -> Value {
    let mut out = json!({ "color": appearance.hex() });
    if let Some(map) = out.as_object_mut() {
        if let Some(m) = appearance.metallic {
            map.insert("metallic".into(), m.into());
        }
        if let Some(r) = appearance.roughness {
            map.insert("roughness".into(), r.into());
        }
        crate::material_cmds::optics_into(map, &appearance.optics);
    }
    out
}

fn body_appearance_cmd(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "body.appearance";
    let mut keys = vec!["body", "color", "metallic", "roughness", "clear"];
    keys.extend(crate::material_cmds::OPTICS_KEYS);
    let p = Params::new(ID, params, &keys)?;
    let id = {
        let doc = session.document()?.history.current();
        body_ref(doc, &p, ID)?
    };
    let clear = p.bool("clear")?.unwrap_or(false);
    let pattern = if clear {
        None
    } else {
        crate::solid_texture::given_pattern(session, &p, id, None)?.flatten()
    };
    let doc = session.document()?.history.current();
    let appearance = if clear {
        if let Some(key) = ["color", "metallic", "roughness"]
            .into_iter()
            .chain(crate::material_cmds::OPTICS_KEYS)
            .find(|key| p.has(key))
        {
            return fail(format!(
                "{ID}: `clear` takes the colour away; leave out `{key}`"
            ));
        }
        None
    } else {
        let mut optics = crate::material_cmds::optics_param(&p, craft3d_doc::Optics::default())?;
        optics.pattern = pattern;
        // A solid texture without a colour: its mean, which formats that
        // cannot hold the texture export.
        let color = match (p.get("color"), &pattern) {
            (Some(value), _) => body_color_param(ID, value)?,
            (None, Some(t)) => BodyColor::Fixed(crate::solid_texture::mean_color(t)),
            (None, None) => return Err(p.missing("color")),
        };
        let appearance = BodyAppearance {
            color,
            metallic: factor(&p, "metallic")?,
            roughness: factor(&p, "roughness")?,
            optics,
        };
        // Expressions are checked against the variables as they are: a
        // colour that does not evaluate is refused with the reason.
        if matches!(appearance.color, BodyColor::Exprs(_)) {
            appearance
                .resolve(crate::appearance::evaluation(doc).as_ref())
                .map_err(|why| CommandError::new(format!("{ID}: `color`: {why}")))?;
        }
        Some(appearance)
    };
    let label = format!("{} colour of {id}", if clear { "Clear" } else { "Set" });
    edit(session, ID, &label, |doc| {
        doc.set_body_appearance(id, appearance.clone())
    })?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    Ok(json!({
        "body": id.to_string(),
        "appearance": appearance.as_ref().map_or(Value::Null, |a| body_appearance_json(&Values::new(doc), a)),
        "bodies": shown_bodies(doc, &regen, |doc, regen, at| {
            body_appearance(doc, regen, at).map(|a| appearance_json(&a))
        }),
    }))
}

/// The faces `face.appearance` names, each as given: the body when it was
/// said, and the name.
type Wanted = Vec<(Option<String>, String)>;

/// The faces `face.appearance` is about, from `faces` (and `body`) or from
/// the selection, and what of the selection was not a face.
fn wanted_faces(session: &mut Session, p: &Params) -> Result<(Wanted, Vec<String>), CommandError> {
    const ID: &str = "face.appearance";
    let mut skipped = Vec::new();
    if p.bool("selection")?.unwrap_or(false) {
        if let Some(key) = ["faces", "body"].into_iter().find(|key| p.has(key)) {
            return fail(format!(
                "{ID}: `selection` takes the selection's faces; leave out `{key}`"
            ));
        }
        let mut faces = Vec::new();
        for item in session.checked_selection().items {
            if item.kind == SelectionKind::Face {
                faces.push((Some(item.feature), item.name));
            } else {
                skipped.push(format!("{} {}", item.kind.name(), item.name));
            }
        }
        if faces.is_empty() {
            let holds = if skipped.is_empty() {
                "it is empty".to_string()
            } else {
                format!("it holds {}", skipped.join(", "))
            };
            return fail(format!(
                "{ID}: the selection has no faces ({holds}); select a face first (in the window, or with select.set)"
            ));
        }
        return Ok((faces, skipped));
    }
    let body = match p.get("body") {
        None => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(other) => {
            return fail(format!(
                "{ID}: `body` must be a feature id or name, got {other}"
            ));
        }
    };
    let names = match p.get("faces") {
        Some(Value::Array(items)) if !items.is_empty() => items,
        Some(Value::Array(_)) => return fail(format!("{ID}: `faces` is empty")),
        Some(other) => {
            return fail(format!(
                "{ID}: `faces` must be a list of persistent names ([\"F3:face(1)\"]), got {other}"
            ));
        }
        None => {
            return fail(format!(
                "{ID}: give `faces` (a list of persistent names) or `selection`: true"
            ));
        }
    };
    if names.len() > MAX_SELECTION {
        return fail(format!(
            "{ID}: at most {MAX_SELECTION} faces at once, got {}",
            names.len()
        ));
    }
    let mut faces = Vec::with_capacity(names.len());
    for name in names {
        let Some(text) = name.as_str() else {
            return fail(format!(
                "{ID}: `faces` must be a list of persistent names, got {name}"
            ));
        };
        faces.push((body.clone(), text.to_string()));
    }
    Ok((faces, skipped))
}

/// The face of `wanted` in the document: the body's feature and the face's
/// own name there (an alias read as the face's name). With `clear`, a face
/// that is gone is found by the entry of its colour.
fn find_face(
    doc: &Document,
    resolver: &mut Resolver<'_>,
    (body, name): (Option<&str>, &str),
    clear: bool,
) -> Result<(FeatureId, PersistentName), CommandError> {
    const ID: &str = "face.appearance";
    let shown: String = name.chars().take(80).collect();
    match resolver.resolve(Some(SelectionKind::Face), name, body) {
        Ok(found) => {
            let feature = feature_named(doc, &found.item.feature).ok_or_else(|| {
                CommandError::new(format!(
                    "{ID}: `{shown}`: no feature {}",
                    found.item.feature
                ))
            })?;
            let face = found
                .item
                .name
                .parse::<PersistentName>()
                .map_err(|e| CommandError::new(format!("{ID}: `{shown}`: {e}")))?;
            Ok((feature, face))
        }
        Err(why) => {
            let lost = clear
                .then(|| name.parse::<PersistentName>().ok())
                .flatten()
                .and_then(|parsed| {
                    let parsed = parsed.canonical();
                    let only = body.and_then(|b| feature_named(doc, b));
                    let mut owners = doc
                        .faces
                        .iter()
                        .filter(|e| e.face == parsed && only.is_none_or(|o| o == e.feature))
                        .map(|e| e.feature);
                    let first = owners.next()?;
                    owners.next().is_none().then_some((first, parsed))
                });
            lost.ok_or_else(|| CommandError::new(format!("{ID}: {why}")))
        }
    }
}

/// The colours of a body's faces and its faces by every name they answer
/// to, for `face.appearance`'s reply.
type Shown<'a> = (Vec<Option<Appearance>>, BTreeMap<&'a PersistentName, usize>);

fn face_appearance_cmd(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "face.appearance";
    let p = Params::new(
        ID,
        params,
        &[
            "faces",
            "body",
            "selection",
            "color",
            "metallic",
            "roughness",
            "clear",
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
        ],
    )?;
    session.document()?;
    let clear = p.bool("clear")?.unwrap_or(false);
    let appearance = if clear {
        if let Some(key) = ["color", "metallic", "roughness"]
            .into_iter()
            .chain(crate::material_cmds::OPTICS_KEYS)
            .find(|key| p.has(key))
        {
            return fail(format!(
                "{ID}: `clear` takes the colour away; leave out `{key}`"
            ));
        }
        None
    } else {
        Some(Appearance {
            color: [0; 3],
            metallic: factor(&p, "metallic")?,
            roughness: factor(&p, "roughness")?,
            optics: crate::material_cmds::optics_param(&p, craft3d_doc::Optics::default())?,
        })
    };
    let (wanted, skipped) = wanted_faces(session, &p)?;
    let faces = {
        let open = session.document_mut()?;
        let regen = open.regen();
        let doc = open.history.current();
        let mut resolver = Resolver::new(doc, &regen);
        let mut faces: Vec<(FeatureId, PersistentName)> = Vec::with_capacity(wanted.len());
        let mut seen = std::collections::BTreeSet::new();
        for (body, name) in &wanted {
            let found = find_face(doc, &mut resolver, (body.as_deref(), name), clear)?;
            if seen.insert(found.clone()) {
                faces.push(found);
            }
        }
        faces
    };
    // The colour, and a solid texture fitted to the body of the first face
    // (its mean colour when no colour is given).
    let appearance = match appearance {
        None => None,
        Some(mut a) => {
            let pattern = match faces.first() {
                Some((feature, _)) => {
                    crate::solid_texture::given_pattern(session, &p, *feature, None)?.flatten()
                }
                None => None,
            };
            a.optics.pattern = pattern;
            a.color = match (p.get("color"), &pattern) {
                (Some(value), _) => color_param(ID, value)?,
                (None, Some(t)) => crate::solid_texture::mean_color(t),
                (None, None) => return Err(p.missing("color")),
            };
            Some(a)
        }
    };
    let label = match (clear, faces.as_slice()) {
        (false, [(_, one)]) => format!("Colour face {one}"),
        (true, [(_, one)]) => format!("Clear colour of face {one}"),
        (false, many) => format!("Colour {} faces", many.len()),
        (true, many) => format!("Clear colour of {} faces", many.len()),
    };
    let to_set = faces.clone();
    edit(session, ID, &label, move |doc| {
        for (feature, face) in to_set {
            doc.set_face_appearance(feature, face, appearance)?;
        }
        Ok(())
    })?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    // What each face shows now (its own colour, or, cleared, what it
    // takes from elsewhere): the colours of a body found once, and its
    // faces by name once.
    let mut bodies: BTreeMap<FeatureId, Option<Shown<'_>>> = BTreeMap::new();
    let mut listed: Vec<Value> = Vec::with_capacity(faces.len());
    for (feature, face) in &faces {
        let shown = bodies
            .entry(*feature)
            .or_insert_with(|| {
                let result = regen.feature(*feature)?;
                let (body, names) = (result.body.as_ref()?, result.names.as_deref()?);
                let mut by_name = BTreeMap::new();
                for index in 0..names.faces.len() {
                    for name in names.names_of(index) {
                        by_name.entry(name).or_insert(index);
                    }
                }
                Some((
                    face_appearances(doc, &regen, *feature, body.faces.len()),
                    by_name,
                ))
            })
            .as_ref()
            .and_then(|(all, by_name)| all.get(*by_name.get(face)?).copied())
            .flatten();
        listed.push(json!({
            "feature": feature.to_string(),
            "face": face.to_string(),
            "shown": shown.as_ref().map_or(Value::Null, appearance_json),
        }));
    }
    let mut out = json!({
        "faces": listed,
        "appearance": appearance.as_ref().map_or(Value::Null, appearance_json),
        "faceAppearances": doc.faces.len(),
    });
    if let Some(map) = out.as_object_mut() {
        let lost = lost_faces_json(doc, &regen);
        if !lost.is_empty() {
            map.insert("lostFaces".into(), lost.into());
        }
        if !skipped.is_empty() {
            map.insert("skipped".into(), skipped.into());
        }
    }
    Ok(out)
}

/// The face colours whose faces are gone, for `face.appearance` and
/// `document.inspect`: one entry each, with a sentence.
pub(crate) fn lost_faces_json(doc: &Document, regen: &Regen) -> Vec<Value> {
    lost_faces(doc, regen)
        .into_iter()
        .map(|lost| {
            let made = doc
                .feature(lost.feature)
                .map_or_else(|| lost.feature.to_string(), |f| f.name.clone());
            json!({
                "feature": lost.feature.to_string(),
                "face": lost.face.to_string(),
                "notice": format!(
                    "the body of `{made}` no longer has a face {}: its colour is kept for when the face comes back (undo) and colours no other face; face.appearance with clear takes it away",
                    lost.face
                ),
            })
        })
        .collect()
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
        let (density, density_expr) =
            density_param(ID, p.get("density").ok_or_else(|| p.missing("density"))?)?;
        let material = Material {
            name,
            density,
            density_expr,
        };
        // An expression is checked against the variables as they are: one
        // that does not evaluate is refused with the reason.
        if material.density_expr.is_some() {
            crate::appearance::density(doc, &material)
                .map_err(|why| CommandError::new(format!("{ID}: `density`: {why}")))?;
        }
        Some(material)
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
    let values = Values::new(doc);
    let bodies: Vec<Value> = regen
        .bodies()
        .filter_map(|(at, _)| {
            let described = material_json(&values, &regen, at)?;
            Some(json!({ "feature": at.to_string(), "material": described }))
        })
        .collect();
    Ok(json!({
        "body": id.to_string(),
        "material": material.as_ref().map_or(Value::Null, |m| described_material(&values, m)),
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
pub(crate) fn metres_per_unit(unit: LengthUnit) -> f64 {
    match unit {
        LengthUnit::Micrometre => 1e-6,
        LengthUnit::Millimetre => 1e-3,
        LengthUnit::Centimetre => 1e-2,
        LengthUnit::Metre => 1.0,
        LengthUnit::Inch => 0.0254,
        LengthUnit::Foot => 0.3048,
    }
}

/// A material as the commands show it: its name and its density in kg/m³
/// now; with an expression also that (`densityExpr`, with the unit it is
/// in when that is not kg/m³: `densityUnit`, kg/m³ per unit of it), and
/// the reason instead of the density when it does not evaluate.
pub(crate) fn described_material(values: &Values, material: &Material) -> Value {
    let mut described = json!({ "name": material.name });
    if let Some(map) = described.as_object_mut() {
        match crate::appearance::density_in(values, material) {
            Ok(density) => map.insert("density".into(), density.into()),
            Err(why) => map.insert("densityError".into(), why.into()),
        };
        if let Some(expr) = &material.density_expr {
            map.insert("densityExpr".into(), expr.source().into());
            if material.density != 1.0 {
                map.insert("densityUnit".into(), material.density.into());
            }
        }
    }
    described
}

/// The material of the body of `at`: its name and density, and the
/// feature it has them from when that is another.
fn material_json(values: &Values, regen: &Regen, at: FeatureId) -> Option<Value> {
    let (from, material) = body_material(values.doc(), regen, at)?;
    let mut described = described_material(values, &material);
    if let (Some(map), true) = (described.as_object_mut(), from != at) {
        map.insert("from".into(), from.to_string().into());
    }
    // Given to a feature that is off: it weighs this body only because
    // nothing that is on gives it a material (docs/design/document.md 4.2).
    if let (Some(map), true) = (
        described.as_object_mut(),
        regen.feature(from).is_some_and(|r| r.is_off()),
    ) {
        map.insert("off".into(), true.into());
    }
    Some(described)
}

/// Measures the body of `at` for [`physical_fields`] ahead of its mass
/// properties: with a material its mass and inertia are measured at once,
/// in about the time of the inertia alone, which a measure of its mass
/// properties first would not leave room for.
///
/// Only ahead: a body whose mass properties are known already is left as
/// it is, its inertia measured when it is asked for. Measured now it
/// would integrate every face again, also for a command that only needs
/// the mass properties (`document.check`).
pub(crate) fn measure_ahead(
    doc: &Document,
    regen: &Regen,
    at: FeatureId,
    body: &Arc<Body>,
    inertia: bool,
) {
    if inertia && crate::mass::mass_known(body).is_none() && body_material(doc, regen, at).is_some()
    {
        let _measured = mass_and_inertia_of(body);
    }
}

/// [`measure_ahead`] for each of `bodies`, in their order, and the mass
/// properties of the runs of those without a material between them
/// measured together (`crate::mass::measure_ahead`): ahead of a command
/// that asks for every body's measure one by one. Nothing after the first
/// body that cannot be measured is measured, as a command that stops there
/// would leave it.
pub(crate) fn measure_all_ahead<'a>(
    doc: &Document,
    regen: &Regen,
    bodies: impl IntoIterator<Item = (FeatureId, &'a Arc<Body>)>,
    inertia: bool,
) {
    let mut plain = Vec::new();
    for (at, body) in bodies {
        // Without the inertia a body with a material is measured as one
        // without: its mass is its volume times its density.
        if inertia && body_material(doc, regen, at).is_some() {
            if !crate::mass::measure_ahead(&plain) {
                return;
            }
            plain.clear();
            // Known already: its inertia is left until asked for
            // ([`measure_ahead`]).
            let measured = match crate::mass::mass_known(body) {
                Some(known) => known,
                None => mass_and_inertia_of(body).0,
            };
            if measured.is_err() {
                return;
            }
        } else {
            plain.push(body);
        }
    }
    crate::mass::measure_ahead(&plain);
}

/// Adds to a body's entry of `document.inspect` or `document.measure`
/// its appearance and material (and from which feature it has them,
/// when that is another), and with a material its mass (kg) and inertia
/// tensor about the centroid (kg · unit²); without `measured` only the
/// appearance and the material, nothing that takes measuring the body;
/// without `inertia` the mass alone (from the body's remembered measure,
/// its volume times the density), never the inertia's integration.
pub(crate) fn physical_fields(
    entry: &mut Map<String, Value>,
    values: &Values,
    regen: &Regen,
    (at, body): (FeatureId, &Arc<Body>),
    unit: &str,
    measured: bool,
    inertia: bool,
) {
    let doc = values.doc();
    // A colour of its own that follows the variables is shown with its
    // expressions, and with the reason when they do not evaluate.
    let own_exprs = doc
        .body_properties(at)
        .filter(|_| !regen.feature(at).is_some_and(|r| r.is_off()))
        .and_then(|p| p.appearance.as_ref())
        .filter(|a| matches!(a.color, BodyColor::Exprs(_)));
    if let Some(own) = own_exprs {
        entry.insert("appearance".into(), body_appearance_json(values, own));
    } else if let Some(appearance) = body_appearance_in(values, regen, at) {
        entry.insert("appearance".into(), appearance_json(&appearance));
    }
    let (Some((_, material)), Some(described)) = (
        body_material(doc, regen, at),
        material_json(values, regen, at),
    ) else {
        return;
    };
    entry.insert("material".into(), described);
    if !measured {
        return;
    }
    let density = match crate::appearance::density_in(values, &material) {
        Ok(density) => density,
        Err(why) => {
            entry.insert("massError".into(), why.into());
            return;
        }
    };
    let m = metres_per_unit(doc.units.length);
    let per_volume = density * m * m * m;
    let (mass, inertia) = if inertia {
        let (mass, turned) = mass_and_inertia_of(body);
        (mass, Some(turned))
    } else {
        (crate::mass::mass_of(body), None)
    };
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
        None => {}
        Some(Ok(inertia)) => {
            let tensor: Vec<Vec<f64>> = inertia
                .tensor
                .iter()
                .map(|row| row.iter().map(|x| x * per_volume).collect())
                .collect();
            entry.insert("inertia".into(), json!(tensor));
            entry.insert("inertiaUnit".into(), format!("kg {unit}^2").into());
        }
        Some(Err(e)) => {
            entry.insert("inertiaError".into(), e.to_string().into());
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::{Session, Workspace};
    use serde_json::json;

    /// `inertia: false` (2026-11-05, the window's Bodies tab): the mass,
    /// the same as with the inertia, without the inertia and without
    /// integrating it; asked for afterwards, it is integrated then.
    #[test]
    fn without_the_inertia_the_mass_is_given_and_the_inertia_not_integrated() {
        let mut s = Session::new(Workspace::none());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            (
                "feature.sphere",
                json!({"name": "Ball", "position": [0, 0, 0], "radius": 10}),
            ),
            (
                "body.material",
                json!({"body": "Ball", "name": "Steel", "density": 7850}),
            ),
        ] {
            s.execute(id, &params).unwrap();
        }
        let inertias = crate::mass::fresh_inertias;
        let before = inertias();
        let quick = s
            .execute("document.inspect", &json!({"inertia": false}))
            .unwrap();
        let measured = s
            .execute(
                "document.measure",
                &json!({"feature": "Ball", "inertia": false}),
            )
            .unwrap();
        assert_eq!(inertias(), before, "no inertia integrated");
        let body = &quick["bodies"][0];
        assert!(
            body.get("inertia").is_none() && body.get("inertiaUnit").is_none(),
            "{body}"
        );
        assert!(measured["bodies"][0].get("inertia").is_none(), "{measured}");
        let full = s.execute("document.inspect", &json!({})).unwrap();
        assert_eq!(inertias(), before + 1, "asked for: integrated once");
        let (Some(a), Some(b)) = (body["mass"].as_f64(), full["bodies"][0]["mass"].as_f64()) else {
            panic!("{quick} {full}");
        };
        // Bit for bit: the same volume times the same density.
        assert_eq!(a.to_bits(), b.to_bits(), "{a} {b}");
        assert!(full["bodies"][0]["inertia"].is_array());
        // A wrong kind of value is refused, never a panic.
        assert!(
            s.execute("document.inspect", &json!({"inertia": "no"}))
                .is_err()
        );
        assert!(
            s.execute("document.measure", &json!({"inertia": 1}))
                .is_err()
        );
    }

    /// A body measured before it had a material is not integrated again
    /// for its inertia by a command that needs only its mass properties
    /// (`document.check`); `document.inspect`, which gives the inertia,
    /// measures it then, once.
    #[test]
    fn inertia_is_measured_when_asked_for_not_ahead_of_a_known_mass() {
        let mut s = Session::new(Workspace::none());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            (
                "feature.sphere",
                json!({"name": "Ball", "position": [0, 0, 0], "radius": 10}),
            ),
            ("document.measure", json!({})),
            (
                "body.material",
                json!({"body": "Ball", "name": "Steel", "density": 7850}),
            ),
        ] {
            s.execute(id, &params).unwrap();
        }
        let fresh = |s: &mut Session, id: &str, params: serde_json::Value| {
            let before = crate::mass::fresh_measures();
            s.execute(id, &params).unwrap();
            crate::mass::fresh_measures() - before
        };
        assert_eq!(fresh(&mut s, "document.check", json!({})), 0);
        assert_eq!(fresh(&mut s, "document.inspect", json!({})), 1);
        assert_eq!(fresh(&mut s, "document.inspect", json!({})), 0);
    }
}
