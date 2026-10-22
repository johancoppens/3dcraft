//! The selection's commands: read it, set it, and select by describing
//! (`select.query`, decision B4 of 2026-10-21: JSON filters). The design
//! note is `docs/design/selection.md`.

use crate::commands::{Run, fail, spec};
use crate::params::Params;
use crate::selection::{
    MAX_SELECTION, Resolver, SelectMode, SelectionItem, SelectionKind, feature_named, is_datum,
    summary,
};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{ConstraintKind, Document, FeatureId, FeatureKind, NameRole, PersistentName};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Bbox3, Vec3};
use craft3d_regen::{Names, Regen};
use craft3d_topo::{Body, EdgeId, FaceId};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("select.get", "Selection", "{}",
        "Returns what is selected in the active document: items (each {kind, name, feature}: kind body, face, edge, vertex, feature, sketch or datum; name the feature's name, or the persistent name of a face, edge or vertex; feature the feature's id, or the feature whose body has the face, edge or vertex), count, summary (\"3 bodies, 2 faces\") and dropped: what dropped out since it was last set because it no longer exists, one line each. The selection is shared by the window (click, Shift+click, a box, the feature list) and agents; it is not saved and not undone. Items are checked against the document as it is now: a name stays selected as long as what it names exists.", doc: true, select_get),
    spec!("select.set", "Select", r#"{"items": [{kind, name, feature}], "names": [string], "mode": "replace|add|remove|toggle|clear" = "replace"}"#,
        "Sets the selection of the active document. items names each thing with its kind (optional: read from the name) and, for a face, an edge or a vertex, feature: the body that has it (optional: the first body in history order that has the name). names is the short form: a feature's name or id (a body when it has one, otherwise the sketch, datum or feature) or a persistent name of a face, edge or vertex (document.inspect faceNames or aliases, document.edges). replace makes them the selection, add adds what is not selected yet (in order), remove takes them out, toggle adds what is not selected and removes what is, clear empties it (no items needed). A name that means nothing is an error, and then nothing changes. At most 10000 items. Returns the selection as select.get does, with added and removed counts.", doc: true, select_set),
    spec!("select.query", "Select by Description", r#"{"kind": "body|face|edge|vertex|feature|sketch|datum" or [kind, ...] = every kind, "where": filter or [filter, ...], "then": "none|replace|add|remove" = "none", "limit": integer = 1000}"#,
        "Finds what fits a description and returns it (items as select.get gives them, with body: the name of the body of a face, edge or vertex, and origin: {feature, name}, the feature that first made it, at the end of its persistent name's way, such as the cylinder that drilled a hole: the feature to edit to change it), in document order: bodies, features, sketches and datums in history order, faces, edges and vertices body by body in the order of document.inspect and document.edges. count is how many fit, truncated says whether more fit than limit (at most 10000), summary is one line (\"6 faces in Housing\"). With then the result also becomes the selection (replace), is added to it or taken out of it; the window shows it. A filter is an object; several keys in one object must all hold, an array of filters too. Filters: all: [filters], any: [filters], not: filter; kind: kind or [kinds]; name: glob (* any run, ? one character) on the name, or for a feature-like item also its id; type: glob on the type of the feature (extrude, boolean, pattern, ...; of the body's feature for a face, edge or vertex); in: the body (feature id or name) a face, edge or vertex belongs to, or that body itself; madeBy: feature: a face, edge or vertex whose persistent name comes from that feature anywhere along its way (the cylinder that drilled a hole, the extrusion whose side it is), a body or feature made by it or from it; copyOf: pattern or mirror feature, with instance: n: entities that are copies made by it (of that instance), or its body; color: \"#rrggbb\" or [r, g, b] or null (no colour of its own), within: distance in RGB = 0: the colour of the body, or of the face; material: glob or null: the body's material; volume (bodies), area (bodies and faces), radius (cylindrical and spherical faces, circular edges), length (edges): a value (\"6 mm\", a number in the document's unit) or {eq, tol} or {min, max}; tol defaults to a millionth of eq; surface: plane|cylinder|cone|sphere|torus|freeform or a list: the kind of a face's surface; hole: true for the wall of a hole, a cylindrical face whose outward normal points at its axis and that goes all the way round it (not a concave fillet), false for the others; a face a boolean split into pieces that keep its name is one item; normal: {along: axis} (a flat face whose outward normal points along it, \"-y\" for the faces facing -y) or {perpendicular: axis} (flat faces whose normal is square to it, and cylinders and cones whose axis lies along it), with tol: degrees = 0.5; axis: {along: axis, tol}: cylinders, cones, tori and circular edges whose axis lies along it, either way round; inside: [[x, y, z], [x, y, z]]: the item lies wholly within the box; above: {z: value} (or x, y; a number is z): wholly at or above it, below: likewise at or below. An axis is x, y, z, -x, -y, -z or [x, y, z]. A filter that does not apply to an item (volume of a face) does not hold for it. Measures and boxes use the exact geometry (face areas by the integrals document.measure sums).", doc: true, select_query),
];

/// Most filters one query holds, nested ones included.
const MAX_FILTERS: usize = 1000;
/// Deepest nesting of filters.
const MAX_DEPTH: usize = 32;
/// Most items a query looks at.
const MAX_CANDIDATES: usize = 4_000_000;
/// Longest glob, in characters.
const MAX_GLOB: usize = 512;
/// Samples along a curve that is not a line or a circle for its length.
const LENGTH_SAMPLES: u32 = 256;
/// The default result size.
const DEFAULT_LIMIT: u64 = 1000;

fn select_get(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("select.get", params, &["context"])?;
    let context = p.bool("context")?.unwrap_or(true);
    session.document()?;
    let view = session.selection();
    let mut result = view.json();
    if context {
        let open = session.document_mut()?;
        let regen = open.regen();
        let items = described(open.history.current(), &regen, &view.items);
        if let Some(map) = result.as_object_mut() {
            map.insert("items".into(), items.into());
        }
    }
    Ok(result)
}

fn select_set(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "select.set";
    let p = Params::new(ID, params, &["items", "names", "set", "mode"])?;
    let mode = match p.str("mode")? {
        None => SelectMode::Replace,
        Some(word) => SelectMode::parse(word).ok_or_else(|| {
            CommandError::new(format!(
                "{ID}: `mode` must be replace, add, remove, toggle or clear"
            ))
        })?,
    };
    let mut wanted: Vec<(Option<SelectionKind>, String, Option<String>)> = Vec::new();
    let too_many = || {
        CommandError::new(format!(
            "{ID}: at most {MAX_SELECTION} items can be selected at once"
        ))
    };
    match p.get("items") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            if items.len() > MAX_SELECTION {
                return Err(too_many());
            }
            for item in items {
                let Value::Object(map) = item else {
                    return fail(format!(
                        "{ID}: each of `items` is an object {{kind, name, feature}}"
                    ));
                };
                if let Some(key) = map
                    .keys()
                    .find(|k| !["kind", "name", "feature"].contains(&k.as_str()))
                {
                    let key: String = key.chars().take(40).collect();
                    return fail(format!(
                        "{ID}: an item has kind, name and feature, not `{key}`"
                    ));
                }
                let kind = match map.get("kind") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(word)) => {
                        Some(SelectionKind::parse(word).ok_or_else(|| kind_error(ID))?)
                    }
                    Some(_) => return Err(kind_error(ID)),
                };
                let feature = match map.get("feature") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(text)) => Some(text.clone()),
                    Some(Value::Number(n)) if n.is_u64() => Some(n.to_string()),
                    Some(_) => {
                        return fail(format!("{ID}: `feature` must be a feature id or name"));
                    }
                };
                let name = match map.get("name") {
                    Some(Value::String(name)) => name.clone(),
                    None | Some(Value::Null) if feature.is_some() => String::new(),
                    _ => return fail(format!("{ID}: each item needs a `name` (a string)")),
                };
                wanted.push((kind, name, feature));
            }
        }
        Some(_) => return fail(format!("{ID}: `items` must be an array of objects")),
    }
    match p.get("names") {
        None | Some(Value::Null) => {}
        Some(Value::Array(names)) => {
            if names.len().saturating_add(wanted.len()) > MAX_SELECTION {
                return Err(too_many());
            }
            for name in names {
                let Some(name) = name.as_str() else {
                    return fail(format!("{ID}: `names` must be an array of strings"));
                };
                wanted.push((None, name.to_string(), None));
            }
        }
        Some(_) => return fail(format!("{ID}: `names` must be an array of strings")),
    }
    let set = p.str("set")?;
    if wanted.is_empty()
        && set.is_none()
        && !matches!(mode, SelectMode::Clear | SelectMode::Replace)
    {
        return fail(format!(
            "{ID}: give `items`, `names` or `set` to {}",
            match mode {
                SelectMode::Add => "add",
                SelectMode::Remove => "remove",
                _ => "toggle",
            }
        ));
    }
    if mode == SelectMode::Clear && (!wanted.is_empty() || set.is_some()) {
        return fail(format!("{ID}: `clear` takes no items"));
    }
    // Checked against the document as it is now, so that what dropped out
    // before is said before the new choice replaces it.
    session.selection();
    let open = session.document_mut()?;
    let regen = open.regen();
    let mut resolver = Resolver::new(open.history.current(), &regen);
    let mut items = Vec::with_capacity(wanted.len());
    for (kind, name, feature) in &wanted {
        let found = resolver
            .resolve(*kind, name, feature.as_deref())
            .map_err(|why| CommandError::new(format!("{ID}: {why}")))?;
        items.push(found.item);
    }
    drop(resolver);
    if let Some(name) = set {
        let held = set_items(open.history.current(), &regen, name)
            .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
        if held.len().saturating_add(items.len()) > MAX_SELECTION {
            return Err(too_many());
        }
        items.extend(held);
    }
    let (added, removed) = open.selection.apply(items, mode);
    let mut result = described_view(open.history.current(), &regen, &open.selection.view());
    if let Some(map) = result.as_object_mut() {
        map.insert("added".into(), added.into());
        map.insert("removed".into(), removed.into());
    }
    Ok(result)
}

