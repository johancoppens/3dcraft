//! Curved flanges of a sheet-metal part (docs/design/sheetmetal.md ›
//! *Curved bends*): their parameters (`curvedFlanges`, `maxStrain` of
//! `feature.sheetMetal` and `sheetMetal.edit`), their rows of the bend
//! table, their bend lines in the flat pattern, and the measure
//! `bendStrain`.

use crate::CommandError;
use crate::commands::fail;
use crate::params::Params;
use crate::verify_cmds::{Ctx, Dim, Outcome};
use craft3d_doc::{
    BendDirection, CurvedFlange, Document, Expr, FeatureId, FeatureKind, FlangeFrom, Geometry,
    MAX_FLANGE_ID, MAX_SHEET_FLANGES, SheetMetal,
};
use craft3d_kernel::sheetmetal::Part;
use serde_json::{Map, Value, json};

const CURVED_KEYS: [&str; 8] = [
    "id",
    "edge",
    "length",
    "angle",
    "direction",
    "radius",
    "kFactor",
    "from",
];

/// `curvedFlanges` as given: each an arc or circle of the part's sketch,
/// its length, angle, direction, radius, K-factor, where its length is
/// measured from, and its id (none: a new one, or on an edit the part's
/// curved flange it equals in every field).
pub(crate) fn curved_value(p: &Params) -> Result<Option<Vec<CurvedFlange>>, CommandError> {
    let id = p.command();
    let Some(value) = p.get("curvedFlanges") else {
        return Ok(None);
    };
    let Value::Array(items) = value else {
        return fail(format!("{id}: `curvedFlanges` is a list of curved flanges"));
    };
    if items.len() > MAX_SHEET_FLANGES {
        return fail(format!(
            "{id}: more than {MAX_SHEET_FLANGES} curved flanges"
        ));
    }
    let mut out = Vec::with_capacity(items.len());
    for (g, item) in items.iter().enumerate() {
        let Value::Object(map) = item else {
            return fail(format!(
                "{id}: curvedFlanges.{g} is an object {{\"edge\", \"length\", \"angle\", ...}}"
            ));
        };
        if let Some(unknown) = map.keys().find(|k| !CURVED_KEYS.contains(&k.as_str())) {
            return fail(format!(
                "{id}: curvedFlanges.{g}: unknown key `{}`; a curved flange has {}",
                unknown.chars().take(40).collect::<String>(),
                CURVED_KEYS.join(", ")
            ));
        }
        let required = |key: &str| -> Result<Expr, CommandError> {
            match map.get(key) {
                None | Some(Value::Null) => {
                    fail(format!("{id}: curvedFlanges.{g} needs its `{key}`"))
                }
                Some(v) => Ok(Expr::new(p.expr_of(key, v)?)),
            }
        };
        let optional = |key: &str| -> Result<Option<Expr>, CommandError> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => Ok(Some(Expr::new(p.expr_of(key, v)?))),
            }
        };
        let edge = map.get("edge").ok_or_else(|| {
            CommandError::new(format!("{id}: curvedFlanges.{g} needs its `edge`"))
        })?;
        let edge = crate::sketch_cmds::entity_id(id, &format!("curvedFlanges.{g}.edge"), edge)?;
        let direction = match map.get("direction") {
            None => BendDirection::Up,
            Some(Value::String(s)) => BendDirection::from_name(s).ok_or_else(|| {
                CommandError::new(format!("{id}: curvedFlanges.{g}.direction is up or down"))
            })?,
            Some(_) => return fail(format!("{id}: curvedFlanges.{g}.direction is up or down")),
        };
        let from = match map.get("from") {
            None => FlangeFrom::Tangent,
            Some(Value::String(s)) => FlangeFrom::from_name(s).ok_or_else(|| {
                CommandError::new(format!("{id}: curvedFlanges.{g}.from is tangent or mould"))
            })?,
            Some(_) => return fail(format!("{id}: curvedFlanges.{g}.from is tangent or mould")),
        };
        let flange_id = match map.get("id") {
            None | Some(Value::Null) => 0,
            Some(v) => v
                .as_u64()
                .filter(|&n| n > 0 && n <= u64::from(MAX_FLANGE_ID))
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    CommandError::new(format!(
                        "{id}: curvedFlanges.{g}.id is a curved flange's id (a whole number from 1 to {MAX_FLANGE_ID}, as sheetMetal.bends lists them)"
                    ))
                })?,
        };
        out.push(CurvedFlange {
            edge,
            length: required("length")?,
            angle: required("angle")?,
            direction,
            radius: optional("radius")?,
            k_factor: optional("kFactor")?,
            from,
            id: flange_id,
        });
    }
    Ok(Some(out))
}

