//! Assertions kept in the document (`assert.add`, `assert.edit`,
//! `assert.delete`, `assert.list`): the design intent written down as
//! measures with what they must be, checked against every regeneration.
//! A broken assertion never stops regeneration; it shows like a failed
//! feature, with what was measured and what was expected, in
//! `document.check`, in the result of the command whose edit broke it,
//! in the window's tree and over MCP. The design note is
//! docs/design/verification.md.

use crate::assert_check::{EDIT_WAIT, Wait};
use crate::commands::{Run, edit, fail, length_unit, spec};
use crate::params::Params;
use crate::verify_cmds::{Ctx, Dim, MEASURES, Outcome, only, take};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Assertion, Document, Expect, Expr};
use craft3d_regen::Regen;
use serde_json::{Map, Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("assert.add", "Add Assertion", r#"{"name": string, "measure": "volume|area|mass|count|extent|boxMin|boxMax|centerOfMass|distance|clearance|thickness|through|overlap|symmetric|volumeBelow|centroidBelow|sectionArea|sectionCentroid|sectionInertia|levelFor", "of": subject, "from": subject, "to": subject, "axis": "x|y|z", "plane": plane, "normal": direction, "offset": expr, "xAxis": direction, "about": "x|y|xy|major|minor", "volume": expr, "tol": expr, "eq": expr, "min": expr, "max": expr, "is": boolean}"#,
        "Adds an assertion to the document: a measure of part of the design and what it must be, checked after every regeneration (one undo step; saved with the document). Measures and their parameters: volume, area, mass (of: bodies, default all; area also of faces; mass needs a material), count (of: a query, the number of things it finds), extent (of: bodies, axis: the size of their box along it), boxMin and boxMax (of: bodies, axis: where their box begins and ends along it, in the model's frame, so each side is bounded on its own), centerOfMass (of: bodies, default all, axis: the coordinate of their centre of mass along it, each body's centroid weighted by its mass; every body needs a material), distance (from, to: faces or bodies, the least distance), clearance (from, to: bodies; a pair that overlaps fails), thickness (of: a body, the thinnest wall), through (of: cylindrical or conical faces, every one cut through along its axis), overlap (of: bodies, to: bodies = each other; whether any pair has a volume in common, so is false asserts that none overlap; the message names the pairs and their common volume, \"Bolt overlaps Plate by 12 mm^3\", or says no overlap), symmetric (of, plane, tol: as measure.symmetry), and what lies on one side of a plane (of: bodies, default all; normal, default z; offset, a length along the normal, default 0; xAxis: the section's x axis; as measure.below, measure.section and measure.levelFor): volumeBelow (the volume below the plane), centroidBelow (axis: the coordinate of its centroid), sectionArea, sectionCentroid (axis), sectionInertia (about: x, y or xy about the section's centroid in its own axes, or major or minor, the principal moments; in the document's unit to the fourth power), levelFor (volume: the offset at which the volume below is that). A subject is a body (feature name or id), a list of them, a query {kind, where} as select.query takes, or a bare filter. What it must be: eq (with tol, default a millionth of eq or ten times the document's tolerance for a length) or min and/or max, expressions in the document's variables (\"wall\", \"2 mm\"), in the measure's unit (length, area or volume in the document's unit, mass in kg); through and symmetric take is (default true); overlap takes is, which must be given. The old form interference (is true for no overlap) is still accepted and written as overlap with the opposite is; saved documents are migrated on load (format 36). A tol may be 0 (exactly; the document's tolerance still applies), and 0 and 1 may carry a power of a unit (0 mm^3). Feature names in the subject are kept as ids, so a renamed feature stays measured. Returns the assertion and its check: status passed, failed (measured, expected), notProven (a least searched for, such as a thickness, lies within the bound but what is proven does not: a warning, never passed) or error (the measure could not be taken, such as a subject that names nothing yet: an assertion may be written before the model meets it). A bound written N unit^k (50 cm^3, which is (50 cm)^3) is refused for any N but 0 and 1: write 50 * (1 cm)^3. Assertions are measured off the caller's thread and only when what they depend on changed; this command waits for its own.", doc: true, names: ["of", "from", "to", "plane"], assert_add),
    spec!("assert.edit", "Edit Assertion", r#"{"name": string, "rename": string, ...the parameters of assert.add}"#,
        "Changes an assertion (one undo step): what is given replaces what it had; measure parameters are replaced as a whole when measure or any of them is given, the expectation when any of eq, tol, min, max or is is given. Returns it with its check.", doc: true, names: ["of", "from", "to", "plane"], assert_edit),
    spec!("assert.delete", "Delete Assertion", r#"{"name": string}"#,
        "Removes an assertion from the document (one undo step).", doc: true, assert_delete),
    spec!("assert.list", "Assertions", r#"{"details": boolean = false, "wait": boolean = true}"#,
        "Lists the document's assertions, each checked against the document as it is now: name, measure (as stored), expected, status (passed, failed, notProven, error, or pending while it is measured: only with wait false), measured with its unit (and what is proven, for a least searched for), message (one line); with details what the measure found (where the thinnest place is, which hole is blocked). passed, failed, errors, notProven and pending count them; ok is true when all passed.", doc: true, assert_list),
];