fn kind_error(id: &str) -> CommandError {
    CommandError::new(format!(
        "{id}: `kind` must be body, face, edge, vertex, feature, sketch or datum"
    ))
}

// ------------------------------------------------------------- filters

/// A filter of `select.query`, parsed and checked.
#[derive(Clone, Debug)]
enum Filter {
    All(Vec<Filter>),
    Any(Vec<Filter>),
    Not(Box<Filter>),
    Kind(BTreeSet<SelectionKind>),
    Name(Glob),
    Type(Glob),
    In(FeatureId),
    MadeBy(FeatureId),
    CopyOf {
        feature: FeatureId,
        instance: Option<u64>,
    },
    Color {
        color: Option<[u8; 3]>,
        within: f64,
    },
    Material(Option<Glob>),
    Measure(Measure, Range),
    Surface(BTreeSet<&'static str>),
    Hole(bool),
    Normal {
        along: Option<Vec3>,
        perpendicular: Option<Vec3>,
        tol: f64,
    },
    Axis {
        along: Vec3,
        tol: f64,
    },
    Inside([Vec3; 2]),
    Above(usize, f64),
    Below(usize, f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Measure {
    Volume,
    Area,
    Radius,
    Length,
}

/// A closed range of values.
#[derive(Clone, Copy, Debug)]
struct Range {
    lo: f64,
    hi: f64,
}

impl Range {
    fn holds(self, value: f64) -> bool {
        value.is_finite() && self.lo <= value && value <= self.hi
    }
}

/// A pattern with `*` (any run of characters) and `?` (one character).
#[derive(Clone, Debug)]
struct Glob(Vec<char>);

impl Glob {
    fn new(text: &str, key: &str) -> Result<Self, String> {
        let chars: Vec<char> = text.chars().collect();
        if chars.len() > MAX_GLOB {
            return Err(format!("`{key}` is at most {MAX_GLOB} characters"));
        }
        Ok(Self(chars))
    }

    /// Whether `text` fits: the usual two-pointer match with one point to
    /// go back to, linear in practice and never more than the product of
    /// the lengths.
    fn matches(&self, text: &str) -> bool {
        let text: Vec<char> = text.chars().collect();
        let pattern = &self.0;
        let (mut t, mut p) = (0_usize, 0_usize);
        let mut star: Option<(usize, usize)> = None;
        while t < text.len() {
            match (pattern.get(p), text.get(t)) {
                (Some('*'), _) => {
                    star = Some((p, t));
                    p += 1;
                }
                (Some('?'), Some(_)) => {
                    p += 1;
                    t += 1;
                }
                (Some(a), Some(b)) if a == b => {
                    p += 1;
                    t += 1;
                }
                _ => match star {
                    Some((sp, st)) => {
                        p = sp + 1;
                        t = st + 1;
                        star = Some((sp, st + 1));
                    }
                    None => return false,
                },
            }
        }
        pattern
            .get(p..)
            .is_some_and(|rest| rest.iter().all(|&c| c == '*'))
    }
}

/// What filters are parsed against: the document, for its features,
/// variables, units and named sets.
pub(crate) struct Parser<'a> {
    doc: &'a Document,
    /// The filters parsed so far.
    count: usize,
    /// The named sets being read, outermost first: a set that uses
    /// itself, directly or through others, is an error.
    open: Vec<String>,
}

/// A query: the kinds it looks at (all when `None`) and its filter.
pub(crate) struct Query {
    kinds: Option<BTreeSet<SelectionKind>>,
    filter: Option<Filter>,
}

impl Query {
    /// The query as one filter: of its kinds, and its filter holds.
    fn into_filter(self) -> Filter {
        let mut all = Vec::new();
        if let Some(kinds) = self.kinds {
            all.push(Filter::Kind(kinds));
        }
        if let Some(filter) = self.filter {
            all.push(filter);
        }
        Filter::All(all)
    }
}

impl<'a> Parser<'a> {
    pub(crate) fn new(doc: &'a Document) -> Self {
        Self {
            doc,
            count: 0,
            open: Vec::new(),
        }
    }

    /// A parser for the query of the set `name`, being defined or
    /// changed: a query that comes back to `name` through other sets is
    /// refused.
    pub(crate) fn for_set(doc: &'a Document, name: &str) -> Self {
        Self {
            doc,
            count: 0,
            open: vec![name.to_string()],
        }
    }

    /// A query as a set keeps it: `{"kind": kind or [kinds], "where":
    /// filter}`, either optional.
    pub(crate) fn query(&mut self, value: &Value, depth: usize) -> Result<Query, String> {
        let Value::Object(map) = value else {
            return Err("a query is an object {kind, where}".into());
        };
        only(map, "query", &["kind", "where"])?;
        let kinds = match map.get("kind") {
            None | Some(Value::Null) => None,
            Some(value) => Some(kinds(value)?),
        };
        let filter = match map.get("where") {
            None | Some(Value::Null) => None,
            Some(value) => Some(self.parse(value, depth)?),
        };
        Ok(Query { kinds, filter })
    }

    /// The filter `set: name`: what the named set holds, its query read
    /// from the document as it is now.
    fn set(&mut self, name: &str, depth: usize) -> Result<Filter, String> {
        let shown: String = name.chars().take(80).collect();
        let set = self
            .doc
            .set_named(name)
            .ok_or_else(|| format!("there is no set `{shown}` (set.list lists them)"))?;
        if self.open.iter().any(|open| open == name) {
            return Err(format!(
                "the set `{shown}` uses itself: {} -> {shown}",
                self.open.join(" -> ")
            ));
        }
        let value: Value = serde_json::from_str(&set.query)
            .map_err(|e| format!("the set `{shown}` is not a query: {e}"))?;
        self.open.push(name.to_string());
        let query = self.query(&value, depth + 1);
        self.open.pop();
        Ok(query
            .map_err(|e| format!("the set `{shown}`: {e}"))?
            .into_filter())
    }

    fn parse(&mut self, value: &Value, depth: usize) -> Result<Filter, String> {
        if depth > MAX_DEPTH {
            return Err(format!("filters are nested at most {MAX_DEPTH} deep"));
        }
        self.count = self.count.saturating_add(1);
        if self.count > MAX_FILTERS {
            return Err(format!("a query has at most {MAX_FILTERS} filters"));
        }
        match value {
            Value::Array(items) => Ok(Filter::All(
                items
                    .iter()
                    .map(|item| self.parse(item, depth + 1))
                    .collect::<Result<_, _>>()?,
            )),
            Value::Object(map) => {
                let mut all = Vec::new();
                for (key, inner) in map {
                    if let Some(filter) = self.key(key, inner, map, depth)? {
                        all.push(filter);
                    }
                }
                if all.is_empty() {
                    return Err("a filter object needs at least one filter".into());
                }
                Ok(if all.len() == 1 {
                    all.remove(0)
                } else {
                    Filter::All(all)
                })
            }
            other => Err(format!(
                "a filter is an object or an array of filters, not {}",
                kind_of(other)
            )),
        }
    }

    /// The filter of one key of an object; `None` for a key that only
    /// modifies another (`instance`, `within`).
    fn key(
        &mut self,
        key: &str,
        value: &Value,
        map: &Map<String, Value>,
        depth: usize,
    ) -> Result<Option<Filter>, String> {
        let list = |parser: &mut Self, value: &Value| -> Result<Vec<Filter>, String> {
            let Value::Array(items) = value else {
                return Err(format!("`{key}` takes an array of filters"));
            };
            items
                .iter()
                .map(|item| parser.parse(item, depth + 1))
                .collect()
        };
        let filter = match key {
            "all" => Filter::All(list(self, value)?),
            "any" => Filter::Any(list(self, value)?),
            "not" => Filter::Not(Box::new(self.parse(value, depth + 1)?)),
            "kind" => Filter::Kind(kinds(value)?),
            "name" => Filter::Name(Glob::new(text(value, key)?, key)?),
            "type" => Filter::Type(Glob::new(text(value, key)?, key)?),
            "set" => self.set(text(value, key)?, depth)?,
            "in" => Filter::In(self.feature(value, key)?),
            "madeBy" => Filter::MadeBy(self.feature(value, key)?),
            "copyOf" => Filter::CopyOf {
                feature: self.feature(value, key)?,
                instance: match map.get("instance") {
                    None | Some(Value::Null) => None,
                    Some(n) => Some(n.as_u64().ok_or("`instance` must be a whole number")?),
                },
            },
            "instance" => {
                if !map.contains_key("copyOf") {
                    return Err("`instance` goes with `copyOf`".into());
                }
                return Ok(None);
            }
            "color" => Filter::Color {
                color: color(value)?,
                within: match map.get("within") {
                    None | Some(Value::Null) => 0.0,
                    Some(n) => n
                        .as_f64()
                        .filter(|d| d.is_finite() && *d >= 0.0)
                        .ok_or("`within` must be a distance in RGB, 0 or more")?,
                },
            },
            "within" => {
                if !map.contains_key("color") {
                    return Err("`within` goes with `color`".into());
                }
                return Ok(None);
            }
            "material" => Filter::Material(match value {
                Value::Null => None,
                other => Some(Glob::new(text(other, key)?, key)?),
            }),
            "volume" => Filter::Measure(Measure::Volume, self.range(value, key, 3)?),
            "area" => Filter::Measure(Measure::Area, self.range(value, key, 2)?),
            "radius" => Filter::Measure(Measure::Radius, self.range(value, key, 1)?),
            "length" => Filter::Measure(Measure::Length, self.range(value, key, 1)?),
            "surface" => Filter::Surface(surfaces(value)?),
            "hole" => Filter::Hole(value.as_bool().ok_or("`hole` must be true or false")?),
            "normal" => {
                let Value::Object(fields) = value else {
                    return Err("`normal` takes {along: axis} or {perpendicular: axis}".into());
                };
                only(fields, "normal", &["along", "perpendicular", "tol"])?;
                let along = fields.get("along").map(axis).transpose()?;
                let perpendicular = fields.get("perpendicular").map(axis).transpose()?;
                if along.is_none() && perpendicular.is_none() {
                    return Err("`normal` takes {along: axis} or {perpendicular: axis}".into());
                }
                Filter::Normal {
                    along,
                    perpendicular,
                    tol: self.angle(fields.get("tol"))?,
                }
            }
            "axis" => {
                let Value::Object(fields) = value else {
                    return Err("`axis` takes {along: axis}".into());
                };
                only(fields, "axis", &["along", "tol"])?;
                Filter::Axis {
                    along: axis(fields.get("along").ok_or("`axis` takes {along: axis}")?)?,
                    tol: self.angle(fields.get("tol"))?,
                }
            }
            "inside" => {
                let wrong = || "`inside` takes a box [[x, y, z], [x, y, z]]".to_string();
                let Value::Array(corners) = value else {
                    return Err(wrong());
                };
                let [a, b] = corners.as_slice() else {
                    return Err(wrong());
                };
                let point = |parser: &Self, v: &Value| -> Result<Vec3, String> {
                    let Value::Array(xyz) = v else {
                        return Err(wrong());
                    };
                    let [x, y, z] = xyz.as_slice() else {
                        return Err(wrong());
                    };
                    Ok(Vec3::new(
                        parser.quantity(x, key, 1)?,
                        parser.quantity(y, key, 1)?,
                        parser.quantity(z, key, 1)?,
                    ))
                };
                let (a, b) = (point(self, a)?, point(self, b)?);
                Filter::Inside([a.min(b), a.max(b)])
            }
            "above" | "below" => {
                let (along, at) = match value {
                    Value::Object(fields) => {
                        only(fields, key, &["x", "y", "z"])?;
                        let mut given = fields.iter();
                        let (Some((name, at)), None) = (given.next(), given.next()) else {
                            return Err(format!("`{key}` takes one of x, y and z"));
                        };
                        let along = match name.as_str() {
                            "x" => 0,
                            "y" => 1,
                            _ => 2,
                        };
                        (along, self.quantity(at, key, 1)?)
                    }
                    other => (2, self.quantity(other, key, 1)?),
                };
                if key == "above" {
                    Filter::Above(along, at)
                } else {
                    Filter::Below(along, at)
                }
            }
            other => {
                let other: String = other.chars().take(40).collect();
                return Err(format!(
                    "unknown filter `{other}`; the filters are all, any, not, set, kind, name, type, in, madeBy, copyOf, color, material, volume, area, radius, length, surface, hole, normal, axis, inside, above and below"
                ));
            }
        };
        Ok(Some(filter))
    }

    fn feature(&self, value: &Value, key: &str) -> Result<FeatureId, String> {
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Number(n) if n.is_u64() => n.to_string(),
            _ => return Err(format!("`{key}` takes a feature id or name")),
        };
        feature_named(self.doc, &text).ok_or_else(|| {
            format!(
                "`{key}`: there is no feature `{}`",
                text.chars().take(80).collect::<String>()
            )
        })
    }

    /// A value of length to the `power`, in the document's unit: a number
    /// is one already, an expression is evaluated with the document's
    /// variables and converted.
    fn quantity(&self, value: &Value, key: &str, power: i32) -> Result<f64, String> {
        let number = match value {
            Value::Number(n) => n.as_f64(),
            Value::String(source) => {
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
                        "`{key}` is a {}, but a length{} is needed",
                        quantity.dimension,
                        match power {
                            2 => " squared (an area)",
                            3 => " cubed (a volume)",
                            _ => "",
                        }
                    ));
                }
            }
            _ => None,
        };
        number
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("`{key}` takes a number or an expression such as \"6 mm\""))
    }

    fn range(&self, value: &Value, key: &str, power: i32) -> Result<Range, String> {
        let around = |eq: f64, tol: Option<f64>| {
            let tol = tol
                .unwrap_or_else(|| 1e-6 * eq.abs() + self.doc.tolerance.linear.abs().powi(power));
            Range {
                lo: eq - tol,
                hi: eq + tol,
            }
        };
        match value {
            Value::Object(fields) => {
                only(fields, key, &["eq", "tol", "min", "max"])?;
                let get = |name: &str| -> Result<Option<f64>, String> {
                    fields
                        .get(name)
                        .map(|v| self.quantity(v, key, power))
                        .transpose()
                };
                let (eq, tol, min, max) = (get("eq")?, get("tol")?, get("min")?, get("max")?);
                if let Some(tol) = tol
                    && tol < 0.0
                {
                    return Err(format!("`{key}`: tol must not be negative"));
                }
                match (eq, min, max) {
                    (Some(eq), None, None) => Ok(around(eq, tol)),
                    (None, min, max) if (min.is_some() || max.is_some()) && tol.is_none() => {
                        Ok(Range {
                            lo: min.unwrap_or(f64::NEG_INFINITY),
                            hi: max.unwrap_or(f64::INFINITY),
                        })
                    }
                    _ => Err(format!(
                        "`{key}` takes a value, {{eq, tol}} or {{min, max}}"
                    )),
                }
            }
            other => Ok(around(self.quantity(other, key, power)?, None)),
        }
    }

    /// An angle tolerance: degrees as a number, or an expression with an
    /// angle unit; half a degree when none is given.
    fn angle(&self, value: Option<&Value>) -> Result<f64, String> {
        let degrees = match value {
            None | Some(Value::Null) => 0.5,
            Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
            Some(Value::String(source)) => {
                let quantity =
                    craft3d_expr::evaluate(source, |_| None).map_err(|e| format!("`tol`: {e}"))?;
                if quantity.dimension == craft3d_expr::Dimension::ANGLE {
                    quantity.value.to_degrees()
                } else if quantity.dimension.is_dimensionless() {
                    quantity.value
                } else {
                    return Err("`tol` is an angle".into());
                }
            }
            Some(_) => f64::NAN,
        };
        if degrees.is_finite() && (0.0..=90.0).contains(&degrees) {
            Ok(degrees.to_radians())
        } else {
            Err("`tol` is an angle from 0 to 90 degrees".into())
        }
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn text<'v>(value: &'v Value, key: &str) -> Result<&'v str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("`{key}` takes a string"))
}

