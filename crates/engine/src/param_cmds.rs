//! Public parameters and document info: what makes a document a Jig
//! (docs/decisions.md, 2026-10-23; docs/design/document.md › *Public
//! parameters*). `variable.publish` and `variable.unpublish` mark a
//! variable as a parameter a form (or an agent) sets without knowing the
//! model; `param.set` sets several in one edit and one regeneration and
//! refuses a value outside its range; `param.list` is what a form and an
//! agent read; `document.info` keeps what a gallery shows.
//!
//! Ranges and steps are expressions, evaluated against the variables as
//! they are (or as a `param.set` would leave them), in the expression
//! language's base units (millimetres and radians) and shown in the
//! parameter's unit.

use crate::commands::{Run, add_failed, doc_error, edit, edit_merging, fail, failing, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{AngleUnit, DocInfo, Document, Expr, ParamKind, PublicParam};
use craft3d_expr::{Dimension, Evaluation, Quantity, Unit};
use serde_json::{Map, Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("variable.publish", "Publish Variable", r#"{"name": string, "label": string, "description": string, "group": string, "order": integer, "kind": "number|integer|boolean|choice" = number, "choices": [string, ...], "choicesFrom": table name or null, "min": expr or null, "max": expr or null, "step": expr or null, "unit": "um|mm|cm|m|in|ft|deg|rad" or null, "default": expr = its value now}"#,
        "Makes a variable a public parameter: one a parameter form, or an agent, sets without knowing the model (a document with one is a Jig). Its expression must be a value, not a formula (40 mm, 20, 1 in / 16). kind: number (any value of its dimension), integer (a plain whole number), boolean (0 or 1), choice (a whole number from 1 to the number of choices, which are the labels shown, the first for 1; choicesFrom takes them from a table's key column instead, row 1 for 1, and follows the table: table.set). min, max and step are expressions and may use other variables (a bore less than the root diameter), not the variable itself. unit is the unit a form shows it in (the document's for a length or angle when not given). default is what a reset goes back to (its value now when not given). On a variable that is public already, only what is given changes, and null takes min, max, step or unit away. Refused, with the reason, when the value or the default is not of its kind or outside its range, or a range does not evaluate, and when it would make another parameter invalid (a default is checked with every parameter at its default). One undo step. Returns the parameter as param.list does.", doc: true, variable_publish),
    spec!("variable.unpublish", "Unpublish Variable", r#"{"name": string}"#,
        "Makes a public parameter an ordinary variable again; its value stays. One undo step; nothing changes when it was not public.", doc: true, variable_unpublish),
    spec!("param.set", "Set Parameters", r#"{"values": {name: value, ...} = {}, "reset": boolean = false, "merge": boolean = false, "clamp": boolean = false}"#,
        "Sets public parameters (param.list), all in one edit and one regeneration. A value is a string of a value with its unit (\"6 mm\", \"20\", \"400 * (1 mm)^2\" for an area; a formula of variables is refused), a number, taken in the parameter's unit (param.list unit; not for a dimension without one, such as an area), true or false (or \"true\", \"false\") for a boolean, or a choice's label (a JSON number is always the choice's number; a label that is also a number, as a table keyed by numbers gives, is refused when it names another row than its number). A whole-number kind (integer, boolean, choice) is stored as the exact whole number. reset first sets every public parameter back to its default, then applies values. A value that is not of its parameter's kind (whole number, 0 or 1, a choice from 1 to the number of choices), has another dimension, or lies outside its range is refused, with the range: nothing is changed, so an agent can correct it (a form clamps before it sends). So is a change that would put another parameter outside a range that depends on it: set both in one param.set, or give clamp: true, which moves such a parameter to the nearest value of its new range (on its step) in the same edit and lists it under clamped ({name, expr}), as a parameter form does. A range or default that does not work (a variable it uses was deleted) does not refuse a value; it is listed under warnings. merge folds this change into the last undo step when that was a param.set of the same parameters (a slider being dragged: one undo step for the whole drag). Returns the parameters set with their values, failed (the features that fail now and did not before) and the assertions' summary, as variable.set does.", doc: true, param_set),
    spec!("param.list", "List Parameters", "{}",
        "Lists the public parameters of the active document as a form shows them: in order (by order, then the variables' order), each with name, label, description, group, kind, choices (and choicesFrom, the table they come from), its expression and value in its unit, default (with atDefault), min, max and step (each its expression and its value now, or error), valid and the problems that make it invalid (a value out of range or not of its kind, a range that does not evaluate). groups lists the groups in the order they first appear; jig says whether there is any parameter; info is document.info.", doc: true, param_list),
    spec!("document.info", "Document Info", r#"{"title": string, "description": string, "author": string, "licence": string, "url": string, "tags": [string, ...]}"#,
        "Sets what a gallery shows of the document: title, description, author, licence (an SPDX identifier, such as CC-BY-4.0 or MIT), url and tags; only what is given changes, and an empty string, an empty list or null clears a field. Saved with the document; one undo step. Without params it changes nothing and returns the info.", doc: true, document_info),
];

/// Most names in an undo step's label.
const LABEL_NAMES: usize = 3;

// ---------------------------------------------------------------- reading

/// The values of the document's variables in base units, or none when the
/// table does not build (a file edited by hand).
fn evaluation(doc: &Document) -> Option<Evaluation> {
    doc.variable_table().ok().map(|table| table.evaluate())
}

/// What `source` evaluates to against `values`.
fn eval(source: &str, values: Option<&Evaluation>) -> Result<Quantity, String> {
    let parsed = craft3d_expr::Expr::parse(source).map_err(|e| e.to_string())?;
    parsed
        .eval(|name| values.and_then(|v| v.value(name)))
        .map_err(|e| e.to_string())
}

/// Whether `source` is a value: an expression without variables.
/// One that does not parse is not called a formula: evaluating it says
/// what is wrong.
pub(crate) fn is_value(source: &str) -> bool {
    craft3d_expr::Expr::parse(source)
        .map_or(true, |e| e.variables().is_empty() && e.tables().is_empty())
}

/// The unit a parameter of `dimension` is shown in: its own when it has
/// one of that dimension, else the document's for a length or an angle,
/// else none (a plain number, or another dimension, shown in the
/// document's units).
fn shown_unit(doc: &Document, public: &PublicParam, dimension: Dimension) -> Option<Unit> {
    if let Some(unit) = public.unit.as_deref().and_then(Unit::from_symbol)
        && unit.dimension() == dimension
    {
        return Some(unit);
    }
    if dimension == Dimension::LENGTH {
        Some(doc.units.length.as_expr_unit())
    } else if dimension == Dimension::ANGLE {
        Some(match doc.units.angle {
            AngleUnit::Degree => Unit::Degree,
            AngleUnit::Radian => Unit::Radian,
        })
    } else {
        None
    }
}

/// `quantity` as a number of `unit`; without one, in the document's
/// length unit (to its power) and radians.
fn shown(doc: &Document, quantity: Quantity, unit: Option<Unit>) -> f64 {
    if let Some(unit) = unit
        && let Ok(value) = quantity.to_unit(unit)
    {
        return value;
    }
    let factor = doc.units.length.as_expr_unit().factor();
    let scale = factor.powi(quantity.dimension.length);
    if scale.is_finite() && scale > 0.0 {
        quantity.value / scale
    } else {
        quantity.value
    }
}

/// The units a value of `dimension` is shown in when it has no unit of
/// its own: `mm^2`, `mm^-1`, `mm*rad`.
fn unit_text(doc: &Document, dimension: Dimension) -> String {
    let mut parts = Vec::new();
    for (symbol, exponent) in [
        (doc.units.length.symbol(), dimension.length),
        ("rad", dimension.angle),
    ] {
        match exponent {
            0 => {}
            1 => parts.push(symbol.to_string()),
            n => parts.push(format!("{symbol}^{n}")),
        }
    }
    parts.join("*")
}

/// A value written for messages: `6 mm`, `20`.
fn written(doc: &Document, quantity: Quantity, unit: Option<Unit>) -> String {
    let number = shown(doc, quantity, unit);
    match unit {
        Some(unit) => format!("{number} {unit}"),
        None if quantity.dimension.is_dimensionless() => number.to_string(),
        None => format!("{number} {}", unit_text(doc, quantity.dimension)),
    }
}

/// Whether `a` is below `b`, beyond rounding (`1 in` is `25.4 mm`).
fn below(a: f64, b: f64) -> bool {
    a < b - 1e-12 * a.abs().max(b.abs())
}

fn whole(value: f64) -> bool {
    (value - value.round()).abs() <= 1e-9 * value.abs().max(1.0)
}

/// The range and step of a parameter, each its expression and what it
/// evaluates to.
type Bounds<'a> = [(&'static str, Option<(&'a Expr, Result<Quantity, String>)>); 3];

/// One public parameter, evaluated.
struct Evaluated<'a> {
    name: &'a str,
    expr: &'a str,
    public: &'a PublicParam,
    value: Result<Quantity, String>,
    default: Result<Quantity, String>,
    /// Against the variables as they are.
    bounds: Bounds<'a>,
    unit: Option<Unit>,
    /// Why the parameter is not one a form can show as valid; empty when
    /// it is. The problems of its value come first.
    problems: Vec<String>,
    /// How many of `problems` are about its value (not of its kind, out of
    /// its range, a formula, not evaluating), which `param.set` refuses;
    /// the others are about its description (a range or a default that
    /// does not work), which a value cannot mend.
    value_problems: usize,
}

impl Evaluated<'_> {
    fn value_problems(&self) -> &[String] {
        self.problems.get(..self.value_problems).unwrap_or(&[])
    }

    fn description_problems(&self) -> &[String] {
        self.problems.get(self.value_problems..).unwrap_or(&[])
    }
}

