//! Outputs (`variable.measure`, `variable.output`, `output.list`):
//! read-only variables computed after regeneration, from a measure of the
//! bodies or from an expression of the variables and of other outputs
//! (docs/design/verification.md › *Outputs*; decided by the developer,
//! 2026-10-27).
//!
//! Regeneration stays one way: geometry first, then the measures, then
//! the outputs and the assertions. An output may be used only by other
//! outputs and by assertions; an edit that makes a variable, a feature, a
//! sketch, a public parameter's range, a named set or a body's colour or
//! material use one is refused by name ([`refuse_misuse`]). Among the
//! outputs the uses form a graph that must have no cycle; an output on one
//! is refused when it is defined, and one in a file is unknown with the
//! cycle named.
//!
//! The measures of outputs are taken as assertions are, off the caller's
//! thread and only when what they depend on changed (`assert_check.rs`,
//! which computes both in stages: every output and assertion whose
//! outputs are all known goes in the next job). A measure that fails, or
//! whose proven range is wider than the document's tolerance, makes its
//! output *unknown* with the reason; whatever uses it is then unknown
//! too, and an assertion on it is notProven, never decided on a number
//! that is not one.

use crate::assert_check::Wait;
use crate::assert_cmds::{AssertionStatus, Checked, dim_of, measure_keys, unit_of};
use crate::commands::{Run, edit, fail, length_unit, spec};
use crate::params::Params;
use crate::verify_cmds::{Dim, MEASURES, dim_of_dimension};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Assertion, DocError, Document, Expr, Output, OutputDef};
use craft3d_expr::{Dimension, Evaluation, Quantity};
use craft3d_math::Portable;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("variable.measure", "Measure into Output", r#"{"name": string, "measure": "volume|cavity|area|mass|count|extent|boxMin|boxMax|centerOfMass|distance|clearance|thickness|through|overlap|symmetric|volumeBelow|centroidBelow|sectionArea|sectionCentroid|sectionInertia|sectionTorsion|sectionShearCentre|sectionExtent|levelFor|curvature|developable|bendRadius|flangeLength|holeToBend|relief|cornerGap|flatExtent|flatArea|kawasakiFails|maekawaFails|flatFoldable|floatingOffset|floatingHeel|floatingTrim|floatingGM|floatingGML|floatingStable|bendable|continuity|draft|textExtent|jointAngle|jointDistance|position|rotation", "of": subject, "edge": edge name, "face": face name, "sketch": id or name, "text": text id, "what": "width|height|capHeight|lines|inkWidth|inkHeight", "from": subject, "to": subject, "axis": "x|y|z", "plane": plane, "offset": expr, "xAxis": direction, "about": "x|y|xy|major|minor", "volume": expr, "measured": "tangent|mould", "mass": expr, "density": expr, "centerOfGravity": [expr, expr, expr], "heel": angle expr, "trim": angle expr, "punch": [[expr, expr], ...], "die": [[expr, expr], ...], "length": expr, "clearance": expr, "segmented": boolean, "formed": [flange, ...], "order": [flange or [flange, ...], ...], "maxWork": integer, "faces": subject, "direction": "x|y|z|-x|-y|-z" or [x, y, z], "minDraft": degrees, "at": [x, y, z], "tol": expr, "mate": mate name, "motion": "distance|x|y", "instance": instance name, "point": "Instance/Feature:name", "label": string or null = kept, "unit": unit or null = kept, "wait": boolean = true}"#,
        "Defines an output, a read-only variable computed after regeneration from a measure of the bodies, or replaces how it is computed (one undo step; saved with the document): the measure and its parameters are those of assert.add (volumeBelow, centroidBelow with axis, sectionInertia with about, sectionTorsion, sectionShearCentre with axis, levelFor with volume, floatingGM, floatingTrim with centerOfGravity, bendable, thickness, distance, mass, …), and the output is the number it measures with its unit (a length, an area, a volume, the document's length unit to the fourth power; a mass in kg (a mass, as 2 kg is), a count and a yes-or-no as plain numbers, yes 1). label and unit are as variable.output's. Its expressions (offset, volume, a plane's point) may use variables and other outputs: the waterline T = levelFor of the displacement, then KB = centroidBelow at offset T. Other outputs (variable.output) and assertions (their bounds, and the measure value) use it by name; a variable, a feature, a sketch, a datum, a public parameter, a named set or a material may not (refused by name: regeneration stays one way, geometry, then measures, then outputs and assertions). An output that would depend on itself is refused with the cycle named. Unknown, with the reason, when the measure fails, is not proven (a floating equilibrium that does not settle, a bend sequence neither proven nor refuted) or what it proves is wider than the document's tolerance (a thickness found at 2 mm but proven only above 1.96 mm); what uses an unknown output is unknown, and an assertion on it notProven. Not a variable: variable.set, param.set and variable.publish refuse it; variable.delete and variable.rename take it. Returns the output (status known, unknown or pending, value in the document's units, unit, message); wait (default true) waits until it is computed.", doc: true, names: ["of", "from", "to", "plane"], variable_measure),
    spec!("variable.output", "Output Variable", r#"{"name": string, "expr": expression, "label": string or null = kept, "unit": unit or null = kept, "wait": boolean = true}"#,
        "Defines an output computed from an expression of the variables and of other outputs, or replaces its expression (one undo step; saved with the document): GM = KB + BM - KG of outputs measured with variable.measure; stress = load / area with load = 1000 N and area measured gives a stress (10 MPa on 100 mm²). label is what a form shows instead of the name (\"Bending stress\"); unit the unit it is shown in, of its dimension: a unit (MPa, N, kN, N m, kg, mm^2) or a product and quotient of units (N/mm², kN*m, kg/m^3); without one it is shown in its dimension's: N for a force, MPa for a pressure or a stress, N m for a moment (or an energy), kg for a mass, the document's length unit to its power otherwise. Both are only how it is shown (its value, and what uses it, do not change), and both are kept when not given (null takes one away). A unit that is not of its dimension is refused with both named; one that stops fitting after an edit (its expression changed) makes it unknown with the reason. Read-only and computed after regeneration as variable.measure's outputs are, with their rules: used only by other outputs and by assertions (refused by name anywhere else), no cycle (refused with the cycle named), unknown with the reason when an output it uses is unknown or it does not evaluate. Returns the output; wait (default true) waits until it is computed.", doc: true, variable_output),
    spec!("output.list", "Outputs", r#"{"wait": boolean = true}"#,
        "Lists the document's outputs in the order they were defined, each computed against the document as it is now: name, label, how it is computed (measure or expr), text (its label, or its name, and its message: \"Bending stress 10 MPa\"), status known (value in its unit, or else in the document's units: N for a force, MPa for a stress, N m for a moment, kg for a mass, the length unit to its power; unit; dimension), unknown (message: why: the measure failed or is not proven, an output it uses is unknown, it does not evaluate, it is on a cycle) or pending (still being measured: only with wait false). known, unknown and pending count them.", doc: true, output_list),
];

