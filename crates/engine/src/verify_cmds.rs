//! The verification queries (`measure.*`): read-only questions about the
//! design's geometry, and the measures assertions are checked with
//! (`assert_cmds.rs`). The design note is docs/design/verification.md;
//! the geometry is in `verify.rs`.
//!
//! What a measure is taken of is named the way `select.query` names
//! things: a body by its feature's name or id, or a query `{"kind",
//! "where"}` (or a bare filter, with the measure's own default kind).

use crate::commands::{Run, length_unit, spec, vec3};
use crate::params::Params;
use crate::select_cmds::{Found, query};
use crate::selection::SelectionKind;
use crate::verify::{self, Part};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Document, FeatureId};
use craft3d_kernel::boolean::{Interference, interference};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use craft3d_regen::Regen;
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Most items a subject may name.
const MAX_ITEMS: usize = 10_000;
/// Most bodies a geometric measure takes part in.
const MAX_PARTS: usize = 200;
/// Most pairs of bodies a clearance or an interference compares.
const MAX_BODY_PAIRS: usize = 500;
/// Most hits a ray lists.
const MAX_HITS: usize = 1_000;

const SUBJECT: &str = "a body (feature name or id), a list of them, a query {kind, where} as select.query takes, or a filter (with the measure's own kind)";

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("measure.distance", "Distance", r#"{"from": subject, "to": subject}"#,
        "The least distance between two sets of faces (a subject is a body by name or id, a list of them, a query {kind, where} as select.query takes, or a bare filter, which selects faces): distance in the document's length unit, from and to: the nearest points, each with the body and the face (persistent name) it lies on. Found on a mesh and settled on the exact faces (their surfaces and edges, by projection), so it is exact for planes, cylinders, cones, spheres and tori and as close as projection gets on freeform faces; atLeast is proven (the mesh's distance less what it can be off by). Read-only. Faces that touch or cross are 0 apart; for whether bodies overlap use measure.clearance.", doc: true, measure_distance),
    spec!("measure.clearance", "Clearance", r#"{"from": subject, "to": subject}"#,
        "The least clearance between two sets of bodies (a bare filter selects bodies): for every pair, status clear (with its distance), touch (0) or overlap (one in the other, or a common volume), and the least: clearance (null when a pair overlaps) with the pair and the nearest points. Read-only; at most 500 pairs.", doc: true, measure_clearance),
    spec!("measure.thickness", "Wall Thickness", r#"{"of": subject}"#,
        "The thinnest place of a body (or of the faces given, the places measured from): at sampled points of every face a line goes into the material along the inward normal, the thickness there is how far it goes before it leaves the body (exact, the line against the faces' surfaces), and the least sample is moved along its face to where it is least. Returns thickness (the least found, at a real place), atLeast (proven: no place is thinner), at (the point and its face), to (where the line leaves, and that face), samples. Exact where the wall has parallel or concentric sides; a searched least otherwise. Near an acute edge the ray method's thickness tends to 0. Read-only.", doc: true, measure_thickness),
    spec!("measure.ray", "Ray Cast", r#"{"from": [expr, expr, expr], "direction": "x|y|z|-x|-y|-z" or [number, number, number], "of": subject = all bodies, "limit": integer = 100}"#,
        "Casts a ray through the model and lists every face it meets, nearest first: distance from the start along the ray, point, body, face (persistent name), enters (true where the ray goes into the material, false where it comes out), edge (true where it crosses at an edge or vertex, met by each face there), tangent (grazes without crossing). Exact (the line against each face's surface, kept to the face). Read-only.", doc: true, measure_ray),
    spec!("measure.through", "Goes Through", r#"{"of": subject}"#,
        "Whether holes go through: for every cylindrical or conical face named (a bare filter selects faces, such as {\"hole\": true} or {\"madeBy\": \"Drill\"}), whether the whole bore is clear: a cylinder on the face's axis of its narrowest radius (less a margin), as long as the body and beyond, has no point in common with the body (the boolean's exact intersection; one line down the axis would pass a blind hole with a vent). Per face: through, the axis (point, direction), radius, and where it is blocked (a point of the material in the bore and the faces around it); why not, also for a face whose axis runs in the material (a boss, not a hole). through: all of them. Read-only.", doc: true, measure_through),
    spec!("measure.symmetry", "Symmetry", r#"{"of": subject = all bodies, "plane": "xy|yz|zx" or datum plane (name or id) or {point: [expr, expr, expr], normal: [number, number, number]}, "tol": length = 10 × the document's tolerance}"#,
        "Whether the bodies (or faces) are their own mirror image in a plane: points of every face (its mesh's vertices, on the exact faces) are mirrored and the distance to the nearest face found exactly; deviation is the largest, symmetric whether it is within tol, worst where (point, body, face). A feature on one side only is found from its own faces. Read-only.", doc: true, measure_symmetry),
];

// ------------------------------------------------------------ context

/// What a measure is taken in.
pub(crate) struct Ctx<'a> {
    pub doc: &'a Document,
    pub regen: &'a Regen,
    pub tolerance: Tolerance,
}

impl<'a> Ctx<'a> {
    pub fn new(doc: &'a Document, regen: &'a Regen) -> Result<Self, String> {
        let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
            .ok_or("the document's tolerance is not positive and finite")?;
        Ok(Ctx {
            doc,
            regen,
            tolerance,
        })
    }

