//! Sheet metal (docs/design/sheetmetal.md): a base flat with flanges bent
//! with an inner radius and a K-factor (`feature.sheetMetal`,
//! `sheetMetal.edit`), its bend table (`sheetMetal.bends`), the
//! manufacturability measures as assertion measures (`bendRadius`,
//! `flangeLength`, `holeToBend`, `relief`), and the flat pattern through
//! `file.export` with `sheetMetal` (`drawing_cmds.rs`).

use crate::commands::{
    Run, add_feature, edit, fail, feature_ref, feature_result, length_unit, spec,
};
use crate::params::Params;
use crate::verify_cmds::{Ctx, Dim, Outcome};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    BendDirection, CornerClose, CornerKind, CornerRelief, Document, Expr, FeatureId, FeatureKind,
    Flange, FlangeFrom, FlangeOn, Geometry, MAX_SHEET_FLANGES, Pick, ReliefShape, SheetCorner,
    SheetMetal,
};
use craft3d_kernel::profile::{Segment, Shape};
use craft3d_kernel::sheetmetal::{
    self, developed_allowance, flange_length, flat_area, flat_faces, holes_to_bends,
    measured_angle, measured_radius,
};
use serde_json::{Map, Value, json};

const FLANGE_KEYS: [&str; 8] = [
    "edge",
    "tip",
    "length",
    "angle",
    "direction",
    "radius",
    "kFactor",
    "from",
];

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.sheetMetal", "Sheet Metal", r#"{"sketch": id or name, "thickness": expr, "radius": expr (inner bend radius), "kFactor": expr (0 to 1), "flanges": [{edge: line entity id of the base, or tip: index of an earlier flange, length: expr, angle: expr (0 to 180 deg), direction: up|down = up, radius: expr = the part's, kFactor: expr = the part's, from: tangent|mould = tangent}, ...] = [], "relief": {shape: rect|round|tear or expr (1 rect, 2 round, 3 tear), size: expr} or null = none, "corner": {kind: open|butt|overlap|miter or expr (1 open, 2 butt, 3 overlap, 4 miter), gap: expr} or null = open, "corners": [{flanges: [i, j], kind, gap: expr = corner's gap, over: i or j = the flange on the edge arriving at the corner}, ...] = [], "cutouts": [sketch id or name, ...] = [], "name": string = automatic, "when": expr = always on}"#,
        "Adds a sheet-metal part: the closed profile of the sketch its flanges' edges are on (lines, arcs, circles, splines; holes allowed; the sketch's only profile when no flange is on an edge) is the base, a flat of the given thickness, and each flange is bent from a straight edge: a line of the base's outline (edge), or the far edge of an earlier flange (tip, its place in the list: a Z or a U on a U). A bend turns its flange by angle, up (towards the side the sketch's normal points to) or down, about an inner radius; the K-factor places its neutral fibre at radius + kFactor * thickness from the axis, where the length is kept, so the flat pattern grows by the bend allowance angle * (radius + kFactor * thickness) per bend. A flange's length is its flat beyond the bend, measured from the bend's tangent line, or with from mould from the virtual sharp of the outside surfaces (shorter by (radius + thickness) tan(angle/2)). The material and the tooling are the caller's: the radius, the K-factor and every limit come in as expressions or tables, there is no built-in catalogue. relief cuts the base's corner where two flanged edges meet (rect: a square of the size, round: a quarter disc, tear: a disc tangent to both edges), each edge and its flange ending size from the corner; without one the two bends meet at the corner point. Its shape may be an expression that picks 1 rect, 2 round or 3 tear (a Jig's choice parameter, a table lookup). corner closes every such corner (corners: the corner of flanges i and j its own way): the flanges' flats run on sideways past where their bends end, the bends stay apart (the relief, if any, is the hole where they would cross), to planes of the folded walls, gap apart: overlap, the covering flange (over) to the other's outside face, flush, and the other to gap short of the covering one's inside face; butt, as overlap with the covering flange also gap short of flush; miter, both to the plane that halves the angle between the walls, gap/2 short of it each (the ends bevelled). kind may be an expression that picks 1 open, 2 butt, 3 overlap or 4 miter. For any bend angles and corner angles; refused with the reason: a gap not more than the tolerance (a butt's not less than the thickness), flanges bent opposite ways, walls leaning so far towards each other that a flange would be cut back, extensions that overlap in the flat pattern or run into each other when bent. The bends fold the part by the origami linkage (feature.fold's) with a radius at each joint, and the folded bottom skin is plated (feature.plate's method), so the bends' skins are the cylinders radius and radius + thickness exactly. Faces: F9:copy(0)/F9:face(k) the top skin of the base (k 0), of bend i (k 2i+1) and of flange i's flat (k 2i+2), F9:copy(1)/... the bottom skin, F9:rim(...)/... the ends. Refused, with the flange: a radius below 0 (or within the tolerance of 0: a sharp bend is not supported), a K-factor outside 0..1, an angle outside (0, 180] degrees, an edge that is not a straight line of the base's outline or has two flanges, a tip of a later flange, flanged edges that meet at a reflex corner or in line, a relief that does not fit, a flange shorter than its setback, and flanges that run into each other or into the base when bent. The sketch is drawn in the plane of the flat pattern, so its other closed profiles (beside the base) are holes and cut-outs of the flange flat each lies wholly inside in the pattern (each flange's flat starts the bend allowance beyond its edge): cut in the flat pattern and in the folded part alike; one across a bend's strip, past or touching the pattern's edge, or with a profile inside it is refused. cutouts adds the closed profiles of other sketches in the base sketch's plane (any origin and axes in it) the same way, each switched on and off with its sketch (a sketch with when, a conditional vent): a hole cut later by another feature (a boolean) is not part of the flat pattern, so a hole that has to be cut flat is drawn in the part's sketch or in a cutouts sketch. sheetMetal.bends gives the bend table; file.export with sheetMetal writes the flat pattern; the measures bendRadius, flangeLength, holeToBend, relief and cornerGap check it in assertions.", doc: true, feature_sheet_metal),
    spec!("sheetMetal.edit", "Edit Sheet Metal", r#"{"feature": id or name, "sketch": id or name, "thickness": expr, "radius": expr, "kFactor": expr, "flanges": [flange, ...] as feature.sheetMetal (the whole list), "relief": {shape, size} or null, "corner": {kind, gap} or null, "corners": [corner, ...] (the whole list), "cutouts": [sketch, ...] (the whole list)}"#,
        "Changes a sheet-metal part: what is given replaces what it had (flanges as a whole list; relief null removes it). One undo step.", doc: true, sheet_metal_edit),
    spec!("sheetMetal.bends", "Bend Table", r#"{"feature": id or name}"#,
        "The bend table of a sheet-metal part, measured on the part as built: per bend (one per flange, in order) what it is on, direction, angle (degrees, between the planes of its two flats as folded), radius (the inner skin's), kFactor, thickness, allowance (the bend allowance angle * (radius + kFactor * thickness)), developed (the width of the bend's neutral band laid flat by the unfold, which must agree with allowance), setback ((radius + thickness) tan(angle/2), the outside setback; null at 180 degrees), deduction (2 setback - allowance, the bend deduction; null at 180 degrees), flangeLength (the flange's flat beyond the bend) and toMould (flangeLength + setback). For the part: flatArea, flatSize (the flat pattern's width and height in the sketch's axes), corners (where two flanged edges meet: point, bends, relief, size, kind (open, butt, overlap, miter), gap (null when open), over (the covering flange of a butt or overlap)), holes (per hole of the base and of each flange's flat, its curves, flange (null for the base), the nearest bend beside that flat and the distance from the hole to that bend's tangent line on the flat's side). Read-only.", doc: true, sheet_metal_bends),
];

fn flange_value(p: &Params, index: usize, value: &Value) -> Result<Flange, CommandError> {
    let id = p.command();
    let Value::Object(map) = value else {
        return fail(format!(
            "{id}: flanges.{index} is an object {{\"edge\" or \"tip\", \"length\", \"angle\", ...}}"
        ));
    };
    if let Some(unknown) = map.keys().find(|k| !FLANGE_KEYS.contains(&k.as_str())) {
        return fail(format!(
            "{id}: flanges.{index}: unknown key `{}`; a flange has {}",
            unknown.chars().take(40).collect::<String>(),
            FLANGE_KEYS.join(", ")
        ));
    }
    let on = match (map.get("edge"), map.get("tip")) {
        (Some(edge), None) => FlangeOn::Base(crate::sketch_cmds::entity_id(id, "edge", edge)?),
        (None, Some(tip)) => {
            let Some(n) = tip.as_u64().filter(|&n| (n as usize) < index) else {
                return fail(format!(
                    "{id}: flanges.{index}.tip is the place of an earlier flange in the list (0 to {})",
                    index.saturating_sub(1)
                ));
            };
            if index == 0 {
                return fail(format!(
                    "{id}: flanges.0 sits on an edge of the base; only a later flange sits on a tip"
                ));
            }
            FlangeOn::Tip(u32::try_from(n).unwrap_or(u32::MAX))
        }
        _ => {
            return fail(format!(
                "{id}: flanges.{index} sits on an `edge` of the base (a line's entity id) or on the `tip` of an earlier flange: give one of them"
            ));
        }
    };
    let expr = |key: &str| -> Result<Option<Expr>, CommandError> {
        match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => Ok(Some(Expr::new(p.expr_of(key, v)?))),
        }
    };
    let required = |key: &str| -> Result<Expr, CommandError> {
        expr(key)?
            .ok_or_else(|| CommandError::new(format!("{id}: flanges.{index} needs its `{key}`")))
    };
    let direction = match map.get("direction") {
        None => BendDirection::Up,
        Some(Value::String(s)) => BendDirection::from_name(s).ok_or_else(|| {
            CommandError::new(format!("{id}: flanges.{index}.direction is up or down"))
        })?,
        Some(_) => return fail(format!("{id}: flanges.{index}.direction is up or down")),
    };
    let from = match map.get("from") {
        None => FlangeFrom::Tangent,
        Some(Value::String(s)) => FlangeFrom::from_name(s).ok_or_else(|| {
            CommandError::new(format!("{id}: flanges.{index}.from is tangent or mould"))
        })?,
        Some(_) => return fail(format!("{id}: flanges.{index}.from is tangent or mould")),
    };
    Ok(Flange {
        on,
        length: required("length")?,
        angle: required("angle")?,
        direction,
        radius: expr("radius")?,
        k_factor: expr("kFactor")?,
        from,
    })
}