// ------------------------------------------------------------- values

/// What an output is, as far as it is computed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OutputValue {
    /// Its value, in base units (millimetres and radians), as an
    /// expression's.
    Known(Quantity),
    /// It has none, and why.
    Unknown(String),
    /// Still being measured (or waiting for an output that is).
    Pending,
}

/// The document's outputs as far as they are computed, by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Resolved {
    values: BTreeMap<String, OutputValue>,
}

impl Resolved {
    /// The outputs as far as they are computed, by name.
    pub(crate) fn entries(&self) -> &BTreeMap<String, OutputValue> {
        &self.values
    }

    /// Outputs as [`Resolved::entries`] gives them.
    pub(crate) fn from_entries(values: BTreeMap<String, OutputValue>) -> Self {
        Self { values }
    }

    /// For [`Evaluation::evaluate_with`]: `None` for a name that is no
    /// output, `Some(None)` for one without a value.
    pub fn lookup(&self, name: &str) -> Option<Option<Quantity>> {
        self.values.get(name).map(|v| match v {
            OutputValue::Known(q) => Some(*q),
            _ => None,
        })
    }

    /// Why output `name` has no value, when it is one without.
    pub fn unknown(&self, name: &str) -> Option<String> {
        match self.values.get(name)? {
            OutputValue::Known(_) => None,
            OutputValue::Unknown(why) => Some(why.clone()),
            OutputValue::Pending => Some("it is still being measured".into()),
        }
    }

