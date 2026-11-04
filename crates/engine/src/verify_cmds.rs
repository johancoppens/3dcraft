//! The verification queries (`measure.*`): read-only questions about the
//! design's geometry, and the measures assertions are checked with
//! (`assert_cmds.rs`). The design note is docs/design/verification.md;
//! the geometry is in `verify.rs`.
//!
//! What a measure is taken of is named the way `select.query` names
//! things: a body by its feature's name or id, or a query `{"kind",
//! "where"}` (or a bare filter, with the measure's own default kind).

use crate::commands::{Run, length_unit, spec, vec3};
use crate::measure_pieces::PairAnswer;
use crate::params::Params;
use crate::select_cmds::{Found, query};
use crate::selection::SelectionKind;
use crate::verify::{self, Part};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Document, FeatureId};
use craft3d_kernel::boolean::{Contact, common_volume};
use craft3d_math::Portable;
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
/// Most pairs of bodies a clearance compares.
const MAX_BODY_PAIRS: usize = 500;
/// Most hits a ray lists.
const MAX_HITS: usize = 1_000;
/// Most pairs of bodies whose boxes meet an overlap checks exactly (each
/// a boolean of the two).
const MAX_OVERLAP_CHECKED: usize = 2_000;

const SUBJECT: &str = "a body (feature name or id; {body, follow: true} for the newest body that grew from it), a list of them, a query {kind, where} as select.query takes, or a filter (with the measure's own kind)";

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("measure.distance", "Distance", r#"{"from": subject, "to": subject}"#,
        "The least distance between two sets of faces (a subject is a body by name or id, a list of them, a query {kind, where} as select.query takes, or a bare filter, which selects faces): distance in the document's length unit, from and to: the nearest points, each with the body and the face (persistent name) it lies on. Found on a mesh and settled on the exact faces (their surfaces and edges, by projection), so it is exact for planes, cylinders, cones, spheres and tori and as close as projection gets on freeform faces; atLeast is proven (the mesh's distance less what it can be off by). Read-only. Faces that touch or cross are 0 apart; for whether bodies overlap use measure.clearance.", doc: true, names: ["from", "to"], measure_distance),
    spec!("measure.clearance", "Clearance", r#"{"from": subject, "to": subject}"#,
        "The least clearance between two sets of bodies (a bare filter selects bodies): for every pair, status clear (with its distance), touch (0) or overlap (one in the other, or a common volume), and the least: clearance (null when a pair overlaps) with the pair and the nearest points. Read-only; at most 500 pairs.", doc: true, names: ["from", "to"], measure_clearance),
    spec!("measure.thickness", "Wall Thickness", r#"{"of": subject}"#,
        "The thinnest place of a body (or of the faces given, the places measured from): at sampled points of every face a line goes into the material along the inward normal, the thickness there is how far it goes before it leaves the body (exact, the line against the faces' surfaces), and the least sample is moved along its face to where it is least. Returns thickness (the least found, at a real place), atLeast (proven: no place is thinner), at (the point and its face), to (where the line leaves, and that face), samples. Exact where the wall has parallel or concentric sides; a searched least otherwise. Near an acute edge the ray method's thickness tends to 0. Read-only.", doc: true, names: ["of"], measure_thickness),
    spec!("measure.ray", "Ray Cast", r#"{"from": [expr, expr, expr], "direction": "x|y|z|-x|-y|-z" or [number, number, number], "of": subject = all bodies, "limit": integer = 100}"#,
        "Casts a ray through the model and lists every face it meets, nearest first: distance from the start along the ray, point, body, face (persistent name), enters (true where the ray goes into the material, false where it comes out), edge (true where it crosses at an edge or vertex, met by each face there), tangent (grazes without crossing). Exact (the line against each face's surface, kept to the face). Read-only.", doc: true, names: ["of"], measure_ray),
    spec!("measure.through", "Goes Through", r#"{"of": subject}"#,
        "Whether holes go through: for every cylindrical or conical face named (a bare filter selects faces, such as {\"hole\": true} or {\"madeBy\": \"Drill\"}), whether the whole bore is clear: a cylinder on the face's axis of its narrowest radius (less a margin), as long as the body and beyond, has no point in common with the body (the boolean's exact intersection; one line down the axis would pass a blind hole with a vent). Per face: through, the axis (point, direction), radius, and where it is blocked (a point of the material in the bore and the faces around it); why not, also for a face whose axis runs in the material (a boss, not a hole). through: all of them. Read-only.", doc: true, names: ["of"], measure_through),
    spec!("measure.symmetry", "Symmetry", r#"{"of": subject = all bodies, "plane": "xy|yz|zx" or datum plane (name or id) or {point: [expr, expr, expr], normal: [number, number, number]}, "tol": length = 10 × the document's tolerance}"#,
        "Whether the bodies (or faces) are their own mirror image in a plane: points of every face (its mesh's vertices, on the exact faces) are mirrored and the distance to the nearest face found exactly; deviation is the largest, symmetric whether it is within tol, worst where (point, body, face). A feature on one side only is found from its own faces. Read-only.", doc: true, names: ["of", "plane"], measure_symmetry),
];

// ------------------------------------------------------------ context

/// What a measure is taken in.
pub(crate) struct Ctx<'a> {
    pub doc: &'a Document,
    pub regen: &'a Regen,
    pub tolerance: Tolerance,
    /// How many times finer than at first a measure that meshes (the box
    /// of a freeform body) is to look: each step an eighth of the
    /// tolerance before. An assertion the first look leaves undecided is
    /// measured again with this raised.
    pub refine: u32,
    /// Most triangles of a mesh made to box one body
    /// ([`BOX_TRIANGLE_BUDGET`]).
    pub box_budget: usize,
    /// The document's outputs as far as they are computed, for the
    /// expressions of an assertion or of an output's measure (an input's
    /// expression never reaches them): none for a measure command.
    pub outputs: Option<&'a crate::output_cmds::Resolved>,
    /// The outputs without a value that expressions evaluated in this
    /// context needed (one in a branch of `if` that is not taken is not
    /// needed), each with the exact error its evaluation returned, in the
    /// order met: what leaves an assertion undecided rather than in error,
    /// when the error it reports is that one ([`Ctx::unknown_in`]).
    pub unknown_used: std::sync::Mutex<Vec<(String, String)>>,
    /// The assembly `doc` is the placed part of, when it is one: what
    /// its joints and poses are measured on (`asm_measure.rs`).
    pub placed: Option<&'a Arc<crate::asm::Placed>>,
    /// What an assertion asks of a least searched for (its `min`): a
    /// proof of a thickness or a distance need go no higher (infinite for
    /// a measure command: proven as close to the value found as it can).
    pub stop: f64,
}

impl<'a> Ctx<'a> {
    pub fn new(doc: &'a Document, regen: &'a Regen) -> Result<Self, String> {
        let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
            .ok_or("the document's tolerance is not positive and finite")?;
        Ok(Ctx {
            doc,
            regen,
            tolerance,
            refine: 0,
            box_budget: BOX_TRIANGLE_BUDGET,
            outputs: None,
            unknown_used: std::sync::Mutex::new(Vec::new()),
            placed: None,
            stop: f64::INFINITY,
        })
    }