    fn feature_name(&self, id: FeatureId) -> String {
        self.doc
            .feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone())
    }

    fn body(&self, id: FeatureId) -> Result<&'a Arc<Body>, String> {
        let outcome = self
            .regen
            .feature(id)
            .ok_or_else(|| format!("there is no feature {id}"))?;
        outcome.own_body().ok_or_else(|| {
            let name = self.feature_name(id);
            match outcome.consumed_by {
                Some(taker) => format!("the body of `{name}` went into {taker}; name that feature"),
                None => format!("`{name}` has no body"),
            }
        })
    }

    /// The persistent name of face `face` of the body of `id`.
    fn face_name(&self, id: FeatureId, face: usize) -> Value {
        self.regen
            .feature(id)
            .and_then(|f| f.names.as_ref())
            .and_then(|n| n.faces.get(face))
            .map_or_else(|| Value::from(face), |name| name.to_string().into())
    }

    /// A length, area or volume (power 1, 2, 3) or a plain number
    /// (power 0) from a number in the document's unit or an expression.
    pub fn quantity(&self, value: &Value, key: &str, power: i32) -> Result<f64, String> {
        let number = match value {
            Value::Number(n) => n.as_f64(),
            Value::String(source) => {
                if let Some(why) = power_of_a_quantity(source) {
                    return Err(format!("`{key}`: {why}"));
                }
                let evaluation = self.doc.variable_table().ok().map(|t| t.evaluate());
                let quantity = craft3d_expr::evaluate(source, |name| {
                    evaluation.as_ref().and_then(|e| e.value(name))
                })
                .map_err(|e| format!("`{key}`: {e}"))?;
                let wanted = craft3d_expr::Dimension::new(power, 0);
                if quantity.dimension.is_dimensionless() {
                    Some(quantity.value)
                } else if quantity.dimension == wanted {
                    let factor = self.doc.units.length.as_expr_unit().factor();
                    Some(quantity.value / factor.powi(power))
                } else {
                    return Err(format!(
                        "`{key}` is a {}, but {} is needed",
                        quantity.dimension,
                        match power {
                            0 => "a number",
                            1 => "a length",
                            2 => "an area",
                            _ => "a volume",
                        }
                    ));
                }
            }
            _ => None,
        };
        number
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("`{key}` must be a finite number or an expression"))
    }

    fn point(&self, value: Option<&Value>, key: &str) -> Result<Vec3, String> {
        let Some(Value::Array(items)) = value else {
            return Err(format!("`{key}` must be a point [x, y, z]"));
        };
        let [x, y, z] = items.as_slice() else {
            return Err(format!("`{key}` must be a point [x, y, z]"));
        };
        Ok(Vec3::new(
            self.quantity(x, key, 1)?,
            self.quantity(y, key, 1)?,
            self.quantity(z, key, 1)?,
        ))
    }
}

/// Refuses `N unit^k` (such as `50 cm^3`): in expressions a power binds
/// to the number with its unit, so that is (50 cm)³, not 50 cm³, and a
/// bound written so would pass silently. Says how to write what was
/// meant. `None` when the text has no such form.
pub(crate) fn power_of_a_quantity(source: &str) -> Option<String> {
    let b = source.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c != b'^' {
            continue;
        }
        let back = |mut j: usize, take: &dyn Fn(u8) -> bool| {
            while j > 0 && b.get(j - 1).is_some_and(|&c| take(c)) {
                j -= 1;
            }
            j
        };
        let end = back(i, &|c| c == b' ');
        let start = back(end, &|c| c.is_ascii_alphabetic());
        let Some(unit) = source.get(start..end).filter(|u| !u.is_empty()) else {
            continue;
        };
        if craft3d_expr::Unit::from_symbol(unit).is_none() {
            continue;
        }
        let number_end = back(start, &|c| c == b' ');
        let number_start = back(number_end, &|c| c.is_ascii_digit() || c == b'.');
        let Some(number) = source
            .get(number_start..number_end)
            .filter(|n| !n.is_empty() && n.bytes().any(|c| c.is_ascii_digit()))
        else {
            continue;
        };
        let k: String = source
            .get(i + 1..)
            .unwrap_or("")
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        return Some(format!(
            "`{number} {unit}^{k}` means ({number} {unit})^{k} in expressions; for {number} {unit}^{k} write `{number} * (1 {unit})^{k}`, or the plain number in the document's unit"
        ));
    }
    None
}

/// A direction: an axis word or three numbers.
fn direction(value: Option<&Value>, key: &str) -> Result<Vec3, String> {
    let wrong = || format!("`{key}` must be x, y, z, -x, -y, -z or [x, y, z] (not zero)");
    let v = match value {
        Some(Value::String(word)) => match word.as_str() {
            "x" | "+x" => Vec3::new(1.0, 0.0, 0.0),
            "y" | "+y" => Vec3::new(0.0, 1.0, 0.0),
            "z" | "+z" => Vec3::new(0.0, 0.0, 1.0),
            "-x" => Vec3::new(-1.0, 0.0, 0.0),
            "-y" => Vec3::new(0.0, -1.0, 0.0),
            "-z" => Vec3::new(0.0, 0.0, -1.0),
            _ => return Err(wrong()),
        },
        Some(Value::Array(items)) => {
            let [x, y, z] = items.as_slice() else {
                return Err(wrong());
            };
            let n = |v: &Value| v.as_f64().filter(|n| n.is_finite()).ok_or_else(wrong);
            Vec3::new(n(x)?, n(y)?, n(z)?)
        }
        _ => return Err(wrong()),
    };
    v.normalized().ok_or_else(wrong)
}