    pub fn get(&self, name: &str) -> Option<&OutputValue> {
        self.values.get(name)
    }

    fn pending(&self, name: &str) -> bool {
        matches!(self.values.get(name), Some(OutputValue::Pending))
    }

    pub fn any_pending(&self) -> bool {
        self.values.values().any(|v| *v == OutputValue::Pending)
    }

    /// What `names` are, for the key of what uses them: an edit that
    /// changes none of them changes nothing it measures.
    pub fn key_text(&self, names: &BTreeSet<String>) -> String {
        let mut text = String::new();
        for name in names {
            if let Some(v) = self.values.get(name) {
                text.push_str(&format!("{name}:={v:?};"));
            }
        }
        text
    }
}

// ------------------------------------------------------------- uses

/// The names the expressions inside a stored measure use (its offset,
/// volume, a plane's point, a tolerance, the expression of `value`), not
/// the names of bodies, faces and kinds.
pub(crate) fn measure_names(measure: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    if let Ok(value) = serde_json::from_str::<Value>(measure) {
        crate::commands::json_expr_names(&value, &mut names, 0);
    }
    names
}

/// The names `output` uses.
fn output_names(output: &Output) -> BTreeSet<String> {
    match &output.def {
        OutputDef::Expr(e) => craft3d_expr::names_in(e.source())
            .into_iter()
            .map(str::to_string)
            .collect(),
        OutputDef::Measure(text) => measure_names(text),
    }
}

/// The outputs of `doc` that `assertion` uses, in its bounds, its
/// condition or its measure's expressions, by name in order: those it may
/// need, which it waits for (whether it does need one, an evaluation
/// tells: a branch of `if` that is not taken is not needed).
pub(crate) fn assertion_uses(doc: &Document, assertion: &Assertion) -> Vec<String> {
    let mut names = measure_names(&assertion.measure);
    for e in assertion.exprs() {
        names.extend(
            craft3d_expr::names_in(e.source())
                .into_iter()
                .map(str::to_string),
        );
    }
    names
        .into_iter()
        .filter(|n| doc.output(n).is_some())
        .collect()
}

/// The outputs each output uses (by index), and for each one on a cycle,
/// the cycle in words.
pub(crate) struct Plan {
    deps: Vec<Vec<usize>>,
    cycle: Vec<Option<String>>,
}

impl Plan {
    pub fn of(doc: &Document) -> Plan {
        let index: BTreeMap<&str, usize> = doc
            .outputs
            .iter()
            .enumerate()
            .map(|(i, o)| (o.name.as_str(), i))
            .collect();
        let deps: Vec<Vec<usize>> = doc
            .outputs
            .iter()
            .map(|o| {
                output_names(o)
                    .iter()
                    .filter_map(|n| index.get(n.as_str()).copied())
                    .collect()
            })
            .collect();
        // A cycle through output `start`: the shortest way back to it
        // (a breadth-first search, so at most every edge once per output).
        let n = deps.len();
        let cycle = (0..n)
            .map(|start| {
                let mut came_from: BTreeMap<usize, usize> = BTreeMap::new();
                let mut seen: BTreeSet<usize> = BTreeSet::from([start]);
                let mut queue: VecDeque<usize> = VecDeque::from([start]);
                while let Some(at) = queue.pop_front() {
                    for &next in deps.get(at).map(Vec::as_slice).unwrap_or(&[]) {
                        if next == start {
                            // Back from `at` to `start`, then round.
                            let mut path = vec![at];
                            let mut back = at;
                            while back != start && path.len() <= n {
                                let Some(&prev) = came_from.get(&back) else {
                                    break;
                                };
                                path.push(prev);
                                back = prev;
                            }
                            path.reverse();
                            path.push(start);
                            let words: Vec<&str> = path
                                .iter()
                                .filter_map(|&i| doc.outputs.get(i).map(|o| o.name.as_str()))
                                .collect();
                            return Some(words.join(" → "));
                        }
                        if seen.insert(next) {
                            came_from.insert(next, at);
                            queue.push_back(next);
                        }
                    }
                }
                None
            })
            .collect();
        Plan { deps, cycle }
    }