fn only(fields: &Map<String, Value>, key: &str, allowed: &[&str]) -> Result<(), String> {
    match fields.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(unknown) => Err(format!(
            "`{key}` takes {}, not `{}`",
            allowed.join(", "),
            unknown.chars().take(40).collect::<String>()
        )),
        None => Ok(()),
    }
}

fn kinds(value: &Value) -> Result<BTreeSet<SelectionKind>, String> {
    let wrong = || {
        "`kind` is body, face, edge, vertex, feature, sketch or datum, or a list of them"
            .to_string()
    };
    let one = |v: &Value| v.as_str().and_then(SelectionKind::parse).ok_or_else(wrong);
    match value {
        Value::Array(items) if !items.is_empty() => items.iter().map(one).collect(),
        Value::Array(_) => Err(wrong()),
        other => Ok(BTreeSet::from([one(other)?])),
    }
}

const SURFACES: [&str; 6] = ["plane", "cylinder", "cone", "sphere", "torus", "freeform"];

fn surfaces(value: &Value) -> Result<BTreeSet<&'static str>, String> {
    let wrong = || format!("`surface` is {} or a list of them", SURFACES.join(", "));
    let one = |v: &Value| {
        v.as_str()
            .and_then(|word| SURFACES.iter().copied().find(|s| *s == word))
            .ok_or_else(wrong)
    };
    match value {
        Value::Array(items) if !items.is_empty() => items.iter().map(one).collect(),
        Value::Array(_) => Err(wrong()),
        other => Ok(BTreeSet::from([one(other)?])),
    }
}

/// An axis: `x`, `-y`, `+z`, or `[x, y, z]`, as a unit vector.
fn axis(value: &Value) -> Result<Vec3, String> {
    let wrong = || "an axis is x, y, z, -x, -y, -z or [x, y, z]".to_string();
    let v = match value {
        Value::String(word) => {
            let (sign, letter) = match word.strip_prefix('-') {
                Some(rest) => (-1.0, rest),
                None => (1.0, word.strip_prefix('+').unwrap_or(word)),
            };
            let unit = match letter {
                "x" | "X" => Vec3::X,
                "y" | "Y" => Vec3::Y,
                "z" | "Z" => Vec3::Z,
                _ => return Err(wrong()),
            };
            unit * sign
        }
        Value::Array(items) => {
            let [x, y, z] = items.as_slice() else {
                return Err(wrong());
            };
            let n = |v: &Value| v.as_f64().filter(|n| n.is_finite()).ok_or_else(wrong);
            Vec3::new(n(x)?, n(y)?, n(z)?)
        }
        _ => return Err(wrong()),
    };
    v.normalized()
        .ok_or_else(|| "an axis needs a direction (not [0, 0, 0])".to_string())
}

