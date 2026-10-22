//! Assertions kept in the document (`assert.add`, `assert.edit`,
//! `assert.delete`, `assert.list`): the design intent written down as
//! measures with what they must be, checked against every regeneration.
//! A broken assertion never stops regeneration; it shows like a failed
//! feature, with what was measured and what was expected, in
//! `document.check`, in the result of the command whose edit broke it,
//! in the window's tree and over MCP. The design note is
//! docs/design/verification.md.

use crate::commands::{Run, edit, fail, length_unit, spec};
use crate::params::Params;
use crate::verify_cmds::{Ctx, Dim, MEASURES, Outcome, only, take};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Assertion, Document, Expect, Expr};
use craft3d_regen::Regen;
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("assert.add", "Add Assertion", r#"{"name": string, "measure": "volume|area|mass|count|extent|distance|clearance|thickness|through|interference|symmetric", "of": subject, "from": subject, "to": subject, "axis": "x|y|z", "plane": plane, "tol": expr, "eq": expr, "min": expr, "max": expr, "is": boolean = true}"#,
        "Adds an assertion to the document: a measure of part of the design and what it must be, checked after every regeneration (one undo step; saved with the document). Measures and their parameters: volume, area, mass (of: bodies, default all; area also of faces; mass needs a material), count (of: a query, the number of things it finds), extent (of: bodies, axis: the size of their box along it), distance (from, to: faces or bodies, the least distance), clearance (from, to: bodies; a pair that overlaps fails), thickness (of: a body, the thinnest wall), through (of: cylindrical or conical faces, every one cut through along its axis), interference (of: bodies, to: bodies = each other; none overlap), symmetric (of, plane, tol: as measure.symmetry). A subject is a body (feature name or id), a list of them, a query {kind, where} as select.query takes, or a bare filter. What it must be: eq (with tol, default a millionth of eq or ten times the document's tolerance for a length) or min and/or max, expressions in the document's variables (\"wall\", \"2 mm\"), in the measure's unit (length, area or volume in the document's unit, mass in kg); through, interference and symmetric take is (default true). Feature names in the subject are kept as ids, so a renamed feature stays measured. Returns the assertion and its check: status passed, failed (measured, expected) or error (the measure could not be taken, such as a subject that names nothing yet: an assertion may be written before the model meets it).", doc: true, assert_add),
    spec!("assert.edit", "Edit Assertion", r#"{"name": string, "rename": string, ...the parameters of assert.add}"#,
        "Changes an assertion (one undo step): what is given replaces what it had; measure parameters are replaced as a whole when measure or any of them is given, the expectation when any of eq, tol, min, max or is is given. Returns it with its check.", doc: true, assert_edit),
    spec!("assert.delete", "Delete Assertion", r#"{"name": string}"#,
        "Removes an assertion from the document (one undo step).", doc: true, assert_delete),
    spec!("assert.list", "Assertions", r#"{"details": boolean = false}"#,
        "Lists the document's assertions, each checked against the document as it is now: name, measure (as stored), expected, status (passed, failed or error), measured with its unit, message (one line); with details what the measure found (where the thinnest place is, which hole is blocked). passed, failed and errors count them; ok is true when none failed and none is in error.", doc: true, assert_list),
];

/// The keys of each measure's parameters.
fn measure_keys(measure: &str) -> &'static [&'static str] {
    match measure {
        "volume" | "area" | "mass" | "count" | "thickness" | "through" => &["of"],
        "extent" => &["of", "axis"],
        "distance" | "clearance" => &["from", "to"],
        "interference" => &["of", "to"],
        "symmetric" => &["of", "plane", "tol"],
        _ => &[],
    }
}