fn bound(bounds: &Bounds, what: &str) -> Option<Quantity> {
    bounds
        .iter()
        .find(|(w, _)| *w == what)
        .and_then(|(_, b)| b.as_ref())
        .and_then(|(_, r)| r.as_ref().ok())
        .copied()
}

/// `from 2 mm to 6 mm` and the like, for messages.
fn range_text(doc: &Document, bounds: &Bounds, unit: Option<Unit>) -> String {
    let at = |what: &str| {
        bounds
            .iter()
            .find(|(w, _)| *w == what)
            .and_then(|(_, b)| b.as_ref())
            .map(|(expr, result)| match result {
                Ok(q) => {
                    let value = written(doc, *q, unit);
                    if is_value(expr.source()) {
                        value
                    } else {
                        format!("{value} (`{}`)", expr.source())
                    }
                }
                Err(_) => format!("`{}`", expr.source()),
            })
    };
    match (at("min"), at("max")) {
        (Some(min), Some(max)) => format!("from {min} to {max}"),
        (Some(min), None) => format!("at least {min}"),
        (None, Some(max)) => format!("at most {max}"),
        (None, None) => "any value".to_string(),
    }
}

/// What is wrong with `value` (`what`: "its value" or "its default") of
/// a parameter described by `public`, shown in `unit`, for its kind and
/// within `bounds`.
fn value_problems(
    doc: &Document,
    public: &PublicParam,
    unit: Option<Unit>,
    bounds: &Bounds,
    what: &str,
    value: Quantity,
    problems: &mut Vec<String>,
) {
    let plain = value.dimension.is_dimensionless();
    match public.kind {
        ParamKind::Number => {}
        ParamKind::Integer => {
            if !(plain && whole(value.value)) {
                problems.push(format!(
                    "{what} {} is not a whole number",
                    written(doc, value, unit)
                ));
            }
        }
        ParamKind::Boolean => {
            if !(plain && (value.value == 0.0 || value.value == 1.0)) {
                problems.push(format!(
                    "{what} {} is not 0 (off) or 1 (on)",
                    written(doc, value, unit)
                ));
            }
        }
        ParamKind::Choice => {
            let n = doc.choices_of(public).len();
            if !(plain && whole(value.value) && value.value >= 1.0 && value.value <= n as f64) {
                problems.push(format!(
                    "{what} {} is not a choice: a whole number from 1 to {n}",
                    written(doc, value, unit)
                ));
            }
        }
    }
    let low = bound(bounds, "min").filter(|min| min.dimension == value.dimension);
    let high = bound(bounds, "max").filter(|max| max.dimension == value.dimension);
    if low.is_some_and(|min| below(value.value, min.value))
        || high.is_some_and(|max| below(max.value, value.value))
    {
        problems.push(format!(
            "{what} {} is outside its range, {}",
            written(doc, value, unit),
            range_text(doc, bounds, unit)
        ));
    }
}

