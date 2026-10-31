//! Scoring a modelling task: the core of the R1 task set and of a CAD gym
//! built on it (`docs/tasks/`, `docs/testing.md` › *The task set*).
//!
//! A task is a JSON file: a description of a part, the variables that
//! drive it, and expectations written as closed-form expressions of those
//! variables (volume, area, centroid, bounding box, counts). A candidate
//! is a script of commands. [`score_task`] runs the script headless in a
//! session that may neither read nor write files, measures what it built,
//! checks it deeply, and returns a scorecard: every expectation with the
//! value expected and the value measured, validity, the number of steps
//! and the steps that failed. Each variant of the task then sets the
//! candidate's variables to other values, regenerates, and holds the same
//! expressions, evaluated with the new values, to the new model.
//!
//! Nothing here panics on a malformed task or script: a task that cannot
//! be read is an error, a script step that fails is a refusal on the
//! scorecard.

use crate::{CommandError, Session, Workspace};
use craft3d_expr::{Dimension, Quantity, Unit, VariableTable};
use craft3d_math::Portable;
use serde_json::{Map, Value, json};

/// The format tag a task file carries.
pub const TASK_FORMAT: &str = "craft3d-task/1";

/// The format tag of a scorecard.
pub const SCORECARD_FORMAT: &str = "craft3d-scorecard/1";

/// Most steps a candidate script may have.
pub const MAX_STEPS: usize = 5000;

/// Default relative tolerance of a volume or an area.
const DEFAULT_REL_TOL: f64 = 1e-6;

/// Default absolute tolerance of a length (centroid, box), in mm.
const DEFAULT_ABS_TOL: f64 = 1e-4;

/// Most expectations and variants a task may have.
const MAX_EXPECTATIONS: usize = 256;
const MAX_VARIANTS: usize = 32;

fn task_error(message: impl Into<String>) -> CommandError {
    CommandError::new(format!("task: {}", message.into()))
}

/// One step of a script: a command id and its params.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub command: String,
    pub params: Value,
}

/// Reads a script: a JSON array whose items are `["id", {params}]` (the
/// params optional) or `{"command": "id", "params": {...}}`, the form
/// `craft3d-cli run --script` takes.
pub fn parse_script(script: &Value) -> Result<Vec<Step>, CommandError> {
    let Value::Array(items) = script else {
        return Err(CommandError::new(
            "script: must be a JSON array of steps, each [\"id\", {params}] or {\"command\": \"id\", \"params\": {...}}",
        ));
    };
    if items.len() > MAX_STEPS {
        return Err(CommandError::new(format!(
            "script: {} steps; the limit is {MAX_STEPS}",
            items.len()
        )));
    }
    let mut steps = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let shape = || {
            CommandError::new(format!(
                "script: step {index} must be [\"id\", {{params}}] or {{\"command\": \"id\", \"params\": {{...}}}}"
            ))
        };
        let (command, params) = match item {
            Value::Array(pair) => match pair.as_slice() {
                [Value::String(command)] => (command.clone(), Value::Null),
                [Value::String(command), params] => (command.clone(), params.clone()),
                _ => return Err(shape()),
            },
            Value::Object(map) => {
                let Some(Value::String(command)) = map.get("command") else {
                    return Err(shape());
                };
                if let Some(unknown) = map.keys().find(|k| *k != "command" && *k != "params") {
                    return Err(CommandError::new(format!(
                        "script: step {index} has an unknown key `{unknown}`; a step has `command` and `params`"
                    )));
                }
                (
                    command.clone(),
                    map.get("params").cloned().unwrap_or(Value::Null),
                )
            }
            _ => return Err(shape()),
        };
        if !(params.is_object() || params.is_null()) {
            return Err(CommandError::new(format!(
                "script: step {index} (`{command}`): params must be a JSON object"
            )));
        }
        steps.push(Step { command, params });
    }
    Ok(steps)
}

/// What an expectation measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Measure {
    Volume,
    Area,
    Centroid,
    Bbox,
    Count(&'static str),
}