/// Refuses keys a measure does not take.
pub(crate) fn only(p: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    match p.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(key) => {
            let key: String = key.chars().take(40).collect();
            Err(format!(
                "unknown parameter `{key}`; this measure takes {}",
                allowed.join(", ")
            ))
        }
        None => Ok(()),
    }
}

// ----------------------------------------------------------- subjects

/// A body taking part, with every face of it (`None`) or the faces named.
type Named = (FeatureId, Arc<Body>, Option<Vec<usize>>);

/// The feature a name or id names.
fn feature_of(doc: &Document, text: &str) -> Result<FeatureId, String> {
    if let Some(f) = doc.feature_by_name(text) {
        return Ok(f.id);
    }
    text.strip_prefix('F')
        .and_then(|d| d.parse::<u64>().ok())
        .and_then(|n| doc.feature(FeatureId(n)))
        .map(|f| f.id)
        .ok_or_else(|| {
            format!(
                "there is no feature named `{}`",
                text.chars().take(80).collect::<String>()
            )
        })
}

/// What a subject names, as a query would list it.
pub(crate) fn subject(ctx: &Ctx<'_>, value: &Value, kind: &[&str]) -> Result<Vec<Found>, String> {
    let body_item = |text: &str| -> Result<Found, String> {
        let id = feature_of(ctx.doc, text)?;
        ctx.body(id)?;
        Ok(Found {
            kind: SelectionKind::Body,
            feature: id,
            indices: Vec::new(),
            name: ctx.feature_name(id),
        })
    };
    match value {
        Value::String(text) => Ok(vec![body_item(text)?]),
        Value::Array(items) => {
            if items.len() > MAX_ITEMS {
                return Err(format!("at most {MAX_ITEMS} bodies"));
            }
            items
                .iter()
                .map(|item| match item {
                    Value::String(text) => body_item(text),
                    _ => Err("a list names bodies by feature name or id".to_string()),
                })
                .collect()
        }
        Value::Object(map) => {
            let (kinds, filter) = if map.contains_key("where") {
                if let Some(key) = map.keys().find(|k| *k != "kind" && *k != "where") {
                    let key: String = key.chars().take(40).collect();
                    return Err(format!("a query has kind and where, not `{key}`"));
                }
                (map.get("kind").cloned(), map.get("where").cloned())
            } else {
                (Some(Value::from(kind.to_vec())), Some(value.clone()))
            };
            query(
                ctx.doc,
                ctx.regen,
                kinds.as_ref(),
                filter.as_ref(),
                MAX_ITEMS,
            )
        }
        _ => Err(format!("a subject is {SUBJECT}")),
    }
}

/// The bodies and faces a subject names: per body, every face (`None`) or
/// the faces named.
fn bodies_of(ctx: &Ctx<'_>, found: &[Found]) -> Result<Vec<Named>, String> {
    let mut by: BTreeMap<FeatureId, Option<Vec<usize>>> = BTreeMap::new();
    let mut order: Vec<FeatureId> = Vec::new();
    for item in found {
        match item.kind {
            SelectionKind::Body | SelectionKind::Feature => {
                ctx.body(item.feature)?;
                if !by.contains_key(&item.feature) {
                    order.push(item.feature);
                }
                by.insert(item.feature, None);
            }
            SelectionKind::Face => {
                let entry = by.entry(item.feature).or_insert_with(|| {
                    order.push(item.feature);
                    Some(Vec::new())
                });
                if let Some(list) = entry {
                    list.extend(item.indices.iter().copied());
                }
            }
            other => {
                return Err(format!(
                    "measures are taken of bodies and faces; the subject names {} `{}`",
                    other.name(),
                    item.name
                ));
            }
        }
    }
    if order.len() > MAX_ITEMS {
        return Err(format!(
            "the subject names {} bodies (or faces of them); at most {MAX_ITEMS}",
            order.len()
        ));
    }
    order
        .into_iter()
        .map(|id| {
            let body = Arc::clone(ctx.body(id)?);
            Ok((id, body, by.remove(&id).flatten()))
        })
        .collect()
}

fn whole_bodies(
    ctx: &Ctx<'_>,
    found: &[Found],
    what: &str,
) -> Result<Vec<(FeatureId, Arc<Body>)>, String> {
    bodies_of(ctx, found)?
        .into_iter()
        .map(|(id, body, faces)| match faces {
            None => Ok((id, body)),
            Some(_) => Err(format!(
                "{what} is of bodies; the subject names faces of `{}`",
                ctx.feature_name(id)
            )),
        })
        .collect()
}