fn flanges_value(p: &Params) -> Result<Option<Vec<Flange>>, CommandError> {
    let id = p.command();
    let Some(value) = p.get("flanges") else {
        return Ok(None);
    };
    let Value::Array(items) = value else {
        return fail(format!("{id}: `flanges` is a list of flanges"));
    };
    if items.len() > MAX_SHEET_FLANGES {
        return fail(format!("{id}: more than {MAX_SHEET_FLANGES} flanges"));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, v)| flange_value(p, i, v))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// A choice among `names` as a parameter gives it: one of the names, or
/// an expression (a number, or any other text) whose value picks one, 1
/// for the first (as a published choice parameter counts), so a Jig's
/// choice or a table lookup drives it. A name wins over a variable of the
/// same name.
fn pick_value<T: Copy>(
    p: &Params,
    what: &str,
    value: &Value,
    all: &[T],
    name: fn(T) -> &'static str,
) -> Result<Pick<T>, CommandError> {
    let id = p.command();
    let listed: Vec<String> = all
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{} ({})", i + 1, name(*t)))
        .collect();
    let names: Vec<&str> = all.iter().map(|t| name(*t)).collect();
    let wrong = || {
        CommandError::new(format!(
            "{id}: {what} is {}, or an expression that picks {}",
            names.join(", "),
            listed.join(", ")
        ))
    };
    match value {
        Value::String(text) => match all.iter().find(|t| name(**t) == text) {
            Some(t) => Ok(Pick::Named(*t)),
            None if text.trim().is_empty() => Err(wrong()),
            None => Ok(Pick::Expr(Expr::new(text.clone()))),
        },
        Value::Number(_) => Ok(Pick::Expr(Expr::new(
            p.expr_of(what, value).map_err(|_| wrong())?,
        ))),
        _ => Err(wrong()),
    }
}