fn is_flag(measure: &str) -> bool {
    matches!(measure, "through" | "interference" | "symmetric")
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

/// An expression parameter.
fn expr(p: &Params, key: &str) -> Result<Option<Expr>, CommandError> {
    Ok(p.expr(key)?.map(Expr::new))
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

const ADD_KEYS: [&str; 14] = [
    "name", "measure", "of", "from", "to", "axis", "plane", "tol", "eq", "min", "max", "is",
    "rename", "details",
];

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
    let p = Params::new(ID, params, &ADD_KEYS[..12])?;
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
    let p = Params::new(ID, params, &ADD_KEYS[..13])?;
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
    let p = Params::new("assert.list", params, &["details"])?;
    let details = p.bool("details")?.unwrap_or(false);
    let checked = session.checked_assertions()?;
    let doc = session.document()?.history.current();
    let listed: Vec<Value> = checked.iter().map(|c| c.json(doc, details)).collect();
    let count = |s: Status| checked.iter().filter(|c| c.state.status == s).count();
    let (passed, failed, errors) = (
        count(Status::Passed),
        count(Status::Failed),
        count(Status::Error),
    );
    Ok(json!({
        "assertions": listed,
        "count": checked.len(),
        "passed": passed,
        "failed": failed,
        "errors": errors,
        "ok": failed == 0 && errors == 0,
    }))
}

/// One assertion with its check, as `assert.add` and `assert.edit`
/// return it.
fn one(session: &mut Session, name: &str, details: bool) -> Run {
    let checked = session.checked_assertions()?;
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
fn shown(v: f64) -> String {
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
        "area" => Dim::Length(2),
        "mass" => Dim::Mass,
        "count" => Dim::Count,
        "through" | "interference" | "symmetric" => Dim::Flag,
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
            checked.state.expected = is.to_string();
            Ok((None, None, Some(*is)))
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
    let outcome: Result<Outcome, String> = take(&ctx, &measure, &params);
    let outcome = match outcome {
        Ok(o) => o,
        Err(e) => {
            checked.state.message = format!("cannot be measured: {e}");
            return checked;
        }
    };
    checked.detail = outcome.detail;
    let pass = if let Some(want) = want {
        let got = outcome.flag.unwrap_or(false);
        checked.value = got.into();
        checked.state.measured = Some(match &outcome.note {
            Some(note) => format!("{got} ({note})"),
            None => got.to_string(),
        });
        got == want
    } else {
        match outcome.number {
            Some(v) => {
                checked.value = v.into();
                checked.state.measured = Some(with_unit(v));
                lo.is_none_or(|lo| v >= lo) && hi.is_none_or(|hi| v <= hi)
            }
            None => {
                checked.value = outcome.note.clone().map_or(Value::Null, Value::from);
                checked.state.measured = outcome.note.clone();
                false
            }
        }
    };
    checked.state.status = if pass { Status::Passed } else { Status::Failed };
    checked.state.message = if pass {
        format!(
            "{measure}: {}",
            checked.state.measured.clone().unwrap_or_default()
        )
    } else {
        format!(
            "{measure}: measured {}, expected {}",
            checked
                .state
                .measured
                .clone()
                .unwrap_or_else(|| "nothing".into()),
            checked.state.expected
        )
    };
    checked
}

/// Checks every assertion of `doc`, each on its own: one that panics is
/// an error, not a lost document.
fn check_all(doc: &Document, regen: &Regen) -> Vec<Checked> {
    doc.assertions
        .iter()
        .map(|assertion| {
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
                    message: "internal error while measuring (a bug in 3DCraft, please report it)"
                        .into(),
                },
                stored: assertion.measure.clone(),
                value: Value::Null,
                unit: None,
                detail: Map::new(),
            })
        })
        .collect()
}

impl Session {
    /// The assertions of the active document checked against it as it is
    /// now, remembered per revision.
    pub(crate) fn checked_assertions(&mut self) -> Result<Arc<Vec<Checked>>, CommandError> {
        let open = self.document_mut()?;
        let revision = open.history.revision();
        if let Some((at, checked)) = &open.assertions
            && *at == revision
        {
            return Ok(Arc::clone(checked));
        }
        let regen = open.regen();
        let checked = Arc::new(check_all(open.history.current(), &regen));
        open.assertions = Some((revision, Arc::clone(&checked)));
        Ok(checked)
    }

    /// The assertions of the active document and how each stands, in the
    /// document's order; empty without a document or assertions.
    pub fn assertion_states(&mut self) -> Vec<AssertionState> {
        if self
            .document()
            .map_or(true, |d| d.history.current().assertions.is_empty())
        {
            return Vec::new();
        }
        self.checked_assertions()
            .map(|c| c.iter().map(|c| c.state.clone()).collect())
            .unwrap_or_default()
    }
}

/// For the result of an edit and for `document.check`: the broken
/// assertions of the active document, or `None` when it has none.
pub(crate) fn summary(session: &mut Session) -> Option<Value> {
    if session
        .document()
        .map_or(true, |d| d.history.current().assertions.is_empty())
    {
        return None;
    }
    let checked = session.checked_assertions().ok()?;
    let broken: Vec<Value> = checked
        .iter()
        .filter(|c| c.state.status != Status::Passed)
        .map(|c| {
            json!({
                "name": c.state.name,
                "status": c.state.status.name(),
                "measured": c.value,
                "unit": c.unit,
                "expected": c.state.expected,
                "message": c.state.message,
            })
        })
        .collect();
    let passed = checked.len() - broken.len();
    Some(json!({
        "count": checked.len(),
        "passed": passed,
        "broken": broken,
    }))
}

/// Adds the assertions' summary to the result of an edit, under
/// `assertions`, when the document has any.
pub(crate) fn add_summary(session: &mut Session, result: &mut Value) {
    if let (Some(s), Some(map)) = (summary(session), result.as_object_mut()) {
        map.insert("assertions".into(), s);
    }
}