    /// The cycle output `name` of `doc` lies on, in words.
    pub fn cycle_of(&self, doc: &Document, name: &str) -> Option<&str> {
        let i = doc.outputs.iter().position(|o| o.name == name)?;
        self.cycle.get(i)?.as_deref()
    }
}

/// Resolves every output of `doc` it can, in stages: an output whose
/// outputs are all resolved is computed (an expression here; a measure by
/// `measured`, which answers its value, or [`OutputValue::Pending`] when
/// it is being measured), until nothing more can be. `inputs` is the
/// variables' evaluation, made when an expression first needs it.
pub(crate) fn resolve(
    doc: &Document,
    plan: &Plan,
    mut measured: impl FnMut(usize, &Output, &Resolved) -> OutputValue,
) -> Resolved {
    let mut resolved = Resolved {
        values: doc
            .outputs
            .iter()
            .map(|o| (o.name.clone(), OutputValue::Pending))
            .collect(),
    };
    for (i, output) in doc.outputs.iter().enumerate() {
        if let Some(Some(cycle)) = plan.cycle.get(i) {
            resolved.values.insert(
                output.name.clone(),
                OutputValue::Unknown(format!("it depends on itself: {cycle}")),
            );
        }
    }
    let mut inputs: Option<Option<Evaluation>> = None;
    let mut asked = vec![false; doc.outputs.len()];
    // Each pass resolves one output at least, or it is the last.
    for _ in 0..=doc.outputs.len() {
        let mut progress = false;
        for (i, output) in doc.outputs.iter().enumerate() {
            if !resolved.pending(&output.name) || asked.get(i).copied().unwrap_or(true) {
                continue;
            }
            // Every output it may use is computed first, so that what it
            // is never depends on what happened to be measured by then;
            // whether it needs one that is unknown, its evaluation tells
            // (lazily: a branch of `if` that is not taken is not needed).
            let deps = plan.deps.get(i).map(Vec::as_slice).unwrap_or(&[]);
            if deps
                .iter()
                .filter_map(|&d| doc.outputs.get(d))
                .any(|d| resolved.pending(&d.name))
            {
                continue;
            }
            let value = match &output.def {
                OutputDef::Expr(e) => {
                    let evaluation = inputs
                        .get_or_insert_with(|| doc.variable_table().ok().map(|t| t.evaluate()));
                    evaluated(evaluation.as_ref(), e, &resolved)
                }
                OutputDef::Measure(_) => {
                    if let Some(slot) = asked.get_mut(i) {
                        *slot = true;
                    }
                    measured(i, output, &resolved)
                }
            };
            if value != OutputValue::Pending {
                progress = true;
            }
            let value = unit_checked(output, value);
            resolved.values.insert(output.name.clone(), value);
        }
        if !progress {
            break;
        }
    }
    resolved
}

/// `value`, or unknown when the output's unit (`Output::unit`) is not
/// of its dimension: a unit that cannot show it is an error, said where
/// the output is, not a number shown in another unit.
fn unit_checked(output: &Output, value: OutputValue) -> OutputValue {
    let (OutputValue::Known(q), Some(unit)) = (&value, &output.unit) else {
        return value;
    };
    match craft3d_expr::ShownUnit::parse(unit).and_then(|u| u.value_of(*q)) {
        Ok(_) => value,
        Err(e) => OutputValue::Unknown(format!("its unit: {e}")),
    }
}

/// The value of the expression of an output.
fn evaluated(inputs: Option<&Evaluation>, e: &Expr, resolved: &Resolved) -> OutputValue {
    let result = match inputs {
        Some(inputs) => inputs.evaluate_with(e.source(), |n| resolved.lookup(n)),
        None => craft3d_expr::evaluate(e.source(), |n| resolved.lookup(n).flatten()),
    };
    match result {
        Ok(q) if q.value.is_finite() => OutputValue::Known(q),
        Ok(_) => OutputValue::Unknown(format!("`{}` is not finite", e.source())),
        Err(err) => match &err.kind {
            // An output it needed has no value: said as that output's.
            craft3d_expr::ErrorKind::DependencyFailed { name }
                if let Some(why) = resolved.unknown(name) =>
            {
                OutputValue::Unknown(format!("it uses `{name}`, which is unknown: {why}"))
            }
            _ => OutputValue::Unknown(format!("`{}`: {err}", e.source())),
        },
    }
}