/// `corner` as given (the part's treatment of its corners): `None` not
/// given, `Some(None)` null (open).
fn corner_value(p: &Params) -> Result<Option<Option<CornerClose>>, CommandError> {
    let id = p.command();
    if !p.has("corner") {
        return Ok(None);
    }
    match p.get("corner") {
        None | Some(Value::Null) => Ok(Some(None)),
        Some(Value::Object(map)) => {
            if let Some(unknown) = map.keys().find(|k| !["kind", "gap"].contains(&k.as_str())) {
                return fail(format!(
                    "{id}: corner: unknown key `{}`; a corner has kind and gap",
                    unknown.chars().take(40).collect::<String>()
                ));
            }
            let kind = map
                .get("kind")
                .ok_or_else(|| CommandError::new(format!("{id}: corner needs its `kind`")))?;
            let kind = pick_value(p, "corner.kind", kind, &CornerKind::ALL, CornerKind::name)?;
            let gap = match map.get("gap") {
                None | Some(Value::Null) => None,
                Some(v) => Some(Expr::new(p.expr_of("corner.gap", v)?)),
            };
            if gap.is_none() && !matches!(kind, Pick::Named(CornerKind::Open)) {
                return fail(format!(
                    "{id}: corner needs its `gap` (between the two flats) unless its kind is open"
                ));
            }
            Ok(Some(Some(CornerClose { kind, gap })))
        }
        Some(_) => fail(format!(
            "{id}: corner is {{\"kind\": \"open|butt|overlap|miter\" or expr, \"gap\": expr}} or null"
        )),
    }
}

/// `corners` as given (corners closed their own way), when it is.
fn corners_value(p: &Params) -> Result<Option<Vec<SheetCorner>>, CommandError> {
    const KEYS: [&str; 4] = ["flanges", "kind", "gap", "over"];
    let id = p.command();
    let Some(value) = p.get("corners") else {
        return Ok(None);
    };
    let items: &[Value] = match value {
        Value::Null => &[],
        Value::Array(items) => items,
        _ => {
            return fail(format!(
                "{id}: corners is a list of {{\"flanges\": [i, j], \"kind\", \"gap\", \"over\"}}"
            ));
        }
    };
    if items.len() > MAX_SHEET_FLANGES {
        return fail(format!(
            "{id}: corners lists at most {MAX_SHEET_FLANGES} corners"
        ));
    }
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Value::Object(map) = item else {
            return fail(format!(
                "{id}: corners.{i} is an object {{\"flanges\": [i, j], \"kind\", \"gap\", \"over\"}}"
            ));
        };
        if let Some(unknown) = map.keys().find(|k| !KEYS.contains(&k.as_str())) {
            return fail(format!(
                "{id}: corners.{i}: unknown key `{}`; a corner has {}",
                unknown.chars().take(40).collect::<String>(),
                KEYS.join(", ")
            ));
        }
        let index = |v: &Value| v.as_u64().and_then(|n| u32::try_from(n).ok());
        let flanges = match map
            .get("flanges")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
        {
            Some([a, b]) => match (index(a), index(b)) {
                (Some(a), Some(b)) if a != b => [a, b],
                _ => {
                    return fail(format!(
                        "{id}: corners.{i}.flanges are the places of two different flanges in the list"
                    ));
                }
            },
            _ => {
                return fail(format!(
                    "{id}: corners.{i}.flanges names the corner by its two flanges: [i, j]"
                ));
            }
        };
        let kind = map
            .get("kind")
            .ok_or_else(|| CommandError::new(format!("{id}: corners.{i} needs its `kind`")))?;
        let kind = pick_value(
            p,
            &format!("corners.{i}.kind"),
            kind,
            &CornerKind::ALL,
            CornerKind::name,
        )?;
        let gap = match map.get("gap") {
            None | Some(Value::Null) => None,
            Some(v) => Some(Expr::new(p.expr_of("corners", v)?)),
        };
        let over = match map.get("over") {
            None | Some(Value::Null) => None,
            Some(v) => match index(v) {
                Some(o) if flanges.contains(&o) => Some(o),
                _ => {
                    return fail(format!(
                        "{id}: corners.{i}.over is the flange of the two ({} or {}) that covers the corner",
                        flanges[0], flanges[1]
                    ));
                }
            },
        };
        out.push(SheetCorner {
            flanges,
            kind,
            gap,
            over,
        });
    }
    Ok(Some(out))
}

/// `relief` as given: `None` not given, `Some(None)` null.
fn relief_value(p: &Params) -> Result<Option<Option<CornerRelief>>, CommandError> {
    let id = p.command();
    if !p.has("relief") {
        return Ok(None);
    }
    match p.get("relief") {
        None | Some(Value::Null) => Ok(Some(None)),
        Some(Value::Object(map)) => {
            if let Some(unknown) = map
                .keys()
                .find(|k| !["shape", "size"].contains(&k.as_str()))
            {
                return fail(format!(
                    "{id}: relief: unknown key `{}`; a relief has shape and size",
                    unknown.chars().take(40).collect::<String>()
                ));
            }
            let shape = map
                .get("shape")
                .ok_or_else(|| CommandError::new(format!("{id}: relief needs its `shape`")))?;
            let shape = pick_value(
                p,
                "relief.shape",
                shape,
                &ReliefShape::ALL,
                ReliefShape::name,
            )?;
            let size = map
                .get("size")
                .ok_or_else(|| CommandError::new(format!("{id}: relief needs its `size`")))?;
            Ok(Some(Some(CornerRelief {
                shape,
                size: Expr::new(p.expr_of("size", size)?),
            })))
        }
        Some(_) => fail(format!(
            "{id}: relief is {{\"shape\": \"rect|round|tear\" or expr, \"size\": expr}} or null"
        )),
    }
}

/// The sketches `cutouts` names (a list of sketch ids or names), when it
/// is given: each a sketch, none twice, at most [`MAX_SHEET_FLANGES`].
fn cutouts_value(p: &Params, doc: &Document) -> Result<Option<Vec<FeatureId>>, CommandError> {
    let id = p.command();
    let Some(value) = p.get("cutouts") else {
        return Ok(None);
    };
    let items: &[Value] = match value {
        Value::Null => &[],
        Value::Array(items) => items,
        _ => {
            return fail(format!(
                "{id}: cutouts is a list of sketches (ids or names) in the base sketch's plane"
            ));
        }
    };
    if items.len() > MAX_SHEET_FLANGES {
        return fail(format!(
            "{id}: cutouts names at most {MAX_SHEET_FLANGES} sketches"
        ));
    }
    let mut out: Vec<FeatureId> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let found = match item {
            Value::String(text) => doc.feature_by_name(text).map(|f| f.id).or_else(|| {
                text.strip_prefix('F')
                    .and_then(|d| d.parse::<u64>().ok())
                    .and_then(|n| doc.feature(FeatureId(n)))
                    .map(|f| f.id)
            }),
            Value::Number(n) => n
                .as_u64()
                .and_then(|n| doc.feature(FeatureId(n)))
                .map(|f| f.id),
            _ => None,
        };
        let Some(sketch) = found else {
            return fail(format!(
                "{id}: cutouts.{i} names no feature: give a sketch's id or name"
            ));
        };
        match doc.feature(sketch).map(|f| (&f.kind, &f.name)) {
            Some((FeatureKind::Sketch(_), _)) => {}
            Some((other, name)) => {
                return fail(format!(
                    "{id}: cutouts.{i}: `{name}` is a {}, not a sketch",
                    other.type_name()
                ));
            }
            None => return fail(format!("{id}: cutouts.{i} names no feature")),
        }
        if out.contains(&sketch) {
            return fail(format!("{id}: cutouts.{i} names a sketch a second time"));
        }
        out.push(sketch);
    }
    Ok(Some(out))
}