impl Measure {
    fn parse(name: &str) -> Option<Measure> {
        Some(match name {
            "volume" => Measure::Volume,
            "area" => Measure::Area,
            "centroid" => Measure::Centroid,
            "bbox" => Measure::Bbox,
            "bodies" => Measure::Count("bodies"),
            "shells" => Measure::Count("shells"),
            "faces" => Measure::Count("faces"),
            "edges" => Measure::Count("edges"),
            "vertices" => Measure::Count("vertices"),
            "threads" => Measure::Count("threads"),
            _ => return None,
        })
    }
}

/// An expectation as the task gives it.
#[derive(Clone, Debug)]
struct Expectation {
    measure: Measure,
    name: String,
    /// One expression, or three (centroid), or six (bbox: min then max).
    exprs: Vec<String>,
    /// Relative for volume and area, absolute (mm) for lengths; counts are
    /// exact.
    tolerance: f64,
}

/// A task file, read.
#[derive(Clone, Debug)]
struct Task {
    id: String,
    units: Unit,
    variables: Vec<(String, String)>,
    expectations: Vec<Expectation>,
    variants: Vec<Vec<(String, String)>>,
}

fn str_field<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a str, CommandError> {
    map.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| task_error(format!("`{key}` must be a string")))
}

fn expr_text(value: &Value, what: &str) -> Result<String, CommandError> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(task_error(format!(
            "{what} must be an expression (a string) or a number"
        ))),
    }
}

fn expr_list(value: Option<&Value>, len: usize, what: &str) -> Result<Vec<String>, CommandError> {
    let Some(Value::Array(items)) = value else {
        return Err(task_error(format!("{what} must be an array of {len}")));
    };
    if items.len() != len {
        return Err(task_error(format!("{what} must be an array of {len}")));
    }
    items.iter().map(|v| expr_text(v, what)).collect()
}

fn parse_expectation(index: usize, value: &Value) -> Result<Expectation, CommandError> {
    let Value::Object(map) = value else {
        return Err(task_error(format!("expect[{index}] must be an object")));
    };
    let name = str_field(map, "measure")?;
    let measure = Measure::parse(name).ok_or_else(|| {
        task_error(format!(
            "expect[{index}]: unknown measure `{name}`; one of volume, area, centroid, bbox, bodies, shells, faces, edges, vertices, threads"
        ))
    })?;
    let what = format!("expect[{index}] ({name})");
    let exprs = match measure {
        Measure::Volume | Measure::Area | Measure::Count(_) => {
            vec![expr_text(
                map.get("value")
                    .ok_or_else(|| task_error(format!("{what}: `value` is missing")))?,
                &what,
            )?]
        }
        Measure::Centroid => expr_list(map.get("value"), 3, &what)?,
        Measure::Bbox => {
            let mut all = expr_list(map.get("min"), 3, &format!("{what} min"))?;
            all.extend(expr_list(map.get("max"), 3, &format!("{what} max"))?);
            all
        }
    };
    let default = match measure {
        Measure::Volume | Measure::Area => DEFAULT_REL_TOL,
        Measure::Centroid | Measure::Bbox => DEFAULT_ABS_TOL,
        Measure::Count(_) => 0.0,
    };
    let tolerance = match map.get("tolerance") {
        None => default,
        Some(v) => v
            .as_f64()
            .filter(|t| t.is_finite() && *t >= 0.0)
            .ok_or_else(|| task_error(format!("{what}: tolerance must be a number >= 0")))?,
    };
    Ok(Expectation {
        measure,
        name: name.to_string(),
        exprs,
        tolerance,
    })
}

fn parse_assignments(value: &Value, what: &str) -> Result<Vec<(String, String)>, CommandError> {
    let Value::Object(map) = value else {
        return Err(task_error(format!(
            "{what} must be an object of name: expression"
        )));
    };
    map.iter()
        .map(|(name, v)| Ok((name.clone(), expr_text(v, what)?)))
        .collect()
}