/// The variables of `doc` with every public parameter at its default:
/// what a reset gives, against which the defaults are checked (a range
/// may depend on other parameters).
fn default_evaluation(doc: &Document) -> Option<Evaluation> {
    let mut table = doc.variable_table().ok()?;
    for variable in &doc.variables {
        if let Some(public) = &variable.public {
            table.set(&variable.name, public.default.source()).ok()?;
        }
    }
    Some(table.evaluate())
}

/// Every public parameter of `doc`, evaluated against `values` (the
/// evaluation of `doc`'s variables), in the variables' order.
fn evaluate_all<'a>(doc: &'a Document, values: Option<&Evaluation>) -> Vec<Evaluated<'a>> {
    let mut out = Vec::new();
    let mut defaults: Option<Option<Evaluation>> = None;
    for variable in &doc.variables {
        let Some(public) = &variable.public else {
            continue;
        };
        let defaults = defaults
            .get_or_insert_with(|| default_evaluation(doc))
            .as_ref();
        let name = variable.name.as_str();
        let expr = variable.expr.source();
        let value = match values.and_then(|v| v.get(name)) {
            Some(Ok(q)) => Ok(*q),
            Some(Err(e)) => Err(e.to_string()),
            None => eval(expr, values),
        };
        let default = eval(public.default.source(), None);
        let bounds: Bounds = public
            .bounds()
            .map(|(what, expr)| (what, expr.map(|e| (e, eval(e.source(), values)))));
        let default_bounds: Bounds = public
            .bounds()
            .map(|(what, expr)| (what, expr.map(|e| (e, eval(e.source(), defaults)))));
        let dimension = value
            .as_ref()
            .or(default.as_ref())
            .map_or(Dimension::NONE, |q| q.dimension);
        let unit = shown_unit(doc, public, dimension);
        let mut about_value = Vec::new();
        if !is_value(expr) {
            about_value.push(format!(
                "its expression `{expr}` is a formula, not a value a form can set"
            ));
        }
        match &value {
            Err(error) => about_value.push(format!("its value does not evaluate: {error}")),
            Ok(q) => value_problems(
                doc,
                public,
                unit,
                &bounds,
                "its value",
                *q,
                &mut about_value,
            ),
        }
        let mut problems = Vec::new();
        if !is_value(public.default.source()) {
            problems.push(format!(
                "its default `{}` is a formula, not a value",
                public.default.source()
            ));
        }
        if let Some(own) = public.unit.as_deref().and_then(Unit::from_symbol)
            && own.dimension() != dimension
        {
            problems.push(format!(
                "its unit {own} is not one of {}",
                dimension.described()
            ));
        }
        for (what, bound) in &bounds {
            let Some((source, result)) = bound else {
                continue;
            };
            let uses_itself = craft3d_expr::Expr::parse(source.source())
                .is_ok_and(|parsed| parsed.variables().contains(&name));
            if uses_itself {
                problems.push(format!(
                    "its {what} `{}` uses `{name}` itself",
                    source.source()
                ));
                continue;
            }
            match result {
                Err(error) => problems.push(format!(
                    "its {what} `{}` does not evaluate: {error}",
                    source.source()
                )),
                Ok(q) if q.dimension != dimension => problems.push(format!(
                    "its {what} `{}` is {}, and its value {}",
                    source.source(),
                    q.dimension.described(),
                    dimension.described()
                )),
                Ok(q) if *what == "step" && q.value <= 0.0 => {
                    problems.push(format!("its step `{}` is not positive", source.source()))
                }
                Ok(_) => {}
            }
        }
        if let (Some(min), Some(max)) = (bound(&bounds, "min"), bound(&bounds, "max"))
            && min.dimension == max.dimension
            && below(max.value, min.value)
        {
            problems.push(format!(
                "its range is empty: {}",
                range_text(doc, &bounds, unit)
            ));
        }
        match &default {
            Err(error) => problems.push(format!("its default does not evaluate: {error}")),
            Ok(q) if value.as_ref().is_ok_and(|v| v.dimension != q.dimension) => {
                problems.push(format!(
                    "its default `{}` is {}, and its value {}",
                    public.default.source(),
                    q.dimension.described(),
                    dimension.described()
                ));
            }
            Ok(q) => value_problems(
                doc,
                public,
                unit,
                &default_bounds,
                "its default",
                *q,
                &mut problems,
            ),
        }
        let value_count = about_value.len();
        about_value.extend(problems);
        out.push(Evaluated {
            name,
            expr,
            public,
            value,
            default,
            bounds,
            unit,
            problems: about_value,
            value_problems: value_count,
        });
    }
    out
}

/// Every problem of the public parameters of `doc`, as (name, problem):
/// what `document.check` lists.
pub(crate) fn parameter_problems(doc: &Document) -> Vec<(String, String)> {
    let values = evaluation(doc);
    evaluate_all(doc, values.as_ref())
        .into_iter()
        .flat_map(|e| {
            let name = e.name.to_string();
            e.problems.into_iter().map(move |p| (name.clone(), p))
        })
        .collect()
}

/// A quantity as `{expr, value}` or `{expr, error}`.
fn quantity_json(
    doc: &Document,
    source: &str,
    result: &Result<Quantity, String>,
    unit: Option<Unit>,
) -> Value {
    match result {
        Ok(q) => json!({ "expr": source, "value": shown(doc, *q, unit) }),
        Err(error) => json!({ "expr": source, "error": error }),
    }
}