fn color(value: &Value) -> Result<Option<[u8; 3]>, String> {
    let wrong = || "`color` is \"#rrggbb\", [r, g, b] (0 to 255) or null".to_string();
    match value {
        Value::Null => Ok(None),
        Value::String(hex) => {
            let digits = hex.strip_prefix('#').unwrap_or(hex);
            if digits.len() != 6 || !digits.is_ascii() {
                return Err(wrong());
            }
            let byte = |at: usize| {
                digits
                    .get(at..at + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    .ok_or_else(wrong)
            };
            Ok(Some([byte(0)?, byte(2)?, byte(4)?]))
        }
        Value::Array(items) => {
            let [r, g, b] = items.as_slice() else {
                return Err(wrong());
            };
            let c = |v: &Value| {
                v.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(wrong)
            };
            Ok(Some([c(r)?, c(g)?, c(b)?]))
        }
        _ => Err(wrong()),
    }
}

// ---------------------------------------------------------- candidates

/// One thing a query looks at.
struct Candidate<'a> {
    kind: SelectionKind,
    /// The feature itself, or the feature whose body has the entity.
    feature: FeatureId,
    /// The place of a face, edge or vertex in its body (of a face cut
    /// into pieces that keep its name: the first piece's).
    index: usize,
    /// A face's pieces: every face of the body with its name, in order.
    /// A boolean can cut a face into pieces that all keep its name; the
    /// name means them together, so they are one item.
    pieces: Vec<usize>,
    name: String,
    /// A face's, edge's or vertex's persistent name.
    persistent: Option<PersistentName>,
    body: Option<&'a Arc<Body>>,
}

/// What filters are evaluated with: the document and its regeneration,
/// and what was worked out per body, once.
struct Context<'a> {
    doc: &'a Document,
    regen: &'a Regen,
    areas: BTreeMap<FeatureId, Vec<Option<f64>>>,
    colors: BTreeMap<FeatureId, Vec<Option<[u8; 3]>>>,
    /// Per feature, the features its body is made from, all the way.
    made_from: BTreeMap<FeatureId, BTreeSet<FeatureId>>,
}

impl<'a> Context<'a> {
    fn new(doc: &'a Document, regen: &'a Regen) -> Self {
        Self {
            doc,
            regen,
            areas: BTreeMap::new(),
            colors: BTreeMap::new(),
            made_from: BTreeMap::new(),
        }
    }

    /// The feature instance `instance` of the body of `feature` was made
    /// from: a boolean's target (0) or tool (1), a pattern's or a
    /// transform's source for every instance.
    fn source_of(&self, feature: FeatureId, instance: u64) -> Option<FeatureId> {
        let built = self
            .regen
            .feature(feature)
            .and_then(|r| r.followed.as_deref())
            .or_else(|| self.doc.feature(feature))?;
        let sources = built.kind.sources();
        usize::try_from(instance)
            .ok()
            .and_then(|i| sources.get(i))
            .or_else(|| sources.first())
            .map(|(id, _)| *id)
    }

    /// The way of a copy through the history: each feature the entity of
    /// persistent name `name` went through as a copy, with the instance
    /// it is in there, from the outside in (names.md, the role copy: a
    /// link lists the instances along the way, a run of instance 0 is
    /// walked feature by feature down to the feature of the link after
    /// it, and a run of instance 0 left out after the listed instances
    /// likewise).
    fn way(&self, name: &PersistentName) -> Vec<(FeatureId, u64)> {
        let mut out = Vec::new();
        for link in chain(name) {
            if link.role != NameRole::Copy {
                continue;
            }
            let stop = link.of.as_deref().map(|of| of.feature);
            let mut at = link.feature;
            if link.sources == [0] {
                // Instance 0 of every feature along the way, until the
                // feature of the name after `/`.
                for _ in 0..=self.doc.features.len() {
                    out.push((at, 0));
                    match self.source_of(at, 0) {
                        Some(next) if Some(next) != stop && next != at => at = next,
                        _ => break,
                    }
                }
            } else {
                let mut walked = true;
                for &n in &link.sources {
                    out.push((at, n));
                    match self.source_of(at, n) {
                        Some(next) => at = next,
                        None => {
                            walked = false;
                            break;
                        }
                    }
                }
                // A run of instance 0 after the listed instances is left
                // out of the name down to the feature of the name after
                // `/` (`PersistentName::canonical`): walk it as instance 0.
                if walked {
                    for _ in 0..=self.doc.features.len() {
                        if Some(at) == stop {
                            break;
                        }
                        out.push((at, 0));
                        match self.source_of(at, 0) {
                            Some(next) if next != at => at = next,
                            _ => break,
                        }
                    }
                }
            }
        }
        out
    }

    /// Whether the entity of persistent name `name` is a copy made by
    /// `pattern` (of instance `instance` when given): its name's way
    /// passes through that feature ([`Context::way`]).
    fn copy_of(&self, name: &PersistentName, pattern: FeatureId, instance: Option<u64>) -> bool {
        self.way(name)
            .into_iter()
            .any(|(at, n)| at == pattern && instance.is_none_or(|want| want == n))
    }