fn parse_task(task: &Value) -> Result<Task, CommandError> {
    let Value::Object(map) = task else {
        return Err(task_error("must be a JSON object"));
    };
    let format = str_field(map, "format")?;
    if format != TASK_FORMAT {
        return Err(task_error(format!(
            "format `{format}` is not `{TASK_FORMAT}`"
        )));
    }
    let id = str_field(map, "id")?.to_string();
    let units = match map.get("units") {
        None => Unit::Millimetre,
        Some(v) => v
            .as_str()
            .and_then(Unit::from_symbol)
            .filter(|u| u.dimension() == Dimension::LENGTH)
            .ok_or_else(|| task_error("`units` must be a length unit: um, mm, cm, m, in or ft"))?,
    };
    let mut variables = Vec::new();
    if let Some(list) = map.get("variables") {
        let Value::Array(items) = list else {
            return Err(task_error("`variables` must be an array"));
        };
        for (index, item) in items.iter().enumerate() {
            let Value::Object(v) = item else {
                return Err(task_error(format!("variables[{index}] must be an object")));
            };
            let name = str_field(v, "name")?.to_string();
            let expr = expr_text(
                v.get("expr")
                    .ok_or_else(|| task_error(format!("variables[{index}]: `expr` is missing")))?,
                "a variable's expr",
            )?;
            variables.push((name, expr));
        }
    }
    let Some(Value::Array(expect)) = map.get("expect") else {
        return Err(task_error("`expect` must be an array"));
    };
    if expect.is_empty() || expect.len() > MAX_EXPECTATIONS {
        return Err(task_error(format!(
            "`expect` must hold 1 to {MAX_EXPECTATIONS} expectations"
        )));
    }
    let expectations = expect
        .iter()
        .enumerate()
        .map(|(i, e)| parse_expectation(i, e))
        .collect::<Result<Vec<_>, _>>()?;
    let mut variants = Vec::new();
    if let Some(list) = map.get("variants") {
        let Value::Array(items) = list else {
            return Err(task_error("`variants` must be an array"));
        };
        if items.len() > MAX_VARIANTS {
            return Err(task_error(format!("at most {MAX_VARIANTS} variants")));
        }
        for (index, item) in items.iter().enumerate() {
            let set = item
                .get("set")
                .ok_or_else(|| task_error(format!("variants[{index}]: `set` is missing")))?;
            let set = parse_assignments(set, &format!("variants[{index}].set"))?;
            for (name, _) in &set {
                if !variables.iter().any(|(n, _)| n == name) {
                    return Err(task_error(format!(
                        "variants[{index}] sets `{name}`, which is not one of the task's variables"
                    )));
                }
            }
            variants.push(set);
        }
    }
    let task = Task {
        id,
        units,
        variables,
        expectations,
        variants,
    };
    // Every expectation has to evaluate, with the right dimension, for the
    // task's own values and for every variant: a typo in a derivation is
    // the task's fault, not the candidate's.
    for overrides in std::iter::once(&Vec::new()).chain(task.variants.iter()) {
        let values = task_values(&task, overrides)?;
        for e in &task.expectations {
            expected_values(e, &values, task.units)?;
        }
    }
    Ok(task)
}

/// The task's variables evaluated, with `overrides` in place of their
/// own expressions.
fn task_values(task: &Task, overrides: &[(String, String)]) -> Result<VariableTable, CommandError> {
    let mut table = VariableTable::new();
    for (name, expr) in &task.variables {
        let expr = overrides
            .iter()
            .find(|(n, _)| n == name)
            .map_or(expr.as_str(), |(_, e)| e.as_str());
        table
            .set(name, expr)
            .map_err(|e| task_error(format!("variable `{name}` = `{expr}`: {e}")))?;
    }
    Ok(table)
}

/// The dimension an expectation's expressions must have.
fn dimension_of(measure: Measure) -> Dimension {
    match measure {
        Measure::Volume => Dimension::new(3, 0),
        Measure::Area => Dimension::new(2, 0),
        Measure::Centroid | Measure::Bbox => Dimension::LENGTH,
        Measure::Count(_) => Dimension::NONE,
    }
}

/// The expected values of an expectation, in millimetres (or a count).
/// A plain number where a length, an area or a volume is expected is in
/// the task's units (`0` for a corner at the origin).
fn expected_values(
    e: &Expectation,
    table: &VariableTable,
    units: Unit,
) -> Result<Vec<f64>, CommandError> {
    let evaluation = table.evaluate();
    let want = dimension_of(e.measure);
    e.exprs
        .iter()
        .map(|source| {
            let q: Quantity = evaluation
                .evaluate(source)
                .map_err(|err| task_error(format!("{} `{source}`: {err}", e.name)))?;
            if q.dimension.is_dimensionless() && want.angle == 0 && want.length > 0 {
                return Ok(q.value * units.factor().ppowi(want.length));
            }
            if q.dimension != want {
                return Err(task_error(format!(
                    "{} `{source}` is a {}, not a {want}",
                    e.name, q.dimension
                )));
            }
            Ok(q.value)
        })
        .collect()
}