/// A parameter as `param.list` gives it.
fn param_json(doc: &Document, e: &Evaluated) -> Value {
    let public = e.public;
    let label = if public.label.is_empty() {
        e.name
    } else {
        public.label.as_str()
    };
    let mut out = Map::new();
    out.insert("name".into(), e.name.into());
    out.insert("label".into(), label.into());
    out.insert("description".into(), public.description.clone().into());
    out.insert("group".into(), public.group.clone().into());
    out.insert("order".into(), public.order.into());
    out.insert("kind".into(), public.kind.name().into());
    let choices = doc.choices_of(public);
    if public.kind == ParamKind::Choice {
        out.insert("choices".into(), choices.clone().into());
        if let Some(table) = &public.choices_from {
            out.insert("choicesFrom".into(), table.clone().into());
        }
    }
    out.insert("expr".into(), e.expr.into());
    match &e.value {
        Ok(q) => {
            let value = shown(doc, *q, e.unit);
            out.insert("value".into(), value.into());
            out.insert("dimension".into(), q.dimension.to_string().into());
            if public.kind == ParamKind::Choice
                && let Some(label) = whole(value)
                    .then(|| value.round() as i64)
                    .and_then(|k| usize::try_from(k.checked_sub(1)?).ok())
                    .and_then(|k| choices.get(k))
            {
                out.insert("choice".into(), label.clone().into());
            }
        }
        Err(error) => {
            out.insert("error".into(), error.clone().into());
        }
    }
    out.insert(
        "unit".into(),
        e.unit.map_or(Value::Null, |u| u.symbol().into()),
    );
    // Another dimension (an area) has no unit symbol: what its numbers
    // are in, for reading (param.set takes it as a string with units).
    if let (None, Ok(q)) = (e.unit, &e.value)
        && !q.dimension.is_dimensionless()
    {
        out.insert("unitText".into(), unit_text(doc, q.dimension).into());
    }
    out.insert(
        "default".into(),
        quantity_json(doc, public.default.source(), &e.default, e.unit),
    );
    let at_default = match (&e.value, &e.default) {
        (Ok(v), Ok(d)) => {
            v.dimension == d.dimension && !below(v.value, d.value) && !below(d.value, v.value)
        }
        _ => false,
    };
    out.insert("atDefault".into(), at_default.into());
    for (what, bound) in &e.bounds {
        let value = bound.as_ref().map_or(Value::Null, |(source, result)| {
            quantity_json(doc, source.source(), result, e.unit)
        });
        out.insert((*what).into(), value);
    }
    out.insert("valid".into(), e.problems.is_empty().into());
    out.insert("problems".into(), e.problems.clone().into());
    Value::Object(out)
}

/// The parameters of `doc` in the order a form shows them: by order,
/// then the variables' order.
fn ordered<'a>(doc: &'a Document) -> Vec<Evaluated<'a>> {
    let values = evaluation(doc);
    let mut all: Vec<(usize, Evaluated)> = evaluate_all(doc, values.as_ref())
        .into_iter()
        .enumerate()
        .collect();
    all.sort_by_key(|(index, e)| (e.public.order, *index));
    all.into_iter().map(|(_, e)| e).collect()
}

fn info_json(info: &DocInfo) -> Value {
    json!({
        "title": info.title,
        "description": info.description,
        "author": info.author,
        "licence": info.licence,
        "url": info.url,
        "tags": info.tags,
    })
}

fn param_list(session: &mut Session, params: &Value) -> Run {
    Params::new("param.list", params, &[])?;
    let doc = session.document()?.history.current();
    let parameters = ordered(doc);
    let mut groups: Vec<&str> = Vec::new();
    for e in &parameters {
        if !groups.contains(&e.public.group.as_str()) {
            groups.push(e.public.group.as_str());
        }
    }
    Ok(json!({
        "jig": !parameters.is_empty(),
        "groups": groups,
        "parameters": parameters.iter().map(|e| param_json(doc, e)).collect::<Vec<_>>(),
        "info": info_json(&doc.info),
    }))
}

/// The parameter called `name` as `param.list` gives it.
fn one_param_json(doc: &Document, name: &str) -> Value {
    let values = evaluation(doc);
    evaluate_all(doc, values.as_ref())
        .iter()
        .find(|e| e.name == name)
        .map_or(Value::Null, |e| param_json(doc, e))
}

/// The variables of `doc`, for an error about one that is not there.
fn variable_names(doc: &Document) -> String {
    let names: Vec<&str> = doc
        .variables
        .iter()
        .map(|v| v.name.as_str())
        .take(20)
        .collect();
    if names.is_empty() {
        "the document has no variables; define it with variable.set first".to_string()
    } else {
        format!("variables: {}", names.join(", "))
    }
}

// --------------------------------------------------------------- publish

/// A string parameter that, when given, replaces `field`.
fn text(p: &Params, key: &str, field: &mut String) -> Result<(), CommandError> {
    if p.has(key) {
        *field = p.str(key)?.unwrap_or_default().to_string();
    }
    Ok(())
}

/// An optional expression parameter: given, it replaces `field`; null
/// clears it; absent leaves it.
fn optional_expr(p: &Params, key: &str, field: &mut Option<Expr>) -> Result<(), CommandError> {
    if p.has(key) {
        *field = p.expr(key)?.map(Expr::new);
    }
    Ok(())
}