/// The keys of each measure's parameters.
fn measure_keys(measure: &str) -> &'static [&'static str] {
    match measure {
        "volume" | "area" | "mass" | "count" | "thickness" | "through" => &["of"],
        "extent" | "boxMin" | "boxMax" | "centerOfMass" => &["of", "axis"],
        "distance" | "clearance" => &["from", "to"],
        "overlap" => &["of", "to"],
        "symmetric" => &["of", "plane", "tol"],
        _ => crate::below_cmds::keys_of(measure),
    }
}

fn is_flag(measure: &str) -> bool {
    matches!(measure, "through" | "overlap" | "symmetric")
}

/// Keys in a subject's filters whose value names a feature.
const FEATURE_KEYS: [&str; 3] = ["in", "madeBy", "copyOf"];

/// A subject with the features it names by name written as their ids,
/// so that renaming a feature does not break the assertion.
fn by_id(doc: &Document, value: &Value, depth: usize) -> Value {
    let id_of = |text: &str| {
        doc.feature_by_name(text)
            .map_or_else(|| text.to_string(), |f| f.id.to_string())
    };
    if depth > 64 {
        return value.clone();
    }
    match value {
        Value::String(text) if depth == 0 => Value::String(id_of(text)),
        Value::Array(items) if depth == 0 => Value::Array(
            items
                .iter()
                .map(|item| match item {
                    Value::String(text) => Value::String(id_of(text)),
                    other => other.clone(),
                })
                .collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| by_id(doc, v, depth + 1)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::String(text) if FEATURE_KEYS.contains(&k.as_str()) => {
                            Value::String(id_of(text))
                        }
                        other => by_id(doc, other, depth + 1),
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Rewrites what the assertions of `doc` name by a feature's name as its
/// id (a subject written before its feature existed keeps the name until
/// then): done before a feature is renamed.
pub(crate) fn assertions_by_id(doc: &mut Document) {
    let mut rewritten = Vec::with_capacity(doc.assertions.len());
    for assertion in &doc.assertions {
        let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(&assertion.measure) else {
            rewritten.push(assertion.measure.clone());
            continue;
        };
        for key in ["of", "from", "to"] {
            if let Some(v) = map.get_mut(key) {
                *v = by_id(doc, v, 0);
            }
        }
        if let Some(v @ Value::String(_)) = map.get_mut("plane") {
            *v = by_id(doc, v, 0);
        }
        rewritten.push(
            serde_json::to_string(&Value::Object(map))
                .unwrap_or_else(|_| assertion.measure.clone()),
        );
    }
    for (assertion, text) in doc.assertions.iter_mut().zip(rewritten) {
        if text.len() <= craft3d_doc::MAX_ASSERTION_TEXT {
            assertion.measure = text;
        }
    }
}

/// Renames variable `old` to `new` in the expressions inside what the
/// assertions of `doc` measure (a symmetry plane's point, its tol); their
/// bounds are renamed by the document itself.
pub(crate) fn rename_variable_in_assertions(
    doc: &mut Document,
    old: &str,
    new: &str,
) -> Result<(), craft3d_doc::DocError> {
    fn walk(
        v: &mut Value,
        old: &str,
        new: &str,
        depth: usize,
    ) -> Result<(), craft3d_expr::VariableError> {
        match v {
            Value::String(text) => {
                if let Ok(renamed) = craft3d_expr::rename_variable(text, old, new) {
                    *text = renamed;
                }
            }
            Value::Array(items) if depth < 4 => {
                for item in items {
                    walk(item, old, new, depth + 1)?;
                }
            }
            Value::Object(map) if depth < 4 => {
                for (k, item) in map.iter_mut() {
                    if ["point", "tol"].contains(&k.as_str()) {
                        walk(item, old, new, depth + 1)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    for assertion in &mut doc.assertions {
        let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(&assertion.measure) else {
            continue;
        };
        for key in ["tol", "plane"].into_iter().chain(crate::below_cmds::EXPRS) {
            if let Some(v) = map.get_mut(key) {
                match v {
                    Value::String(_) if key == "plane" => {}
                    _ => walk(v, old, new, if key == "plane" { 1 } else { 0 })
                        .map_err(craft3d_doc::DocError::Variable)?,
                }
            }
        }
        if let Ok(text) = serde_json::to_string(&Value::Object(map))
            && text.len() <= craft3d_doc::MAX_ASSERTION_TEXT
        {
            assertion.measure = text;
        }
    }
    Ok(())
}

/// The measure of an assertion as stored: `{"measure": name, ...}` with
/// its parameters, the subjects' features by id.
fn stored_measure(
    doc: &Document,
    id: &str,
    p: &Map<String, Value>,
) -> Result<String, CommandError> {
    let name = p.get("measure").and_then(Value::as_str).ok_or_else(|| {
        CommandError::new(format!(
            "{id}: `measure` is required: one of {}",
            MEASURES.join(", ")
        ))
    })?;
    if !MEASURES.contains(&name) {
        return fail(format!(
            "{id}: unknown measure `{}`; one of {}",
            name.chars().take(40).collect::<String>(),
            MEASURES.join(", ")
        ));
    }
    let mut out = Map::new();
    out.insert("measure".into(), name.into());
    for &key in measure_keys(name) {
        if let Some(v) = p.get(key).filter(|v| !v.is_null()) {
            let v = match key {
                "of" | "from" | "to" => by_id(doc, v, 0),
                "plane" => match v {
                    Value::String(text)
                        if !matches!(text.as_str(), "xy" | "yx" | "yz" | "zy" | "zx" | "xz") =>
                    {
                        by_id(doc, v, 0)
                    }
                    other => other.clone(),
                },
                _ => v.clone(),
            };
            out.insert(key.into(), v);
        }
    }
    let text = serde_json::to_string(&Value::Object(out))
        .map_err(|e| CommandError::new(format!("{id}: {e}")))?;
    if text.len() > craft3d_doc::MAX_ASSERTION_TEXT {
        return fail(format!(
            "{id}: what is measured is longer than {} bytes",
            craft3d_doc::MAX_ASSERTION_TEXT
        ));
    }
    Ok(text)
}

/// An expression parameter; `N unit^k` is refused
/// ([`crate::verify_cmds::power_of_a_quantity`]).
fn expr(p: &Params, key: &str) -> Result<Option<Expr>, CommandError> {
    let Some(source) = p.expr(key)? else {
        return Ok(None);
    };
    if let Some(why) = crate::verify_cmds::power_of_a_quantity(&source) {
        return fail(format!("{}: `{key}`: {why}", p.command()));
    }
    Ok(Some(Expr::new(source)))
}

/// What the assertion expects, from `eq`, `tol`, `min`, `max` and `is`.
fn expectation(p: &Params, id: &str, measure: &str) -> Result<Expect, CommandError> {
    let is = p.bool("is")?;
    let has = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    if is_flag(measure) {
        if ["eq", "min", "max"].iter().any(|k| has(k)) || (measure != "symmetric" && has("tol")) {
            return fail(format!(
                "{id}: {measure} is yes or no: give `is` (default true), not eq, tol, min or max"
            ));
        }
        // Whether bodies may overlap is said, not defaulted: an
        // assertion that they must overlap (a press fit) and one that
        // none do are both written.
        if measure == "overlap" && is.is_none() {
            return fail(format!(
                "{id}: overlap needs `is`: false asserts that no two of the bodies overlap, true that they do"
            ));
        }
        return Ok(Expect::Is(is.unwrap_or(true)));
    }
    if is.is_some() {
        return fail(format!(
            "{id}: {measure} is a number: give eq (and tol) or min and/or max, not `is`"
        ));
    }
    match (expr(p, "eq")?, expr(p, "min")?, expr(p, "max")?) {
        (Some(value), None, None) => Ok(Expect::Equal {
            value,
            tol: expr(p, "tol")?,
        }),
        (None, min, max) if min.is_some() || max.is_some() => {
            if has("tol") {
                return fail(format!("{id}: `tol` goes with eq, not with min or max"));
            }
            Ok(Expect::Within { min, max })
        }
        (Some(_), _, _) => fail(format!("{id}: give eq or min and/or max, not both")),
        _ => fail(format!(
            "{id}: say what {measure} must be: eq (and tol) or min and/or max"
        )),
    }
}

/// `interference`, what `overlap` was called before format 36, with the
/// opposite yes and no (true when none of the bodies overlap). Saved
/// documents are migrated on load; a caller that still sends it is taken
/// at its word.
const OLD_INTERFERENCE: &str = "interference";

/// The parameters of `assert.add` or `assert.edit` with the old measure
/// `interference` written as `overlap` and its `is` turned round (absent,
/// it meant true); any others as they are. An `is` that is no boolean is
/// left for the parameters to refuse.
fn without_old_name(params: &Value) -> Value {
    let mut params = params.clone();
    if let Value::Object(map) = &mut params
        && map.get("measure").and_then(Value::as_str) == Some(OLD_INTERFERENCE)
    {
        map.insert("measure".into(), "overlap".into());
        match map.get("is") {
            None | Some(Value::Null) => {
                map.insert("is".into(), false.into());
            }
            Some(Value::Bool(none_overlap)) => {
                let is = !*none_overlap;
                map.insert("is".into(), is.into());
            }
            Some(_) => {}
        }
    }
    params
}

const ADD_KEYS: [&str; 19] = [
    "name", "measure", "of", "from", "to", "axis", "plane", "tol", "eq", "min", "max", "is",
    "normal", "offset", "xAxis", "about", "volume", "rename", "details",
];

/// How many of [`ADD_KEYS`] assert.add takes; assert.edit takes `rename`
/// too.
const ADD: usize = 17;

fn params_map(params: &Value) -> Map<String, Value> {
    match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    }
}

/// Refuses parameters the measure does not take.
fn check_keys(
    id: &str,
    measure: &str,
    p: &Map<String, Value>,
    extra: &[&str],
) -> Result<(), CommandError> {
    let mut allowed: Vec<&str> = vec!["name", "measure", "eq", "min", "max", "is", "tol"];
    allowed.extend_from_slice(measure_keys(measure));
    allowed.extend_from_slice(extra);
    only(p, &allowed).map_err(|e| CommandError::new(format!("{id}: {measure}: {e}")))
}

fn assert_add(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "assert.add";
    let params = &without_old_name(params);
    let p = Params::new(ID, params, &ADD_KEYS[..ADD])?;
    let name = p.required_str("name")?.to_string();
    let map = params_map(params);
    let measure = p.required_str("measure")?;
    check_keys(ID, measure, &map, &[])?;
    let doc = session.document()?.history.current();
    let text = stored_measure(doc, ID, &map)?;
    let expect = expectation(&p, ID, measure)?;
    let assertion = Assertion {
        name: name.clone(),
        measure: text,
        expect,
    };
    edit(session, ID, &format!("Add assertion {name}"), |doc| {
        doc.add_assertion(assertion)
    })?;
    one(session, &name, true)
}

fn assert_edit(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "assert.edit";
    let params = &without_old_name(params);
    let p = Params::new(ID, params, &ADD_KEYS[..ADD + 1])?;
    let name = p.required_str("name")?.to_string();
    let map = params_map(params);
    let doc = session.document()?.history.current();
    let old = doc
        .assertion(&name)
        .ok_or_else(|| CommandError::new(format!("{ID}: there is no assertion named `{name}`")))?
        .clone();
    let old_measure: Map<String, Value> = serde_json::from_str(&old.measure).unwrap_or_default();
    let old_name = old_measure
        .get("measure")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let measure_given = map.contains_key("measure")
        || MEASURES
            .iter()
            .flat_map(|m| measure_keys(m))
            .any(|k| *k != "tol" && map.contains_key(*k));
    let measure = p.str("measure")?.unwrap_or(&old_name).to_string();
    check_keys(ID, &measure, &map, &["rename"])?;
    let text = if measure_given {
        let mut given = map.clone();
        given.insert("measure".into(), measure.clone().into());
        stored_measure(doc, ID, &given)?
    } else if measure == "symmetric" && map.contains_key("tol") {
        let mut given = old_measure.clone();
        if let Some(tol) = map.get("tol") {
            given.insert("tol".into(), tol.clone());
        }
        stored_measure(doc, ID, &given)?
    } else {
        old.measure.clone()
    };
    let expect_given = ["eq", "min", "max", "is"]
        .iter()
        .any(|k| map.contains_key(*k))
        || (map.contains_key("tol") && measure != "symmetric");
    let expect = if expect_given || measure != old_name {
        expectation(&p, ID, &measure)?
    } else {
        old.expect.clone()
    };
    let new_name = p.str("rename")?.unwrap_or(&name).to_string();
    let assertion = Assertion {
        name: new_name.clone(),
        measure: text,
        expect,
    };
    edit(session, ID, &format!("Edit assertion {name}"), |doc| {
        doc.replace_assertion(&name, assertion)
    })?;
    one(session, &new_name, true)
}

fn assert_delete(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "assert.delete";
    let p = Params::new(ID, params, &["name"])?;
    let name = p.required_str("name")?.to_string();
    if session
        .document()?
        .history
        .current()
        .assertion(&name)
        .is_none()
    {
        return fail(format!("{ID}: there is no assertion named `{name}`"));
    }
    edit(session, ID, &format!("Delete assertion {name}"), |doc| {
        doc.remove_assertion(&name).map(|_| ())
    })
    .map_err(|e| CommandError::new(e.message().to_string()))?;
    Ok(json!({ "deleted": name }))
}

fn assert_list(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("assert.list", params, &["details", "wait"])?;
    let details = p.bool("details")?.unwrap_or(false);
    let wait = if p.bool("wait")?.unwrap_or(true) {
        Wait::All
    } else {
        Wait::No
    };
    let checked = session.assertion_checks(wait)?;
    let doc = session.document()?.history.current();
    let listed: Vec<Value> = checked.iter().map(|c| c.json(doc, details)).collect();
    let count = |s: Status| checked.iter().filter(|c| c.state.status == s).count();
    let passed = count(Status::Passed);
    Ok(json!({
        "assertions": listed,
        "count": checked.len(),
        "passed": passed,
        "failed": count(Status::Failed),
        "errors": count(Status::Error),
        "notProven": count(Status::NotProven),
        "pending": count(Status::Pending),
        "ok": passed == checked.len(),
    }))
}

/// One assertion with its check, as `assert.add` and `assert.edit`
/// return it.
fn one(session: &mut Session, name: &str, details: bool) -> Run {
    let checked = session.assertion_checks(Wait::All)?;
    let doc = session.document()?.history.current();
    checked
        .iter()
        .find(|c| c.state.name == name)
        .map(|c| c.json(doc, details))
        .ok_or_else(|| CommandError::new(format!("there is no assertion named `{name}`")))
}

// ------------------------------------------------------------- checks

/// How an assertion stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssertionStatus {
    Passed,
    Failed,
    /// The measure was taken but does not settle it: the least found is
    /// within the bound while what is proven is not (a thickness or a
    /// distance searched for). Shown like a warning, never as passed.
    NotProven,
    /// Being measured (on another thread): asked again, it is one of the
    /// others.
    Pending,
    /// The measure could not be taken (its subject names nothing, the
    /// geometry refused); counted as broken.
    Error,
}
use AssertionStatus as Status;

impl AssertionStatus {
    pub fn name(self) -> &'static str {
        match self {
            Status::Passed => "passed",
            Status::Failed => "failed",
            Status::NotProven => "notProven",
            Status::Pending => "pending",
            Status::Error => "error",
        }
    }
}

/// An assertion as checked against the document, for a viewer.
#[derive(Clone, Debug, PartialEq)]
pub struct AssertionState {
    pub name: String,
    /// The measure's name.
    pub measure: String,
    pub status: AssertionStatus,
    /// What was measured, in words with its unit ("14 mm", "false",
    /// "overlap"); none when the measure could not be taken.
    pub measured: Option<String>,
    /// What it must be ("≥ 16 mm", "true").
    pub expected: String,
    /// One line: what failed and why.
    pub message: String,
}

/// An assertion checked, with what the measure found.
#[derive(Clone, Debug)]
pub(crate) struct Checked {
    pub state: AssertionState,
    pub stored: String,
    /// The measured number or yes or no, as JSON.
    pub value: Value,
    pub unit: Option<String>,
    pub detail: Map<String, Value>,
}

impl Checked {
    fn json(&self, doc: &Document, details: bool) -> Value {
        let measure: Value = serde_json::from_str(&self.stored).unwrap_or(Value::Null);
        let expect = doc
            .assertion(&self.state.name)
            .map_or(Value::Null, |a| expect_json(&a.expect));
        let mut out = json!({
            "name": self.state.name,
            "measure": measure,
            "expect": expect,
            "status": self.state.status.name(),
            "measured": self.value,
            "unit": self.unit,
            "expected": self.state.expected,
            "message": self.state.message,
        });
        if let (true, Some(map)) = (details, out.as_object_mut()) {
            map.insert("detail".into(), Value::Object(self.detail.clone()));
        }
        out
    }
}

fn expect_json(expect: &Expect) -> Value {
    match expect {
        Expect::Is(is) => json!({ "is": is }),
        Expect::Equal { value, tol } => {
            json!({ "eq": value.source(), "tol": tol.as_ref().map(Expr::source) })
        }
        Expect::Within { min, max } => json!({
            "min": min.as_ref().map(Expr::source),
            "max": max.as_ref().map(Expr::source),
        }),
    }
}

/// A number for reading: at most nine significant digits, no trailing
/// zeros.
pub(crate) fn shown(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return format!("{v}");
    }
    let digits = 9 - (v.abs().log10().floor() as i32) - 1;
    let digits = usize::try_from(digits.clamp(0, 15)).unwrap_or(0);
    let text = format!("{v:.digits$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

fn unit_of(dim: Dim, length: &str) -> Option<String> {
    match dim {
        Dim::Length(1) => Some(length.to_string()),
        Dim::Length(k) => Some(format!("{length}^{k}")),
        Dim::Mass => Some("kg".to_string()),
        Dim::Count | Dim::Flag => None,
    }
}

fn dim_of(measure: &str) -> Dim {
    match measure {
        "volume" => Dim::Length(3),
        "area" | "sectionArea" => Dim::Length(2),
        "volumeBelow" => Dim::Length(3),
        "sectionInertia" => Dim::Length(4),
        "mass" => Dim::Mass,
        "count" => Dim::Count,
        "through" | "overlap" | "symmetric" => Dim::Flag,
        _ => Dim::Length(1),
    }
}

/// Checks one assertion against the document.
fn check(doc: &Document, regen: &Regen, assertion: &Assertion) -> Checked {
    let length = length_unit(doc);
    let parsed: Option<Map<String, Value>> = serde_json::from_str(&assertion.measure).ok();
    let measure = parsed
        .as_ref()
        .and_then(|m| m.get("measure"))
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let dim = dim_of(&measure);
    let unit = unit_of(dim, length);
    let mut checked = Checked {
        state: AssertionState {
            name: assertion.name.clone(),
            measure: measure.clone(),
            status: Status::Error,
            measured: None,
            expected: String::new(),
            message: String::new(),
        },
        stored: assertion.measure.clone(),
        value: Value::Null,
        unit: unit.clone(),
        detail: Map::new(),
    };
    let with_unit = |v: f64| match &unit {
        Some(u) => format!("{} {u}", shown(v)),
        None => shown(v),
    };
    // What it must be, as numbers.
    let ctx = match Ctx::new(doc, regen) {
        Ok(ctx) => ctx,
        Err(e) => {
            checked.state.message = e;
            return checked;
        }
    };
    let power = match dim {
        Dim::Length(k) => k,
        _ => 0,
    };
    let value_of =
        |e: &Expr, key: &str| ctx.quantity(&Value::String(e.source().to_string()), key, power);
    type Bounds = (Option<f64>, Option<f64>, Option<bool>);
    let bounds: Result<Bounds, String> = match &assertion.expect {
        Expect::Is(is) => {
            let is = *is;
            checked.state.expected = if measure == "overlap" {
                if is { "an overlap" } else { "no overlap" }.to_string()
            } else {
                is.to_string()
            };
            Ok((None, None, Some(is)))
        }
        Expect::Equal { value, tol } => (|| {
            let v = value_of(value, "eq")?;
            let t = match tol {
                Some(t) => value_of(t, "tol")?.abs(),
                None => match dim {
                    Dim::Count => 0.0,
                    Dim::Length(1) => (1e-6 * v.abs()).max(10.0 * doc.tolerance.linear),
                    _ => 1e-6 * v.abs(),
                },
            };
            checked.state.expected = format!("{} ± {}", with_unit(v), shown(t));
            Ok((Some(v - t), Some(v + t), None))
        })(),
        Expect::Within { min, max } => (|| {
            let lo = min.as_ref().map(|m| value_of(m, "min")).transpose()?;
            let hi = max.as_ref().map(|m| value_of(m, "max")).transpose()?;
            checked.state.expected = match (lo, hi) {
                (Some(lo), Some(hi)) => format!("{} to {}", with_unit(lo), with_unit(hi)),
                (Some(lo), None) => format!("≥ {}", with_unit(lo)),
                (None, Some(hi)) => format!("≤ {}", with_unit(hi)),
                (None, None) => String::new(),
            };
            Ok((lo, hi, None))
        })(),
    };
    let (lo, hi, want) = match bounds {
        Ok(b) => b,
        Err(e) => {
            checked.state.message = format!("what it must be cannot be worked out: {e}");
            return checked;
        }
    };
    let Some(mut params) = parsed else {
        checked.state.message = "what it measures is not readable".into();
        return checked;
    };
    params.remove("measure");
    // A box found on a mesh that leaves the bound undecided is looked at
    // again on a finer mesh, a few times, while one can be made finer.
    let refinable = matches!(measure.as_str(), "extent" | "boxMin" | "boxMax");
    let mut ctx = ctx;
    let (outcome, verdict) = loop {
        let outcome: Result<Outcome, String> = take(&ctx, &measure, &params);
        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => {
                checked.state.message = format!("cannot be measured: {e}");
                return checked;
            }
        };
        let verdict = decide(&outcome, want, (lo, hi), dim, doc.tolerance.linear);
        let finer = outcome
            .detail
            .get("meshTolerance")
            .and_then(Value::as_f64)
            .is_some_and(|t| t > 10.0 * doc.tolerance.linear * (1.0 + 1e-9));
        if verdict.is_some() || !refinable || !finer || ctx.refine >= MAX_REFINE {
            break (outcome, verdict);
        }
        ctx.refine = ctx.refine.saturating_add(1);
    };
    checked.detail = outcome.detail;
    if want.is_some() {
        let got = outcome.flag.unwrap_or(false);
        checked.value = got.into();
        checked.state.measured = Some(match &outcome.note {
            // Said in words: which bodies overlap by how much, or none.
            Some(note) if measure == "overlap" => note.clone(),
            Some(note) => format!("{got} ({note})"),
            None => got.to_string(),
        });
    } else {
        match outcome.number {
            Some(v) => {
                checked.value = v.into();
                checked.state.measured = Some(with_unit(v));
                let lower = outcome.lower.unwrap_or(v);
                let upper = outcome.upper.unwrap_or(v);
                if lower < upper {
                    // Which bound is proven beside the value: the side it
                    // is not on (see `Outcome::lower`).
                    let proven = if v >= upper {
                        checked.detail.insert("atLeast".into(), lower.into());
                        format!("at least {}", with_unit(lower))
                    } else if v <= lower {
                        checked.detail.insert("atMost".into(), upper.into());
                        format!("at most {}", with_unit(upper))
                    } else {
                        checked.detail.insert("atLeast".into(), lower.into());
                        checked.detail.insert("atMost".into(), upper.into());
                        format!("between {} and {}", with_unit(lower), with_unit(upper))
                    };
                    checked.state.measured = Some(format!("{} ({proven})", with_unit(v)));
                }
            }
            None => {
                checked.value = outcome.note.clone().map_or(Value::Null, Value::from);
                checked.state.measured = outcome.note.clone();
            }
        }
    }
    let measured = checked
        .state
        .measured
        .clone()
        .unwrap_or_else(|| "nothing".into());
    (checked.state.status, checked.state.message) = match verdict {
        // What an overlap found is a sentence of its own.
        Some(true) if measure == "overlap" => (Status::Passed, measured),
        Some(false) if measure == "overlap" => (
            Status::Failed,
            format!("{measured}, expected {}", checked.state.expected),
        ),
        Some(true) => (Status::Passed, format!("{measure}: {measured}")),
        Some(false) => (
            Status::Failed,
            format!(
                "{measure}: measured {measured}, expected {}",
                checked.state.expected
            ),
        ),
        None => (
            Status::NotProven,
            format!(
                "{measure}: not proven: found {measured}, expected {}; what was measured was not pinned down closely enough to tell",
                checked.state.expected
            ),
        ),
    };
    checked
}

/// How many times a box found on a mesh is looked at again, each time on
/// a mesh eight times finer, while that leaves the bound undecided.
const MAX_REFINE: u32 = 3;

/// Whether `outcome` meets what is wanted: yes, no, or not proven either
/// way (`None`). `want` for a yes-or-no measure; else the bounds `lo` and
/// `hi` against the range the true value lies in, `[lower, upper]`: a
/// bound holds when the whole of that range is within it, fails when none
/// of it is, and is not proven otherwise. Within the document's tolerance
/// `linear` (of a length; its square or cube for an area or a volume, and
/// rounding for the rest) a bound is met: what the geometry cannot tell
/// apart does not fail it.
pub(crate) fn decide(
    outcome: &Outcome,
    want: Option<bool>,
    (lo, hi): (Option<f64>, Option<f64>),
    dim: Dim,
    linear: f64,
) -> Option<bool> {
    if let Some(want) = want {
        return Some(outcome.flag.unwrap_or(false) == want);
    }
    let Some(v) = outcome.number else {
        return Some(false);
    };
    let lower = outcome.lower.unwrap_or(v);
    let upper = outcome.upper.unwrap_or(v);
    let slack = |b: f64| {
        let tol = match dim {
            Dim::Length(k) => linear.powi(k),
            _ => 0.0,
        };
        tol.max(1e-12 * b.abs())
    };
    let at_least = |lo: f64| {
        if lower >= lo - slack(lo) {
            Some(true)
        } else if upper < lo - slack(lo) {
            Some(false)
        } else {
            None
        }
    };
    let at_most = |hi: f64| {
        if upper <= hi + slack(hi) {
            Some(true)
        } else if lower > hi + slack(hi) {
            Some(false)
        } else {
            None
        }
    };
    let checks = [lo.map(at_least), hi.map(at_most)];
    if checks.contains(&Some(Some(false))) {
        Some(false)
    } else if checks.contains(&Some(None)) {
        None
    } else {
        Some(true)
    }
}

/// Checks one assertion; one that panics is an error, not a lost
/// document.
pub(crate) fn check_one(doc: &Document, regen: &Regen, assertion: &Assertion) -> Checked {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check(doc, regen, assertion)
    }))
    .unwrap_or_else(|_| Checked {
        state: AssertionState {
            name: assertion.name.clone(),
            measure: String::new(),
            status: Status::Error,
            measured: None,
            expected: String::new(),
            message: "internal error while measuring (a bug in 3DCraft, please report it)".into(),
        },
        stored: assertion.measure.clone(),
        value: Value::Null,
        unit: None,
        detail: Map::new(),
    })
}

/// For the result of an edit and for `document.check`: how the
/// assertions of the active document stand, `None` when it has none:
/// `{count, passed, broken: [failed or in error], unproven, pending}`.
pub(crate) fn summary(session: &mut Session, wait: Wait) -> Option<Value> {
    let checked = session.assertion_checks(wait).ok()?;
    if checked.is_empty() {
        return None;
    }
    let entry = |c: &Checked| {
        json!({
            "name": c.state.name,
            "status": c.state.status.name(),
            "measured": c.value,
            "unit": c.unit,
            "expected": c.state.expected,
            "message": c.state.message,
        })
    };
    let of = |want: &[Status]| -> Vec<Value> {
        checked
            .iter()
            .filter(|c| want.contains(&c.state.status))
            .map(entry)
            .collect()
    };
    let count = |s: Status| checked.iter().filter(|c| c.state.status == s).count();
    Some(json!({
        "count": checked.len(),
        "passed": count(Status::Passed),
        "broken": of(&[Status::Failed, Status::Error]),
        "unproven": of(&[Status::NotProven]),
        "pending": count(Status::Pending),
    }))
}

/// Adds the assertions' summary to the result of an edit, under
/// `assertions`, when the document has any: waiting at most
/// [`EDIT_WAIT`] for those the edit touched.
pub(crate) fn add_summary(session: &mut Session, result: &mut Value) {
    add_summary_waiting(session, result, false);
}

/// How long an edit with `wait: true` waits for its assertions at most:
/// what is not measured by then stays pending, with `whenPending`.
pub(crate) const WAIT_CEILING: std::time::Duration = std::time::Duration::from_secs(60);

/// [`add_summary`], waiting for every assertion when `all` (an edit's
/// `wait: true`), for [`WAIT_CEILING`] at most. Assertions still being measured are counted under
/// `pending`, and `whenPending` then says how to have their results, so
/// a script does not read "pending" as an answer.
pub(crate) fn add_summary_waiting(session: &mut Session, result: &mut Value, all: bool) {
    let wait = Wait::For(if all { WAIT_CEILING } else { EDIT_WAIT });
    let Some(mut s) = summary(session, wait) else {
        return;
    };
    let pending = s.get("pending").and_then(Value::as_u64).unwrap_or(0);
    if pending > 0
        && let Some(summary) = s.as_object_mut()
    {
        summary.insert("whenPending".into(), Value::String(format!(
            "{pending} assertion(s) are still being measured in the background; assert.list (it waits by default) gives their results, or give this edit wait: true (it waits up to 60 s)"
        )));
    }
    if let Some(map) = result.as_object_mut() {
        map.insert("assertions".into(), s);
    }
}

/// The `wait` parameter of an edit that returns the assertions' summary:
/// false (the default) waits a short while for the assertions the edit
/// touched, true until every one is measured.
pub(crate) fn wait_param(p: &crate::params::Params) -> Result<bool, CommandError> {
    Ok(p.bool("wait")?.unwrap_or(false))
}