/// What the document measures, in millimetres.
struct Measured {
    volume: f64,
    area: f64,
    centroid: Option<[f64; 3]>,
    bbox: Option<[f64; 6]>,
    bbox_tolerance: f64,
    counts: Map<String, Value>,
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite())
}

fn triple(value: Option<&Value>) -> Option<[f64; 3]> {
    let items = value?.as_array()?;
    match items.as_slice() {
        [x, y, z] => Some([number(x)?, number(y)?, number(z)?]),
        _ => None,
    }
}

fn measure(inspect: &Value) -> Measured {
    let factor = inspect
        .pointer("/units/length")
        .and_then(Value::as_str)
        .and_then(Unit::from_symbol)
        .map_or(1.0, Unit::factor);
    let bodies: &[Value] = inspect
        .get("bodies")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let mut volume = 0.0;
    let mut area = 0.0;
    let mut moment = [0.0; 3];
    let mut bbox: Option<[f64; 6]> = None;
    let mut bbox_tolerance: f64 = 0.0;
    let mut counts = Map::new();
    counts.insert("bodies".into(), bodies.len().into());
    for key in ["shells", "faces", "edges", "vertices"] {
        let sum: u64 = bodies
            .iter()
            .filter_map(|b| b.get(key).and_then(Value::as_u64))
            .fold(0, u64::saturating_add);
        counts.insert(key.into(), sum.into());
    }
    // Threads are marks on faces (cosmetic) or grooves (modeled): counted
    // as the features that put one on and were built.
    let threads = inspect
        .get("features")
        .and_then(Value::as_array)
        .map_or(0, |features| {
            features
                .iter()
                .filter(|f| {
                    f.pointer("/params/type").and_then(Value::as_str) == Some("thread")
                        && f.get("status").and_then(Value::as_str) == Some("ok")
                })
                .count()
        });
    counts.insert("threads".into(), threads.into());
    for body in bodies {
        let v = body.get("volume").and_then(number).unwrap_or(0.0) * factor.ppowi(3);
        volume += v;
        area += body.get("area").and_then(number).unwrap_or(0.0) * factor.ppowi(2);
        if let Some(c) = triple(body.get("centroid")) {
            for (m, c) in moment.iter_mut().zip(c) {
                *m += v * c * factor;
            }
        }
        if let Some(Value::Array(corners)) = body.get("bbox")
            && let [lo, hi] = corners.as_slice()
            && let (Some(lo), Some(hi)) = (triple(Some(lo)), triple(Some(hi)))
        {
            let b = [
                lo[0] * factor,
                lo[1] * factor,
                lo[2] * factor,
                hi[0] * factor,
                hi[1] * factor,
                hi[2] * factor,
            ];
            bbox = Some(match bbox {
                None => b,
                Some(a) => [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].min(b[2]),
                    a[3].max(b[3]),
                    a[4].max(b[4]),
                    a[5].max(b[5]),
                ],
            });
        }
        if let Some(t) = body.get("bboxTolerance").and_then(number) {
            bbox_tolerance = bbox_tolerance.max(t * factor);
        }
    }
    let centroid = (volume.abs() > 0.0).then(|| moment.map(|m| m / volume));
    Measured {
        volume,
        area,
        centroid,
        bbox,
        bbox_tolerance,
        counts,
    }
}

/// An expected count as a whole number in JSON (`3`, not `3.0`).
fn count_json(value: f64) -> Value {
    let rounded = value.round();
    if rounded.is_finite() && rounded.abs() < 9.0e15 {
        #[allow(clippy::cast_possible_truncation)]
        let whole = rounded as i64;
        json!(whole)
    } else {
        Value::Null
    }
}