    fn feature_kind(&self, id: FeatureId) -> Option<&'a FeatureKind> {
        self.doc.feature(id).map(|f| &f.kind)
    }

    /// The features the body of `id` was made from, `id` included.
    fn made_from(&mut self, id: FeatureId) -> &BTreeSet<FeatureId> {
        let doc = self.doc;
        let regen = self.regen;
        self.made_from.entry(id).or_insert_with(|| {
            let mut seen = BTreeSet::from([id]);
            let mut todo = vec![id];
            // Bounded: each feature is visited once.
            while let Some(at) = todo.pop() {
                let built = regen
                    .feature(at)
                    .and_then(|r| r.followed.as_deref())
                    .or_else(|| doc.feature(at));
                for (source, _) in built.map(|f| f.kind.sources()).unwrap_or_default() {
                    if seen.insert(source) {
                        todo.push(source);
                    }
                }
            }
            seen
        })
    }

    /// The area of a face: of all its pieces together.
    fn face_area(&mut self, c: &Candidate<'_>) -> Option<f64> {
        let body = c.body?;
        let areas = self.areas.entry(c.feature).or_insert_with(|| {
            craft3d_topo::face_areas(body)
                .into_iter()
                .map(Result::ok)
                .collect()
        });
        c.pieces
            .iter()
            .map(|&k| areas.get(k).copied().flatten())
            .sum()
    }

    fn face_color(&mut self, c: &Candidate<'_>) -> Option<[u8; 3]> {
        let (doc, regen) = (self.doc, self.regen);
        let faces = c.body.map_or(0, |b| b.faces.len());
        self.colors
            .entry(c.feature)
            .or_insert_with(|| {
                crate::appearance::face_appearances(doc, regen, c.feature, faces)
                    .into_iter()
                    .map(|a| a.map(|a| a.color))
                    .collect()
            })
            .get(c.index)
            .copied()
            .flatten()
    }

    fn eval(&mut self, filter: &Filter, c: &Candidate<'_>) -> bool {
        match filter {
            Filter::All(all) => all.iter().all(|f| self.eval(f, c)),
            Filter::Any(any) => any.iter().any(|f| self.eval(f, c)),
            Filter::Not(inner) => !self.eval(inner, c),
            Filter::Kind(kinds) => kinds.contains(&c.kind),
            Filter::Name(glob) => {
                glob.matches(&c.name)
                    || (!c.kind.is_entity() && glob.matches(&c.feature.to_string()))
            }
            Filter::Type(glob) => self
                .feature_kind(c.feature)
                .is_some_and(|k| glob.matches(k.type_name())),
            Filter::In(body) => {
                (c.kind.is_entity() || c.kind == SelectionKind::Body) && c.feature == *body
            }
            Filter::MadeBy(maker) => match &c.persistent {
                Some(name) => chain(name).any(|link| link.feature == *maker),
                None => self.made_from(c.feature).contains(maker),
            },
            Filter::CopyOf { feature, instance } => match &c.persistent {
                Some(name) => self.copy_of(name, *feature, *instance),
                None => c.kind == SelectionKind::Body && c.feature == *feature,
            },
            Filter::Color { color, within } => {
                let has = match c.kind {
                    SelectionKind::Face => self.face_color(c),
                    SelectionKind::Body | SelectionKind::Edge | SelectionKind::Vertex => {
                        crate::appearance::body_appearance(self.doc, self.regen, c.feature)
                            .map(|a| a.color)
                    }
                    _ => return false,
                };
                match (has, color) {
                    (None, None) => true,
                    (Some(has), Some(want)) => {
                        let d: f64 = has
                            .iter()
                            .zip(want)
                            .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                            .sum::<f64>()
                            .sqrt();
                        d <= *within
                    }
                    _ => false,
                }
            }
            Filter::Material(glob) => {
                if c.body.is_none() {
                    return false;
                }
                let has = crate::appearance::body_material(self.doc, self.regen, c.feature)
                    .map(|(_, material)| material.name);
                match (has, glob) {
                    (None, None) => true,
                    (Some(name), Some(glob)) => glob.matches(&name),
                    _ => false,
                }
            }
            Filter::Measure(measure, range) => self
                .measure(*measure, c)
                .is_some_and(|value| range.holds(value)),
            Filter::Surface(kinds) => {
                surface_of(c).is_some_and(|(surface, _)| kinds.contains(surface_kind(surface)))
            }
            Filter::Hole(wanted) => c.kind == SelectionKind::Face && is_hole(c) == *wanted,
            Filter::Normal {
                along,
                perpendicular,
                tol,
            } => {
                let Some((surface, reversed)) = surface_of(c) else {
                    return false;
                };
                let square = |v: Vec3, d: Vec3| v.dot(d).abs() <= tol.sin();
                let parallel = |v: Vec3, d: Vec3| v.dot(d) >= tol.cos();
                match surface {
                    Surface::Plane(plane) => {
                        let z = plane.frame().z();
                        let n = if reversed { -z } else { z };
                        along.is_none_or(|d| parallel(n, d))
                            && perpendicular.is_none_or(|d| square(n, d))
                    }
                    Surface::Cylinder(_) | Surface::Cone(_) => {
                        let a = surface_axis(surface).unwrap_or(Vec3::ZERO);
                        along.is_none()
                            && perpendicular.is_some_and(|d| parallel(a, d) || parallel(-a, d))
                    }
                    _ => false,
                }
            }
            Filter::Axis { along, tol } => {
                let a = match c.kind {
                    SelectionKind::Face => surface_of(c).and_then(|(s, _)| surface_axis(s)),
                    SelectionKind::Edge => edge_of(c).and_then(|edge| match &edge.curve {
                        Curve3::Circle(circle) => Some(circle.frame().z()),
                        _ => None,
                    }),
                    _ => None,
                };
                a.is_some_and(|a| a.dot(*along).abs() >= tol.cos())
            }
            Filter::Inside([lo, hi]) => self.bounds(c).is_some_and(|(a, b)| {
                let slack = self.doc.tolerance.linear.abs();
                (0..3).all(|k| {
                    axis_of(a, k) >= axis_of(*lo, k) - slack
                        && axis_of(b, k) <= axis_of(*hi, k) + slack
                })
            }),
            Filter::Above(k, at) => self
                .bounds(c)
                .is_some_and(|(a, _)| axis_of(a, *k) >= at - self.doc.tolerance.linear.abs()),
            Filter::Below(k, at) => self
                .bounds(c)
                .is_some_and(|(_, b)| axis_of(b, *k) <= at + self.doc.tolerance.linear.abs()),
        }
    }

    fn measure(&mut self, measure: Measure, c: &Candidate<'_>) -> Option<f64> {
        match (measure, c.kind) {
            (Measure::Volume, SelectionKind::Body) => {
                crate::mass::mass_of(c.body?).ok().map(|m| m.volume)
            }
            (Measure::Area, SelectionKind::Body) => {
                crate::mass::mass_of(c.body?).ok().map(|m| m.area)
            }
            (Measure::Area, SelectionKind::Face) => self.face_area(c),
            (Measure::Radius, SelectionKind::Face) => match surface_of(c)?.0 {
                Surface::Cylinder(s) => Some(s.radius()),
                Surface::Sphere(s) => Some(s.radius()),
                _ => None,
            },
            (Measure::Radius, SelectionKind::Edge) => match &edge_of(c)?.curve {
                Curve3::Circle(circle) => Some(circle.radius()),
                _ => None,
            },
            (Measure::Length, SelectionKind::Edge) => edge_length(edge_of(c)?),
            _ => None,
        }
    }

    /// The box around the item. A vertex, an edge and a flat face are
    /// boxed closely: a line, a circle or an ellipse exactly, a freeform
    /// edge by the hull of the control points of the part of the curve
    /// the edge is (cut into short Bézier pieces, so the hull hugs it), a
    /// flat face by the boxes of its edges (a flat face lies within the
    /// hull of its boundary). A curved face and a body by a box that
    /// certainly contains them (`Body::held_bbox`). A box from the hull
    /// of a whole spline would stick out far beyond a face with a spline
    /// edge, and `inside` would miss it (the bottom of the showcase
    /// housing, 2026-10-21).
    fn bounds(&self, c: &Candidate<'_>) -> Option<(Vec3, Vec3)> {
        let body = c.body?;
        match c.kind {
            SelectionKind::Body => body.held_bbox().ok()?.corners(),
            SelectionKind::Face => {
                let mut all = Bbox3::EMPTY;
                for &k in &c.pieces {
                    let (lo, hi) = face_box(body, k)?;
                    all.add(lo);
                    all.add(hi);
                }
                all.corners()
            }
            SelectionKind::Edge => edge_box(body, c.index),
            SelectionKind::Vertex => {
                let p = body.vertices.get(c.index)?.point;
                Some((p, p))
            }
            _ => None,
        }
    }
}

/// How often each Bézier piece of a freeform edge is halved before its
/// control points box it: the hull of a piece closes in on the curve as
/// the piece shortens.
const HULL_HALVINGS: u32 = 3;

/// A box around face `k` of `body`: by its edges when it is flat, else
/// `Body::held_face_bbox`.
fn face_box(body: &Body, k: usize) -> Option<(Vec3, Vec3)> {
    let face = body.faces.get(k)?;
    let held = || {
        body.held_face_bbox(FaceId(u32::try_from(k).ok()?))
            .ok()?
            .corners()
    };
    if !matches!(face.surface, Surface::Plane(_)) {
        return held();
    }
    let mut all = Bbox3::EMPTY;
    for &l in &face.loops {
        for coedge in body.loop_(l).map(|l| l.coedges.as_slice())? {
            let Some((lo, hi)) = usize::try_from(coedge.edge.0)
                .ok()
                .and_then(|edge| edge_box(body, edge))
            else {
                return held();
            };
            all.add(lo);
            all.add(hi);
        }
    }
    all.corners().or_else(held)
}

/// A box around edge `k` of `body`: exact for a line, a circle and an
/// ellipse (`Body::edge_bbox`), the hull of the control points of the
/// part of a freeform curve the edge is, cut fine, otherwise.
fn edge_box(body: &Body, k: usize) -> Option<(Vec3, Vec3)> {
    let edge = body.edges.get(k)?;
    let exact = || {
        body.edge_bbox(EdgeId(u32::try_from(k).ok()?))
            .ok()?
            .corners()
    };
    let Curve3::Nurbs(curve) = &edge.curve else {
        return exact();
    };
    let mut all = Bbox3::EMPTY;
    for vertex in [edge.start, edge.end] {
        all.add(body.vertex(vertex)?.point);
    }
    let (a, b) = edge.range;
    let (t0, t1) = if a <= b { (a, b) } else { (b, a) };
    if !(t0.is_finite() && t1.is_finite()) {
        return exact();
    }
    for ((lo, hi), points) in curve.bezier_pieces() {
        if hi <= lo || hi.is_nan() || lo.is_nan() || hi <= t0 || lo >= t1 {
            continue;
        }
        let mut row: Vec<(Vec3, f64)> = Vec::with_capacity(points.len());
        for (p, w) in points {
            // The hull holds the curve only for positive weights.
            if !(w > 0.0 && w.is_finite() && p.is_finite()) {
                return exact();
            }
            row.push((p * w, w));
        }
        let s = (t0.max(lo) - lo) / (hi - lo);
        let e = (t1.min(hi) - lo) / (hi - lo);
        if e < 1.0 {
            row = halves(&row, e).0;
        }
        if s > 0.0 && e > 0.0 {
            row = halves(&row, (s / e).clamp(0.0, 1.0)).1;
        }
        let mut pieces = vec![row];
        for _ in 0..HULL_HALVINGS {
            pieces = pieces
                .iter()
                .flat_map(|piece| {
                    let (left, right) = halves(piece, 0.5);
                    [left, right]
                })
                .collect();
        }
        for (hp, w) in pieces.iter().flatten() {
            let p = *hp * (1.0 / *w);
            if !p.is_finite() {
                return exact();
            }
            all.add(p);
        }
    }
    all.corners()
}

/// A Bézier piece in homogeneous coordinates (point times weight, weight)
/// cut at `t` (de Casteljau): the part before and the part after.
type Homogeneous = Vec<(Vec3, f64)>;

fn halves(row: &[(Vec3, f64)], t: f64) -> (Homogeneous, Homogeneous) {
    let mut work: Homogeneous = row.to_vec();
    let mut left = Vec::with_capacity(row.len());
    let mut right = Vec::with_capacity(row.len());
    while let Some(&first) = work.first() {
        left.push(first);
        if let Some(&last) = work.last() {
            right.push(last);
        }
        work = work
            .windows(2)
            .filter_map(|pair| match pair {
                [(p, w), (q, v)] => Some((*p * (1.0 - t) + *q * t, w * (1.0 - t) + v * t)),
                _ => None,
            })
            .collect();
    }
    right.reverse();
    (left, right)
}