    pub(crate) fn feature_name(&self, id: FeatureId) -> String {
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
                Some(taker) => format!(
                    "the body of `{name}` went into {taker}, `{}`; name that feature, or follow the body: {{\"body\": \"{name}\", \"follow\": true}}",
                    self.feature_name(taker)
                ),
                None => format!("`{name}` has no body"),
            }
        })
    }

    /// The feature whose body is now the newest body that grew from the
    /// body of `start`: the body followed through every later feature it
    /// went into as the body that feature changes (its source, a
    /// boolean's target, or a union's tool, whose material the union
    /// keeps). `start` itself while its body is its own. A body that went
    /// into a subtraction or an intersection as the tool did not grow
    /// into the result, and one split into several bodies grew into each
    /// of them: both are errors that say which feature to name instead.
    fn grown_from(&self, start: FeatureId) -> Result<FeatureId, String> {
        use craft3d_doc::{BooleanOp, FeatureKind};
        let mut at = start;
        // Each feature takes a body made before it, so the way is no
        // longer than the history; bounded all the same.
        for _ in 0..=self.regen.features.len() {
            let Some(taker) = self.regen.feature(at).and_then(|r| r.consumed_by) else {
                return Ok(at);
            };
            let (name, by) = (self.feature_name(at), self.feature_name(taker));
            match self.doc.feature(taker).map(|f| &f.kind) {
                Some(FeatureKind::Split(split)) if split.source == at => {
                    return Err(format!(
                        "the body of `{name}` was split by `{by}` into several bodies; name the piece meant"
                    ));
                }
                Some(FeatureKind::Boolean(b)) if b.tool == at && b.op != BooleanOp::Union => {
                    return Err(format!(
                        "the body of `{name}` went into `{by}` as the tool of a {}, so no body grew from it",
                        if b.op == BooleanOp::Subtract {
                            "subtraction"
                        } else {
                            "intersection"
                        }
                    ));
                }
                _ => at = taker,
            }
        }
        Err(format!(
            "the bodies `{}` went into do not end (a bug in 3DCraft, please report it)",
            self.feature_name(start)
        ))
    }

    /// The persistent name of face `face` of the body of `id`.
    pub(crate) fn face_name(&self, id: FeatureId, face: usize) -> Value {
        self.regen
            .feature(id)
            .and_then(|f| f.names.as_ref())
            .and_then(|n| n.faces.get(face))
            .map_or_else(|| Value::from(face), |name| name.to_string().into())
    }

    /// The output without a value that the error `error` (as an
    /// evaluation in this context reported it, perhaps inside a measure's
    /// own words) is about: one this context needed, whose evaluation
    /// returned exactly an error that `error` holds (matched on that
    /// string, not on how an error happens to be worded). An unrelated
    /// error, met after an unknown output was needed elsewhere, is about
    /// none.
    pub fn unknown_in(&self, error: &str) -> Option<String> {
        let used = self
            .unknown_used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        used.iter()
            .find(|(_, returned)| !returned.is_empty() && error.contains(returned.as_str()))
            .map(|(name, _)| name.clone())
    }

    /// A length, area or volume (power 1, 2, 3) or a plain number
    /// (power 0) from a number in the document's unit or an expression.
    pub fn quantity(&self, value: &Value, key: &str, power: i32) -> Result<f64, String> {
        self.quantity_of(value, key, craft3d_expr::Dimension::new(power, 0))
    }

    /// The value of the expression `source` in the document's variables
    /// and, where this context has them, its outputs.
    pub fn evaluate(&self, source: &str, key: &str) -> Result<craft3d_expr::Quantity, String> {
        if let Some(why) = power_of_a_quantity(source) {
            return Err(format!("`{key}`: {why}"));
        }
        let evaluation = self.doc.variable_table().ok().map(|t| t.evaluate());
        match (&evaluation, self.outputs) {
            (Some(evaluation), Some(outputs)) => {
                evaluation.evaluate_with(source, |name| outputs.lookup(name))
            }
            (Some(evaluation), None) => evaluation.evaluate(source),
            (None, _) => craft3d_expr::evaluate(source, |_| None),
        }
        .map_err(|e| {
            let returned = format!("`{key}`: {e}");
            // An output that has no value counts only where it is used:
            // the evaluation is lazy (the branch of `if` not taken is not
            // needed), and says which one it needed; kept with the exact
            // error returned, which is what `unknown_in` matches.
            if let (craft3d_expr::ErrorKind::DependencyFailed { name }, Some(outputs)) =
                (&e.kind, self.outputs)
                && outputs.unknown(name).is_some()
            {
                let mut used = self
                    .unknown_used
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let entry = (name.clone(), returned.clone());
                if !used.contains(&entry) {
                    used.push(entry);
                }
            }
            returned
        })
    }

    /// A value of dimension `wanted` in the document's units (a length
    /// unit to its power; an angle in radians), from a number in them or
    /// an expression; a plain number is taken as in those units.
    pub fn quantity_of(
        &self,
        value: &Value,
        key: &str,
        wanted: craft3d_expr::Dimension,
    ) -> Result<f64, String> {
        let number = match value {
            Value::Number(n) => n.as_f64(),
            Value::String(source) => {
                let quantity = self.evaluate(source, key)?;
                if quantity.dimension.is_dimensionless() {
                    Some(quantity.value)
                } else if quantity.dimension == wanted {
                    Some(quantity.value / scale_of(self.doc, wanted))
                } else {
                    return Err(format!(
                        "`{key}` is {}, but {} is needed",
                        quantity.dimension.described(),
                        match (wanted.length, wanted.angle, wanted.is_mechanical()) {
                            (0, 0, false) => "a number".to_string(),
                            (1, 0, false) => "a length".to_string(),
                            (2, 0, false) => "an area".to_string(),
                            (3, 0, false) => "a volume".to_string(),
                            _ => wanted.described(),
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

    pub(crate) fn point(&self, value: Option<&Value>, key: &str) -> Result<Vec3, String> {
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
        // (0 mm)^3 is 0 mm^3 and (1 mm)^3 is 1 mm^3: nothing to mistake,
        // so a zero tolerance can be written as one (`0 mm^3`). Only a
        // number of its own: the 1 of `2e1` is part of twenty.
        let own = number_start == 0
            || b.get(number_start - 1)
                .is_some_and(|c| !(c.is_ascii_alphanumeric() || *c == b'_' || *c == b'.'));
        if own && number.parse::<f64>().is_ok_and(|n| n == 0.0 || n == 1.0) {
            continue;
        }
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
pub(crate) fn direction(value: Option<&Value>, key: &str) -> Result<Vec3, String> {
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
    let found = |id: FeatureId| -> Result<Found, String> {
        ctx.body(id)?;
        Ok(Found {
            kind: SelectionKind::Body,
            feature: id,
            indices: Vec::new(),
            name: ctx.feature_name(id),
        })
    };
    let body_item = |text: &str| -> Result<Found, String> { found(feature_of(ctx.doc, text)?) };
    // `{body, follow}`: the body of a feature, or (`follow`, the default
    // of this form) the newest body that grew from it.
    let followed = |map: &Map<String, Value>| -> Result<Found, String> {
        if let Some(key) = map.keys().find(|k| *k != "body" && *k != "follow") {
            let key: String = key.chars().take(40).collect();
            return Err(format!("a body to follow is {{body, follow}}, not `{key}`"));
        }
        let Some(Value::String(text)) = map.get("body") else {
            return Err("`body` names a feature by name or id".into());
        };
        let follow = match map.get("follow") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err("`follow` is true or false".into()),
        };
        let id = feature_of(ctx.doc, text)?;
        found(if follow { ctx.grown_from(id)? } else { id })
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
                    Value::Object(map) if map.contains_key("body") => followed(map),
                    _ => Err(
                        "a list names bodies by feature name or id, or as {body, follow}"
                            .to_string(),
                    ),
                })
                .collect()
        }
        Value::Object(map) if map.contains_key("body") => Ok(vec![followed(map)?]),
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
/// The faces a subject names (default every body), per body: the
/// feature, its body, the faces' indices (every face of a body named as a
/// whole) and a name for messages.
#[allow(clippy::type_complexity)]
pub(crate) fn faces_of_subject(
    ctx: &Ctx<'_>,
    of: Option<&Value>,
) -> Result<Vec<(FeatureId, Arc<Body>, Vec<usize>, String)>, String> {
    let found = match of {
        None | Some(Value::Null) => all_bodies(ctx),
        Some(v) => nonempty(subject(ctx, v, &["face"])?, "of")?,
    };
    let mut out: Vec<(FeatureId, Arc<Body>, Vec<usize>, String)> = Vec::new();
    for item in &found {
        let body = Arc::clone(ctx.body(item.feature)?);
        let faces: Vec<usize> = match item.kind {
            SelectionKind::Face => item.indices.clone(),
            SelectionKind::Body | SelectionKind::Feature => (0..body.faces.len()).collect(),
            other => {
                return Err(format!(
                    "curvature is measured of faces and bodies; the subject names {} `{}`",
                    other.name(),
                    item.name
                ));
            }
        };
        let name = if item.name.is_empty() {
            ctx.feature_name(item.feature)
        } else {
            item.name.clone()
        };
        out.push((item.feature, body, faces, name));
    }
    Ok(out)
}

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

pub(crate) fn whole_bodies(
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

pub(crate) fn all_bodies(ctx: &Ctx<'_>) -> Vec<Found> {
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

pub(crate) fn nonempty(found: Vec<Found>, key: &str) -> Result<Vec<Found>, String> {
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
    /// A quantity of another dimension than a length's power (a plain
    /// number, an angle, a length times an angle): what the measure
    /// `value` gives for an expression of such a dimension. In the
    /// document's length unit, and radians.
    Of(craft3d_expr::Dimension),
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
    /// and `lower` what nothing can be below; for the side of a box found
    /// on a mesh, the other way round (`number` is reached, the other
    /// bound is where the body stays within). Which side `number` is on
    /// says which one is proven beside it: equal to `upper`, at least
    /// `lower`; equal to `lower`, at most `upper`; else between the two.
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    pub detail: Map<String, Value>,
    /// Why the measure has no answer it can stand by: a search that did
    /// not settle (a floating equilibrium), a proof that ran out of work
    /// or could tell neither way (a bend sequence). An assertion on it is
    /// notProven with this reason, never passed or failed, and an output
    /// of it unknown.
    pub unproven: Option<String>,
}

impl Outcome {
    pub(crate) fn number(dim: Dim, value: f64) -> Self {
        Outcome {
            dim,
            number: Some(value),
            flag: None,
            note: None,
            lower: Some(value),
            upper: Some(value),
            detail: Map::new(),
            unproven: None,
        }
    }

    pub(crate) fn flag(value: bool) -> Self {
        Outcome {
            dim: Dim::Flag,
            number: None,
            flag: Some(value),
            note: None,
            lower: None,
            upper: None,
            detail: Map::new(),
            unproven: None,
        }
    }

    /// A measure of dimension `dim` that found no answer it can stand by,
    /// and why (see [`Outcome::unproven`]).
    pub(crate) fn unproven(dim: Dim, why: String) -> Self {
        Outcome {
            dim,
            number: None,
            flag: None,
            note: None,
            lower: None,
            upper: None,
            detail: Map::new(),
            unproven: Some(why),
        }
    }
}

/// The measures, by name: what `assert.add` takes under `measure`.
pub(crate) const MEASURES: [&str; 53] = [
    "volume",
    "cavity",
    "area",
    "mass",
    "count",
    "extent",
    "boxMin",
    "boxMax",
    "centerOfMass",
    "distance",
    "clearance",
    "thickness",
    "through",
    "overlap",
    "symmetric",
    "volumeBelow",
    "centroidBelow",
    "sectionArea",
    "sectionCentroid",
    "sectionInertia",
    "sectionTorsion",
    "sectionShearCentre",
    "sectionExtent",
    "levelFor",
    "curvature",
    "developable",
    "bendRadius",
    "flangeLength",
    "holeToBend",
    "relief",
    "kawasakiFails",
    "maekawaFails",
    "flatFoldable",
    "value",
    "cornerGap",
    "flatExtent",
    "flatArea",
    "louvreStretch",
    "bendStrain",
    "floatingOffset",
    "floatingHeel",
    "floatingTrim",
    "floatingGM",
    "floatingGML",
    "floatingStable",
    "bendable",
    "continuity",
    "draft",
    "textExtent",
    "jointAngle",
    "jointDistance",
    "position",
    "rotation",
];

/// Takes the measure `name` with the parameters `p`.
pub(crate) fn take(ctx: &Ctx<'_>, name: &str, p: &Map<String, Value>) -> Result<Outcome, String> {
    match name {
        "volume" | "cavity" | "area" | "mass" => mass_measure(ctx, name, p),
        "count" => count(ctx, p),
        "extent" | "boxMin" | "boxMax" => extent(ctx, name, p),
        "centerOfMass" => center_of_mass(ctx, p),
        "distance" => distance(ctx, p),
        "clearance" => clearance(ctx, p),
        "thickness" => thickness(ctx, p),
        "through" => through(ctx, p),
        "overlap" => overlap(ctx, p),
        "symmetric" => symmetric(ctx, p),
        "volumeBelow" | "centroidBelow" | "sectionArea" | "sectionCentroid" | "sectionInertia"
        | "sectionTorsion" | "sectionShearCentre" | "sectionExtent" | "levelFor"
        | "floatingOffset" | "floatingHeel" | "floatingTrim" | "floatingGM" | "floatingGML"
        | "floatingStable" => crate::below_cmds::measure(ctx, name, p),
        "curvature" | "developable" => crate::develop_cmds::take(ctx, name, p),
        "bendRadius" | "flangeLength" | "holeToBend" | "relief" | "cornerGap" | "flatExtent"
        | "flatArea" | "louvreStretch" | "bendStrain" => crate::sheetmetal_cmds::take(ctx, name, p),
        "kawasakiFails" | "maekawaFails" | "flatFoldable" => crate::fold_cmds::take(ctx, name, p),
        "bendable" => crate::bendseq_cmds::take(ctx, p),
        "draft" => crate::analysis::draft_measure(ctx, p),
        "value" => value(ctx, p),
        "continuity" => crate::continuity_cmds::take(ctx, p),
        "textExtent" => crate::text_cmds::take(ctx, p),
        "jointAngle" | "jointDistance" | "position" | "rotation" => {
            crate::asm_measure::take(ctx, name, p)
        }
        _ => Err(format!(
            "unknown measure `{}`; one of {}",
            name.chars().take(40).collect::<String>(),
            MEASURES.join(", ")
        )),
    }
}

/// The dimension a value of `dimension` is measured in: a length's power
/// as the other measures are, anything else as it is.
pub(crate) fn dim_of_dimension(dimension: craft3d_expr::Dimension) -> Dim {
    if dimension == craft3d_expr::Dimension::MASS {
        Dim::Mass
    } else if dimension.angle == 0 && !dimension.is_mechanical() && dimension.length != 0 {
        Dim::Length(dimension.length)
    } else {
        Dim::Of(dimension)
    }
}

/// The dimension of a measure's numbers: a mass in kg, a count and a yes
/// or no plain.
pub(crate) fn dimension_of_dim(dim: Dim) -> craft3d_expr::Dimension {
    match dim {
        Dim::Length(k) => craft3d_expr::Dimension::new(k, 0),
        Dim::Of(d) => d,
        Dim::Mass => craft3d_expr::Dimension::MASS,
        Dim::Count | Dim::Flag => craft3d_expr::Dimension::NONE,
    }
}

/// How many base units one of the units a value of `dimension` is given
/// in, in `doc` (a number of them is "in the document's units"): its
/// length unit to the power for a length, an area, a volume; N, MPa, N m
/// and kg for a force, a stress, a moment and a mass, whatever the length
/// unit (`Dimension::default_unit`). 1 where that would not be a number.
pub(crate) fn scale_of(doc: &Document, dimension: craft3d_expr::Dimension) -> f64 {
    let scale = dimension
        .default_unit(doc.units.length.as_expr_unit())
        .factor;
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// The measure `value`: the value of the expression `expr` in the
/// document's variables and outputs, exactly (docs/design/verification.md
/// › *Outputs*): what an assertion on an output measures.
fn value(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["expr"])?;
    let Some(Value::String(source)) = p.get("expr") else {
        return Err("`expr` is required: an expression of the variables and outputs".into());
    };
    let quantity = ctx.evaluate(source, "expr")?;
    let value = quantity.value / scale_of(ctx.doc, quantity.dimension);
    if !value.is_finite() {
        return Err(format!("`expr`: `{source}` is not finite"));
    }
    Ok(Outcome::number(dim_of_dimension(quantity.dimension), value))
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
                let mass = mass_of_in(ctx, id, &body)
                    .map_err(|e| format!("`{fname}` cannot be measured: {e}"))?;
                match name {
                    "volume" => mass.volume,
                    // What the body's cavities hold: the shells that enclose
                    // a negative volume (their faces point into the hollow,
                    // craft3d_topo::ShellVolume), so that a shell's capacity
                    // is measured on the shell itself.
                    "cavity" => {
                        let mut held = 0.0;
                        for shell in crate::mass::shells_of(&body) {
                            let shell =
                                shell.map_err(|e| format!("`{fname}` cannot be measured: {e}"))?;
                            if !shell.volume.is_finite() {
                                return Err(format!(
                                    "`{fname}` cannot be measured: a shell's volume is not a number"
                                ));
                            }
                            if shell.volume < 0.0 {
                                held -= shell.volume;
                            }
                        }
                        held
                    }
                    "area" => mass.area,
                    _ => {
                        let mut entry = Map::new();
                        crate::appearance_cmds::physical_fields(
                            &mut entry,
                            &crate::appearance::Values::new(ctx.doc),
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
        "volume" | "cavity" => Dim::Length(3),
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

/// The size of the bodies' box along an axis (`extent`), or where it
/// begins (`boxMin`) or ends (`boxMax`) along it: one side at a time, so
/// that what sticks out on each side can be bounded on its own.
fn extent(ctx: &Ctx<'_>, name: &str, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["of", "axis"])?;
    let axis = axis_param(p)?;
    let found = match p.get("of") {
        None | Some(Value::Null) => all_bodies(ctx),
        Some(v) => nonempty(subject(ctx, v, &["body"])?, "of")?,
    };
    // The box the bodies reach (`inner`; none when a body's is not known)
    // and the one they stay within (`outer`).
    let (mut inner, mut outer) = (Some(Bbox3::from_points([])), Bbox3::from_points([]));
    let mut any = false;
    let mut meshed_at: Option<f64> = None;
    for (id, body) in whole_bodies(ctx, &found, name)? {
        // A freeform body is boxed on a mesh whose tolerance its area
        // sets: measured as the mass properties are everywhere
        // ([`mass_of_in`]). Any other body's box is exact, measuring
        // nothing.
        if is_freeform_body(&body) {
            let _measured = mass_of_in(ctx, id, &body);
        }
        let boxed = body_box(&body, ctx.doc.tolerance.linear, ctx.refine, ctx.box_budget)
            .map_err(|e| format!("`{}` has no box: {e}", ctx.feature_name(id)))?;
        let Some(boxed) = boxed else {
            continue;
        };
        any = true;
        outer.add(boxed.outer.0);
        outer.add(boxed.outer.1);
        match (inner.as_mut(), boxed.inner) {
            (Some(all), Some((lo, hi))) => {
                all.add(lo);
                all.add(hi);
            }
            _ => inner = None,
        }
        if let Some(t) = boxed.meshed_at {
            meshed_at = Some(meshed_at.map_or(t, |m: f64| m.max(t)));
        }
    }
    let (olo, ohi) = outer
        .corners()
        .filter(|_| any)
        .ok_or("there are no bodies to measure")?;
    let pick = |v: Vec3| [v.x, v.y, v.z].get(axis).copied().unwrap_or(0.0);
    let (olo_a, ohi_a) = (pick(olo), pick(ohi));
    let corners = inner.as_ref().and_then(Bbox3::corners);
    // The value, and the range the true one lies in. With the box reached
    // the value is its side (reached: the other bound is where the bodies
    // stay within); without, it is the outer side, and the range is all
    // the outer box allows.
    let (value, lower, upper) = match corners {
        Some((ilo, ihi)) => {
            let (ilo, ihi) = (pick(ilo), pick(ihi));
            match name {
                "boxMin" => (ilo, olo_a, ilo),
                "boxMax" => (ihi, ihi, ohi_a),
                _ => (ihi - ilo, (ihi - ilo).max(0.0), ohi_a - olo_a),
            }
        }
        None => match name {
            "boxMin" => (olo_a, olo_a, ohi_a),
            "boxMax" => (ohi_a, olo_a, ohi_a),
            _ => (ohi_a - olo_a, 0.0, ohi_a - olo_a),
        },
    };
    let mut out = Outcome::number(Dim::Length(1), value);
    out.lower = Some(lower.min(value));
    out.upper = Some(upper.max(value));
    if let Some((lo, hi)) = corners {
        out.detail.insert("min".into(), vec3(lo));
        out.detail.insert("max".into(), vec3(hi));
    }
    if lower < upper {
        out.detail
            .insert("within".into(), json!([vec3(olo), vec3(ohi)]));
    }
    if let Some(t) = meshed_at {
        out.detail.insert("meshTolerance".into(), t.into());
    }
    Ok(out)
}

/// Most triangles a mesh made to box one body may have: past it the mesh
/// is made coarser, as the viewer's is.
const BOX_TRIANGLE_BUDGET: usize = 2_000_000;

/// How much farther than the chordal tolerance the outer box is grown: the
/// tessellator measures its deviation from the surface at a few points of
/// each triangle and chord (the centroid and the sides' midpoints; a
/// chord's quarters), so the bound it keeps is a measured one, not a
/// proof, and a margin is kept beside it.
const BOX_SAFETY: f64 = 2.0;

/// A body's box: the one it reaches, when known, and the one it stays
/// within.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BodyBox {
    inner: Option<(Vec3, Vec3)>,
    outer: (Vec3, Vec3),
    /// The chordal tolerance of the mesh it was found on, if it was.
    meshed_at: Option<f64>,
}

/// A body's box. Exact ([`Body::held_bbox`], a true bound and the tight
/// one there) for a body of planes, cylinders, cones, spheres and tori
/// bounded by lines, circles and ellipses. That box takes a freeform face
/// (a Bézier patch or a NURBS surface), or a NURBS edge, by its whole
/// control net, however little of it the trimmed face uses (the flank of
/// a cut gear tooth), so such a body is meshed instead:
///
/// - The mesh's vertices lie on the body, to within the largest tolerance
///   of its entities (an edge of a STEP import or an intersection curve
///   may lie that far from its faces), so the mesh's box shrunk by it is
///   reached.
/// - Within the tessellator's measured tolerance no point of the body is
///   farther from the mesh than the chordal tolerance, so the mesh's box
///   grown by [`BOX_SAFETY`] times it, kept within the held box, is where
///   the body stays.
///
/// The chordal tolerance is a hundred-thousandth of the side of a square
/// of the body's area, an eighth of that for each step of `refine`, never
/// below ten times the document's linear tolerance `linear`, and coarser
/// where `budget` triangles would not do. Where the body cannot be meshed
/// within the budget, or measured, the box it reaches is not known and the
/// held box is where it stays. `None` for a body with nothing to box.
fn body_box(
    body: &Arc<Body>,
    linear: f64,
    refine: u32,
    budget: usize,
) -> Result<Option<BodyBox>, String> {
    let held = body.held_bbox().map_err(|e| e.to_string())?;
    let Some((hlo, hhi)) = held.corners() else {
        return Ok(None);
    };
    if !is_freeform_body(body) {
        return Ok(Some(BodyBox {
            inner: Some((hlo, hhi)),
            outer: (hlo, hhi),
            meshed_at: None,
        }));
    }
    let unknown = BodyBox {
        inner: None,
        outer: (hlo, hhi),
        meshed_at: None,
    };
    let area = match crate::mass::mass_of(body) {
        Ok(m) if m.area.is_finite() && m.area > 0.0 => m.area,
        _ => return Ok(Some(unknown)),
    };
    let floor = 10.0 * linear;
    let fine = 8.0_f64.ppowi(i32::try_from(refine.min(8)).unwrap_or(8));
    let tolerance = (area.sqrt() * 1e-5 / fine).max(floor);
    let params = craft3d_tess::TessParams {
        chordal_tolerance: tolerance,
        max_triangles: Some(budget),
        ..craft3d_tess::TessParams::default()
    };
    let Some((mlo, mhi, used)) = crate::mass::mesh_box_within(body, &params) else {
        return Ok(Some(unknown));
    };
    let grow = BOX_SAFETY * used;
    let grow = Vec3::new(grow, grow, grow);
    let olo = (mlo - grow).max(hlo).min(mlo);
    let ohi = (mhi + grow).min(hhi).max(mhi);
    let off = body.tolerances.largest().unwrap_or(0.0).max(linear);
    let off = Vec3::new(off, off, off);
    let (ilo, ihi) = (mlo + off, mhi - off);
    // Shrunk past itself along an axis (a body thinner than its own
    // tolerance): nothing is known to be reached.
    let inner = (ilo.x <= ihi.x && ilo.y <= ihi.y && ilo.z <= ihi.z).then_some((ilo, ihi));
    Ok(Some(BodyBox {
        inner,
        outer: (olo, ohi),
        meshed_at: Some(used),
    }))
}

/// The mass properties of the body `id` made, as `crate::mass::mass_of`
/// gives them; a body with a material is measured with its inertia
/// (`crate::appearance_cmds::measure_ahead`): both at once take about the
/// time of one, and a body with a material is asked for its inertia
/// sooner or later (`document.check`, `document.inspect`), which measured
/// on its own would integrate every face again.
fn mass_of_in(
    ctx: &Ctx<'_>,
    id: craft3d_doc::FeatureId,
    body: &Arc<Body>,
) -> Result<craft3d_topo::MassProperties, craft3d_topo::MassError> {
    crate::appearance_cmds::measure_ahead(ctx.doc, ctx.regen, id, body);
    crate::mass::mass_of(body)
}

/// Whether `body` has a freeform face or edge: then its box comes from a
/// mesh ([`body_box`]), else it is exact.
fn is_freeform_body(body: &Body) -> bool {
    body.faces.iter().any(|f| f.surface.is_freeform())
        || body.edges.iter().any(|e| e.curve.is_freeform())
}

/// `axis` of a measure: 0, 1 or 2.
fn axis_param(p: &Map<String, Value>) -> Result<usize, String> {
    match p.get("axis").and_then(Value::as_str) {
        Some("x") => Ok(0),
        Some("y") => Ok(1),
        Some("z") => Ok(2),
        _ => Err("`axis` must be x, y or z".into()),
    }
}

/// The centre of mass of the bodies (all of them by default) along an
/// axis: the centroid of each body weighted by its mass, so every body
/// needs a material. The coordinate is in the model's frame, in the
/// document's length unit; the whole point is in the detail.
fn center_of_mass(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["of", "axis"])?;
    let axis = axis_param(p)?;
    let found = match p.get("of") {
        None | Some(Value::Null) => all_bodies(ctx),
        Some(v) => nonempty(subject(ctx, v, &["body"])?, "of")?,
    };
    let m = crate::appearance_cmds::metres_per_unit(ctx.doc.units.length);
    let (mut mass, mut moment) = (0.0, Vec3::ZERO);
    let mut listed = Vec::new();
    for (id, body) in whole_bodies(ctx, &found, "centerOfMass")? {
        let fname = ctx.feature_name(id);
        let Some((_, material)) = crate::appearance::body_material(ctx.doc, ctx.regen, id) else {
            return Err(format!(
                "`{fname}` has no material, so no mass; give it one with body.material"
            ));
        };
        let props =
            mass_of_in(ctx, id, &body).map_err(|e| format!("`{fname}` cannot be measured: {e}"))?;
        let centroid = props
            .centroid
            .ok_or_else(|| format!("`{fname}` has no volume, so no centre of mass"))?;
        let density = crate::appearance::density(ctx.doc, &material)
            .map_err(|why| format!("`{fname}`: {why}"))?;
        let body_mass = props.volume * density * m * m * m;
        if !body_mass.is_finite() {
            return Err(format!("the mass of `{fname}` is not a finite number"));
        }
        mass += body_mass;
        moment += centroid * body_mass;
        listed.push(json!({
            "feature": id.to_string(), "name": fname, "mass": body_mass, "centroid": vec3(centroid),
        }));
    }
    if listed.is_empty() {
        return Err("there are no bodies to measure".into());
    }
    if !(mass.is_finite() && mass > 0.0) {
        return Err("the bodies have no mass (a density of 0?), so no centre of mass".into());
    }
    let center = moment * (1.0 / mass);
    let at = [center.x, center.y, center.z]
        .get(axis)
        .copied()
        .unwrap_or(0.0);
    if !at.is_finite() {
        return Err("the centre of mass is not a finite point".into());
    }
    let mut out = Outcome::number(Dim::Length(1), at);
    out.detail.insert("center".into(), vec3(center));
    out.detail.insert("mass".into(), mass.into());
    out.detail.insert("bodies".into(), listed.into());
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
    let near = verify::distance_to(&pa, &pb, ctx.stop)?;
    let mut out = Outcome::number(Dim::Length(1), near.distance);
    out.lower = Some(near.lower);
    out.detail.insert("atLeast".into(), near.lower.into());
    out.detail
        .insert("from".into(), place_json(ctx, &ba, near.face_a, near.a));
    out.detail
        .insert("to".into(), place_json(ctx, &bb, near.face_b, near.b));
    Ok(out)
}

/// The parts of `bodies`, every one made and checked now; or, for a
/// measure taken in pieces that was begun in an earlier one (which did
/// that), none yet: [`part_in`] makes each as a pair needs it.
fn made_parts<'b>(ctx: &Ctx<'_>, bodies: &'b [Named]) -> Result<Vec<Option<Part<'b>>>, String> {
    if crate::measure_pieces::resumed() {
        Ok(bodies.iter().map(|_| None).collect())
    } else {
        Ok(parts(ctx, bodies)?.into_iter().map(Some).collect())
    }
}

/// The part of `bodies[i]` in `made`, made now when it is not there yet.
fn part_in<'p, 'b>(
    ctx: &Ctx<'_>,
    bodies: &'b [Named],
    made: &'p mut [Option<Part<'b>>],
    i: usize,
) -> Result<&'p Part<'b>, String> {
    const NONE: &str = "internal error (a bug in 3DCraft, please report it): no such part";
    let slot = made.get_mut(i).ok_or(NONE)?;
    if slot.is_none() {
        let (id, body, faces) = bodies.get(i).ok_or(NONE)?;
        let part = Part::new(body, faces.as_deref(), &ctx.tolerance)
            .map_err(|e| format!("`{}`: {e}", ctx.feature_name(*id)))?;
        *slot = Some(part);
    }
    slot.as_ref().ok_or_else(|| NONE.to_string())
}

/// One pair of a clearance: the nearest places of `a.0[a.2]` and
/// `b.0[b.2]` and whether they are clear, touch or overlap.
fn clearance_pair<'b>(
    ctx: &Ctx<'_>,
    (ba, pa, i): (&'b [Named], &mut [Option<Part<'b>>], usize),
    (bb, pb, j): (&'b [Named], &mut [Option<Part<'b>>], usize),
    tol: f64,
) -> Result<(verify::Nearest, &'static str), String> {
    const NONE: &str = "internal error (a bug in 3DCraft, please report it): no such body";
    let (ida, body_a, _) = ba.get(i).ok_or(NONE)?;
    let (idb, body_b, _) = bb.get(j).ok_or(NONE)?;
    let part_a = part_in(ctx, ba, pa, i)?;
    let part_b = part_in(ctx, bb, pb, j)?;
    let near = verify::distance_to(
        std::slice::from_ref(part_a),
        std::slice::from_ref(part_b),
        ctx.stop,
    )?;
    let mut status = "clear";
    if near.distance <= 2.0 * tol {
        // Only whether they touch or overlap: not the area they share.
        match craft3d_kernel::boolean::contact(body_a, body_b, &ctx.tolerance) {
            Ok(Contact::Clear) => {}
            Ok(Contact::Touch) => status = "touch",
            Ok(Contact::Overlap) => status = "overlap",
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
    Ok((near, status))
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
    // Taken a pair at a time (`measure_pieces`), the parts are made as
    // the pairs need them: the first piece made and checked every one.
    let (mut pa, mut pb) = (made_parts(ctx, &ba)?, made_parts(ctx, &bb)?);
    let tol = ctx.tolerance.linear;
    let mut pairs = Vec::new();
    let mut least: Option<(f64, Value)> = None;
    let mut lower = f64::INFINITY;
    let mut overlap = false;
    for (i, (ida, ..)) in ba.iter().enumerate() {
        for (j, (idb, ..)) in bb.iter().enumerate() {
            if ida == idb {
                continue;
            }
            let answer = crate::measure_pieces::pair(|| {
                PairAnswer::Clearance(clearance_pair(
                    ctx,
                    (&ba, &mut pa, i),
                    (&bb, &mut pb, j),
                    tol,
                ))
            })?;
            let PairAnswer::Clearance(answer) = answer else {
                return Err("internal error (a bug in 3DCraft, please report it): a pair of another measure".into());
            };
            let (near, status) = answer?;
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
        if let Some(thin) = verify::thickness_to(part, ctx.tolerance.linear, ctx.stop)? {
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

fn overlap(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["of", "to"])?;
    let a = match p.get("of") {
        None | Some(Value::Null) => all_bodies(ctx),
        Some(v) => nonempty(subject(ctx, v, &["body"])?, "of")?,
    };
    let ba = whole_bodies(ctx, &a, "overlap")?;
    let bb = match p.get("to") {
        None | Some(Value::Null) => None,
        Some(v) => Some(whole_bodies(
            ctx,
            &nonempty(subject(ctx, v, &["body"])?, "to")?,
            "overlap",
        )?),
    };
    // Only pairs whose boxes (grown by twice the tolerance) meet can have
    // a volume in common: those are found by sweeping the boxes along x,
    // the others cost nothing, and every pair that meets is checked
    // exactly. More than the bound is an error, never a part of them.
    let boxes = |set: &[(FeatureId, Arc<Body>)]| -> Vec<Option<Bbox3>> {
        set.iter().map(|(_, body)| body.bbox().ok()).collect()
    };
    let (boxes_a, boxes_b) = (boxes(&ba), bb.as_deref().map(boxes));
    let all_pairs = match &bb {
        None => ba.len().saturating_mul(ba.len().saturating_sub(1)) / 2,
        Some(bb) => ba.len().saturating_mul(bb.len()),
    };
    let pairs = meeting_pairs(
        &boxes_a,
        boxes_b.as_deref(),
        2.0 * ctx.tolerance.linear,
        MAX_OVERLAP_CHECKED,
    )
    .ok_or_else(|| {
        format!(
            "more than {MAX_OVERLAP_CHECKED} of the {all_pairs} pairs of bodies have boxes that meet, more than one overlap checks exactly; name fewer bodies with `of` and `to`"
        )
    })?;
    let pair = |n: usize| {
        let (i, j, other) = *pairs.get(n)?;
        let (ida, a) = ba.get(i)?;
        let (idb, b) = if other {
            bb.as_ref().and_then(|bb| bb.get(j))?
        } else {
            ba.get(j)?
        };
        (ida != idb).then_some((*ida, a, *idb, b))
    };
    // Only whether a pair has a volume in common, and how much: the
    // intersection alone (`common_volume`), not the kind of contact that
    // `document.interference` reports. The pairs on several threads, the
    // answers in their order: the first pair in that order that cannot be
    // told is the error, as one after the other.
    let common = |n: usize| {
        let (_, a, _, b) = pair(n)?;
        Some(
            common_volume(a, b, &ctx.tolerance)
                .map(|common| {
                    common.map(|common| {
                        craft3d_topo::mass_properties(&common.body)
                            .ok()
                            .map(|m| m.volume)
                    })
                })
                .map_err(|e| e.to_string()),
        )
    };
    // Taken a pair at a time (`measure_pieces`, a host with one thread):
    // one after the other, each from the pieces before or measured now.
    let answers = if crate::measure_pieces::active() {
        let mut answers = Vec::with_capacity(pairs.len());
        for n in 0..pairs.len() {
            match crate::measure_pieces::pair(|| PairAnswer::Overlap(common(n)))? {
                PairAnswer::Overlap(answer) => answers.push(answer),
                PairAnswer::Clearance(_) => {
                    return Err("internal error (a bug in 3DCraft, please report it): a pair of another measure".into());
                }
            }
        }
        answers
    } else {
        let indices: Vec<usize> = (0..pairs.len()).collect();
        crate::interference_cmds::in_order(&indices, common)
    };
    let mut overlaps = Vec::new();
    for (n, answer) in answers.into_iter().enumerate() {
        let Some((ida, _, idb, _)) = pair(n) else {
            continue;
        };
        let cannot = |why: String| {
            format!(
                "whether `{}` and `{}` overlap cannot be told: {why}",
                ctx.feature_name(ida),
                ctx.feature_name(idb)
            )
        };
        match answer {
            Some(Ok(Some(volume))) => {
                overlaps.push(json!({"a": ctx.feature_name(ida), "b": ctx.feature_name(idb), "volume": volume}));
            }
            Some(Ok(None)) => {}
            Some(Err(e)) => return Err(cannot(e)),
            None => {
                return Err(cannot(
                    "internal error (a bug in 3DCraft, please report it)".into(),
                ));
            }
        }
    }
    // In words: which pairs overlap and by how much ("Bolt overlaps Plate
    // by 12 mm^3"), the first few of them, or that none do.
    const SAID: usize = 3;
    let unit = length_unit(ctx.doc);
    let mut words: Vec<String> = overlaps
        .iter()
        .take(SAID)
        .map(|o| {
            let (a, b) = (
                o["a"].as_str().unwrap_or("?"),
                o["b"].as_str().unwrap_or("?"),
            );
            match o["volume"].as_f64() {
                Some(v) => format!(
                    "{a} overlaps {b} by {} {unit}^3",
                    crate::assert_cmds::shown(v.abs())
                ),
                None => format!("{a} overlaps {b}"),
            }
        })
        .collect();
    if overlaps.len() > SAID {
        words.push(format!(
            "{} more pairs overlap",
            overlaps.len().saturating_sub(SAID)
        ));
    }
    let mut out = Outcome::flag(!overlaps.is_empty());
    out.number = Some(overlaps.len() as f64);
    out.detail.insert("pairs".into(), all_pairs.into());
    out.detail.insert("checked".into(), pairs.len().into());
    out.note = Some(if words.is_empty() {
        "no overlap".to_string()
    } else {
        words.join("; ")
    });
    out.detail.insert("overlaps".into(), overlaps.into());
    Ok(out)
}

/// The pairs of `a` (with `b`: one of `a` and one of `b`, marked `true`;
/// without: two of `a`, `i < j`) whose boxes, grown by `margin`, meet, in
/// order; a body without a box meets every other (the exact check
/// decides). Swept along x: sorted by the boxes' low x, each box is
/// tested only against the boxes still open there. `None` when more than
/// `cap` pairs meet.
pub(crate) fn meeting_pairs(
    a: &[Option<Bbox3>],
    b: Option<&[Option<Bbox3>]>,
    margin: f64,
    cap: usize,
) -> Option<Vec<(usize, usize, bool)>> {
    // (side, index, its box's corners): side 0 is `a`, 1 is `b`.
    type Item = (u8, usize, Option<(Vec3, Vec3)>);
    let corners = |bx: &Option<Bbox3>| bx.as_ref().and_then(Bbox3::corners);
    let mut items: Vec<Item> = a
        .iter()
        .enumerate()
        .map(|(i, bx)| (0, i, corners(bx)))
        .collect();
    if let Some(b) = b {
        items.extend(b.iter().enumerate().map(|(j, bx)| (1, j, corners(bx))));
    }
    let two_sets = b.is_some();
    let mut out: Vec<(usize, usize, bool)> = Vec::new();
    let mut add = |x: &Item, y: &Item| -> bool {
        let pair = if two_sets {
            match (x.0, y.0) {
                (0, 1) => (x.1, y.1, true),
                (1, 0) => (y.1, x.1, true),
                _ => return true,
            }
        } else {
            (x.1.min(y.1), x.1.max(y.1), false)
        };
        out.push(pair);
        out.len() <= cap
    };
    let (mut boxed, unboxed): (Vec<Item>, Vec<Item>) =
        items.into_iter().partition(|item| item.2.is_some());
    // Those without a box meet everything.
    for (k, x) in unboxed.iter().enumerate() {
        for y in unboxed.iter().skip(k.saturating_add(1)).chain(&boxed) {
            if !add(x, y) {
                return None;
            }
        }
    }
    let lo_x = |item: &Item| item.2.map_or(f64::NEG_INFINITY, |(lo, _)| lo.x);
    boxed.sort_by(|x, y| {
        lo_x(x)
            .total_cmp(&lo_x(y))
            .then(x.0.cmp(&y.0))
            .then(x.1.cmp(&y.1))
    });
    let mut open: Vec<&Item> = Vec::new();
    for item in &boxed {
        let Some((lo, hi)) = item.2 else {
            continue;
        };
        open.retain(|o| o.2.is_some_and(|(_, ohi)| ohi.x + margin >= lo.x));
        for o in &open {
            let Some((olo, ohi)) = o.2 else {
                continue;
            };
            let meet = olo.y <= hi.y + margin
                && lo.y <= ohi.y + margin
                && olo.z <= hi.z + margin
                && lo.z <= ohi.z + margin;
            if meet && !add(o, item) {
                return None;
            }
        }
        open.push(item);
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// A plane: a point on it and its normal. A base plane, a datum plane, a
/// flat face by its persistent name (its outward normal; one face of that
/// name in the document), or `{point, normal}`.
pub(crate) fn plane(ctx: &Ctx<'_>, value: Option<&Value>) -> Result<(Vec3, Vec3), String> {
    let wrong = || {
        "`plane` must be xy, yz, zx, a datum plane (name or id), a flat face by its name or {point, normal}"
            .to_string()
    };
    match value {
        Some(Value::String(word)) => match word.as_str() {
            "xy" | "yx" => Ok((Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0))),
            "yz" | "zy" => Ok((Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0))),
            "zx" | "xz" => Ok((Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0))),
            _ => match feature_of(ctx.doc, word) {
                Ok(id) => match ctx.regen.feature(id).and_then(|f| f.datum.as_ref()) {
                    Some(craft3d_regen::Datum::Plane(frame)) => Ok((frame.origin(), frame.z())),
                    _ => Err(format!("`{word}` is not a placed datum plane")),
                },
                Err(_) => face_plane(ctx, word).map_err(|why| format!("{}: {why}", wrong())),
            },
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

/// The plane of the flat face named `name`: a point of it and its outward
/// normal, from the face's exact surface.
fn face_plane(ctx: &Ctx<'_>, name: &str) -> Result<(Vec3, Vec3), String> {
    let found = query(
        ctx.doc,
        ctx.regen,
        Some(&json!(["face"])),
        Some(&json!({ "name": name })),
        2,
    )?;
    let [item] = found.as_slice() else {
        return Err(if found.is_empty() {
            format!(
                "there is no face named `{}`",
                name.chars().take(80).collect::<String>()
            )
        } else {
            format!(
                "more than one face is named `{}`",
                name.chars().take(80).collect::<String>()
            )
        });
    };
    let body = ctx.body(item.feature)?;
    let face = item
        .indices
        .first()
        .and_then(|&i| body.faces.get(i))
        .ok_or("the face is not in its body")?;
    let craft3d_geom::Surface::Plane(plane) = &face.surface else {
        return Err(format!("the face `{name}` is not flat"));
    };
    let frame = plane.frame();
    let normal = if face.reversed { -frame.z() } else { frame.z() };
    Ok((frame.origin(), normal))
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::Workspace;

    /// The sweep finds exactly the pairs a test of every pair finds, in
    /// the same order, for one set and for two, with boxes that touch,
    /// nest, are missing and are far apart; past the cap it says so.
    #[test]
    fn the_box_sweep_finds_the_pairs_every_pair_finds() {
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut boxes = |n: usize| -> Vec<Option<Bbox3>> {
            (0..n)
                .map(|k| {
                    if k % 17 == 5 {
                        return None;
                    }
                    let c = Vec3::new(next() * 50.0, next() * 50.0, next() * 5.0);
                    let h = Vec3::new(next() * 4.0, next() * 4.0, next() * 4.0);
                    Some(Bbox3::from_points([c - h, c + h]))
                })
                .collect()
        };
        let (a, b) = (boxes(120), boxes(45));
        let margin = 1e-6;
        let meet = |x: &Option<Bbox3>, y: &Option<Bbox3>| {
            crate::interference_cmds::boxes_meet(x.as_ref(), y.as_ref(), margin)
        };
        let mut one = Vec::new();
        for (i, x) in a.iter().enumerate() {
            for (j, y) in a.iter().enumerate().skip(i + 1) {
                if meet(x, y) {
                    one.push((i, j, false));
                }
            }
        }
        let mut two = Vec::new();
        for (i, x) in a.iter().enumerate() {
            for (j, y) in b.iter().enumerate() {
                if meet(x, y) {
                    two.push((i, j, true));
                }
            }
        }
        assert!(
            one.len() > 100 && two.len() > 50,
            "{} {}",
            one.len(),
            two.len()
        );
        assert_eq!(
            meeting_pairs(&a, None, margin, usize::MAX),
            Some(one.clone())
        );
        assert_eq!(meeting_pairs(&a, Some(&b), margin, usize::MAX), Some(two));
        assert_eq!(
            meeting_pairs(&a, None, margin, one.len()),
            Some(one.clone())
        );
        assert_eq!(meeting_pairs(&a, None, margin, one.len() - 1), None);
        // Two boxes that only touch, along each axis.
        let unit = |o: Vec3| Some(Bbox3::from_points([o, o + Vec3::new(1.0, 1.0, 1.0)]));
        for d in [Vec3::X, Vec3::Y, Vec3::Z] {
            let pair = [unit(Vec3::ZERO), unit(d)];
            assert_eq!(
                meeting_pairs(&pair, None, 0.0, 10),
                Some(vec![(0, 1, false)])
            );
            let apart = [unit(Vec3::ZERO), unit(d * 1.5)];
            assert_eq!(meeting_pairs(&apart, None, 0.0, 10), Some(vec![]));
        }
    }

    /// A parabolic segment (a NURBS face from `sketch.curve`) extruded:
    /// its body.
    fn segment() -> Arc<Body> {
        let mut s = Session::new(Workspace::none());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("sketch.create", json!({"name": "Dish"})),
            (
                "sketch.curve",
                json!({"sketch": "Dish", "x": "t", "y": "t^2 / 40 mm", "t": ["-20 mm", "20 mm"]}),
            ),
            (
                "sketch.line",
                json!({"sketch": "Dish", "from": "e2", "to": "e1"}),
            ),
            (
                "feature.extrude",
                json!({"sketch": "Dish", "distance": 5, "name": "Segment"}),
            ),
        ] {
            s.execute(id, &params)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        let regen = s.document_mut().unwrap().regen();
        let (_, body) = regen.bodies().next().unwrap();
        Arc::clone(body)
    }

    /// Past the triangle budget the mesh is made coarser; a body no mesh
    /// within the budget fits has no box it is known to reach, and the
    /// held box (a true bound) is where it stays.
    #[test]
    fn a_box_too_fine_for_the_budget_is_coarser_or_only_the_held_box() {
        let body = segment();
        let held = body.held_bbox().unwrap().corners().unwrap();
        let fine = body_box(&body, 1e-6, 0, BOX_TRIANGLE_BUDGET)
            .unwrap()
            .unwrap();
        let fine_at = fine.meshed_at.unwrap();
        // A small budget: a coarser mesh, still a box reached.
        let coarse = body_box(&body, 1e-6, 0, 400).unwrap().unwrap();
        assert!(coarse.meshed_at.unwrap() > fine_at, "{coarse:?}");
        // What is reached lies within where it stays, and that within the
        // held box.
        for b in [fine, coarse] {
            let ((ilo, ihi), (olo, ohi)) = (b.inner.unwrap(), b.outer);
            assert!(olo.x <= ilo.x && olo.y <= ilo.y && olo.z <= ilo.z, "{b:?}");
            assert!(ihi.x <= ohi.x && ihi.y <= ohi.y && ihi.z <= ohi.z, "{b:?}");
            assert!(
                olo.x >= held.0.x && olo.y >= held.0.y && ohi.x <= held.1.x,
                "{b:?}"
            );
            assert!(ohi.y <= held.1.y, "{b:?}");
        }
        // A budget of one triangle: no mesh fits.
        let none = body_box(&body, 1e-6, 0, 1).unwrap().unwrap();
        assert_eq!(none.inner, None);
        assert_eq!(none.outer, held);
        assert_eq!(none.meshed_at, None);
        // Finer on request, an eighth each step, down to ten linear
        // tolerances.
        let finer = body_box(&body, 1e-6, 1, BOX_TRIANGLE_BUDGET)
            .unwrap()
            .unwrap();
        assert!(finer.meshed_at.unwrap() < fine_at / 7.0, "{finer:?}");
        let floor = body_box(&body, 1e-3, 3, BOX_TRIANGLE_BUDGET)
            .unwrap()
            .unwrap();
        assert!((floor.meshed_at.unwrap() - 1e-2).abs() < 1e-12, "{floor:?}");
    }

    /// Where no mesh fits the budget, the box measures still answer, as a
    /// range: all the held box allows. A bound on the side the held box
    /// holds is met soundly (a max on boxMax or extent, a min on boxMin);
    /// any other is left unproven, never passed.
    #[test]
    fn without_a_mesh_a_box_measure_is_the_range_the_held_box_allows() {
        let mut s = Session::new(Workspace::none());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("sketch.create", json!({"name": "Dish"})),
            (
                "sketch.curve",
                json!({"sketch": "Dish", "x": "t", "y": "t^2 / 40 mm", "t": ["-20 mm", "20 mm"]}),
            ),
            (
                "sketch.line",
                json!({"sketch": "Dish", "from": "e2", "to": "e1"}),
            ),
            (
                "feature.extrude",
                json!({"sketch": "Dish", "distance": 5, "name": "Segment"}),
            ),
        ] {
            s.execute(id, &params)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        let open = s.document_mut().unwrap();
        let regen = open.regen();
        let doc = Arc::clone(open.history.current());
        let mut ctx = Ctx::new(&doc, &regen).unwrap();
        ctx.box_budget = 1;
        let mut along_x = Map::new();
        along_x.insert("axis".into(), "x".into());
        let decide = |out: &Outcome, lo: Option<f64>, hi: Option<f64>| {
            crate::assert_cmds::decide(out, None, (lo, hi), Dim::Length(1), 1e-6)
        };
        // The segment spans x from -20 to 20; the held box at least that.
        let max = take(&ctx, "boxMax", &along_x).unwrap();
        let (lower, upper) = (max.lower.unwrap(), max.upper.unwrap());
        assert!(
            upper >= 20.0 - 1e-9 && lower <= -20.0 + 1e-9,
            "{lower} {upper}"
        );
        assert_eq!(max.number, Some(upper));
        // At most where the held box ends: met. At least 19: not proven.
        assert_eq!(decide(&max, None, Some(upper)), Some(true));
        assert_eq!(decide(&max, Some(19.0), None), None);
        let min = take(&ctx, "boxMin", &along_x).unwrap();
        let lowest = min.lower.unwrap();
        assert_eq!(decide(&min, Some(lowest), None), Some(true));
        assert_eq!(decide(&min, None, Some(-19.0)), None);
        let extent = take(&ctx, "extent", &along_x).unwrap();
        assert_eq!(extent.lower, Some(0.0));
        assert_eq!(
            decide(&extent, None, Some(extent.upper.unwrap())),
            Some(true)
        );
        assert_eq!(decide(&extent, Some(39.0), None), None);
    }
}