/// The measure of an output, as the assertion it is checked as: the
/// measure with no bound (`assert_check` measures it with the assertions,
/// by the same key and memo; such an assertion is never a document's).
pub(crate) fn as_assertion(output: &Output) -> Option<Assertion> {
    match &output.def {
        OutputDef::Measure(text) => {
            // A yes-or-no measure is taken as one (its answer is then a
            // flag); a number with no bound.
            let name = serde_json::from_str::<Map<String, Value>>(text)
                .ok()
                .and_then(|m| m.get("measure").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            let expect = if crate::assert_cmds::is_flag(&name) {
                craft3d_doc::Expect::Is(true)
            } else {
                craft3d_doc::Expect::Within {
                    min: None,
                    max: None,
                }
            };
            Some(Assertion {
                name: output.name.clone(),
                measure: text.clone(),
                expect,
                when: None,
            })
        }
        OutputDef::Expr(_) => None,
    }
}

/// The value of a measured output from what its measure found: known when
/// the range it proves is within the document's tolerance (of the
/// dimension; rounding for the rest), else unknown, with what was found.
pub(crate) fn from_checked(doc: &Document, checked: &Checked) -> OutputValue {
    match checked.state.status {
        AssertionStatus::Pending => return OutputValue::Pending,
        AssertionStatus::Error => {
            return OutputValue::Unknown(checked.state.message.clone());
        }
        // An output its measure needed is unknown (the measure's own
        // expressions are lazy as an output's are).
        AssertionStatus::NotProven if checked.value.is_null() => {
            return OutputValue::Unknown(checked.state.message.clone());
        }
        _ => {}
    }
    let measure = checked.state.measure.as_str();
    let dim = dim_of(measure);
    match &checked.value {
        Value::Bool(yes) => {
            OutputValue::Known(Quantity::dimensionless(if *yes { 1.0 } else { 0.0 }))
        }
        Value::Number(n) => {
            let Some(v) = n.as_f64().filter(|v| v.is_finite()) else {
                return OutputValue::Unknown(format!("{measure} measured no finite number"));
            };
            let at = |key: &str| checked.detail.get(key).and_then(Value::as_f64);
            let lower = at("atLeast").unwrap_or(v);
            let upper = at("atMost").unwrap_or(v);
            let tol = match dim {
                Dim::Length(k) if k > 0 => doc.tolerance.linear.ppowi(k),
                _ => 0.0,
            };
            if upper - lower > tol.max(1e-9 * v.abs()) {
                return OutputValue::Unknown(format!(
                    "{measure} is not proven closely enough to be a value: found {}",
                    checked.state.measured.clone().unwrap_or_default()
                ));
            }
            // A mass is one (in kg), as `2 kg` is.
            let dimension = crate::verify_cmds::dimension_of_dim(dim);
            OutputValue::Known(Quantity::new(
                v * crate::verify_cmds::scale_of(doc, dimension),
                dimension,
            ))
        }
        Value::String(note) => OutputValue::Unknown(format!("{measure}: {note}")),
        _ => OutputValue::Unknown(format!("{measure} measured nothing")),
    }
}

// ------------------------------------------------------------- states

/// How an output stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputStatus {
    Known,
    Unknown,
    Pending,
}

impl OutputStatus {
    pub fn name(self) -> &'static str {
        match self {
            OutputStatus::Known => "known",
            OutputStatus::Unknown => "unknown",
            OutputStatus::Pending => "pending",
        }
    }
}