fn axis_of(v: Vec3, k: usize) -> f64 {
    match k {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

/// A name and the names it is the image of, outermost first.
fn chain(name: &PersistentName) -> impl Iterator<Item = &PersistentName> {
    let mut at = Some(name);
    // Bounded by the name itself: each link is a smaller part of it.
    std::iter::from_fn(move || {
        let here = at?;
        at = here.of.as_deref();
        Some(here)
    })
}

/// Largest gap, in radians, between the directions round its axis at which
/// a hole's wall has boundary points: a wall that goes all the way round
/// has points in every direction (its rims, its seam); a concave fillet
/// or the end of a slot has a gap of a quarter or half a turn.
const HOLE_GAP: f64 = std::f64::consts::PI / 6.0;
/// Points looked at along each edge of a hole's boundary.
const HOLE_SAMPLES: u32 = 24;

/// Whether a face is the wall of a hole: a cylinder whose outward normal
/// points at its axis (its material is outside it), and whose pieces
/// together go all the way round the axis. A concave fillet (a cylinder
/// too, but a quarter turn) is not one; neither is a cone (a countersink)
/// or a freeform surface, for now.
fn is_hole(c: &Candidate<'_>) -> bool {
    let Some(body) = c.body else {
        return false;
    };
    let Some((Surface::Cylinder(cylinder), true)) = surface_of(c) else {
        return false;
    };
    let frame = cylinder.frame();
    let (origin, x, y) = (frame.origin(), frame.x(), frame.y());
    let mut angles: Vec<f64> = Vec::new();
    for &k in &c.pieces {
        let Some(face) = body.faces.get(k) else {
            return false;
        };
        // Every piece is a wall of the same cylinder, its inside out.
        if !face.reversed || face.surface != Surface::Cylinder(*cylinder) {
            return false;
        }
        for &l in &face.loops {
            for coedge in body
                .loop_(l)
                .map(|l| l.coedges.as_slice())
                .unwrap_or_default()
            {
                let Some(edge) = body.edge(coedge.edge) else {
                    continue;
                };
                let (t0, t1) = edge.range;
                for i in 0..=HOLE_SAMPLES {
                    let t = t0 + (t1 - t0) * f64::from(i) / f64::from(HOLE_SAMPLES);
                    let d = edge.curve.eval(t) - origin;
                    let a = d.dot(y).atan2(d.dot(x));
                    if a.is_finite() {
                        angles.push(a);
                    }
                }
            }
        }
    }
    if angles.is_empty() {
        return false;
    }
    angles.sort_by(f64::total_cmp);
    let (Some(&first), Some(&last)) = (angles.first(), angles.last()) else {
        return false;
    };
    let wrap = first + std::f64::consts::TAU - last;
    let widest = angles
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some(b - a),
            _ => None,
        })
        .fold(wrap, f64::max);
    widest <= HOLE_GAP
}

fn surface_of<'a>(c: &Candidate<'a>) -> Option<(&'a Surface, bool)> {
    if c.kind != SelectionKind::Face {
        return None;
    }
    let face = c.body?.faces.get(c.index)?;
    Some((&face.surface, face.reversed))
}

fn edge_of<'a>(c: &Candidate<'a>) -> Option<&'a craft3d_topo::Edge> {
    if c.kind != SelectionKind::Edge {
        return None;
    }
    c.body?.edges.get(c.index)
}

fn surface_kind(surface: &Surface) -> &'static str {
    match surface {
        Surface::Plane(_) => "plane",
        Surface::Cylinder(_) => "cylinder",
        Surface::Cone(_) => "cone",
        Surface::Sphere(_) => "sphere",
        Surface::Torus(_) => "torus",
        Surface::Bezier(_) | Surface::Nurbs(_) => "freeform",
    }
}

fn surface_axis(surface: &Surface) -> Option<Vec3> {
    match surface {
        Surface::Cylinder(s) => Some(s.frame().z()),
        Surface::Cone(s) => Some(s.frame().z()),
        Surface::Torus(s) => Some(s.frame().z()),
        _ => None,
    }
}

/// The length of an edge: exact for a line and a circular arc, the
/// polygon through [`LENGTH_SAMPLES`] points otherwise.
fn edge_length(edge: &craft3d_topo::Edge) -> Option<f64> {
    let (t0, t1) = edge.range;
    let length = match &edge.curve {
        Curve3::Line(_) => (edge.curve.eval(t1) - edge.curve.eval(t0)).length(),
        Curve3::Circle(circle) => circle.radius() * (t1 - t0).abs(),
        _ => {
            let mut previous = edge.curve.eval(t0);
            let mut total = 0.0;
            for i in 1..=LENGTH_SAMPLES {
                let t = t0 + (t1 - t0) * f64::from(i) / f64::from(LENGTH_SAMPLES);
                let point = edge.curve.eval(t);
                total += (point - previous).length();
                previous = point;
            }
            total
        }
    };
    length.is_finite().then_some(length)
}

/// Every candidate of `kinds`, in document order.
fn candidates<'a>(
    doc: &'a Document,
    regen: &'a Regen,
    kinds: &BTreeSet<SelectionKind>,
) -> Result<Vec<Candidate<'a>>, String> {
    let mut out = Vec::new();
    let mut push = |c: Candidate<'a>| -> Result<(), String> {
        if out.len() >= MAX_CANDIDATES {
            return Err(format!(
                "the query looks at more than {MAX_CANDIDATES} items; ask for fewer kinds"
            ));
        }
        out.push(c);
        Ok(())
    };
    let wants = |kind| kinds.contains(&kind);
    for feature in &doc.features {
        let outcome = regen.feature(feature.id);
        let body = outcome.and_then(|o| o.own_body());
        let feature_like = |kind: SelectionKind| Candidate {
            kind,
            feature: feature.id,
            index: 0,
            pieces: Vec::new(),
            name: feature.name.clone(),
            persistent: None,
            body,
        };
        if wants(SelectionKind::Body) && body.is_some() {
            push(feature_like(SelectionKind::Body))?;
        }
        if wants(SelectionKind::Feature) {
            push(feature_like(SelectionKind::Feature))?;
        }
        if wants(SelectionKind::Sketch) && matches!(feature.kind, FeatureKind::Sketch(_)) {
            push(feature_like(SelectionKind::Sketch))?;
        }
        if wants(SelectionKind::Datum) && is_datum(&feature.kind) {
            push(feature_like(SelectionKind::Datum))?;
        }
    }
    for (feature, body) in regen.bodies() {
        let Some(names) = regen.feature(feature).and_then(|o| o.names.as_ref()) else {
            continue;
        };
        entities(feature, body, names, kinds, &mut push)?;
    }
    Ok(out)
}

fn entities<'a>(
    feature: FeatureId,
    body: &'a Arc<Body>,
    names: &Names,
    kinds: &BTreeSet<SelectionKind>,
    push: &mut impl FnMut(Candidate<'a>) -> Result<(), String>,
) -> Result<(), String> {
    let entity = |kind, index, name: PersistentName| Candidate {
        kind,
        feature,
        index,
        pieces: vec![index],
        name: name.to_string(),
        persistent: Some(name),
        body: Some(body),
    };
    if kinds.contains(&SelectionKind::Face) {
        // One item per name, at its first piece, with all its pieces.
        let mut faces: Vec<Candidate<'a>> = Vec::new();
        let mut at: BTreeMap<String, usize> = BTreeMap::new();
        for (index, name) in names.faces.iter().enumerate().take(body.faces.len()) {
            let text = name.to_string();
            match at.get(&text).and_then(|&k| faces.get_mut(k)) {
                Some(first) => first.pieces.push(index),
                None => {
                    at.insert(text, faces.len());
                    faces.push(entity(SelectionKind::Face, index, name.clone()));
                }
            }
        }
        for face in faces {
            push(face)?;
        }
    }
    if kinds.contains(&SelectionKind::Edge) {
        for index in 0..body.edges.len() {
            if let Some(name) = names.edge_name(index) {
                push(entity(SelectionKind::Edge, index, name))?;
            }
        }
    }
    if kinds.contains(&SelectionKind::Vertex) {
        for index in 0..body.vertices.len() {
            if let Some(name) = names.vertex_name(index) {
                push(entity(SelectionKind::Vertex, index, name))?;
            }
        }
    }
    Ok(())
}

/// One item a query found.
pub(crate) struct Hit {
    pub(crate) item: SelectionItem,
    /// As the query lists it: the item, with `body` and `origin` for a
    /// face, an edge or a vertex, and its context when asked for.
    pub(crate) listed: Value,
}

/// The first `limit` items `query` finds in `doc` as `regen` built it,
/// in document order, and how many fit; each with its context
/// ([`Context::describe`]) when `context`.
pub(crate) fn run_query(
    doc: &Document,
    regen: &Regen,
    query: Query,
    limit: usize,
    context: bool,
) -> Result<(Vec<Hit>, usize), String> {
    let kinds = query
        .kinds
        .unwrap_or_else(|| SelectionKind::ALL.into_iter().collect());
    let filter = query.filter;
    let all = candidates(doc, regen, &kinds)?;
    let mut cx = Context::new(doc, regen);
    let mut count = 0_usize;
    let mut found: Vec<Hit> = Vec::new();
    for c in &all {
        if filter.as_ref().is_some_and(|f| !cx.eval(f, c)) {
            continue;
        }
        count += 1;
        if found.len() < limit {
            let item = SelectionItem {
                kind: c.kind,
                name: c.name.clone(),
                feature: c.feature.to_string(),
            };
            let mut listed = item.json();
            if let Some(map) = listed.as_object_mut() {
                map.extend(cx.placed(c));
                if context {
                    map.extend(cx.describe(c));
                }
            }
            found.push(Hit { item, listed });
        }
    }
    Ok((found, count))
}

/// The kinds of `select.query`'s `kind`: every kind when not given.
fn kinds_param(p: &Params, id: &str) -> Result<Option<BTreeSet<SelectionKind>>, CommandError> {
    match p.get("kind") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => kinds(value)
            .map(Some)
            .map_err(|e| CommandError::new(format!("{id}: {e}"))),
    }
}