fn all_bodies(ctx: &Ctx<'_>) -> Vec<Found> {
    ctx.regen
        .bodies()
        .map(|(id, _)| Found {
            kind: SelectionKind::Body,
            feature: id,
            indices: Vec::new(),
            name: ctx.feature_name(id),
        })
        .collect()
}

fn required<'v>(p: &'v Map<String, Value>, key: &str) -> Result<&'v Value, String> {
    p.get(key)
        .filter(|v| !v.is_null())
        .ok_or_else(|| format!("`{key}` is required: {SUBJECT}"))
}

fn nonempty(found: Vec<Found>, key: &str) -> Result<Vec<Found>, String> {
    if found.is_empty() {
        Err(format!(
            "`{key}` names nothing in the document as it is now"
        ))
    } else {
        Ok(found)
    }
}

// ----------------------------------------------------------- measures

/// What a measure is: a number of a dimension, or yes or no.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dim {
    /// A length to this power: 1 a length, 2 an area, 3 a volume.
    Length(i32),
    /// Kilograms.
    Mass,
    /// A count.
    Count,
    /// Yes or no.
    Flag,
}

/// What a measure found.
pub(crate) struct Outcome {
    pub dim: Dim,
    /// The number; `None` for a flag, or where there is none (a clearance
    /// of bodies that overlap).
    pub number: Option<f64>,
    pub flag: Option<bool>,
    /// What it found in words when the number cannot say it.
    pub note: Option<String>,
    /// Proven bounds on the true value: it is at least `lower` and at
    /// most `upper`. Equal to `number` for what is measured exactly (an
    /// integral, a count, a box); for the least of something searched for
    /// (a thickness, a distance) `upper` is what was found at a real place
    /// and `lower` what nothing can be below.
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    pub detail: Map<String, Value>,
}

impl Outcome {
    fn number(dim: Dim, value: f64) -> Self {
        Outcome {
            dim,
            number: Some(value),
            flag: None,
            note: None,
            lower: Some(value),
            upper: Some(value),
            detail: Map::new(),
        }
    }

    fn flag(value: bool) -> Self {
        Outcome {
            dim: Dim::Flag,
            number: None,
            flag: Some(value),
            note: None,
            lower: None,
            upper: None,
            detail: Map::new(),
        }
    }
}

/// The measures, by name: what `assert.add` takes under `measure`.
pub(crate) const MEASURES: [&str; 11] = [
    "volume",
    "area",
    "mass",
    "count",
    "extent",
    "distance",
    "clearance",
    "thickness",
    "through",
    "interference",
    "symmetric",
];

/// Takes the measure `name` with the parameters `p`.
pub(crate) fn take(ctx: &Ctx<'_>, name: &str, p: &Map<String, Value>) -> Result<Outcome, String> {
    match name {
        "volume" | "area" | "mass" => mass_measure(ctx, name, p),
        "count" => count(ctx, p),
        "extent" => extent(ctx, p),
        "distance" => distance(ctx, p),
        "clearance" => clearance(ctx, p),
        "thickness" => thickness(ctx, p),
        "through" => through(ctx, p),
        "interference" => no_interference(ctx, p),
        "symmetric" => symmetric(ctx, p),
        _ => Err(format!(
            "unknown measure `{}`; one of {}",
            name.chars().take(40).collect::<String>(),
            MEASURES.join(", ")
        )),
    }
}

fn mass_measure(ctx: &Ctx<'_>, name: &str, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["of"])?;
    let kind = if name == "area" { "face" } else { "body" };
    let found = match p.get("of") {
        None | Some(Value::Null) => all_bodies(ctx),
        Some(v) => nonempty(subject(ctx, v, &[kind])?, "of")?,
    };
    let unit = length_unit(ctx.doc);
    let mut total = 0.0;
    let mut listed = Vec::new();
    for (id, body, faces) in bodies_of(ctx, &found)? {
        let fname = ctx.feature_name(id);
        let value = match (name, faces) {
            ("area", Some(faces)) => {
                let areas = craft3d_topo::face_areas(&body);
                let mut sum = 0.0;
                for face in faces {
                    match areas.get(face) {
                        Some(Ok(a)) => sum += a,
                        Some(Err(e)) => {
                            return Err(format!("a face of `{fname}` cannot be measured: {e}"));
                        }
                        None => return Err(format!("a face of `{fname}` is missing")),
                    }
                }
                sum
            }
            (_, Some(_)) => {
                return Err(format!(
                    "{name} is of bodies; the subject names faces of `{fname}`"
                ));
            }
            (_, None) => {
                let mass = crate::mass::mass_of(&body)
                    .map_err(|e| format!("`{fname}` cannot be measured: {e}"))?;
                match name {
                    "volume" => mass.volume,
                    "area" => mass.area,
                    _ => {
                        let mut entry = Map::new();
                        crate::appearance_cmds::physical_fields(
                            &mut entry,
                            ctx.doc,
                            ctx.regen,
                            (id, &body),
                            unit,
                            true,
                        );
                        entry.get("mass").and_then(Value::as_f64).ok_or_else(|| {
                            format!("`{fname}` has no material, so no mass; give it one with body.material")
                        })?
                    }
                }
            }
        };
        total += value;
        listed.push(json!({"feature": id.to_string(), "name": fname, name: value}));
    }
    let dim = match name {
        "volume" => Dim::Length(3),
        "area" => Dim::Length(2),
        _ => Dim::Mass,
    };
    let mut out = Outcome::number(dim, total);
    out.detail.insert("bodies".into(), listed.into());
    Ok(out)
}