/// Holds one expectation to what was measured.
fn check(e: &Expectation, expected: &[f64], m: &Measured) -> Value {
    let close = |want: f64, got: f64, tol: f64| (want - got).abs() <= tol;
    let (measured, pass, tolerance): (Value, bool, Value) = match e.measure {
        Measure::Volume | Measure::Area => {
            let got = if e.measure == Measure::Volume {
                m.volume
            } else {
                m.area
            };
            let want = expected.first().copied().unwrap_or(f64::NAN);
            let tol = e.tolerance * want.abs().max(1e-12);
            (
                got.into(),
                close(want, got, tol),
                json!({ "relative": e.tolerance }),
            )
        }
        Measure::Centroid => match m.centroid {
            None => (Value::Null, false, json!({ "absolute": e.tolerance })),
            Some(c) => (
                json!(c),
                expected.len() == 3
                    && expected
                        .iter()
                        .zip(c)
                        .all(|(w, g)| close(*w, g, e.tolerance)),
                json!({ "absolute": e.tolerance }),
            ),
        },
        Measure::Bbox => {
            // The box is taken from a tessellation and lies inside the
            // true box by at most bboxTolerance (document.inspect).
            let tol = e.tolerance + m.bbox_tolerance;
            match m.bbox {
                None => (Value::Null, false, json!({ "absolute": tol })),
                Some(b) => (
                    json!([[b[0], b[1], b[2]], [b[3], b[4], b[5]]]),
                    expected.len() == 6 && expected.iter().zip(b).all(|(w, g)| close(*w, g, tol)),
                    json!({ "absolute": tol }),
                ),
            }
        }
        Measure::Count(key) => {
            let got = m.counts.get(key).and_then(Value::as_f64).unwrap_or(-1.0);
            let want = expected.first().copied().unwrap_or(f64::NAN);
            (
                m.counts.get(key).cloned().unwrap_or(Value::Null),
                close(want.round(), got, 0.0),
                json!({ "absolute": 0 }),
            )
        }
    };
    let expected: Value = match e.measure {
        Measure::Volume | Measure::Area => expected.first().copied().into(),
        Measure::Count(_) => count_json(expected.first().copied().unwrap_or(f64::NAN)),
        Measure::Centroid => json!(expected),
        Measure::Bbox => match expected {
            [a, b, c, d, e, f] => json!([[a, b, c], [d, e, f]]),
            _ => Value::Null,
        },
    };
    json!({
        "measure": e.name,
        "expected": expected,
        "measured": measured,
        "tolerance": tolerance,
        "pass": pass,
    })
}

/// Measures the active document and holds it to the task: the
/// expectations evaluated with `values`, and a deep check.
fn evaluate_state(session: &mut Session, task: &Task, values: &VariableTable) -> Value {
    let inspect = session.execute("document.inspect", &json!({}));
    let check_result = session.execute("document.check", &json!({ "deep": true }));
    let (valid, problems) = match &check_result {
        Ok(c) => (
            c.get("ok").and_then(Value::as_bool).unwrap_or(false),
            c.get("problems").cloned().unwrap_or(Value::Null),
        ),
        Err(e) => (false, json!([e.message()])),
    };
    let measured = match &inspect {
        Ok(i) => measure(i),
        Err(_) => measure(&Value::Null),
    };
    let failed_features = inspect
        .as_ref()
        .map_or_else(|_| Vec::new(), failed_features);
    let mut expectations = Vec::with_capacity(task.expectations.len());
    for e in &task.expectations {
        expectations.push(match expected_values(e, values, task.units) {
            Ok(expected) => check(e, &expected, &measured),
            Err(err) => json!({ "measure": e.name, "pass": false, "error": err.message() }),
        });
    }
    let has_body = measured
        .counts
        .get("bodies")
        .and_then(Value::as_u64)
        .is_some_and(|n| n > 0);
    let valid = valid && has_body;
    let passed = expectations
        .iter()
        .filter(|e| e.get("pass").and_then(Value::as_bool) == Some(true))
        .count();
    json!({
        "valid": valid,
        "problems": problems,
        "failedFeatures": failed_features,
        "expectations": expectations,
        "passed": passed,
        "total": task.expectations.len(),
        "pass": valid && passed == task.expectations.len(),
    })
}