fn variable_publish(session: &mut Session, params: &Value) -> Run {
    let command = "variable.publish";
    let p = Params::new(
        command,
        params,
        &[
            "name",
            "label",
            "description",
            "group",
            "order",
            "kind",
            "choices",
            "choicesFrom",
            "min",
            "max",
            "step",
            "unit",
            "default",
        ],
    )?;
    let name = p.required_str("name")?;
    let doc = session.document()?.history.current();
    let Some(variable) = doc.variable(name) else {
        return fail(format!(
            "{command}: there is no variable `{name}` to publish; {}",
            variable_names(doc)
        ));
    };
    if !is_value(variable.expr.source()) {
        return fail(format!(
            "{command}: `{name}` is `{}`, a formula; only a variable whose expression is a value (40 mm, 20) can be public. Publish the variables it is worked out from instead, or give it a value with variable.set",
            variable.expr.source()
        ));
    }
    let mut public = variable.public.clone().unwrap_or_else(|| PublicParam {
        default: variable.expr.clone(),
        ..PublicParam::default()
    });
    text(&p, "label", &mut public.label)?;
    text(&p, "description", &mut public.description)?;
    text(&p, "group", &mut public.group)?;
    if p.has("order") {
        public.order = match p.get("order") {
            None => 0,
            Some(value) => value
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or_else(|| {
                    CommandError::new(format!(
                        "{command}: `order` must be a whole number from {} to {}, got {value}",
                        i32::MIN,
                        i32::MAX
                    ))
                })?,
        };
    }
    if let Some(kind) = p.str("kind")? {
        public.kind = ParamKind::from_name(kind).ok_or_else(|| {
            CommandError::new(format!(
                "{command}: unknown kind `{kind}`; use number, integer, boolean or choice"
            ))
        })?;
    }
    if p.has("choices") {
        public.choices = match p.get("choices") {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str().map(str::to_string).ok_or_else(|| {
                        CommandError::new(format!(
                            "{command}: `choices` must be a list of labels (strings)"
                        ))
                    })
                })
                .collect::<Result<_, _>>()?,
            Some(_) => {
                return fail(format!(
                    "{command}: `choices` must be a list of labels (strings)"
                ));
            }
        };
    }
    if p.has("choicesFrom") {
        public.choices_from = p.str("choicesFrom")?.map(str::to_string);
        if public.choices_from.is_some() && !p.has("choices") {
            // The table's keys take the place of the labels.
            public.choices.clear();
        }
    } else if p.has("choices") && !public.choices.is_empty() {
        // Labels given take the place of a table.
        public.choices_from = None;
    }
    if public.kind != ParamKind::Choice && !p.has("choices") {
        // Choices left from when it was a choice go with the kind.
        public.choices.clear();
    }
    if public.kind != ParamKind::Choice && !p.has("choicesFrom") {
        public.choices_from = None;
    }
    optional_expr(&p, "min", &mut public.min)?;
    optional_expr(&p, "max", &mut public.max)?;
    optional_expr(&p, "step", &mut public.step)?;
    if p.has("unit") {
        public.unit = p.str("unit")?.map(str::to_string);
    }
    if let Some(default) = p.expr("default")? {
        public.default = Expr::new(default);
    }
    // Checked on the document as it would be: this parameter, and every
    // other one it would make invalid (its range may use this one, and a
    // default is checked with every parameter at its default).
    let mut next = (**doc).clone();
    next.publish_variable(name, public.clone())
        .map_err(doc_error(command))?;
    let mut refusals: Vec<String> = Vec::new();
    let values = evaluation(&next);
    let after = evaluate_all(&next, values.as_ref());
    if let Some(e) = after.iter().find(|e| e.name == name)
        && !e.problems.is_empty()
    {
        refusals.push(format!(
            "`{name}` cannot be published so: {}",
            e.problems.join("; ")
        ));
    }
    let values_before = evaluation(doc);
    let before = evaluate_all(doc, values_before.as_ref());
    refusals.extend(
        newly_invalid(&before, &after, &[name], |e| &e.problems)
            .into_iter()
            .map(|(other, problems)| format!("this would make `{other}` invalid ({problems})")),
    );
    if !refusals.is_empty() {
        return fail(format!("{command}: {}", refusals.join("; ")));
    }
    edit(session, command, &format!("Publish {name}"), |doc| {
        doc.publish_variable(name, public)
    })?;
    let doc = session.document()?.history.current();
    Ok(one_param_json(doc, name))
}

fn variable_unpublish(session: &mut Session, params: &Value) -> Run {
    let command = "variable.unpublish";
    let p = Params::new(command, params, &["name"])?;
    let name = p.required_str("name")?;
    let doc = session.document()?.history.current();
    let Some(variable) = doc.variable(name) else {
        return fail(format!(
            "{command}: there is no variable `{name}`; {}",
            variable_names(doc)
        ));
    };
    let was = variable.public.is_some();
    edit(session, command, &format!("Unpublish {name}"), |doc| {
        doc.unpublish_variable(name)
    })?;
    Ok(json!({ "name": name, "public": false, "changed": was }))
}

// ------------------------------------------------------------------- set

/// For a parameter of a whole-number kind, a value that is a whole number
/// (within 1e-9, as a count is) written as exactly that number, so the
/// document holds what regeneration counts with; any other value as given
/// (and refused by the check after, when it is not of its kind).
fn whole_source(kind: ParamKind, source: String) -> String {
    if kind == ParamKind::Number {
        return source;
    }
    match eval(&source, None) {
        Ok(q)
            if q.dimension.is_dimensionless()
                && q.value.is_finite()
                && whole(q.value)
                && q.value.abs() < 1e15 =>
        {
            format!("{}", q.value.round() as i64)
        }
        _ => source,
    }
}

/// The parameters of `after` that are invalid by `problems` and were not
/// in `before` (or are not there), other than those in `skip`: each name
/// with its problems, joined.
fn newly_invalid<'a>(
    before: &[Evaluated],
    after: &'a [Evaluated],
    skip: &[&str],
    problems: impl Fn(&'a Evaluated<'a>) -> &'a [String],
) -> Vec<(&'a str, String)> {
    after
        .iter()
        .filter(|e| !skip.contains(&e.name))
        .filter_map(|e| {
            let now = problems(e);
            let was_valid = before
                .iter()
                .find(|b| b.name == e.name)
                .is_none_or(|b| b.problems.is_empty());
            (!now.is_empty() && was_valid).then(|| (e.name, now.join("; ")))
        })
        .collect()
}

/// What an edit from `before` to `after` made invalid among the public
/// parameters that were valid, as warnings: for the designer's commands
/// (`variable.set`, `variable.delete`), which do not refuse it.
pub(crate) fn invalidated(before: &Document, after: &Document) -> Vec<String> {
    if !after.is_jig() {
        return Vec::new();
    }
    let values_before = evaluation(before);
    let values_after = evaluation(after);
    let was = evaluate_all(before, values_before.as_ref());
    let now = evaluate_all(after, values_after.as_ref());
    newly_invalid(&was, &now, &[], |e| &e.problems)
        .into_iter()
        .map(|(name, problems)| format!("the parameter `{name}` is now invalid: {problems}"))
        .collect()
}