/// Refuses flanges whose edge is no line of the sketch.
fn check_flanges(
    doc: &Document,
    id: &str,
    sketch: FeatureId,
    flanges: &[Flange],
) -> Result<(), CommandError> {
    let feature = doc
        .feature(sketch)
        .ok_or_else(|| CommandError::new(format!("{id}: there is no feature {sketch}")))?;
    let FeatureKind::Sketch(drawn) = &feature.kind else {
        return fail(format!(
            "{id}: `{}` is a {}, not a sketch: the base of a sheet-metal part is the profile of a sketch",
            feature.name,
            feature.kind.type_name()
        ));
    };
    for (i, f) in flanges.iter().enumerate() {
        match f.on {
            FlangeOn::Base(line) => {
                let entity = drawn.entity(line).map_err(|_| {
                    CommandError::new(format!(
                        "{id}: flanges.{i}: the sketch `{}` has no {line}",
                        feature.name
                    ))
                })?;
                if !matches!(entity.geometry, Geometry::Line { .. }) {
                    return fail(format!(
                        "{id}: flanges.{i}: {line} is a {}, not a line: a flange sits on a straight edge",
                        entity.geometry.type_name()
                    ));
                }
            }
            FlangeOn::Tip(j) => {
                if j as usize >= i {
                    return fail(format!(
                        "{id}: flanges.{i}.tip: flange {j} is not an earlier one"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn feature_sheet_metal(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.sheetMetal";
    let p = Params::new(
        ID,
        params,
        &[
            "sketch",
            "thickness",
            "radius",
            "kFactor",
            "flanges",
            "relief",
            "cutouts",
            "corner",
            "corners",
            "name",
        ],
    )?;
    let doc = session.document()?.history.current();
    let sketch = feature_ref(doc, &p, ID, "sketch")?;
    let cutouts = cutouts_value(&p, doc)?.unwrap_or_default();
    let thickness = Expr::new(p.expr("thickness")?.ok_or_else(|| p.missing("thickness"))?);
    let radius = Expr::new(p.expr("radius")?.ok_or_else(|| p.missing("radius"))?);
    let k_factor = Expr::new(p.expr("kFactor")?.ok_or_else(|| p.missing("kFactor"))?);
    let flanges = flanges_value(&p)?.unwrap_or_default();
    check_flanges(doc, ID, sketch, &flanges)?;
    let relief = relief_value(&p)?.flatten();
    let corner = corner_value(&p)?.flatten();
    let corners = corners_value(&p)?.unwrap_or_default();
    check_corners(ID, &flanges, &corners)?;
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::SheetMetal(SheetMetal {
            sketch,
            thickness,
            radius,
            k_factor,
            flanges,
            relief,
            cutouts,
            corner,
            corners,
        }),
    )
}

/// Refuses corners named by flanges the list does not have, or on a tip
/// (a corner is where two flanges on edges of the base meet).
fn check_corners(
    id: &str,
    flanges: &[Flange],
    corners: &[SheetCorner],
) -> Result<(), CommandError> {
    for (i, c) in corners.iter().enumerate() {
        for f in c.flanges {
            match flanges.get(f as usize).map(|f| f.on) {
                Some(FlangeOn::Base(_)) => {}
                Some(FlangeOn::Tip(_)) => {
                    return fail(format!(
                        "{id}: corners.{i}: flange {f} sits on a tip; a corner is where two flanges on edges of the base meet"
                    ));
                }
                None => {
                    return fail(format!(
                        "{id}: corners.{i}: there is no flange {f} (the part has {})",
                        flanges.len()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The sheet-metal part `key` names.
fn part_ref(
    session: &Session,
    p: &Params,
    key: &str,
) -> Result<(FeatureId, SheetMetal), CommandError> {
    let id = p.command();
    let doc = session.document()?.history.current();
    let feature = feature_ref(doc, p, id, key)?;
    match doc.feature(feature).map(|f| (&f.kind, &f.name)) {
        Some((FeatureKind::SheetMetal(part), _)) => Ok((feature, part.clone())),
        Some((other, name)) => fail(format!(
            "{id}: `{name}` is a {}, not a sheet-metal part",
            other.type_name()
        )),
        None => fail(format!("{id}: there is no feature {feature}")),
    }
}

fn sheet_metal_edit(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "sheetMetal.edit";
    const KEYS: [&str; 9] = [
        "sketch",
        "thickness",
        "radius",
        "kFactor",
        "flanges",
        "relief",
        "cutouts",
        "corner",
        "corners",
    ];
    let mut allowed = vec!["feature"];
    allowed.extend(KEYS);
    let p = Params::new(ID, params, &allowed)?;
    let (id, mut part) = part_ref(session, &p, "feature")?;
    if !KEYS.iter().any(|k| p.has(k)) {
        return fail(format!("{ID}: give {}", KEYS.join(", ")));
    }
    if p.has("sketch") {
        part.sketch = feature_ref(session.document()?.history.current(), &p, ID, "sketch")?;
    }
    for (key, slot) in [
        ("thickness", &mut part.thickness),
        ("radius", &mut part.radius),
        ("kFactor", &mut part.k_factor),
    ] {
        if let Some(e) = p.expr(key)? {
            *slot = Expr::new(e);
        }
    }
    if let Some(flanges) = flanges_value(&p)? {
        part.flanges = flanges;
    }
    if let Some(relief) = relief_value(&p)? {
        part.relief = relief;
    }
    if let Some(cutouts) = cutouts_value(&p, session.document()?.history.current())? {
        part.cutouts = cutouts;
    }
    if let Some(corner) = corner_value(&p)? {
        part.corner = corner;
    }
    if let Some(corners) = corners_value(&p)? {
        part.corners = corners;
    }
    check_corners(ID, &part.flanges, &part.corners)?;
    check_flanges(
        session.document()?.history.current(),
        ID,
        part.sketch,
        &part.flanges,
    )?;
    edit(session, ID, "Edit sheet metal", |doc| {
        doc.feature_mut(id)?.kind = FeatureKind::SheetMetal(part);
        Ok(())
    })?;
    feature_result(session, id)
}

fn on_text(made: &SheetMetal, i: usize) -> String {
    match made.flanges.get(i).map(|f| f.on) {
        Some(FlangeOn::Base(line)) => line.to_string(),
        Some(FlangeOn::Tip(j)) => format!("tip of flange {j}"),
        None => "?".to_string(),
    }
}

fn sheet_metal_bends(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "sheetMetal.bends";
    let p = Params::new(ID, params, &["feature"])?;
    let (id, made) = part_ref(session, &p, "feature")?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let built = craft3d_regen::sheet_metal_part(doc, &regen, id)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let part = &built.part;
    let tol = built.tolerance;
    let mut bends = Vec::with_capacity(part.bends.len());
    for (i, b) in part.bends.iter().enumerate() {
        let developed = developed_allowance(b, &tol).map_err(|e| {
            CommandError::new(format!(
                "{ID}: bend {i}: the unfold of its neutral band: {e}"
            ))
        })?;
        let setback = b.setback(tol.angular);
        let length = flange_length(part, i);
        bends.push(json!({
            "flange": i,
            "on": on_text(&made, i),
            "direction": if b.angle > 0.0 { "up" } else { "down" },
            "angle": measured_angle(part, i).map(f64::to_degrees),
            "radius": measured_radius(part, i),
            "kFactor": b.k,
            "thickness": b.thickness,
            "allowance": b.allowance,
            "developed": developed,
            "setback": setback,
            "deduction": b.deduction(tol.angular),
            "flangeLength": length,
            "toMould": length.zip(setback).map(|(l, s)| l + s),
        }));
    }
    let chains = sheetmetal::outline(part, &tol);
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for s in chains.iter().flatten() {
        for q in [s.start, s.end] {
            lo = [lo[0].min(q[0]), lo[1].min(q[1])];
            hi = [hi[0].max(q[0]), hi[1].max(q[1])];
        }
    }
    let corners: Vec<Value> = part
        .corners
        .iter()
        .map(|c| {
            json!({
                "point": c.point,
                "bends": c.bends,
                "relief": c.relief.map(|r| r.kind.name()),
                "size": c.relief.map(|r| r.size),
                "kind": c.closing.kind.name(),
                "gap": closed(c).then_some(c.closing.gap),
                "over": c.closing.over,
            })
        })
        .collect();
    let holes: Vec<Value> = holes_to_bends(part, &tol)
        .iter()
        .map(|h| {
            let curves = hole_curves(part, h);
            json!({"curves": curves, "flange": h.flat.checked_sub(1), "bend": h.bend, "distance": h.distance, "lower": h.lower, "upper": h.upper, "exact": h.exact})
        })
        .collect();
    // A later feature that took the part's body (a boolean that drilled
    // it) is not in the flat pattern: what it cut is not cut flat.
    let changed_by = regen
        .feature(id)
        .and_then(|r| r.consumed_by)
        .and_then(|taker| doc.feature(taker))
        .map(|f| f.name.clone());
    Ok(json!({
        "feature": id.to_string(),
        "changedBy": changed_by,
        "bends": bends,
        "flatArea": flat_area(part),
        "flatSize": [hi[0] - lo[0], hi[1] - lo[1]],
        "corners": corners,
        "holes": holes,
        "lengthUnit": length_unit(doc),
    }))
}

/// Whether a corner is closed (its flats run on past their bends).
fn closed(c: &sheetmetal::Corner) -> bool {
    c.closing.kind != sheetmetal::CornerKind::Open
}

/// The gap at a closed corner as the part has it: the least distance
/// between the faces made from the corner's two flats (their skins and
/// rims), measured on the solid's faces (`verify::distance`, with what is
/// proven below it).
fn measured_gap(
    part: &sheetmetal::Part,
    c: &sheetmetal::Corner,
    tolerance: &craft3d_math::Tolerance,
) -> Result<crate::verify::Nearest, String> {
    let [fa, fb] = c.bends;
    let (a, b) = (flat_faces(part, fa + 1), flat_faces(part, fb + 1));
    let pa = crate::verify::Part::new(&part.solid.body, Some(&a), tolerance)?;
    let pb = crate::verify::Part::new(&part.solid.body, Some(&b), tolerance)?;
    crate::verify::distance(&[pa], &[pb])
}

/// The sketch curves bounding a hole the measure lists, as `e7`.
fn hole_curves(part: &sheetmetal::Part, h: &sheetmetal::HoleToBend) -> Vec<String> {
    let mut ids: Vec<u64> = part
        .flats
        .get(h.flat)
        .and_then(|f| f.holes.get(h.hole))
        .into_iter()
        .flatten()
        .map(|s| s.source)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter().map(|c| format!("e{c}")).collect()
}

/// The drawing segment of a pattern segment.
fn drawn_segment(s: &Segment) -> Option<craft3d_drawing::Segment> {
    use craft3d_drawing::Segment as D;
    Some(match &s.shape {
        Shape::Line => D::Line { to: s.end },
        Shape::Arc { center, radius } => D::Arc {
            to: s.end,
            center: *center,
            radius: *radius,
            ccw: !s.reversed,
        },
        Shape::Circle { .. } => return None,
        Shape::Spline(spline) => {
            let control: Vec<[f64; 2]> = spline.control.clone();
            let curve =
                craft3d_drawing::Spline::new(spline.degree, spline.knots.clone(), control, None)
                    .ok()?;
            let curve = if s.reversed { curve.reversed() } else { curve };
            D::Spline(curve.with_ends(s.start, s.end))
        }
    })
}

/// Most parts one sheet of flat patterns holds.
const MAX_SHEET_PARTS: usize = 1024;

/// The flat pattern of the part `sheetMetal` names, for `file.export`:
/// the outline on the layer outline, the bend lines (the middle of each
/// bend's strip) on bendUp (blue, dashed) and bendDown (red, dash-dot).
/// With a list of parts, their flat patterns side by side on one sheet
/// ([`pattern_sheet`]).
pub(crate) fn flat_pattern(
    session: &mut Session,
    p: &Params,
    command: &str,
) -> Result<crate::drawing_cmds::Source, CommandError> {
    if let Some(Value::Array(items)) = p.get("sheetMetal") {
        let items = items.clone();
        return pattern_sheet(session, p, command, &items);
    }
    let (id, _) = part_ref(session, p, "sheetMetal")?;
    one_pattern(session, id, command)
}

/// The flat patterns of the parts `items` names on one sheet, as a sheet
/// of a set's faces lays them out (`file.export {set}`): left to right in
/// the order given by their boxes, `gap` apart (5 mm by default), in rows
/// of at most `width` (a new row above the last). Each part keeps its
/// sketch's axes, moved; the outlines share the layer outline and the bend
/// lines bendUp and bendDown, so a cutter reads one style per layer.
fn pattern_sheet(
    session: &mut Session,
    p: &Params,
    command: &str,
    items: &[Value],
) -> Result<crate::drawing_cmds::Source, CommandError> {
    use craft3d_drawing::{Drawing, Layer, Unit};
    if items.is_empty() {
        return fail(format!(
            "{command}: `sheetMetal` names no part: give a part's id or name, or a list of them"
        ));
    }
    if items.len() > MAX_SHEET_PARTS {
        return fail(format!(
            "{command}: a sheet holds at most {MAX_SHEET_PARTS} flat patterns"
        ));
    }
    let length = |session: &Session, key: &str| -> Result<Option<f64>, CommandError> {
        match p.expr(key)? {
            None => Ok(None),
            Some(source) => session
                .evaluate_typed(&source, crate::typed::TypedKind::Length)
                .map(Some)
                .map_err(|e| CommandError::new(format!("{command}: `{key}`: {e}"))),
        }
    };
    let unit = session.document()?.history.current().units.length;
    let gap = match length(session, "gap")? {
        Some(gap) if gap >= 0.0 => gap,
        Some(_) => return fail(format!("{command}: `gap` must not be negative")),
        None => 5.0 / crate::drawing_cmds::drawing_unit(unit).millimetres(),
    };
    let width = match length(session, "width")? {
        Some(width) if width > 0.0 => Some(width),
        Some(_) => return fail(format!("{command}: `width` must be positive")),
        None => None,
    };
    let slack = session.document()?.history.current().tolerance.linear;
    let mut ids = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let doc = session.document()?.history.current();
        let found = match item {
            Value::String(text) => doc.feature_by_name(text).map(|f| f.id).or_else(|| {
                text.strip_prefix('F')
                    .and_then(|d| d.parse::<u64>().ok())
                    .and_then(|n| doc.feature(FeatureId(n)))
                    .map(|f| f.id)
            }),
            Value::Number(n) => n
                .as_u64()
                .and_then(|n| doc.feature(FeatureId(n)))
                .map(|f| f.id),
            _ => None,
        };
        let Some(id) = found else {
            return fail(format!(
                "{command}: sheetMetal.{i} names no feature: give a sheet-metal part's id or name"
            ));
        };
        if let Some(f) = doc.feature(id)
            && !matches!(f.kind, FeatureKind::SheetMetal(_))
        {
            return fail(format!(
                "{command}: sheetMetal.{i}: `{}` is a {}, not a sheet-metal part",
                f.name,
                f.kind.type_name()
            ));
        }
        if ids.contains(&id) {
            let name = doc
                .feature(id)
                .map_or_else(|| id.to_string(), |f| f.name.clone());
            return fail(format!(
                "{command}: sheetMetal.{i}: `{name}` is on the sheet already"
            ));
        }
        ids.push(id);
    }
    let mut placed: Vec<Layer> = Vec::new();
    let mut parts = Vec::with_capacity(ids.len());
    let (mut x, mut y, mut row_height, mut rows, mut in_row) =
        (0.0_f64, 0.0_f64, 0.0_f64, 0_usize, 0_usize);
    let mut area = 0.0;
    for id in ids {
        let source = one_pattern(session, id, command)?;
        let name = source
            .json
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let [x0, y0, x1, y1] = Drawing {
            unit: Unit::Millimeter,
            layers: source.layers.clone(),
        }
        .bbox()
        .ok_or_else(|| CommandError::new(format!("{command}: `{name}` has nothing to draw")))?;
        let (w, h) = (x1 - x0, y1 - y0);
        if let Some(width) = width {
            if w > width + slack {
                return fail(format!(
                    "{command}: the flat pattern of `{name}` is {w} wide, more than the sheet's width {width}"
                ));
            }
            if in_row > 0 && x + w > width + slack {
                y += row_height + gap;
                (x, row_height, in_row) = (0.0, 0.0, 0);
            }
        }
        if in_row == 0 {
            rows += 1;
        }
        let reach = (x + w).max(y + h);
        if !(reach.is_finite() && reach * f64::EPSILON <= slack) {
            return fail(format!(
                "{command}: the sheet would reach {reach}, where a coordinate cannot be held to the document's tolerance {slack}; use a smaller `gap`"
            ));
        }
        let at = [x - x0, y - y0];
        for layer in source.layers {
            let paths: Vec<_> = layer.paths.iter().map(|path| path.translated(at)).collect();
            match placed.iter_mut().find(|l| l.name == layer.name) {
                Some(shared) => shared.paths.extend(paths),
                None => placed.push(Layer { paths, ..layer }),
            }
        }
        let mut json = source.json;
        area += json.get("flatArea").and_then(Value::as_f64).unwrap_or(0.0);
        if let Some(map) = json.as_object_mut() {
            map.insert("at".into(), json!(at));
            map.insert("bbox".into(), json!([x, y, x + w, y + h]));
        }
        parts.push(json);
        x += w + gap;
        row_height = row_height.max(h);
        in_row += 1;
    }
    let count = |layer: &str| {
        placed
            .iter()
            .find(|l| l.name == layer)
            .map_or(0, |l| l.paths.len())
    };
    Ok(crate::drawing_cmds::Source {
        json: json!({
            "sheetMetal": parts,
            "rows": rows,
            "gap": gap,
            "bendsUp": count("bendUp"),
            "bendsDown": count("bendDown"),
            "flatArea": area,
        }),
        layers: placed,
        paths: Vec::new(),
        construction: Vec::new(),
    })
}

/// The flat pattern of part `id` (see [`flat_pattern`]).
fn one_pattern(
    session: &mut Session,
    id: FeatureId,
    command: &str,
) -> Result<crate::drawing_cmds::Source, CommandError> {
    use craft3d_drawing::{Color, Layer, Path};
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let name = doc
        .feature(id)
        .map_or_else(|| id.to_string(), |f| f.name.clone());
    let built = craft3d_regen::sheet_metal_part(doc, &regen, id)
        .map_err(|e| CommandError::new(format!("{command}: the sheet-metal part `{name}`: {e}")))?;
    let part = &built.part;
    // What a later feature cut (a boolean that drilled the part) is not in
    // the flat pattern; the result says which feature took the body.
    let changed_by = regen
        .feature(id)
        .and_then(|r| r.consumed_by)
        .and_then(|taker| doc.feature(taker))
        .map(|f| f.name.clone());
    let chains = sheetmetal::outline(part, &built.tolerance);
    let mut outline: Vec<Path> = Vec::with_capacity(chains.len());
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for chain in &chains {
        for s in chain {
            for q in [s.start, s.end] {
                lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                hi = [hi[0].max(q[0]), hi[1].max(q[1])];
            }
        }
        match chain.as_slice() {
            [only] if matches!(only.shape, Shape::Circle { .. }) => {
                if let Shape::Circle { center, radius } = only.shape {
                    outline.push(Path::Circle { center, radius });
                }
            }
            _ => {
                let Some(first) = chain.first() else { continue };
                let segments: Vec<craft3d_drawing::Segment> =
                    chain.iter().filter_map(drawn_segment).collect();
                if segments.len() != chain.len() {
                    return Err(CommandError::new(format!(
                        "{command}: the flat pattern of `{name}` has a curve the drawing cannot hold"
                    )));
                }
                outline.push(Path::Chain {
                    start: first.start,
                    segments,
                    closed: true,
                });
            }
        }
    }
    let size = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    let dash = if size.is_finite() && size > 0.0 {
        size / 60.0
    } else {
        1.0
    };
    let (mut up, mut down) = (Vec::new(), Vec::new());
    for b in &part.bends {
        let (a, z) = b.centre_line();
        let path = Path::Chain {
            start: a,
            segments: vec![craft3d_drawing::Segment::Line { to: z }],
            closed: false,
        };
        if b.angle > 0.0 {
            up.push(path);
        } else {
            down.push(path);
        }
    }
    let counts = (up.len(), down.len());
    let mut layers = vec![Layer {
        name: "outline".into(),
        color: Color::Black,
        dash: None,
        dot: false,
        paths: outline,
    }];
    for (layer, color, dot, paths) in [
        ("bendUp", Color::Blue, false, up),
        ("bendDown", Color::Red, true, down),
    ] {
        if !paths.is_empty() {
            layers.push(Layer {
                name: layer.into(),
                color,
                dash: Some(dash),
                dot,
                paths,
            });
        }
    }
    Ok(crate::drawing_cmds::Source {
        json: json!({
            "sheetMetal": id.to_string(),
            "name": name,
            "bendsUp": counts.0,
            "bendsDown": counts.1,
            "flatArea": flat_area(part),
            "changedBy": changed_by,
        }),
        layers,
        paths: Vec::new(),
        construction: Vec::new(),
    })
}

/// Most parts one `of` list names.
const MAX_PARTS: usize = 1024;

/// Whether feature `id` is in the model as it stands: not switched off
/// (suppressed, or its `when` false) and not after the rollback marker.
fn switched_off(ctx: &Ctx<'_>, id: FeatureId) -> bool {
    ctx.regen.feature(id).is_some_and(|r| {
        matches!(
            r.status,
            craft3d_regen::Status::Suppressed
                | craft3d_regen::Status::Inactive
                | craft3d_regen::Status::RolledBack
        )
    })
}

fn is_part(doc: &Document, id: FeatureId) -> bool {
    matches!(
        doc.feature(id).map(|f| &f.kind),
        Some(FeatureKind::SheetMetal(_))
    )
}

/// Why `id` is no sheet-metal part, naming the parts whose bodies went
/// into it (a boolean that drilled one): those are what the measures read.
fn not_a_part(ctx: &Ctx<'_>, id: FeatureId) -> String {
    let kind = ctx
        .doc
        .feature(id)
        .map_or("feature", |f| f.kind.type_name());
    let mut from = Vec::new();
    for part in ctx.doc.features.iter().filter(|f| is_part(ctx.doc, f.id)) {
        // Down the chain of the features that took the part's body.
        let mut at = part.id;
        for _ in 0..=ctx.doc.features.len() {
            let Some(taker) = ctx.regen.feature(at).and_then(|r| r.consumed_by) else {
                break;
            };
            if taker == id {
                from.push(format!("`{}`", part.name));
                break;
            }
            at = taker;
        }
    }
    let name = ctx.feature_name(id);
    if from.is_empty() {
        format!("`{name}` is a {kind}, not a sheet-metal part")
    } else {
        format!(
            "`{name}` is a {kind}, not a sheet-metal part; it is made from the sheet-metal part {}: name that, the measures read the part as it was built",
            from.join(", ")
        )
    }
}

/// The sheet-metal part a name or id in `of` names: by the feature,
/// whatever took its body later (a boolean that drilled it), so the
/// measures read the part as it was built.
fn part_named(ctx: &Ctx<'_>, text: &str) -> Result<FeatureId, String> {
    let shown: String = text.chars().take(80).collect();
    let id = ctx
        .doc
        .feature_by_name(text)
        .map(|f| f.id)
        .or_else(|| {
            text.strip_prefix('F')
                .and_then(|d| d.parse::<u64>().ok())
                .and_then(|n| ctx.doc.feature(FeatureId(n)))
                .map(|f| f.id)
        })
        .ok_or_else(|| format!("there is no feature named `{shown}`"))?;
    if is_part(ctx.doc, id) {
        Ok(id)
    } else {
        Err(not_a_part(ctx, id))
    }
}

/// The sheet-metal parts an assertion's `of` names: by feature name or id
/// (one, or a list), or a query of bodies. When it names none, every part
/// in the model as it stands: a part switched off (`when`, suppressed) or
/// rolled back is left out; one named is refused with that reason.
fn parts_of(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Vec<FeatureId>, String> {
    let ids: Vec<FeatureId> = match p.get("of") {
        None | Some(Value::Null) => {
            let all: Vec<FeatureId> = ctx
                .doc
                .features
                .iter()
                .filter(|f| is_part(ctx.doc, f.id))
                .map(|f| f.id)
                .collect();
            if all.is_empty() {
                return Err("the document has no sheet-metal part".to_string());
            }
            let on: Vec<FeatureId> = all
                .iter()
                .copied()
                .filter(|&id| !switched_off(ctx, id))
                .collect();
            if on.is_empty() {
                let names: Vec<String> = all
                    .iter()
                    .map(|&id| format!("`{}`", ctx.feature_name(id)))
                    .collect();
                return Err(format!(
                    "every sheet-metal part is switched off or rolled back: {}",
                    names.join(", ")
                ));
            }
            return Ok(on);
        }
        Some(Value::String(text)) => vec![part_named(ctx, text)?],
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            if items.len() > MAX_PARTS {
                return Err(format!("`of` names at most {MAX_PARTS} parts"));
            }
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|text| part_named(ctx, text))
                .collect::<Result<_, _>>()?
        }
        Some(v) => {
            // A query names what fits it: a part switched off fits no
            // design as it stands, so it is left out as with no `of`.
            let found: Vec<FeatureId> = crate::verify_cmds::nonempty(
                crate::verify_cmds::subject(ctx, v, &["body"])?,
                "of",
            )?
            .iter()
            .map(|f| f.feature)
            .collect();
            let on: Vec<FeatureId> = found
                .iter()
                .copied()
                .filter(|&id| !switched_off(ctx, id))
                .collect();
            if on.is_empty() {
                return Err(
                    "every sheet-metal part `of` names is switched off or rolled back".to_string(),
                );
            }
            on
        }
    };
    if ids.is_empty() {
        return Err("`of` names no sheet-metal part".to_string());
    }
    if let Some(&other) = ids.iter().find(|&&id| !is_part(ctx.doc, id)) {
        return Err(not_a_part(ctx, other));
    }
    Ok(ids)
}

/// The measures `bendRadius` (the smallest inner radius of the bends),
/// `flangeLength` (the shortest flange, from its bend's tangent line or,
/// with `measured: mould`, from the virtual sharp), `holeToBend` (the
/// least distance from a hole of the base to a bend's tangent line) and
/// `relief` (whether every corner where two flanged edges meet has a
/// relief).
pub(crate) fn take(ctx: &Ctx<'_>, name: &str, p: &Map<String, Value>) -> Result<Outcome, String> {
    if name == "cornerGap" {
        return corner_gap(ctx, p);
    }
    let keys: &[&str] = if name == "flangeLength" {
        &["of", "measured"]
    } else {
        &["of"]
    };
    crate::verify_cmds::only(p, keys)?;
    let mould = match p.get("measured") {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) if s == "tangent" => false,
        Some(Value::String(s)) if s == "mould" => true,
        Some(_) => return Err("`measured` is tangent or mould".to_string()),
    };
    let mut least = f64::INFINITY;
    // Proven bounds on the least (holes with a spline are searched).
    let (mut lower, mut upper) = (f64::INFINITY, f64::INFINITY);
    let mut every = true;
    let mut listed = Vec::new();
    for id in parts_of(ctx, p)? {
        let fname = ctx.feature_name(id);
        let built = craft3d_regen::sheet_metal_part(ctx.doc, ctx.regen, id)
            .map_err(|e| format!("`{fname}`: {e}"))?;
        let part = &built.part;
        match name {
            "bendRadius" => {
                for i in 0..part.bends.len() {
                    let r = measured_radius(part, i)
                        .ok_or_else(|| format!("`{fname}`: bend {i} has no inner skin"))?;
                    least = least.min(r);
                    listed.push(json!({"part": fname, "bend": i, "radius": r}));
                }
            }
            "flangeLength" => {
                for (i, b) in part.bends.iter().enumerate() {
                    let mut l = flange_length(part, i)
                        .ok_or_else(|| format!("`{fname}`: flange {i} has no flat"))?;
                    if mould {
                        l += b.setback(built.tolerance.angular).ok_or_else(|| {
                            format!(
                                "`{fname}`: flange {i} is bent 180 degrees, which has no mould line"
                            )
                        })?;
                    }
                    least = least.min(l);
                    listed.push(json!({"part": fname, "flange": i, "length": l}));
                }
            }
            "holeToBend" => {
                for h in holes_to_bends(part, &built.tolerance) {
                    least = least.min(h.distance);
                    // The kernel's proven bounds: a searched distance is
                    // within an eighth of the tolerance of the true one
                    // either way, unbounded when the search ran out.
                    lower = lower.min(h.lower);
                    upper = upper.min(h.upper);
                    let curves = hole_curves(part, &h);
                    listed.push(json!({"part": fname, "hole": curves, "flange": h.flat.checked_sub(1), "bend": h.bend, "distance": h.distance, "lower": h.lower, "upper": h.upper, "exact": h.exact}));
                }
            }
            _ => {
                for c in &part.corners {
                    every &= c.relief.is_some();
                    listed.push(json!({
                        "part": fname,
                        "point": c.point,
                        "bends": c.bends,
                        "relief": c.relief.map(|r| r.kind.name()),
                    }));
                }
            }
        }
    }
    let mut out = if name == "relief" {
        Outcome::flag(every)
    } else {
        if !least.is_finite() {
            return Err(match name {
                "holeToBend" => "no hole of the part lies beside a bend".to_string(),
                _ => "the part has no bends".to_string(),
            });
        }
        let mut out = Outcome::number(Dim::Length(1), least);
        if name == "holeToBend" {
            out.lower = Some(lower.min(least));
            out.upper = Some(upper.max(least));
        }
        out
    };
    let key = match name {
        "bendRadius" => "bends",
        "flangeLength" => "flanges",
        "holeToBend" => "holes",
        _ => "corners",
    };
    listed.truncate(200);
    out.detail.insert(key.into(), listed.into());
    Ok(out)
}

/// The measure `cornerGap`: the least gap at the closed corners of the
/// parts, each measured on the solid between the faces of the corner's
/// two flats; what is proven below it is the measure's lower bound.
fn corner_gap(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    crate::verify_cmds::only(p, &["of"])?;
    let (mut least, mut lower) = (f64::INFINITY, f64::INFINITY);
    let mut listed = Vec::new();
    for id in parts_of(ctx, p)? {
        let fname = ctx.feature_name(id);
        let built = craft3d_regen::sheet_metal_part(ctx.doc, ctx.regen, id)
            .map_err(|e| format!("`{fname}`: {e}"))?;
        let part = &built.part;
        for c in part.corners.iter().filter(|c| closed(c)) {
            let near = measured_gap(part, c, &built.tolerance)
                .map_err(|e| format!("`{fname}`: the corner of flanges {:?}: {e}", c.bends))?;
            least = least.min(near.distance);
            lower = lower.min(near.lower);
            listed.push(json!({
                "part": fname,
                "bends": c.bends,
                "kind": c.closing.kind.name(),
                "gap": c.closing.gap,
                "measured": near.distance,
                "atLeast": near.lower,
            }));
        }
    }
    if !least.is_finite() {
        return Err("no corner of the part is closed (corner kind open everywhere)".to_string());
    }
    let mut out = Outcome::number(Dim::Length(1), least);
    out.lower = Some(lower.min(least));
    listed.truncate(200);
    out.detail.insert("corners".into(), listed.into());
    Ok(out)
}