/// An output as computed against the document, for a viewer.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputState {
    pub name: String,
    /// The expression, or the measure's name.
    pub definition: String,
    /// Whether it is measured (else an expression).
    pub measured: bool,
    pub status: OutputStatus,
    /// What a form shows instead of the name, when it has one.
    pub label: Option<String>,
    /// In its unit when it has one, else in the document's units (its
    /// length unit to the power of its dimension, radians; N, MPa, N m, kg
    /// for a force, a stress, a moment, a mass); none unless known.
    pub value: Option<f64>,
    /// "MPa", "mm^3", "mm", "rad"; none for a plain number.
    pub unit: Option<String>,
    /// The value with its unit, or why there is none.
    pub message: String,
    pub(crate) dimension: Option<Dimension>,
    pub(crate) stored: String,
}

pub(crate) fn states(doc: &Document, resolved: &Resolved) -> Vec<OutputState> {
    let length = length_unit(doc);
    doc.outputs
        .iter()
        .map(|o| {
            let (definition, measured, stored) = match &o.def {
                OutputDef::Expr(e) => (e.source().to_string(), false, e.source().to_string()),
                OutputDef::Measure(text) => {
                    let name = serde_json::from_str::<Map<String, Value>>(text)
                        .ok()
                        .and_then(|m| m.get("measure").and_then(Value::as_str).map(str::to_string))
                        .unwrap_or_default();
                    (name, true, text.clone())
                }
            };
            let mut state = OutputState {
                name: o.name.clone(),
                definition,
                measured,
                status: OutputStatus::Pending,
                label: o.label.clone(),
                value: None,
                unit: None,
                message: "being measured".into(),
                dimension: None,
                stored,
            };
            match resolved.get(&o.name) {
                Some(OutputValue::Known(q)) => {
                    // In its own unit (checked when it was resolved), else
                    // in the document's units.
                    let own = o
                        .unit
                        .as_deref()
                        .and_then(|u| craft3d_expr::ShownUnit::parse(u).ok())
                        .and_then(|u| u.value_of(*q).ok().map(|v| (v, u.text)));
                    let (value, unit) = match own {
                        Some((v, text)) => (v, Some(text)),
                        None => (
                            q.value / crate::verify_cmds::scale_of(doc, q.dimension),
                            unit_of(dim_of_dimension(q.dimension), length),
                        ),
                    };
                    state.status = OutputStatus::Known;
                    state.value = Some(value);
                    state.message = match &unit {
                        Some(u) => format!("{} {u}", crate::assert_cmds::shown(value)),
                        None => crate::assert_cmds::shown(value),
                    };
                    state.unit = unit;
                    state.dimension = Some(q.dimension);
                }
                Some(OutputValue::Unknown(why)) => {
                    state.status = OutputStatus::Unknown;
                    state.message.clone_from(why);
                }
                Some(OutputValue::Pending) | None => {}
            }
            state
        })
        .collect()
}

pub(crate) fn state_json(state: &OutputState) -> Value {
    let mut out = json!({
        "name": state.name,
        "status": state.status.name(),
        "label": state.label,
        "value": state.value,
        "unit": state.unit,
        "message": state.message,
        "text": format!("{} {}", state.label.as_deref().unwrap_or(&state.name), state.message),
    });
    if let Some(map) = out.as_object_mut() {
        if state.measured {
            let measure: Value = serde_json::from_str(&state.stored).unwrap_or(Value::Null);
            map.insert("measure".into(), measure);
        } else {
            map.insert("expr".into(), state.stored.clone().into());
        }
        if let Some(d) = state.dimension {
            map.insert("dimension".into(), d.to_string().into());
        }
    }
    out
}

impl Session {
    /// The outputs of the active document and how each stands, without
    /// waiting (those being measured are pending); empty without a
    /// document or outputs.
    pub fn output_states(&mut self) -> Vec<OutputState> {
        self.outputs(Wait::No).unwrap_or_default()
    }

    pub(crate) fn outputs(&mut self, wait: Wait) -> Result<Vec<OutputState>, CommandError> {
        let (_, resolved) = self.checks_and_outputs(wait)?;
        Ok(states(self.document()?.history.current(), &resolved))
    }
}

// ------------------------------------------------------------- rules

/// For every output `doc` has, what uses it where only other outputs and
/// assertions may: one phrase each.
pub(crate) fn misuse(doc: &Document) -> Vec<String> {
    let mut found = Vec::new();
    for output in &doc.outputs {
        for (may, user) in crate::commands::variable_users_kinds(doc, &output.name) {
            if !may {
                found.push(format!("{user} uses `{}`", output.name));
            }
        }
    }
    found
}