fn count(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["of"])?;
    let found = match required(p, "of")? {
        Value::Object(map) if !map.contains_key("where") => query(
            ctx.doc,
            ctx.regen,
            None,
            Some(&Value::Object(map.clone())),
            MAX_ITEMS,
        )?,
        v => subject(ctx, v, &["face"])?,
    };
    let mut out = Outcome::number(Dim::Count, found.len() as f64);
    let names: Vec<Value> = found
        .iter()
        .take(50)
        .map(|f| json!({"kind": f.kind.name(), "name": f.name, "feature": f.feature.to_string()}))
        .collect();
    out.detail.insert("items".into(), names.into());
    Ok(out)
}

fn extent(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["of", "axis"])?;
    let axis = match p.get("axis").and_then(Value::as_str) {
        Some("x") => 0,
        Some("y") => 1,
        Some("z") => 2,
        _ => return Err("`axis` must be x, y or z".into()),
    };
    let found = match p.get("of") {
        None | Some(Value::Null) => all_bodies(ctx),
        Some(v) => nonempty(subject(ctx, v, &["body"])?, "of")?,
    };
    let mut all = Bbox3::from_points([]);
    for (id, body) in whole_bodies(ctx, &found, "extent")? {
        let b = body
            .bbox()
            .map_err(|e| format!("`{}` has no box: {e}", ctx.feature_name(id)))?;
        if let Some((lo, hi)) = b.corners() {
            all.add(lo);
            all.add(hi);
        }
    }
    let (lo, hi) = all.corners().ok_or("there are no bodies to measure")?;
    let pick = |v: Vec3| [v.x, v.y, v.z].get(axis).copied().unwrap_or(0.0);
    let mut out = Outcome::number(Dim::Length(1), pick(hi) - pick(lo));
    out.detail.insert("min".into(), vec3(lo));
    out.detail.insert("max".into(), vec3(hi));
    Ok(out)
}

/// The parts (bodies with their meshes) of the bodies and faces named.
fn parts<'b>(ctx: &Ctx<'_>, bodies: &'b [Named]) -> Result<Vec<Part<'b>>, String> {
    if bodies.len() > MAX_PARTS {
        return Err(format!(
            "the subject names {} bodies (or faces of them); a geometric measure takes at most {MAX_PARTS}",
            bodies.len()
        ));
    }
    bodies
        .iter()
        .map(|(id, body, faces)| {
            Part::new(body, faces.as_deref(), &ctx.tolerance)
                .map_err(|e| format!("`{}`: {e}", ctx.feature_name(*id)))
        })
        .collect()
}

fn place_json(ctx: &Ctx<'_>, bodies: &[Named], (part, face): (usize, usize), point: Vec3) -> Value {
    let id = bodies.get(part).map(|(id, ..)| *id);
    json!({
        "point": vec3(point),
        "body": id.map(|id| ctx.feature_name(id)),
        "feature": id.map(|id| id.to_string()),
        "face": id.map_or(Value::Null, |id| ctx.face_name(id, face)),
    })
}

fn distance(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["from", "to"])?;
    let a = nonempty(subject(ctx, required(p, "from")?, &["face"])?, "from")?;
    let b = nonempty(subject(ctx, required(p, "to")?, &["face"])?, "to")?;
    let (ba, bb) = (bodies_of(ctx, &a)?, bodies_of(ctx, &b)?);
    let (pa, pb) = (parts(ctx, &ba)?, parts(ctx, &bb)?);
    let near = verify::distance(&pa, &pb)?;
    let mut out = Outcome::number(Dim::Length(1), near.distance);
    out.lower = Some(near.lower);
    out.detail.insert("atLeast".into(), near.lower.into());
    out.detail
        .insert("from".into(), place_json(ctx, &ba, near.face_a, near.a));
    out.detail
        .insert("to".into(), place_json(ctx, &bb, near.face_b, near.b));
    Ok(out)
}