/// The features that failed to build: a command that adds a feature
/// succeeds even when the kernel refuses the feature, which then stays in
/// the history with its reason.
fn failed_features(inspect: &Value) -> Vec<Value> {
    inspect
        .get("features")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter(|f| f.get("status").and_then(Value::as_str) == Some("failed"))
        .map(|f| {
            json!({
                "feature": f.get("id").cloned().unwrap_or(Value::Null),
                "name": f.get("name").cloned().unwrap_or(Value::Null),
                "error": f.get("error").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

/// The candidate's own expression of a variable, if it has one.
fn candidate_variable(session: &mut Session, name: &str) -> Option<String> {
    let inspect = session.execute("document.inspect", &json!({})).ok()?;
    inspect
        .get("variables")?
        .as_array()?
        .iter()
        .find(|v| v.get("name").and_then(Value::as_str) == Some(name))?
        .get("expr")?
        .as_str()
        .map(str::to_string)
}

/// Runs `script` for `task` (both JSON as read from their files) in a
/// fresh headless session and returns the scorecard
/// (`docs/testing.md` › *The task set*). Fails only when the task or the
/// script cannot be read; a step that fails is counted, not fatal.
pub fn score_task(task: &Value, script: &Value) -> Result<Value, CommandError> {
    let task_value = task;
    let task = parse_task(task_value)?;
    let steps = parse_script(script)?;
    let mut session = Session::new(Workspace::none());
    session.execute("file.new", &json!({ "units": task.units.symbol() }))?;

    let mut failed = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        // A variable.set whose expression is kept but cannot be worked out
        // fails the step too, as `craft3d-cli run` counts it.
        let error = match session.execute(&step.command, &step.params) {
            Err(error) => Some(error.message().to_string()),
            Ok(result) if step.command == "variable.set" => result
                .get("error")
                .and_then(Value::as_str)
                .map(|e| format!("variable.set: the expression cannot be worked out: {e}")),
            Ok(_) => None,
        };
        if let Some(error) = error {
            failed.push(json!({
                "step": index,
                "command": step.command,
                "error": error,
            }));
        }
    }

    let base_values = task_values(&task, &[])?;
    let base = evaluate_state(&mut session, &task, &base_values);
    let mut all_pass = base.get("pass").and_then(Value::as_bool) == Some(true);
    let mut passed = base.get("passed").and_then(Value::as_u64).unwrap_or(0)
        + u64::from(base.get("valid").and_then(Value::as_bool) == Some(true));
    let mut total = task.expectations.len() as u64 + 1;

    let mut variants = Vec::with_capacity(task.variants.len());
    for set in &task.variants {
        let mut missing = Vec::new();
        let mut errors = Vec::new();
        let mut originals = Vec::new();
        for (name, expr) in set {
            match candidate_variable(&mut session, name) {
                None => missing.push(name.clone()),
                Some(original) => {
                    originals.push((name.clone(), original));
                    if let Err(e) =
                        session.execute("variable.set", &json!({ "name": name, "expr": expr }))
                    {
                        errors.push(e.message().to_string());
                    }
                }
            }
        }
        let values = task_values(&task, set)?;
        let mut state = if missing.is_empty() {
            evaluate_state(&mut session, &task, &values)
        } else {
            json!({ "valid": false, "pass": false, "passed": 0, "total": task.expectations.len() })
        };
        // Back to the candidate's own values for the next variant.
        for (name, original) in originals {
            if let Err(e) =
                session.execute("variable.set", &json!({ "name": name, "expr": original }))
            {
                errors.push(e.message().to_string());
            }
        }
        let pass = state.get("pass").and_then(Value::as_bool) == Some(true) && errors.is_empty();
        all_pass &= pass;
        passed += state.get("passed").and_then(Value::as_u64).unwrap_or(0)
            + u64::from(state.get("valid").and_then(Value::as_bool) == Some(true));
        total += task.expectations.len() as u64 + 1;
        if let Value::Object(map) = &mut state {
            map.insert(
                "set".into(),
                Value::Object(
                    set.iter()
                        .map(|(n, e)| (n.clone(), Value::String(e.clone())))
                        .collect(),
                ),
            );
            map.insert("missingVariables".into(), json!(missing));
            map.insert("errors".into(), json!(errors));
            map.insert("pass".into(), pass.into());
        }
        variants.push(state);
    }

    let failed_features = base
        .get("failedFeatures")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let refusals = failed.len() + failed_features.as_array().map_or(0, Vec::len);
    Ok(json!({
        "format": SCORECARD_FORMAT,
        "task": task.id,
        "steps": steps.len(),
        "refusals": refusals,
        "failedSteps": failed,
        "failedFeatures": failed_features,
        "valid": base.get("valid").cloned().unwrap_or(Value::Bool(false)),
        "base": base,
        "variants": variants,
        "passed": passed,
        "total": total,
        "score": if total == 0 { 0.0 } else { passed as f64 / total as f64 },
        "pass": all_pass,
    }))
}

/// The reference solution of a task file (its `solution`).
pub fn reference_solution(task: &Value) -> Result<&Value, CommandError> {
    task.get("solution")
        .ok_or_else(|| task_error("`solution` is missing"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plate_task() -> Value {
        json!({
            "format": TASK_FORMAT,
            "id": "unit-plate",
            "variables": [{"name": "w", "expr": "20 mm"}, {"name": "t", "expr": "5 mm"}],
            "expect": [
                {"measure": "volume", "value": "w * w * t"},
                {"measure": "area", "value": "2 * w * w + 4 * w * t"},
                {"measure": "centroid", "value": ["w / 2", "w / 2", "t / 2"]},
                {"measure": "bbox", "min": [0, 0, 0], "max": ["w", "w", "t"]},
                {"measure": "bodies", "value": 1},
                {"measure": "faces", "value": 6}
            ],
            "variants": [{"set": {"w": "30 mm"}}],
            "solution": [
                ["variable.set", {"name": "w", "expr": "20 mm"}],
                ["variable.set", {"name": "t", "expr": "5 mm"}],
                ["feature.block", {"size": ["w", "w", "t"]}]
            ]
        })
    }

    #[test]
    fn a_reference_solution_scores_full_marks() {
        let task = plate_task();
        let card = score_task(&task, reference_solution(&task).unwrap()).unwrap();
        assert_eq!(card["pass"], true, "{card:#}");
        assert_eq!(card["passed"], card["total"]);
        assert_eq!(card["total"], 14);
        assert_eq!(card["refusals"], 0);
    }

    #[test]
    fn a_candidate_without_variables_fails_the_variant_only() {
        let task = plate_task();
        let script = json!([["feature.block", {"size": [20, 20, 5]}]]);
        let card = score_task(&task, &script).unwrap();
        assert_eq!(card["base"]["pass"], true, "{card:#}");
        assert_eq!(card["variants"][0]["pass"], false);
        assert_eq!(card["variants"][0]["missingVariables"], json!(["w"]));
        assert_eq!(card["pass"], false);
    }

    #[test]
    fn a_wrong_part_and_a_failing_step_are_on_the_card() {
        let task = plate_task();
        let script = json!([
            ["feature.block", {"size": [20, 20, 6]}],
            ["feature.fillet", {"edges": [], "radius": 1}],
            {"command": "no.such"}
        ]);
        let card = score_task(&task, &script).unwrap();
        assert_eq!(card["refusals"], 2, "{card:#}");
        assert_eq!(card["failedSteps"][0]["step"], 1);
        assert_eq!(card["base"]["expectations"][0]["pass"], false);
        assert_eq!(card["base"]["expectations"][4]["pass"], true);
        assert_eq!(card["valid"], true);
    }

    #[test]
    fn a_broken_task_is_an_error_not_a_score() {
        let mut task = plate_task();
        task["expect"][0]["value"] = json!("w * t");
        let e = score_task(&task, &json!([])).unwrap_err();
        assert!(e.message().contains("not a length^3"), "{}", e.message());
        let mut task = plate_task();
        task["variants"][0]["set"] = json!({"nope": "1 mm"});
        assert!(score_task(&task, &json!([])).is_err());
        for bad in [json!(null), json!([]), json!({"format": "x"})] {
            assert!(score_task(&bad, &json!([])).is_err());
        }
        for bad in [
            json!({}),
            json!([1]),
            json!([["a", 3]]),
            json!([{"command": "a", "x": 1}]),
        ] {
            assert!(score_task(&plate_task(), &bad).is_err(), "{bad}");
        }
    }
}