/// Refuses an edit that made something other than an output or an
/// assertion use an output (what used one before the edit, in a file
/// written so, is left for the edit that mends it).
pub(crate) fn refuse_misuse(before: &[String], doc: &Document) -> Result<(), DocError> {
    if doc.outputs.is_empty() {
        return Ok(());
    }
    let new: Vec<String> = misuse(doc)
        .into_iter()
        .filter(|m| !before.contains(m))
        .collect();
    if new.is_empty() {
        return Ok(());
    }
    Err(DocError::Invalid(format!(
        "{}, an output: an output is computed after regeneration and may be used only by other outputs and by assertions, so that regeneration stays one way (geometry, then measures, then outputs and assertions); give it a variable instead",
        new.join(", ")
    )))
}

// ------------------------------------------------------------- commands

fn wait_of(p: &Params) -> Result<Wait, CommandError> {
    Ok(if p.bool("wait")?.unwrap_or(true) {
        Wait::All
    } else {
        Wait::No
    })
}

/// Refuses a name that is not an output's to take: a variable's (an
/// input), a table's, or a word the expressions use.
fn check_name(doc: &Document, id: &str, name: &str) -> Result<(), CommandError> {
    craft3d_expr::check_variable_name(name)
        .map_err(|e| CommandError::new(format!("{id}: `name`: {e}")))?;
    if doc.variable(name).is_some() {
        return fail(format!(
            "{id}: `{name}` is a variable (an input); an output is read-only and needs a name of its own (variable.delete the variable first to make it an output)"
        ));
    }
    Ok(())
}

/// Sets `output`, refusing it when it would depend on itself.
fn set(session: &mut Session, id: &'static str, output: Output, wait: Wait) -> Run {
    let name = output.name.clone();
    let mut next = session.document()?.history.current().as_ref().clone();
    next.set_output(output.clone())
        .map_err(|e| CommandError::new(format!("{id}: {e}")))?;
    if let Some(cycle) = Plan::of(&next).cycle_of(&next, &name) {
        return fail(format!(
            "{id}: `{name}` would depend on itself ({cycle}): the outputs may use each other only without a cycle"
        ));
    }
    edit(session, id, &format!("Set output {name}"), |doc| {
        doc.set_output(output)
    })?;
    one(session, &name, wait)
}

fn one(session: &mut Session, name: &str, wait: Wait) -> Run {
    let states = session.outputs(wait)?;
    states
        .iter()
        .find(|s| s.name == name)
        .map(state_json)
        .ok_or_else(|| CommandError::new(format!("there is no output named `{name}`")))
}

fn variable_measure(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "variable.measure";
    let mut keys: Vec<&str> = vec!["name", "measure", "label", "unit", "wait"];
    for m in MEASURES {
        for k in measure_keys(m) {
            if !keys.contains(k) {
                keys.push(k);
            }
        }
    }
    let p = Params::new(ID, params, &keys)?;
    let wait = wait_of(&p)?;
    let name = p.required_str("name")?.to_string();
    let measure = p.required_str("measure")?;
    if measure == "value" {
        return fail(format!(
            "{ID}: an output computed from an expression is defined with variable.output {{\"name\", \"expr\"}}"
        ));
    }
    let map: Map<String, Value> = match params {
        Value::Object(map) => map
            .iter()
            .filter(|(k, _)| k.as_str() != "wait")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        _ => Map::new(),
    };
    if MEASURES.contains(&measure) {
        let mut allowed: Vec<&str> = vec!["name", "measure", "label", "unit"];
        allowed.extend_from_slice(measure_keys(measure));
        crate::verify_cmds::only(&map, &allowed)
            .map_err(|e| CommandError::new(format!("{ID}: {measure}: {e}")))?;
    }
    let map: Map<String, Value> = map
        .into_iter()
        .filter(|(k, _)| k != "label" && k != "unit")
        .collect();
    let doc = session.document()?.history.current();
    check_name(doc, ID, &name)?;
    let text = crate::assert_cmds::stored_measure(doc, ID, &map)?;
    let output = shown_as(
        session,
        &p,
        Output::new(name, OutputDef::Measure(text)),
        wait,
    )?;
    set(session, ID, output, wait)
}