fn clearance(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["from", "to"])?;
    let a = nonempty(subject(ctx, required(p, "from")?, &["body"])?, "from")?;
    let b = nonempty(subject(ctx, required(p, "to")?, &["body"])?, "to")?;
    let ba: Vec<_> = whole_bodies(ctx, &a, "clearance")?
        .into_iter()
        .map(|(id, body)| (id, body, None))
        .collect();
    let bb: Vec<_> = whole_bodies(ctx, &b, "clearance")?
        .into_iter()
        .map(|(id, body)| (id, body, None))
        .collect();
    if ba.len().saturating_mul(bb.len()) > MAX_BODY_PAIRS {
        return Err(format!(
            "more than {MAX_BODY_PAIRS} pairs of bodies; name fewer"
        ));
    }
    let (pa, pb) = (parts(ctx, &ba)?, parts(ctx, &bb)?);
    let tol = ctx.tolerance.linear;
    let mut pairs = Vec::new();
    let mut least: Option<(f64, Value)> = None;
    let mut lower = f64::INFINITY;
    let mut overlap = false;
    for (i, (ida, body_a, _)) in ba.iter().enumerate() {
        for (j, (idb, body_b, _)) in bb.iter().enumerate() {
            if ida == idb {
                continue;
            }
            let (Some(part_a), Some(part_b)) = (pa.get(i), pb.get(j)) else {
                continue;
            };
            let near =
                verify::distance(std::slice::from_ref(part_a), std::slice::from_ref(part_b))?;
            let mut status = "clear";
            if near.distance <= 2.0 * tol {
                match interference(body_a, body_b, &ctx.tolerance) {
                    Ok(Interference::Clear) => {}
                    Ok(Interference::Touch { .. }) => status = "touch",
                    Ok(Interference::Overlap(_)) => status = "overlap",
                    Err(e) => {
                        return Err(format!(
                            "whether `{}` and `{}` overlap cannot be told: {e}",
                            ctx.feature_name(*ida),
                            ctx.feature_name(*idb)
                        ));
                    }
                }
            } else {
                // Apart everywhere: one may still hold the other.
                let inside = |probe: &craft3d_kernel::boolean::probe::Probe<'_>, other: &Body| {
                    other.vertices.first().is_some_and(|v| {
                        matches!(
                            probe.place(v.point),
                            Ok(craft3d_kernel::boolean::Place::Inside)
                        )
                    })
                };
                if inside(&part_b.probe, body_a) || inside(&part_a.probe, body_b) {
                    status = "overlap";
                }
            }
            let value = match status {
                "clear" => Some(near.distance),
                "touch" => Some(0.0),
                _ => None,
            };
            lower = lower.min(if status == "clear" { near.lower } else { 0.0 });
            overlap |= value.is_none();
            let entry = json!({
                "a": ctx.feature_name(*ida), "b": ctx.feature_name(*idb),
                "status": status, "distance": value,
                "from": place_json(ctx, &ba, (i, near.face_a.1), near.a),
                "to": place_json(ctx, &bb, (j, near.face_b.1), near.b),
            });
            if let Some(v) = value
                && least.as_ref().is_none_or(|(l, _)| v < *l)
            {
                least = Some((v, entry.clone()));
            }
            pairs.push(entry);
        }
    }
    if pairs.is_empty() {
        return Err("there is no pair of two different bodies".into());
    }
    let mut out = Outcome::number(Dim::Length(1), 0.0);
    if overlap {
        out.number = None;
        out.note = Some("overlap".into());
    } else if let Some((v, _)) = &least {
        out.number = Some(*v);
    }
    out.upper = out.number;
    out.lower = out.number.map(|v| lower.min(v));
    if let Some(at_least) = out.lower {
        out.detail.insert("atLeast".into(), at_least.into());
    }
    if let Some((_, pair)) = least {
        out.detail.insert("least".into(), pair);
    }
    out.detail.insert("pairs".into(), pairs.into());
    Ok(out)
}

fn thickness(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["of"])?;
    let found = nonempty(subject(ctx, required(p, "of")?, &["body", "face"])?, "of")?;
    let bodies = bodies_of(ctx, &found)?;
    let parts = parts(ctx, &bodies)?;
    let mut best: Option<(usize, verify::Thin)> = None;
    let mut samples = 0;
    let mut lower = f64::INFINITY;
    for (i, part) in parts.iter().enumerate() {
        if let Some(thin) = verify::thickness(part, ctx.tolerance.linear)? {
            samples += thin.samples;
            lower = lower.min(thin.lower);
            if best
                .as_ref()
                .is_none_or(|(_, b)| thin.thickness < b.thickness)
            {
                best = Some((i, thin));
            }
        }
    }
    let (i, thin) = best.ok_or("no point of the faces could be measured from")?;
    let mut out = Outcome::number(Dim::Length(1), thin.thickness);
    out.lower = Some(lower.min(thin.thickness));
    out.detail
        .insert("atLeast".into(), lower.min(thin.thickness).into());
    out.detail.insert(
        "at".into(),
        place_json(ctx, &bodies, (i, thin.face), thin.at),
    );
    out.detail.insert(
        "to".into(),
        place_json(ctx, &bodies, (i, thin.opposite), thin.to),
    );
    out.detail.insert("samples".into(), samples.into());
    Ok(out)
}