/// The expression a `param.set` value stands for, for the parameter `e`.
fn value_source(
    command: &str,
    doc: &Document,
    e: &Evaluated,
    value: &Value,
) -> Result<String, CommandError> {
    let name = e.name;
    let kind = e.public.kind;
    let dimension = e
        .default
        .as_ref()
        .or(e.value.as_ref())
        .map_or(Dimension::NONE, |q| q.dimension);
    match value {
        Value::Bool(on) if kind == ParamKind::Boolean => {
            Ok(if *on { "1" } else { "0" }.to_string())
        }
        Value::String(text)
            if kind == ParamKind::Boolean
                && matches!(text.to_ascii_lowercase().as_str(), "true" | "false") =>
        {
            Ok(if text.eq_ignore_ascii_case("true") {
                "1"
            } else {
                "0"
            }
            .to_string())
        }
        Value::String(label)
            if kind == ParamKind::Choice && doc.choices_of(e.public).iter().any(|c| c == label) =>
        {
            let at = doc
                .choices_of(e.public)
                .iter()
                .position(|c| c == label)
                .unwrap_or_default()
                .saturating_add(1);
            // A label that is also a number (a table keyed by numbers)
            // could mean the row of that number as well: refused unless
            // both are the same row.
            if let Ok(number) = label.trim().parse::<f64>()
                && number != at as f64
            {
                return fail(format!(
                    "{command}: `{name}`: \"{label}\" is both the label of choice {at} and the number {label}; give the label's row number as a JSON number ({at}) or the row {label} as a JSON number"
                ));
            }
            Ok(at.to_string())
        }
        Value::Bool(_) => fail(format!(
            "{command}: `{name}` is a {} parameter, not a boolean; give a number or a string",
            e.public.kind.name()
        )),
        Value::Number(n) => {
            let Some(x) = n.as_f64().filter(|x| x.is_finite()) else {
                return fail(format!("{command}: `{name}` must be a finite number"));
            };
            let unit = shown_unit(doc, e.public, dimension);
            if unit.is_none() && !dimension.is_dimensionless() {
                return fail(format!(
                    "{command}: `{name}` is {}, which a plain number cannot give; write it with its units as a string, such as \"{x} * (1 mm)^2\" for an area",
                    dimension.described()
                ));
            }
            Ok(whole_source(
                kind,
                match unit {
                    Some(unit) => format!("{x} {unit}"),
                    None => x.to_string(),
                },
            ))
        }
        Value::String(source) => {
            let parsed = craft3d_expr::Expr::parse(source).map_err(|error| {
                CommandError::new(format!(
                    "{command}: `{name}`: `{source}` does not read: {error}"
                ))
            })?;
            if let Some(table) = parsed.tables().first() {
                return fail(format!(
                    "{command}: `{name}` takes a value, not a lookup: `{source}` looks a value up in table `{table}`"
                ));
            }
            let uses = parsed.variables();
            if !uses.is_empty() && kind == ParamKind::Choice {
                return fail(format!(
                    "{command}: `{name}` is a choice: give one of its choices ({}) or its number from 1 to {}; `{source}` is none of them",
                    doc.choices_of(e.public).join(", "),
                    doc.choices_of(e.public).len()
                ));
            }
            if !uses.is_empty() && kind == ParamKind::Boolean {
                return fail(format!(
                    "{command}: `{name}` is on or off: give true or false (or 1 or 0), not `{source}`"
                ));
            }
            if !uses.is_empty() {
                return fail(format!(
                    "{command}: `{name}` takes a value, not a formula: `{source}` uses {}",
                    uses.iter()
                        .map(|u| format!("`{u}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok(whole_source(kind, source.clone()))
        }
        _ => fail(format!(
            "{command}: the value of `{name}` must be {}",
            match kind {
                ParamKind::Boolean => "true or false (or 1 or 0)".to_string(),
                ParamKind::Choice => format!(
                    "one of its choices ({}) or its number from 1 to {}",
                    doc.choices_of(e.public).join(", "),
                    doc.choices_of(e.public).len()
                ),
                ParamKind::Integer => "a whole number".to_string(),
                ParamKind::Number =>
                    "a string such as \"6 mm\" or a number in its unit".to_string(),
            }
        )),
    }
}

/// The parameters a `param.set` sets, each with its new expression, and
/// its warnings.
pub(crate) type Prepared = (Vec<(String, String)>, Vec<String>);

/// What `param.set` with `given` (after a `reset` to the defaults) would
/// set on `doc`, checked as `param.set` checks it: each parameter with
/// the expression it would get, in the order of `given` (after the
/// defaults on a reset), and the warnings; or why it is refused (nothing
/// is changed then). `document.sample` checks its points with it.
pub(crate) fn prepare_set(
    command: &str,
    doc: &Document,
    given: &Map<String, Value>,
    reset: bool,
) -> Result<Prepared, CommandError> {
    prepare_set_clamping(command, doc, given, reset, false).map(|(prepared, _)| prepared)
}

/// The parameters `param.set` with `clamp` moved into their new range,
/// each with the value it got (`bore`, `3.1 mm`).
pub(crate) type Clamped = Vec<(String, String)>;

/// [`prepare_set`], and with `clamp` every other parameter the change
/// would put outside its range (one whose range uses a parameter set)
/// moved to the nearest value of its new range instead of refusing: set
/// in the same edit and listed.
pub(crate) fn prepare_set_clamping(
    command: &str,
    doc: &Document,
    given: &Map<String, Value>,
    reset: bool,
    clamp: bool,
) -> Result<(Prepared, Clamped), CommandError> {
    let values_before = evaluation(doc);
    let before = evaluate_all(doc, values_before.as_ref());
    if before.is_empty() {
        return fail(format!(
            "{command}: the document has no public parameters; publish a variable with variable.publish first"
        ));
    }
    // What to set, in the variables' order: every default on a reset,
    // then the values given.
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut put = |name: &str, source: String| match sources.iter_mut().find(|(n, _)| n == name) {
        Some(entry) => entry.1 = source,
        None => sources.push((name.to_string(), source)),
    };
    if reset {
        for e in &before {
            put(e.name, e.public.default.source().to_string());
        }
    }
    for (name, value) in given.iter() {
        let Some(e) = before.iter().find(|e| e.name == name) else {
            let public: Vec<&str> = before.iter().map(|e| e.name).collect();
            return fail(format!(
                "{command}: `{name}` is not a public parameter; the parameters are {}",
                public.join(", ")
            ));
        };
        put(name, value_source(command, doc, e, value)?);
    }
    // Checked on the document as it would be: each value set, and every
    // other parameter whose range moved with them.
    let mut next = doc.clone();
    for (name, source) in &sources {
        next.set_variable(name, source)
            .map_err(doc_error(command))?;
    }
    let mut clamped: Clamped = Vec::new();
    if clamp {
        // A clamped parameter may move the range of another: again until
        // nothing moves, at most once per parameter.
        for _ in 0..=before.len() {
            let moved = clamped_into_range(&before, &sources, &next);
            if moved.is_empty() {
                break;
            }
            for (name, source) in moved {
                next.set_variable(&name, &source)
                    .map_err(doc_error(command))?;
                sources.push((name.clone(), source.clone()));
                clamped.push((name, source));
            }
        }
    }
    let values_after = evaluation(&next);
    let after = evaluate_all(&next, values_after.as_ref());
    let mut refusals = Vec::new();
    let mut warnings = Vec::new();
    for e in &after {
        let set = sources.iter().any(|(n, _)| n == e.name);
        // Only what is wrong with a value refuses it; a range or a default
        // that does not work is the designer's to mend, and said.
        let was_valid = before
            .iter()
            .find(|b| b.name == e.name)
            .is_some_and(|b| b.value_problems().is_empty());
        if set && !e.description_problems().is_empty() {
            warnings.push(format!(
                "`{}`: {}",
                e.name,
                e.description_problems().join("; ")
            ));
        }
        let old_dimension = before
            .iter()
            .find(|b| b.name == e.name)
            .and_then(|b| b.default.as_ref().ok())
            .map(|q| q.dimension);
        if set
            && let (Ok(q), Some(dimension)) = (&e.value, old_dimension)
            && q.dimension != dimension
        {
            refusals.push(format!(
                "`{}` is {}, and `{}` gives {}",
                e.name,
                dimension.described(),
                e.expr,
                q.dimension.described()
            ));
            continue;
        }
        if e.value_problems().is_empty() || (!set && !was_valid) {
            continue;
        }
        let problems = e.value_problems().join("; ");
        if set {
            refusals.push(format!("`{}`: {problems}", e.name));
        } else {
            refusals.push(format!(
                "this would make `{}` invalid ({problems}); set it in the same param.set, or give clamp: true to move it into its new range",
                e.name
            ));
        }
    }
    if !refusals.is_empty() {
        return fail(format!(
            "{command}: nothing was changed: {}",
            refusals.join("; ")
        ));
    }
    Ok(((sources, warnings), clamped))
}

/// The parameters of `next` not among `sources` that were valid in the
/// document before (`before`) and are outside their range now, each with
/// the source of the nearest value in its range: on its step where that
/// stays in the range, a whole number for a whole kind, a short decimal
/// where that stays in the range. A parameter whose trouble is not its
/// range (or whose range is empty) is left for the refusal to name.
fn clamped_into_range(
    before: &[Evaluated],
    sources: &[(String, String)],
    next: &Document,
) -> Vec<(String, String)> {
    let values = evaluation(next);
    let after = evaluate_all(next, values.as_ref());
    let ranges = ranges(next);
    let mut moved = Vec::new();
    for e in &after {
        if sources.iter().any(|(n, _)| n == e.name) || e.value_problems().is_empty() {
            continue;
        }
        let Some(old) = before.iter().find(|b| b.name == e.name) else {
            continue;
        };
        if !old.value_problems().is_empty() {
            continue;
        }
        let (Ok(q), Ok(was)) = (&e.value, &old.value) else {
            continue;
        };
        let Some(range) = ranges.iter().find(|r| r.name == e.name) else {
            continue;
        };
        if q.dimension != was.dimension || q.dimension != range.dimension {
            continue;
        }
        let value = shown(next, *q, e.unit);
        let Some(number) = nearest_in_range(range, value) else {
            continue;
        };
        if (number - value).abs() <= 1e-12 * number.abs().max(value.abs()) {
            continue;
        }
        moved.push((e.name.to_string(), value_text(range, number)));
    }
    moved
}

/// The value of `range` nearest `value` (both in the parameter's unit),
/// or none when the range is empty or does not evaluate.
fn nearest_in_range(range: &Range, value: f64) -> Option<f64> {
    let lo = range.min.unwrap_or(f64::NEG_INFINITY);
    let hi = range.max.unwrap_or(f64::INFINITY);
    if !value.is_finite() || below(hi, lo) {
        return None;
    }
    let inside = |x: f64| x.is_finite() && !below(x, lo) && !below(hi, x);
    if range.kind != ParamKind::Number {
        let number = if value < lo { lo.ceil() } else { hi.floor() };
        return inside(number).then_some(number);
    }
    let mut number = value.clamp(lo, hi);
    if let Some(step) = range.step {
        let base = range.min.unwrap_or(0.0);
        let mut snapped = base + ((number - base) / step).round() * step;
        if below(hi, snapped) {
            snapped -= step;
        }
        if below(snapped, lo) {
            snapped += step;
        }
        if inside(snapped) {
            number = snapped;
        }
    }
    // A short decimal where it stays in the range: 3.1, not
    // 3.1000000000000005.
    let short = (number * 1e6).round() / 1e6;
    if inside(short) {
        number = short;
    }
    inside(number).then_some(number)
}

/// A public parameter's range as `document.sample` samples it: each
/// number in the parameter's unit (as `param.list` shows it), against the
/// variables of the document it was taken from, so a range that uses
/// other parameters is the one at that point.
pub(crate) struct Range {
    pub(crate) name: String,
    pub(crate) kind: ParamKind,
    /// How many choices a choice has.
    pub(crate) choices: usize,
    /// The default as written (what a reset sets).
    pub(crate) default_source: String,
    pub(crate) default: Option<f64>,
    /// None when not given or when it does not evaluate to a number of
    /// the parameter's dimension.
    pub(crate) min: Option<f64>,
    pub(crate) max: Option<f64>,
    /// Only a positive one.
    pub(crate) step: Option<f64>,
    /// The parameter's unit and dimension, for [`value_text`].
    unit: Option<Unit>,
    dimension: Dimension,
    length_symbol: &'static str,
}

/// Every public parameter of `doc` with its range evaluated against
/// `doc`'s variables, in the variables' order.
pub(crate) fn ranges(doc: &Document) -> Vec<Range> {
    let values = evaluation(doc);
    evaluate_all(doc, values.as_ref())
        .iter()
        .map(|e| {
            let dimension = e
                .default
                .as_ref()
                .or(e.value.as_ref())
                .map_or(Dimension::NONE, |q| q.dimension);
            let number = |what: &str| {
                bound(&e.bounds, what)
                    .filter(|q| q.dimension == dimension)
                    .map(|q| shown(doc, q, e.unit))
                    .filter(|x| x.is_finite())
            };
            Range {
                name: e.name.to_string(),
                kind: e.public.kind,
                choices: e.public.choices.len(),
                default_source: e.public.default.source().to_string(),
                default: e
                    .default
                    .as_ref()
                    .ok()
                    .map(|q| shown(doc, *q, e.unit))
                    .filter(|x| x.is_finite()),
                min: number("min"),
                max: number("max"),
                step: number("step").filter(|s| *s > 0.0),
                unit: e.unit,
                dimension,
                length_symbol: doc.units.length.symbol(),
            }
        })
        .collect()
}

/// `number` (in the parameter's unit) written as `param.set` takes it:
/// `12.5 mm`, `20`, `400 * (1 mm)^2`; a whole-number kind as the exact
/// whole number.
pub(crate) fn value_text(range: &Range, number: f64) -> String {
    if range.kind != ParamKind::Number {
        return whole_source(range.kind, number.to_string());
    }
    match range.unit {
        Some(unit) => format!("{number} {unit}"),
        None if range.dimension.is_dimensionless() => number.to_string(),
        None => {
            let mut text = number.to_string();
            for (symbol, exponent) in [
                (range.length_symbol, range.dimension.length),
                ("rad", range.dimension.angle),
            ] {
                if exponent != 0 {
                    text.push_str(&format!(" * (1 {symbol})^{exponent}"));
                }
            }
            text
        }
    }
}

fn param_set(session: &mut Session, params: &Value) -> Run {
    let command = "param.set";
    let p = Params::new(command, params, &["values", "reset", "merge", "clamp"])?;
    let reset = p.bool("reset")?.unwrap_or(false);
    let merge = p.bool("merge")?.unwrap_or(false);
    let clamp = p.bool("clamp")?.unwrap_or(false);
    let given: Map<String, Value> = match p.get("values") {
        None => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => {
            return fail(format!(
                "{command}: `values` must be an object of parameter names and values, such as {{\"teeth\": 20, \"bore\": \"5 mm\"}}"
            ));
        }
    };
    let doc = session.document()?.history.current();
    let ((sources, warnings), clamped) = prepare_set_clamping(command, doc, &given, reset, clamp)?;
    let mut names: Vec<&str> = sources.iter().map(|(n, _)| n.as_str()).collect();
    let mut label = names
        .iter()
        .take(LABEL_NAMES)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > LABEL_NAMES {
        label.push_str(", …");
    }
    let label = if reset {
        "Reset parameters".to_string()
    } else {
        format!("Set {label}")
    };
    names.sort_unstable();
    let key = format!("param.set {}", names.join(","));
    let failed_before = failing(session)?;
    let changed_before = session.document()?.history.revision();
    edit_merging(session, command, &label, Some(&key), merge, |doc| {
        for (name, source) in &sources {
            doc.set_variable(name, source)?;
        }
        Ok(())
    })?;
    let doc = session.document()?.history.current();
    let changed = session.document()?.history.revision() != changed_before;
    let values_now = evaluation(doc);
    let set: Vec<Value> = evaluate_all(doc, values_now.as_ref())
        .iter()
        .filter(|e| sources.iter().any(|(n, _)| n == e.name))
        .map(|e| {
            let mut entry = json!({ "name": e.name, "expr": e.expr });
            if let (Some(map), Ok(q)) = (entry.as_object_mut(), &e.value) {
                map.insert("value".into(), shown(doc, *q, e.unit).into());
                map.insert(
                    "unit".into(),
                    e.unit.map_or(Value::Null, |u| u.symbol().into()),
                );
            }
            entry
        })
        .collect();
    let mut result = json!({ "set": set, "changed": changed });
    if let (false, Some(map)) = (warnings.is_empty(), result.as_object_mut()) {
        map.insert("warnings".into(), warnings.into());
    }
    if let (false, Some(map)) = (clamped.is_empty(), result.as_object_mut()) {
        let clamped: Vec<Value> = clamped
            .iter()
            .map(|(name, expr)| json!({ "name": name, "expr": expr }))
            .collect();
        map.insert("clamped".into(), clamped.into());
    }
    add_failed(session, &failed_before, None, &mut result)?;
    crate::assert_cmds::add_summary(session, &mut result);
    Ok(result)
}

// ------------------------------------------------------------------ info

fn document_info(session: &mut Session, params: &Value) -> Run {
    let command = "document.info";
    let p = Params::new(
        command,
        params,
        &["title", "description", "author", "licence", "url", "tags"],
    )?;
    let mut info = session.document()?.history.current().info.clone();
    text(&p, "title", &mut info.title)?;
    text(&p, "description", &mut info.description)?;
    text(&p, "author", &mut info.author)?;
    text(&p, "licence", &mut info.licence)?;
    text(&p, "url", &mut info.url)?;
    if p.has("tags") {
        info.tags = match p.get("tags") {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str().map(str::to_string).ok_or_else(|| {
                        CommandError::new(format!("{command}: `tags` must be a list of strings"))
                    })
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return fail(format!("{command}: `tags` must be a list of strings")),
        };
    }
    edit(session, command, "Document info", |doc| doc.set_info(info))?;
    Ok(info_json(&session.document()?.history.current().info))
}