fn select_query(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "select.query";
    let p = Params::new(ID, params, &["kind", "where", "then", "limit", "context"])?;
    let then = match p.str("then")? {
        None | Some("none") => None,
        Some("replace") => Some(SelectMode::Replace),
        Some("add") => Some(SelectMode::Add),
        Some("remove") => Some(SelectMode::Remove),
        Some(_) => return fail(format!("{ID}: `then` must be none, replace, add or remove")),
    };
    let limit = p
        .integer("limit", 1, MAX_SELECTION as u64)?
        .unwrap_or(DEFAULT_LIMIT);
    let limit = usize::try_from(limit).unwrap_or(MAX_SELECTION);
    let context = p.bool("context")?.unwrap_or(false);
    let kinds = kinds_param(&p, ID)?;
    // Checked against the document as it is now before it changes.
    if then.is_some() {
        session.selection();
    }
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let filter = match p.get("where") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            Parser::new(doc)
                .parse(value, 0)
                .map_err(|e| CommandError::new(format!("{ID}: {e}")))?,
        ),
    };
    let query = Query { kinds, filter };
    let (found, count) = run_query(doc, &regen, query, limit, context)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let items: Vec<SelectionItem> = found.iter().map(|hit| hit.item.clone()).collect();
    let mut line = summary(&items);
    let bodies: BTreeSet<&str> = found
        .iter()
        .filter_map(|hit| hit.listed.get("body").and_then(Value::as_str))
        .collect();
    if !bodies.is_empty() && items.iter().all(|item| item.kind.is_entity()) {
        let shown: Vec<&str> = bodies.iter().copied().take(3).collect();
        line = format!("{line} in {}", shown.join(", "));
        if bodies.len() > 3 {
            line = format!("{line} and {} more", bodies.len() - 3);
        }
    }
    if count > found.len() {
        line = format!(
            "{count} fit; the first {} are given (narrow the query or raise limit, at most {MAX_SELECTION})",
            found.len()
        );
    }
    let truncated = count > found.len();
    let listed: Vec<Value> = found.into_iter().map(|hit| hit.listed).collect();
    let mut result = json!({
        "items": listed,
        "count": count,
        "truncated": truncated,
        "summary": line,
    });
    if let Some(mode) = then {
        open.selection.apply(items, mode);
        if let Some(map) = result.as_object_mut() {
            map.insert(
                "selection".into(),
                described_view(doc, &regen, &open.selection.view()),
            );
        }
    }
    Ok(result)
}

/// The items of the named set `name` as it holds now: at most
/// [`MAX_SELECTION`], more is an error.
pub(crate) fn set_items(
    doc: &Document,
    regen: &Regen,
    name: &str,
) -> Result<Vec<SelectionItem>, String> {
    let filter = Parser::new(doc).set(name, 0)?;
    let query = Query {
        kinds: None,
        filter: Some(filter),
    };
    let (found, count) = run_query(doc, regen, query, MAX_SELECTION, false)?;
    if count > found.len() {
        return Err(format!(
            "the set `{}` holds {count} items; at most {MAX_SELECTION} can be selected",
            name.chars().take(80).collect::<String>()
        ));
    }
    Ok(found.into_iter().map(|hit| hit.item).collect())
}

/// What `select.get` says of each selected item: the item, where it is
/// (`body`, `origin`) and its context ([`Context::describe`]), in the
/// order of the selection.
pub(crate) fn described(doc: &Document, regen: &Regen, items: &[SelectionItem]) -> Vec<Value> {
    let mut cx = Context::new(doc, regen);
    let mut names: BTreeMap<FeatureId, BTreeMap<(SelectionKind, String), Vec<usize>>> =
        BTreeMap::new();
    items
        .iter()
        .map(|item| {
            let mut entry = item.json();
            if let Some(c) = candidate_of(regen, item, &mut names)
                && let Some(map) = entry.as_object_mut()
            {
                map.extend(cx.placed(&c));
                map.extend(cx.describe(&c));
            }
            entry
        })
        .collect()
}

/// The selection as `select.get` gives it: its items with their context.
fn described_view(doc: &Document, regen: &Regen, view: &crate::selection::SelectionView) -> Value {
    let mut out = view.json();
    if let Some(map) = out.as_object_mut() {
        map.insert("items".into(), described(doc, regen, &view.items).into());
    }
    out
}

/// The candidate a selected item is: its body and its place there.
fn candidate_of<'a>(
    regen: &'a Regen,
    item: &SelectionItem,
    names: &mut BTreeMap<FeatureId, BTreeMap<(SelectionKind, String), Vec<usize>>>,
) -> Option<Candidate<'a>> {
    let feature = FeatureId(item.feature.strip_prefix('F')?.parse().ok()?);
    let outcome = regen.feature(feature)?;
    let body = outcome.own_body();
    if !item.kind.is_entity() {
        return Some(Candidate {
            kind: item.kind,
            feature,
            index: 0,
            pieces: Vec::new(),
            name: item.name.clone(),
            persistent: None,
            body,
        });
    }
    let (body, own) = (body?, outcome.names.as_ref()?);
    // Per body, once: where each name is.
    let table = names.entry(feature).or_insert_with(|| {
        let mut table: BTreeMap<(SelectionKind, String), Vec<usize>> = BTreeMap::new();
        for (k, name) in own.faces.iter().enumerate().take(body.faces.len()) {
            table
                .entry((SelectionKind::Face, name.to_string()))
                .or_default()
                .push(k);
        }
        for k in 0..body.edges.len() {
            if let Some(name) = own.edge_name(k) {
                table
                    .entry((SelectionKind::Edge, name.to_string()))
                    .or_default()
                    .push(k);
            }
        }
        for k in 0..body.vertices.len() {
            if let Some(name) = own.vertex_name(k) {
                table
                    .entry((SelectionKind::Vertex, name.to_string()))
                    .or_default()
                    .push(k);
            }
        }
        table
    });
    let pieces = table.get(&(item.kind, item.name.clone()))?.clone();
    Some(Candidate {
        kind: item.kind,
        feature,
        index: *pieces.first()?,
        pieces,
        name: item.name.clone(),
        persistent: item.name.parse().ok(),
        body: Some(body),
    })
}

/// A number rounded for reading: 12 significant digits, so a radius of
/// 3 reads as 3 and not 2.9999999999999996.
fn tidy(x: f64) -> Value {
    if !x.is_finite() {
        return Value::Null;
    }
    let rounded: f64 = format!("{x:.12e}").parse().unwrap_or(x);
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    json!(rounded)
}

fn point(v: Vec3) -> Value {
    json!([tidy(v.x), tidy(v.y), tidy(v.z)])
}