/// `maxStrain` as given: `Some(None)` for null (no limit).
pub(crate) fn max_strain_value(p: &Params) -> Result<Option<Option<Expr>>, CommandError> {
    if !p.has("maxStrain") {
        return Ok(None);
    }
    match p.get("maxStrain") {
        None => Ok(Some(None)),
        Some(v) => Ok(Some(Some(Expr::new(p.expr_of("maxStrain", v)?)))),
    }
}

/// The curved flanges `new` with their ids (one each, from the flanges'
/// counter) and the part's next id past them, as formed louvres take
/// theirs: given with its id it is the part's curved flange of that id
/// (on an edit), without one the part's curved flange it equals in every
/// field, else a new one with the next id.
pub(crate) fn curved_with_ids(
    id: &str,
    part: &SheetMetal,
    mut new: Vec<CurvedFlange>,
    edit: bool,
) -> Result<(Vec<CurvedFlange>, u32), CommandError> {
    let old = if edit {
        part.curved_flanges.as_slice()
    } else {
        &[]
    };
    let mut taken = std::collections::BTreeSet::new();
    for (g, f) in new.iter().enumerate() {
        if f.id == 0 {
            continue;
        }
        if !taken.insert(f.id) {
            return fail(format!(
                "{id}: curvedFlanges.{g}.id {} is given to another curved flange of the list too",
                f.id
            ));
        }
        if !old.iter().any(|o| o.id == f.id) {
            return fail(format!(
                "{id}: curvedFlanges.{g}.id {} is no curved flange of this part; give no id for a new one",
                f.id
            ));
        }
    }
    let mut next = part.next_flange_id.max(1);
    for g in 0..new.len() {
        let Some(f) = new.get(g) else { continue };
        if f.id != 0 {
            continue;
        }
        let same = old.iter().find(|o| {
            !taken.contains(&o.id)
                && CurvedFlange {
                    id: 0,
                    ..(*o).clone()
                } == *f
        });
        let assigned = match same {
            Some(o) => o.id,
            None => {
                let n = next;
                if n > MAX_FLANGE_ID {
                    return fail(format!(
                        "{id}: the part has used up its flange ids (the highest is {MAX_FLANGE_ID})"
                    ));
                }
                next = next.checked_add(1).ok_or_else(|| {
                    CommandError::new(format!("{id}: the part has used up its flange ids"))
                })?;
                n
            }
        };
        taken.insert(assigned);
        if let Some(f) = new.get_mut(g) {
            f.id = assigned;
        }
    }
    let made = SheetMetal {
        curved_flanges: new,
        next_flange_id: next,
        ..part.clone()
    };
    if let Some(fault) = made.flange_ids_fault() {
        return fail(format!("{id}: {fault}"));
    }
    Ok((made.curved_flanges, next))
}

/// Refuses a curved flange whose edge is no arc or circle of the part's
/// sketch.
pub(crate) fn check_curved(
    doc: &Document,
    id: &str,
    sketch: FeatureId,
    list: &[CurvedFlange],
) -> Result<(), CommandError> {
    if list.is_empty() {
        return Ok(());
    }
    let feature = doc
        .feature(sketch)
        .ok_or_else(|| CommandError::new(format!("{id}: there is no feature {sketch}")))?;
    let FeatureKind::Sketch(drawn) = &feature.kind else {
        return fail(format!("{id}: `{}` is not a sketch", feature.name));
    };
    for (g, f) in list.iter().enumerate() {
        let entity = drawn.entity(f.edge).map_err(|_| {
            CommandError::new(format!(
                "{id}: curvedFlanges.{g}: the sketch `{}` has no {}",
                feature.name, f.edge
            ))
        })?;
        if !matches!(
            entity.geometry,
            Geometry::Arc { .. } | Geometry::Circle { .. }
        ) {
            return fail(format!(
                "{id}: curvedFlanges.{g}: {} is a {}, not an arc or a circle: a flange on a line is one of `flanges`",
                f.edge,
                entity.geometry.type_name()
            ));
        }
    }
    Ok(())
}