fn variable_output(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "variable.output";
    let p = Params::new(ID, params, &["name", "expr", "label", "unit", "wait"])?;
    let wait = wait_of(&p)?;
    let name = p.required_str("name")?.to_string();
    let expr = p.expr("expr")?.ok_or_else(|| p.missing("expr"))?;
    check_name(session.document()?.history.current(), ID, &name)?;
    let output = shown_as(
        session,
        &p,
        Output::new(name, OutputDef::Expr(Expr::new(expr))),
        wait,
    )?;
    set(session, ID, output, wait)
}

/// `output` with the label and the unit `p` gives; one not given is kept
/// from the output it replaces, null takes it away. A unit that does not
/// read, or (as far as it is known now) is not of the output's dimension,
/// is refused, with both dimensions named.
fn shown_as(
    session: &mut Session,
    p: &Params,
    mut output: Output,
    wait: Wait,
) -> Result<Output, CommandError> {
    let id = p.command();
    let before = session
        .document()?
        .history
        .current()
        .output(&output.name)
        .cloned();
    let kept = |key: &str, old: Option<String>| -> Result<Option<String>, CommandError> {
        if !p.has(key) {
            return Ok(old);
        }
        Ok(p.str(key)?.map(str::to_string))
    };
    output.label = kept("label", before.as_ref().and_then(|o| o.label.clone()))?
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());
    output.unit = kept("unit", before.and_then(|o| o.unit))?;
    let Some(text) = &output.unit else {
        return Ok(output);
    };
    let unit = craft3d_expr::ShownUnit::parse(text)
        .map_err(|e| CommandError::new(format!("{id}: `unit`: {e}")))?;
    // Its dimension: a measure's is known by its name; an expression's
    // by evaluating it against the variables and the outputs as far as
    // they are computed (one that needs an output still unknown is
    // checked when it is computed: it is unknown then, with the reason).
    let dimension = match &output.def {
        OutputDef::Measure(stored) => {
            let name = serde_json::from_str::<Map<String, Value>>(stored)
                .ok()
                .and_then(|m| m.get("measure").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            Some(crate::verify_cmds::dimension_of_dim(dim_of(&name)))
        }
        OutputDef::Expr(e) => {
            let (_, resolved) = session.checks_and_outputs(wait)?;
            let doc = session.document()?.history.current();
            let inputs = doc.variable_table().ok().map(|t| t.evaluate());
            match evaluated(inputs.as_ref(), e, &resolved) {
                OutputValue::Known(q) => Some(q.dimension),
                _ => None,
            }
        }
    };
    if let Some(dimension) = dimension
        && dimension != unit.dimension
    {
        return Err(CommandError::new(format!(
            "{id}: `unit`: `{}` is {}, but `{}` is {}; give a unit of its dimension{}",
            unit.text,
            unit.dimension.described(),
            output.name,
            dimension.described(),
            match dimension
                .default_unit(
                    session
                        .document()?
                        .history
                        .current()
                        .units
                        .length
                        .as_expr_unit()
                )
                .text
                .as_str()
            {
                "" => String::new(),
                default => format!(" (such as `{default}`)"),
            }
        )));
    }
    Ok(output)
}

fn output_list(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("output.list", params, &["wait"])?;
    let states = session.outputs(wait_of(&p)?)?;
    let count = |s: OutputStatus| states.iter().filter(|o| o.status == s).count();
    Ok(json!({
        "outputs": states.iter().map(state_json).collect::<Vec<_>>(),
        "count": states.len(),
        "known": count(OutputStatus::Known),
        "unknown": count(OutputStatus::Unknown),
        "pending": count(OutputStatus::Pending),
    }))
}

/// The users of output `name` (other outputs and assertions), for
/// `variable.delete`.
pub(crate) fn users(doc: &Document, name: &str) -> Vec<String> {
    crate::commands::variable_users_kinds(doc, name)
        .into_iter()
        .map(|(_, user)| user)
        .collect()
}