impl Context<'_> {
    /// Where an entity is: `body` (its body's feature's name) and
    /// `origin` (the feature at the end of its name's way, that first
    /// made it, with its name and type).
    fn placed(&self, c: &Candidate<'_>) -> Map<String, Value> {
        let mut map = Map::new();
        if !c.kind.is_entity() {
            return map;
        }
        if let Some(body) = self.doc.feature(c.feature) {
            map.insert("body".into(), body.name.clone().into());
        }
        let origin = c
            .persistent
            .as_ref()
            .and_then(|name| chain(name).last())
            .map(|link| link.feature);
        if let Some(origin) = origin {
            let made = self.doc.feature(origin);
            map.insert(
                "origin".into(),
                json!({
                    "feature": origin.to_string(),
                    "name": made.map(|f| f.name.clone()),
                    "type": made.map(|f| f.kind.type_name()),
                }),
            );
        }
        map
    }

    /// What an agent needs to act on an item without asking again: for a
    /// face its `surface` (type and key measure: radius and axis of a
    /// cylinder, outward normal of a plane, ...), for an edge its `curve`,
    /// for a vertex its `point`; `drivers`, the parameters that set its
    /// measure (a cylinder feature's radius, the diameter constraint of
    /// the sketch circle an extrusion swept, a fillet's radius), each with
    /// the edit that changes it; `copyOf`, the patterns and mirrors it is
    /// a copy made by, with the instance. A feature-like item gets its
    /// `type`. Cheap: no measure is integrated.
    fn describe(&mut self, c: &Candidate<'_>) -> Map<String, Value> {
        let mut map = Map::new();
        if !c.kind.is_entity() {
            if let Some(feature) = self.doc.feature(c.feature) {
                map.insert("type".into(), feature.kind.type_name().into());
            }
            return map;
        }
        match c.kind {
            SelectionKind::Face => {
                if let Some(surface) = self.surface_json(c) {
                    map.insert("surface".into(), surface);
                }
            }
            SelectionKind::Edge => {
                if let Some(edge) = edge_of(c) {
                    map.insert("curve".into(), curve_json(edge));
                }
            }
            _ => {
                if let Some(v) = c.body.and_then(|b| b.vertices.get(c.index)) {
                    map.insert("point".into(), point(v.point));
                }
            }
        }
        let mut drivers = c
            .persistent
            .as_ref()
            .and_then(|name| chain(name).last())
            .map(|origin| self.drivers_of(origin))
            .unwrap_or_default();
        if drivers.is_empty() && c.kind == SelectionKind::Edge {
            drivers = self.edge_drivers(c);
        }
        map.insert("drivers".into(), drivers.into());
        let copies = c
            .persistent
            .as_ref()
            .map(|name| self.copies(name))
            .unwrap_or_default();
        map.insert("copyOf".into(), copies.into());
        map
    }

    fn surface_json(&self, c: &Candidate<'_>) -> Option<Value> {
        let (surface, reversed) = surface_of(c)?;
        let kind = surface_kind(surface);
        Some(match surface {
            Surface::Plane(plane) => {
                let z = plane.frame().z();
                let n = if reversed { -z } else { z };
                json!({"type": kind, "normal": point(n), "point": point(plane.frame().origin())})
            }
            Surface::Cylinder(s) => json!({
                "type": kind, "radius": tidy(s.radius()), "axis": point(s.frame().z()),
                "center": point(s.frame().origin()), "hole": is_hole(c),
            }),
            Surface::Cone(s) => json!({
                "type": kind, "radius": tidy(s.radius()),
                "halfAngle": tidy(s.half_angle().to_degrees()),
                "axis": point(s.frame().z()), "center": point(s.frame().origin()),
            }),
            Surface::Sphere(s) => json!({
                "type": kind, "radius": tidy(s.radius()), "center": point(s.frame().origin()),
            }),
            Surface::Torus(s) => {
                let (major, minor) = s.radii();
                json!({
                    "type": kind, "major": tidy(major), "minor": tidy(minor),
                    "axis": point(s.frame().z()), "center": point(s.frame().origin()),
                })
            }
            Surface::Bezier(_) | Surface::Nurbs(_) => json!({"type": kind}),
        })
    }

    /// The parameters that set what the feature of `origin` (the last
    /// link of an entity's name) made: one entry per parameter.
    fn drivers_of(&self, origin: &PersistentName) -> Vec<Value> {
        let Some(feature) = self.doc.feature(origin.feature) else {
            return Vec::new();
        };
        let id = feature.id.to_string();
        let first = origin.sources.first().copied();
        let param = |param: &str, expr: &craft3d_doc::Expr| {
            json!({
                "feature": id, "name": feature.name, "type": feature.kind.type_name(),
                "param": param, "expr": expr.0,
                "edit": {"command": "feature.edit", "params": {"feature": id, param: expr.0}},
            })
        };
        let role = origin.role;
        match &feature.kind {
            FeatureKind::Block(block) => {
                let axis = match first {
                    Some(0 | 1) => 2,
                    Some(2 | 3) => 1,
                    Some(_) => 0,
                    None => return Vec::new(),
                };
                let size: Vec<&str> = block.size.iter().map(|e| e.0.as_str()).collect();
                let Some(expr) = block.size.get(axis) else {
                    return Vec::new();
                };
                vec![json!({
                    "feature": id, "name": feature.name, "type": "block",
                    "param": "size", "index": axis, "expr": expr.0,
                    "edit": {"command": "feature.edit", "params": {"feature": id, "size": size}},
                })]
            }
            FeatureKind::Cylinder(cylinder) => match (role, first) {
                (NameRole::Face, Some(0)) | (NameRole::Edge, _) => {
                    vec![param("radius", &cylinder.radius)]
                }
                _ => vec![param("height", &cylinder.height)],
            },
            FeatureKind::Cone(cone) => match (role, first) {
                (NameRole::Face, Some(0)) => {
                    vec![param("radius", &cone.radius), param("height", &cone.height)]
                }
                (NameRole::Edge, _) => vec![param("radius", &cone.radius)],
                _ => vec![param("height", &cone.height)],
            },
            FeatureKind::Sphere(sphere) => vec![param("radius", &sphere.radius)],
            FeatureKind::Torus(torus) => {
                vec![param("minor", &torus.minor), param("major", &torus.major)]
            }
            FeatureKind::Fillet(fillet) => vec![param("radius", &fillet.radius)],
            FeatureKind::Chamfer(chamfer) => {
                let distance = match &chamfer.size {
                    craft3d_doc::ChamferSize::Equal { distance }
                    | craft3d_doc::ChamferSize::TwoDistances { distance, .. }
                    | craft3d_doc::ChamferSize::DistanceAngle { distance, .. } => distance,
                };
                vec![param("distance", distance)]
            }
            FeatureKind::Extrude(extrude) => match role {
                NameRole::EndCap => vec![param("distance", &extrude.distance)],
                NameRole::Side
                | NameRole::StartEdge
                | NameRole::EndEdge
                | NameRole::StartVertex
                | NameRole::EndVertex => first
                    .map(|curve| self.curve_drivers(extrude.sketch, curve))
                    .unwrap_or_default(),
                _ => Vec::new(),
            },
            FeatureKind::Revolve(revolve) => match role {
                NameRole::Side | NameRole::StartEdge | NameRole::EndEdge => first
                    .map(|curve| self.curve_drivers(revolve.sketch, curve))
                    .unwrap_or_default(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// What sizes curve `curve` (an entity id, the first number of a
    /// side's name) of the sketch `sketch`: a radius or diameter
    /// constraint on it or, through equal constraints, on a curve equal to
    /// it; the length of a line; or the curve alone when nothing sizes it.
    fn curve_drivers(&self, sketch: FeatureId, curve: u64) -> Vec<Value> {
        let Some(feature) = self.doc.feature(sketch) else {
            return Vec::new();
        };
        let FeatureKind::Sketch(drawn) = &feature.kind else {
            return Vec::new();
        };
        let Some(entity) = drawn.entities.iter().find(|e| e.id.0 == curve) else {
            return Vec::new();
        };
        let shape = match entity.geometry {
            craft3d_doc::Geometry::Circle { .. } => "circle",
            craft3d_doc::Geometry::Arc { .. } => "arc",
            craft3d_doc::Geometry::Line { .. } => "line",
            craft3d_doc::Geometry::Spline { .. } => "spline",
            _ => "point",
        };
        let sid = sketch.to_string();
        // The curve, then the curves equal to it, nearest first: each
        // looked at once, so a ring of equal constraints ends.
        let mut todo = std::collections::VecDeque::from([entity.id]);
        let mut seen = BTreeSet::from([entity.id]);
        while let Some(at) = todo.pop_front() {
            for constraint in &drawn.constraints {
                let sized = match &constraint.kind {
                    ConstraintKind::Radius { curve, value } if *curve == at => {
                        Some(("radius", value))
                    }
                    ConstraintKind::Diameter { curve, value } if *curve == at => {
                        Some(("diameter", value))
                    }
                    ConstraintKind::Distance {
                        a, b: None, value, ..
                    } if *a == at => Some(("distance", value)),
                    _ => None,
                };
                if let Some((kind, value)) = sized {
                    let mut entry = json!({
                        "sketch": sid, "sketchName": feature.name, "entity": entity.id.to_string(),
                        "curve": shape, "constraint": constraint.id.to_string(), "kind": kind,
                        "expr": value.0,
                        "edit": {"command": "sketch.edit", "params": {
                            "sketch": sid, "constraint": constraint.id.to_string(), "value": value.0,
                        }},
                    });
                    if at != entity.id
                        && let Some(map) = entry.as_object_mut()
                    {
                        map.insert("via".into(), at.to_string().into());
                    }
                    return vec![entry];
                }
            }
            for constraint in &drawn.constraints {
                if let ConstraintKind::Equal { a, b } = &constraint.kind {
                    for (from, to) in [(a, b), (b, a)] {
                        if *from == at && seen.insert(*to) {
                            todo.push_back(*to);
                        }
                    }
                }
            }
        }
        vec![json!({
            "sketch": sid, "sketchName": feature.name, "entity": entity.id.to_string(),
            "curve": shape, "constraint": null,
        })]
    }

    /// The drivers of an edge named by the faces it lies between: those
    /// of the first of its faces that is curved (the wall whose rim it is).
    fn edge_drivers(&mut self, c: &Candidate<'_>) -> Vec<Value> {
        let Some(body) = c.body else {
            return Vec::new();
        };
        let Some(names) = self.regen.feature(c.feature).and_then(|o| o.names.as_ref()) else {
            return Vec::new();
        };
        let faces = craft3d_regen::faces_of_edges(body)
            .into_iter()
            .nth(c.index)
            .unwrap_or_default();
        for face in faces {
            let Ok(k) = usize::try_from(face.0) else {
                continue;
            };
            let curved = body
                .faces
                .get(k)
                .is_some_and(|f| !matches!(f.surface, Surface::Plane(_)));
            let Some(name) = names.faces.get(k) else {
                continue;
            };
            if curved && let Some(origin) = chain(name).last() {
                let drivers = self.drivers_of(origin);
                if !drivers.is_empty() {
                    return drivers;
                }
            }
        }
        Vec::new()
    }

    /// The patterns and mirrors an entity is a copy made by, outermost
    /// first, with the instance it is in each.
    fn copies(&self, name: &PersistentName) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        for (feature, instance) in self.way(name) {
            let Some(made) = self.doc.feature(feature) else {
                continue;
            };
            if matches!(made.kind, FeatureKind::Pattern(_) | FeatureKind::Mirror(_)) {
                let entry = json!({
                    "feature": feature.to_string(), "name": made.name,
                    "type": made.kind.type_name(), "instance": instance,
                });
                if out.last() != Some(&entry) {
                    out.push(entry);
                }
            }
        }
        out
    }
}

/// An edge's curve: its type and key measure.
fn curve_json(edge: &craft3d_topo::Edge) -> Value {
    let length = edge_length(edge).map_or(Value::Null, tidy);
    match &edge.curve {
        Curve3::Line(_) => {
            let (t0, t1) = edge.range;
            let d = (edge.curve.eval(t1) - edge.curve.eval(t0))
                .normalized()
                .map_or(Value::Null, point);
            json!({"type": "line", "direction": d, "length": length})
        }
        Curve3::Circle(circle) => json!({
            "type": "circle", "radius": tidy(circle.radius()), "axis": point(circle.frame().z()),
            "center": point(circle.frame().origin()), "length": length,
        }),
        Curve3::Ellipse(_) => json!({"type": "ellipse", "length": length}),
        Curve3::Nurbs(_) => json!({"type": "freeform", "length": length}),
    }
}