/// The strain bound as the bend table and the export give it.
fn strain_json(c: &craft3d_kernel::sheetmetal::curved::Curved) -> Value {
    json!({
        "least": c.strain.least,
        "most": c.strain.most,
        "largest": c.strain.largest(),
        "proven": true,
    })
}

/// The bend table's rows of the curved flanges.
pub(crate) fn bends_json(part: &Part, made: &SheetMetal) -> Vec<Value> {
    part.curved
        .iter()
        .enumerate()
        .map(|(g, c)| {
            json!({
                "curvedFlange": g,
                "id": c.id,
                "edge": made.curved_flanges.get(g).map(|f| f.edge.to_string()),
                "direction": if c.angle > 0.0 { "up" } else { "down" },
                "angle": c.theta().to_degrees(),
                "radius": c.radius,
                "kFactor": c.k,
                "thickness": c.thickness,
                "allowance": c.allowance,
                "flangeLength": c.flat_length,
                "edgeRadius": c.edge_radius,
                "sweep": c.sweep.to_degrees(),
                "grows": if c.grows > 0.0 { "out" } else { "in" },
                "tipRadius": c.tip_radius(),
                "flatArea": c.area(),
                "volume": c.volume(),
                "strain": strain_json(c),
                "approximated": true,
            })
        })
        .collect()
}

/// The export's statement of the approximation: per curved flange its id
/// and the proven strain along its edge the pattern implies.
pub(crate) fn export_json(part: &Part) -> Value {
    let list: Vec<Value> = part
        .curved
        .iter()
        .map(|c| json!({"id": c.id, "strain": strain_json(c)}))
        .collect();
    json!({
        "approximated": !list.is_empty(),
        "method": "radial development: the section's lengths kept along every radial line (bend allowance angle * (radius + kFactor * thickness)), the material stretched or compressed along the edge by the strain given",
        "curvedFlanges": list,
    })
}

/// The bend lines of the curved flanges (the arc across the middle of
/// each strip), up and down.
pub(crate) fn bend_lines(part: &Part) -> (Vec<craft3d_drawing::Path>, Vec<craft3d_drawing::Path>) {
    use craft3d_drawing::{Path, Segment};
    use craft3d_math::Portable;
    let (mut up, mut down) = (Vec::new(), Vec::new());
    for c in &part.curved {
        let (center, radius, begin, sweep) = c.bend_line();
        let path = if c.whole() {
            Path::Circle { center, radius }
        } else {
            let at = |a: f64| {
                let (s, k) = a.psin_cos();
                [center[0] + radius * k, center[1] + radius * s]
            };
            Path::Chain {
                start: at(begin),
                segments: vec![Segment::Arc {
                    to: at(begin + sweep),
                    center,
                    radius,
                    ccw: true,
                }],
                closed: false,
            }
        };
        if c.angle > 0.0 {
            up.push(path);
        } else {
            down.push(path);
        }
    }
    (up, down)
}

/// The measure `bendStrain`: the largest stretch or compression along the
/// edge that the parts' curved flanges' approximated flat patterns imply,
/// proven (docs/design/sheetmetal.md › *Curved bends*). Over no curved
/// flange it has no value: an error that says so.
pub(crate) fn bend_strain(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    crate::verify_cmds::only(p, &["of"])?;
    let mut most = f64::NEG_INFINITY;
    let mut listed = Vec::new();
    let mut names = Vec::new();
    for id in crate::sheetmetal_cmds::parts_of(ctx, p)? {
        let fname = ctx.feature_name(id);
        let built = craft3d_regen::sheet_metal_part(ctx.doc, ctx.regen, id)
            .map_err(|e| format!("`{fname}`: {e}"))?;
        for c in &built.part.curved {
            let largest = c.strain.largest();
            most = most.max(largest);
            listed.push(json!({
                "part": fname,
                "id": c.id,
                "strain": largest,
                "least": c.strain.least,
                "most": c.strain.most,
            }));
        }
        names.push(fname);
    }
    if !most.is_finite() {
        return Err(format!(
            "{} {} no curved flange, so bendStrain has no value; while the design has none, switch the assertion off with when",
            names.join(", "),
            if names.len() > 1 { "have" } else { "has" }
        ));
    }
    let mut out = Outcome::number(Dim::Of(craft3d_expr::Dimension::NONE), most);
    out.detail.insert("curvedFlanges".into(), listed.into());
    Ok(out)
}