fn through(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["of"])?;
    let found = nonempty(subject(ctx, required(p, "of")?, &["face"])?, "of")?;
    if let Some(item) = found.iter().find(|f| f.kind != SelectionKind::Face) {
        return Err(format!(
            "through is asked of faces; the subject names {} `{}`",
            item.kind.name(),
            item.name
        ));
    }
    // The bodies once, each with the faces asked about.
    let bodies = bodies_of(ctx, &found)?;
    let parts = parts(ctx, &bodies)?;
    let mut listed = Vec::new();
    let (mut all, mut count) = (true, 0_usize);
    for item in &found {
        let Some(at) = bodies.iter().position(|(id, ..)| *id == item.feature) else {
            continue;
        };
        let Some(part) = parts.get(at) else {
            continue;
        };
        let mut entry = Map::new();
        entry.insert("face".into(), item.name.clone().into());
        entry.insert("body".into(), ctx.feature_name(item.feature).into());
        let mut is_through = true;
        let mut blocked = Vec::new();
        for &face in &item.indices {
            let t = verify::through(part, face, &ctx.tolerance)?;
            entry.insert(
                "axis".into(),
                json!({"point": vec3(t.middle), "direction": vec3(t.axis)}),
            );
            entry.insert("radius".into(), t.radius.into());
            if !t.through {
                is_through = false;
                if let Some(why) = &t.why {
                    entry.insert("why".into(), why.clone().into());
                }
                for (point, faces) in &t.blocked {
                    blocked.push(json!({
                        "point": vec3(*point),
                        "faces": faces.iter().take(20).map(|f| ctx.face_name(item.feature, *f)).collect::<Vec<_>>(),
                    }));
                }
            }
        }
        entry.insert("through".into(), is_through.into());
        if !blocked.is_empty() {
            entry.insert("blocked".into(), blocked.into());
        }
        all &= is_through;
        count += usize::from(is_through);
        listed.push(Value::Object(entry));
    }
    let mut out = Outcome::flag(all);
    out.number = Some(count as f64);
    out.detail.insert("faces".into(), listed.into());
    out.detail.insert("throughCount".into(), count.into());
    Ok(out)
}

fn no_interference(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["of", "to"])?;
    let a = match p.get("of") {
        None | Some(Value::Null) => all_bodies(ctx),
        Some(v) => nonempty(subject(ctx, v, &["body"])?, "of")?,
    };
    let ba = whole_bodies(ctx, &a, "interference")?;
    let bb = match p.get("to") {
        None | Some(Value::Null) => None,
        Some(v) => Some(whole_bodies(
            ctx,
            &nonempty(subject(ctx, v, &["body"])?, "to")?,
            "interference",
        )?),
    };
    let mut pairs: Vec<(usize, usize, bool)> = Vec::new();
    match &bb {
        None => {
            for i in 0..ba.len() {
                for j in i.saturating_add(1)..ba.len() {
                    pairs.push((i, j, false));
                }
            }
        }
        Some(bb) => {
            for i in 0..ba.len() {
                for j in 0..bb.len() {
                    pairs.push((i, j, true));
                }
            }
        }
    }
    if pairs.len() > MAX_BODY_PAIRS {
        return Err(format!(
            "more than {MAX_BODY_PAIRS} pairs of bodies; name fewer"
        ));
    }
    let mut overlaps = Vec::new();
    for (i, j, other) in pairs {
        let (Some((ida, a)), Some((idb, b))) = (
            ba.get(i),
            if other {
                bb.as_ref().and_then(|bb| bb.get(j))
            } else {
                ba.get(j)
            },
        ) else {
            continue;
        };
        if ida == idb {
            continue;
        }
        match interference(a, b, &ctx.tolerance) {
            Ok(Interference::Overlap(common)) => {
                let volume = craft3d_topo::mass_properties(&common.body)
                    .ok()
                    .map(|m| m.volume);
                overlaps.push(json!({"a": ctx.feature_name(*ida), "b": ctx.feature_name(*idb), "volume": volume}));
            }
            Ok(_) => {}
            Err(e) => {
                return Err(format!(
                    "whether `{}` and `{}` overlap cannot be told: {e}",
                    ctx.feature_name(*ida),
                    ctx.feature_name(*idb)
                ));
            }
        }
    }
    let mut out = Outcome::flag(overlaps.is_empty());
    out.number = Some(overlaps.len() as f64);
    if !overlaps.is_empty() {
        out.note = Some(format!("{} pairs overlap", overlaps.len()));
    }
    out.detail.insert("overlaps".into(), overlaps.into());
    Ok(out)
}

/// A plane: a point on it and its normal.
fn plane(ctx: &Ctx<'_>, value: Option<&Value>) -> Result<(Vec3, Vec3), String> {
    let wrong =
        || "`plane` must be xy, yz, zx, a datum plane (name or id) or {point, normal}".to_string();
    match value {
        Some(Value::String(word)) => match word.as_str() {
            "xy" | "yx" => Ok((Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0))),
            "yz" | "zy" => Ok((Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0))),
            "zx" | "xz" => Ok((Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0))),
            _ => {
                let id = feature_of(ctx.doc, word).map_err(|_| wrong())?;
                match ctx.regen.feature(id).and_then(|f| f.datum.as_ref()) {
                    Some(craft3d_regen::Datum::Plane(frame)) => Ok((frame.origin(), frame.z())),
                    _ => Err(format!("`{word}` is not a placed datum plane")),
                }
            }
        },
        Some(Value::Object(map)) => {
            only(map, &["point", "normal"]).map_err(|_| wrong())?;
            let point = ctx.point(map.get("point"), "plane.point")?;
            let normal = direction(map.get("normal"), "plane.normal")?;
            Ok((point, normal))
        }
        _ => Err(wrong()),
    }
}

fn symmetric(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["of", "plane", "tol"])?;
    let found = match p.get("of") {
        None | Some(Value::Null) => all_bodies(ctx),
        Some(v) => nonempty(subject(ctx, v, &["body"])?, "of")?,
    };
    let (point, normal) = plane(ctx, p.get("plane"))?;
    let tol = match p.get("tol") {
        None | Some(Value::Null) => 10.0 * ctx.tolerance.linear,
        Some(v) => {
            let t = ctx.quantity(v, "tol", 1)?;
            if t <= 0.0 {
                return Err("`tol` must be a positive length".into());
            }
            t
        }
    };
    let bodies = bodies_of(ctx, &found)?;
    let parts = parts(ctx, &bodies)?;
    let mirror = verify::symmetry(&parts, point, normal)?;
    let mut out = Outcome::flag(mirror.deviation <= tol);
    out.number = Some(mirror.deviation);
    out.detail
        .insert("deviation".into(), mirror.deviation.into());
    out.detail.insert("tol".into(), tol.into());
    out.detail.insert(
        "worst".into(),
        place_json(
            ctx,
            &bodies,
            (mirror.worst_part, mirror.worst_face),
            mirror.worst,
        ),
    );
    out.detail.insert("samples".into(), mirror.samples.into());
    Ok(out)
}

// ------------------------------------------------------------ commands

/// Runs measure `name` for the command `id`: its parameters checked, the
/// outcome's number under `key` with the details.
fn command(
    session: &mut Session,
    params: &Value,
    id: &'static str,
    name: &str,
    keys: &[&str],
    key: &str,
) -> Run {
    Params::new(id, params, keys)?;
    let map = match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let ctx = Ctx::new(doc, &regen).map_err(|e| CommandError::new(format!("{id}: {e}")))?;
    let out = take(&ctx, name, &map).map_err(|e| CommandError::new(format!("{id}: {e}")))?;
    let mut result = out.detail;
    match out.dim {
        Dim::Flag => {
            result.insert(key.into(), out.flag.into());
        }
        _ => {
            result.insert(key.into(), out.number.into());
        }
    }
    result.insert("lengthUnit".into(), length_unit(doc).into());
    Ok(Value::Object(result))
}

fn measure_distance(session: &mut Session, params: &Value) -> Run {
    command(
        session,
        params,
        "measure.distance",
        "distance",
        &["from", "to"],
        "distance",
    )
}

fn measure_clearance(session: &mut Session, params: &Value) -> Run {
    command(
        session,
        params,
        "measure.clearance",
        "clearance",
        &["from", "to"],
        "clearance",
    )
}

fn measure_thickness(session: &mut Session, params: &Value) -> Run {
    command(
        session,
        params,
        "measure.thickness",
        "thickness",
        &["of"],
        "thickness",
    )
}

fn measure_through(session: &mut Session, params: &Value) -> Run {
    command(
        session,
        params,
        "measure.through",
        "through",
        &["of"],
        "through",
    )
}

fn measure_symmetry(session: &mut Session, params: &Value) -> Run {
    let mut result = command(
        session,
        params,
        "measure.symmetry",
        "symmetric",
        &["of", "plane", "tol"],
        "symmetric",
    )?;
    if let Some(map) = result.as_object_mut()
        && !map.contains_key("plane")
    {
        map.insert(
            "plane".into(),
            params.get("plane").cloned().unwrap_or(Value::Null),
        );
    }
    Ok(result)
}

fn measure_ray(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "measure.ray";
    let p = Params::new(ID, params, &["from", "direction", "of", "limit"])?;
    let limit = p.integer("limit", 1, MAX_HITS as u64)?.unwrap_or(100);
    let limit = usize::try_from(limit).unwrap_or(MAX_HITS);
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let err = |e: String| CommandError::new(format!("{ID}: {e}"));
    let ctx = Ctx::new(doc, &regen).map_err(err)?;
    let from = ctx.point(p.get("from"), "from").map_err(err)?;
    let dir = direction(p.get("direction"), "direction").map_err(err)?;
    let found = match p.get("of") {
        None | Some(Value::Null) => all_bodies(&ctx),
        Some(v) => nonempty(subject(&ctx, v, &["body"]).map_err(err)?, "of").map_err(err)?,
    };
    let bodies = whole_bodies(&ctx, &found, "a ray").map_err(err)?;
    let mut hits: Vec<(f64, FeatureId, craft3d_kernel::boolean::probe::LineHit)> = Vec::new();
    for (id, body) in &bodies {
        let probe = craft3d_kernel::boolean::probe::Probe::new(body, &ctx.tolerance)
            .map_err(|e| err(format!("`{}`: {e}", ctx.feature_name(*id))))?;
        let found = probe
            .line_hits(from, dir, 0.0, f64::INFINITY)
            .map_err(|e| err(format!("`{}`: {e}", ctx.feature_name(*id))))?;
        hits.extend(found.into_iter().map(|h| (h.t, *id, h)));
    }
    hits.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.face.cmp(&b.2.face))
    });
    let count = hits.len();
    let listed: Vec<Value> = hits
        .iter()
        .take(limit)
        .map(|(t, id, h)| {
            json!({
                "distance": t,
                "point": vec3(h.point),
                "body": ctx.feature_name(*id),
                "feature": id.to_string(),
                "face": ctx.face_name(*id, h.face),
                "enters": h.facing.map(|f| f < 0.0),
                "edge": h.boundary,
                "tangent": h.tangent,
            })
        })
        .collect();
    Ok(json!({
        "hits": listed,
        "count": count,
        "truncated": count > limit,
        "from": vec3(from),
        "direction": vec3(dir),
        "lengthUnit": length_unit(doc),
    }))
}
