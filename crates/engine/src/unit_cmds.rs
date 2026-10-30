//! `document.setUnit`: changes the length unit of the active document
//! after `file.new` (docs/design/document.md › *Changing the length
//! unit*).
//!
//! A plain number read as a length is in the document's unit, so a unit
//! can change in one of two ways. Keeping the numbers changes only the
//! unit: a block of size 10 becomes 10 of the new unit, while what is
//! written with a unit (`10 mm`) keeps its size. Keeping the sizes leaves
//! the model as large as it was: every plain number that is read as a
//! length gets the old unit written on it (`10` becomes `10 mm`, an
//! expression `w / 2` becomes `(w / 2) * 1 mm`), the numbers the document
//! stores in its unit (the tolerance, the solved points and circles of
//! sketches) are converted, and what the change cannot prove it converts
//! it refuses. It is then proven on the result: the document regenerated
//! in the new unit, with every feature built (no marker, none switched
//! off), has every body the size and place it had (volume, area, centroid
//! and box by the factor's powers) and every feature the status it had, or the
//! change is refused and the document left as it was.

use crate::assert_check::Wait;
use crate::assert_cmds::Checked;
use crate::commands::{Run, edit, fail, spec};
use crate::output_cmds::{OutputValue, Resolved};
use crate::params::Params;
use crate::verify_cmds::Dim;
use crate::{CommandError, CommandSpec, OpenDocument, Session};
use craft3d_doc::sketch::{ConstraintKind, Geometry, PlaceMap};
use craft3d_doc::{Document, Expect, Expr, FeatureKind, LengthUnit, OutputDef, PatternLayout};
use craft3d_expr::{Dimension, Evaluation, Quantity};
use craft3d_math::Vec3;
use serde_json::{Map, Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "document.setUnit",
    "Set Length Unit",
    r#"{"units": "um|mm|cm|m|in|ft", "keep": "numbers|sizes"}"#,
    "Changes the length unit of the active document (one undo step). A plain number read as a length is in the document's unit, so keep says what stays: numbers keeps every number as written and changes only the unit (a block of size 10 is then 10 of the new unit; a value written with a unit, 10 mm, keeps its size, and so does the tolerance's number); sizes keeps the model as large as it was: every plain number read as a length gets the old unit written on it (10 becomes 10 mm, w / 2 becomes (w / 2) * 1 mm; variables are left as they are, their uses get the unit), assertions' bounds likewise, the tolerance and the sketches' points and circles are converted, and the result is proven: regenerated in the new unit, every feature (also those rolled back or switched off) has the status it had and every body the volume, area, centroid and box it had, or the change is refused and the document stays as it was. Formula curves' x(t) and y(t) that are plain numbers get the unit too; so do the lengths in named sets' queries and in assertions' and measured outputs' subjects (volume, area, radius, length, inside, above, below) and the measures' own lengths (offset, a symmetry's tol, a level's volume, a plane's point, a centre of gravity, a press brake's tools with their length and clearance; a mass, a density, a heel or trim and maxWork are not lengths and stay); a value assertion's bounds get the unit to the power of its expression's dimension. Every assertion must then have the status it had and every output its value, or the change is refused. sizes refuses what it cannot convert for certain, naming it: a plain number whose dimension is not known (a parameter or a filter of a name not known, the bounds of a value assertion whose expression uses an output that is unknown), a formula curve whose dimension changes along t, a parameter whose expression does not evaluate. Returns {units, was, keep, rewritten: the expressions given a unit}.",
    doc: true,
    set_unit
)];

const ID: &str = "document.setUnit";

fn set_unit(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(ID, params, &["units", "keep"])?;
    let symbol = p.required_str("units")?;
    let to = LengthUnit::from_symbol(symbol).ok_or_else(|| {
        CommandError::new(format!(
            "{ID}: unknown length unit `{}`; use one of um, mm, cm, m, in, ft",
            symbol.chars().take(20).collect::<String>()
        ))
    })?;
    let keep_sizes = match p.required_str("keep")? {
        "numbers" => false,
        "sizes" => true,
        other => {
            return fail(format!(
                "{ID}: `keep` is numbers or sizes, not `{}`",
                other.chars().take(20).collect::<String>()
            ));
        }
    };
    let current = std::sync::Arc::clone(session.document()?.history.current());
    let from = current.units.length;
    let keep = if keep_sizes { "sizes" } else { "numbers" };
    if from == to {
        return Ok(
            json!({"units": to.symbol(), "was": from.symbol(), "keep": keep, "rewritten": 0}),
        );
    }
    let mut changed = (*current).clone();
    changed.units.length = to;
    let mut rewritten = 0;
    if keep_sizes {
        // The assertions and outputs as they stand now: the outputs'
        // values for the expressions that use them, and what the change
        // must leave as it is.
        let checks = (!current.assertions.is_empty() || !current.outputs.is_empty())
            .then(|| session.checks_and_outputs(Wait::All))
            .transpose()?;
        let powers;
        (rewritten, powers) =
            keep_the_sizes(&mut changed, from, to, checks.as_ref().map(|c| &*c.1))
                .map_err(|e| CommandError::new(format!("{ID}: keep sizes: {e}")))?;
        // Every feature is proven, also those the marker rolls back and
        // those switched off: both documents are regenerated whole, with
        // the marker cleared and every feature on.
        let (all_before, all_after) = (everything(&current), everything(&changed));
        let before = craft3d_regen::regenerate(&all_before);
        let after = craft3d_regen::regenerate(&all_after);
        same_sizes(&before, &after, &changed, factor(from, to))
            .and_then(|()| same_sets(&all_before, &before, &all_after, &after))
            .map_err(|e| {
                CommandError::new(format!(
                    "{ID}: keep sizes: not proven, so nothing was changed: {e}"
                ))
            })?;
        if let Some(checks) = &checks {
            let now = checked_alone(session, changed.clone())?;
            same_checks(checks, &now, &powers, factor(from, to)).map_err(|e| {
                CommandError::new(format!(
                    "{ID}: keep sizes: not proven, so nothing was changed: {e}"
                ))
            })?;
        }
    }
    let label = format!("Set length unit to {}", to.symbol());
    edit(session, ID, &label, |doc| {
        *doc = changed;
        Ok(())
    })?;
    Ok(json!({"units": to.symbol(), "was": from.symbol(), "keep": keep, "rewritten": rewritten}))
}

/// `doc` in the length unit `to`, as large as it was: what
/// `document.setUnit {keep: sizes}` makes of it, proven the same way
/// (every feature's status, every body's volume, area, centroid and box,
/// the named sets), without measuring its assertions. What an assembly in
/// another unit regenerates a part source in (docs/design/assemblies.md).
pub(crate) fn in_unit(doc: Document, to: LengthUnit) -> Result<Document, String> {
    let from = doc.units.length;
    if from == to {
        return Ok(doc);
    }
    let mut changed = doc.clone();
    changed.units.length = to;
    keep_the_sizes(&mut changed, from, to, None)?;
    let (all_before, all_after) = (everything(&doc), everything(&changed));
    let before = craft3d_regen::regenerate(&all_before);
    let after = craft3d_regen::regenerate(&all_after);
    same_sizes(&before, &after, &changed, factor(from, to))
        .and_then(|()| same_sets(&all_before, &before, &all_after, &after))
        .map_err(|e| format!("not proven the same size in {}: {e}", to.symbol()))?;
    Ok(changed)
}

/// The number in `to` of one `from`: what a length's number is multiplied
/// by when its size is kept.
fn factor(from: LengthUnit, to: LengthUnit) -> f64 {
    from.as_expr_unit().factor() / to.as_expr_unit().factor()
}

/// Whether a plain number of a parameter called `name` (its path, such as
/// `size.x`, `flanges.2.radius`, `c4.value`) is read as a length: `Some`
/// for the names a length or a number that is not one has, `None` for a
/// name not known, which is refused.
fn is_length(name: &str) -> Option<bool> {
    const LENGTHS: &[&str] = &[
        "position",
        "size",
        "radius",
        "height",
        "major",
        "minor",
        "distance",
        "thickness",
        "translate",
        "pivot",
        "point",
        "at",
        "step",
        "step2",
        "length",
        "pitch",
        "second",
        "center",
        "offset",
        "gap",
        "fixed",
        "through",
        "origin",
        "diameter",
    ];
    const OTHERS: &[&str] = &[
        "angle", "axis", "normal", "count", "count2", "turns", "kFactor", "kind", "shape", "when",
        "xAxis", "from", "to",
    ];
    // A fold's angle at a crease (`crease.e7[2]`).
    if name.starts_with("crease.") {
        return Some(false);
    }
    // The last part that is not an index or an axis says what it is.
    let word = name
        .split('.')
        .rev()
        .find(|part| !matches!(*part, "x" | "y" | "z") && part.parse::<u64>().is_err())?;
    if LENGTHS.contains(&word) {
        Some(true)
    } else if OTHERS.contains(&word) {
        Some(false)
    } else {
        None
    }
}

/// What the expressions of the document are evaluated with: its
/// variables and, where it has them, its outputs as they are computed
/// before the change (an output's value is in base units, so the same in
/// either unit).
struct Values<'a> {
    variables: &'a Evaluation,
    outputs: Option<&'a Resolved>,
}

impl Values<'_> {
    fn eval(&self, source: &str) -> Result<Quantity, String> {
        match self.outputs {
            Some(outputs) => self
                .variables
                .evaluate_with(source, |name| outputs.lookup(name)),
            None => self.variables.evaluate(source),
        }
        .map_err(|e| e.to_string())
    }
}

/// `source`, a plain number read in the document's unit as a quantity of
/// dimension `dim` (a length to a power, perhaps times an angle), written
/// with the unit `unit`, and proven to be that: a plain length gets the
/// unit after it, anything else is multiplied by one of it to the power.
fn with_unit(
    values: &Values<'_>,
    source: &str,
    unit: LengthUnit,
    dim: Dimension,
) -> Result<String, String> {
    let symbol = unit.symbol();
    let plain = values
        .eval(source)
        .map_err(|e| format!("`{source}` does not evaluate: {e}"))?;
    let trimmed = source.trim();
    let mut written = match (dim.length, trimmed.parse::<f64>()) {
        (1, Ok(n)) if n.is_finite() => format!("{trimmed} {symbol}"),
        (1, _) => format!("({trimmed}) * 1 {symbol}"),
        (0, _) => format!("({trimmed})"),
        (k, _) => format!("({trimmed}) * (1 {symbol})^{k}"),
    };
    // An exponent is a literal, negative or not, never in parentheses.
    for (other, power) in [("rad", dim.angle), ("kg", dim.mass), ("s", dim.time)] {
        if power != 0 {
            written.push_str(&format!(" * (1 {other})^{power}"));
        }
    }
    let got = values
        .eval(&written)
        .map_err(|e| format!("`{written}` does not evaluate: {e}"))?;
    let want = plain.value * unit.as_expr_unit().factor().powi(dim.length);
    let close = (got.value - want).abs() <= 1e-12 * want.abs().max(f64::MIN_POSITIVE);
    if got.dimension != dim || !close {
        return Err(format!(
            "`{source}` could not be written with its unit (`{written}`)"
        ));
    }
    Ok(written)
}

/// The length's power each filter of a query reads its values in
/// (select.query: `volume`, `area`, `radius`, `length`, a box `inside`,
/// `above` and `below`); the filters whose values are no lengths (names,
/// kinds, colours, counts, directions, angles in degrees) are kept as they
/// are, and a filter not known with a number in it is refused.
const FILTER_LENGTHS: [(&str, i32); 4] = [("volume", 3), ("area", 2), ("radius", 1), ("length", 1)];
const FILTER_OTHERS: [&str; 16] = [
    "kind", "name", "type", "status", "set", "in", "madeBy", "copyOf", "instance", "color",
    "within", "material", "surface", "hole", "normal", "axis",
];

/// Rewrites the numbers of queries, subjects and measures that are read
/// in the document's unit so that they keep their sizes in another unit.
struct Sizer<'a> {
    values: Values<'a>,
    from: LengthUnit,
    rewritten: usize,
}

impl Sizer<'_> {
    /// A value read as a length to `power` (a number, or an expression
    /// that is a plain number) with the old unit written on it; a value
    /// with a unit, or one that is no value, as it is.
    fn sized(&mut self, value: &Value, power: i32, what: &str) -> Result<Value, String> {
        let source = match value {
            Value::Number(n) => n.to_string(),
            Value::String(text) => {
                let quantity = self
                    .values
                    .eval(text)
                    .map_err(|e| format!("{what}: `{text}` does not evaluate: {e}"))?;
                if !quantity.dimension.is_dimensionless() {
                    return Ok(value.clone());
                }
                text.clone()
            }
            other => return Ok(other.clone()),
        };
        let written = with_unit(&self.values, &source, self.from, Dimension::new(power, 0))
            .map_err(|e| format!("{what}: {e}"))?;
        self.rewritten = self.rewritten.saturating_add(1);
        Ok(Value::String(written))
    }

    /// Every number or expression in `value`, a list of them or a list of
    /// such lists (two deep at most), sized as a length.
    fn lengths(&mut self, value: &Value, depth: usize, what: &str) -> Result<Value, String> {
        match value {
            Value::Array(items) if depth < 2 => Ok(Value::Array(
                items
                    .iter()
                    .map(|item| self.lengths(item, depth + 1, what))
                    .collect::<Result<_, String>>()?,
            )),
            Value::Array(_) => Ok(value.clone()),
            other => self.sized(other, 1, what),
        }
    }

    /// A value or a range `{eq, tol}` / `{min, max}` of a filter.
    fn range(&mut self, value: &Value, power: i32, what: &str) -> Result<Value, String> {
        match value {
            Value::Object(fields) => {
                let mut out = Map::new();
                for (key, v) in fields {
                    let v = match key.as_str() {
                        "eq" | "tol" | "min" | "max" => {
                            self.sized(v, power, &format!("{what}.{key}"))?
                        }
                        _ => v.clone(),
                    };
                    out.insert(key.clone(), v);
                }
                Ok(Value::Object(out))
            }
            other => self.sized(other, power, what),
        }
    }

    /// A filter of select.query, its lengths sized.
    fn filter(&mut self, value: &Value, what: &str, depth: usize) -> Result<Value, String> {
        if depth > 64 {
            return Err(format!("{what}: filters nested too deep"));
        }
        let Value::Object(map) = value else {
            return match value {
                Value::Array(items) => Ok(Value::Array(
                    items
                        .iter()
                        .map(|item| self.filter(item, what, depth + 1))
                        .collect::<Result<_, _>>()?,
                )),
                other => Ok(other.clone()),
            };
        };
        let mut out = Map::new();
        for (key, inner) in map {
            let here = format!("{what}: `{key}`");
            let v = match key.as_str() {
                "all" | "any" | "not" => self.filter(inner, what, depth + 1)?,
                "inside" => match inner {
                    Value::Array(corners) => Value::Array(
                        corners
                            .iter()
                            .map(|corner| match corner {
                                Value::Array(xyz) => Ok(Value::Array(
                                    xyz.iter()
                                        .map(|c| self.sized(c, 1, &here))
                                        .collect::<Result<_, String>>()?,
                                )),
                                other => Ok(other.clone()),
                            })
                            .collect::<Result<_, String>>()?,
                    ),
                    other => other.clone(),
                },
                "above" | "below" => match inner {
                    Value::Object(along) => {
                        let mut sized = Map::new();
                        for (axis, at) in along {
                            sized.insert(axis.clone(), self.sized(at, 1, &here)?);
                        }
                        Value::Object(sized)
                    }
                    other => self.sized(other, 1, &here)?,
                },
                word => match FILTER_LENGTHS.iter().find(|(k, _)| *k == word) {
                    Some(&(_, power)) => self.range(inner, power, &here)?,
                    None if FILTER_OTHERS.contains(&word) || !has_plain_number(inner, 0) => {
                        inner.clone()
                    }
                    None => {
                        return Err(format!(
                            "{here} holds a number, and whether it is a length is not known"
                        ));
                    }
                },
            };
            out.insert(key.clone(), v);
        }
        Ok(Value::Object(out))
    }

    /// A query `{kind, where}`, its filter sized.
    fn query(&mut self, value: &Value, what: &str) -> Result<Value, String> {
        let Value::Object(map) = value else {
            return Ok(value.clone());
        };
        let mut out = map.clone();
        if let Some(filter) = map.get("where") {
            out.insert("where".into(), self.filter(filter, what, 0)?);
        }
        Ok(Value::Object(out))
    }

    /// A subject (an assertion's or a measured output's `of`, `from`,
    /// `to`): bodies by name or id are kept, a query or a bare filter is
    /// sized.
    fn subject(&mut self, value: &Value, what: &str) -> Result<Value, String> {
        match value {
            Value::Object(map) if map.contains_key("body") => Ok(value.clone()),
            Value::Object(map) if map.contains_key("where") => self.query(value, what),
            Value::Object(_) => self.filter(value, what, 0),
            other => Ok(other.clone()),
        }
    }

    /// A stored measure `{"measure": name, ...}` with its subjects and its
    /// own lengths sized (`offset`, a symmetry's `tol`, a level's
    /// `volume`, a plane's `point`); `None` when nothing changed.
    fn measure(&mut self, text: &str, what: &str) -> Result<Option<String>, String> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| format!("{what} is not a measure: {e}"))?;
        let Value::Object(map) = &value else {
            return Err(format!("{what} is not a measure"));
        };
        let mut out = Map::new();
        for (key, v) in map {
            let here = format!("{what}: `{key}`");
            let sized = match key.as_str() {
                "measure" | "axis" | "xAxis" | "about" | "measured" | "expr" => v.clone(),
                "of" | "from" | "to" => self.subject(v, &here)?,
                // Not lengths: a mass in kg and a density per cubic metre
                // whatever the unit, angles (a plain one in degrees), a
                // count of distances.
                "mass" | "density" | "heel" | "trim" | "maxWork" => v.clone(),
                "offset" | "tol" | "length" | "clearance" => self.sized(v, 1, &here)?,
                "volume" => self.sized(v, 3, &here)?,
                // A point (the centre of gravity) and polygons of points
                // (a press brake's tools): each coordinate a length.
                "centerOfGravity" | "punch" | "die" => self.lengths(v, 0, &here)?,
                "plane" => match v {
                    Value::Object(plane) => {
                        let mut sized = plane.clone();
                        if let Some(Value::Array(point)) = plane.get("point") {
                            sized.insert(
                                "point".into(),
                                Value::Array(
                                    point
                                        .iter()
                                        .map(|c| self.sized(c, 1, &here))
                                        .collect::<Result<_, String>>()?,
                                ),
                            );
                        }
                        Value::Object(sized)
                    }
                    other => other.clone(),
                },
                _ if !has_plain_number(v, 0) => v.clone(),
                _ => {
                    return Err(format!(
                        "{here} holds a number, and whether it is a length is not known"
                    ));
                }
            };
            out.insert(key.clone(), sized);
        }
        let out = Value::Object(out);
        if out == value {
            return Ok(None);
        }
        let written = serde_json::to_string(&out).map_err(|e| format!("{what}: {e}"))?;
        if written.len() > craft3d_doc::MAX_ASSERTION_TEXT {
            return Err(format!(
                "{what} would be longer than {} bytes with its units",
                craft3d_doc::MAX_ASSERTION_TEXT
            ));
        }
        Ok(Some(written))
    }
}

/// The curves of a sketch given by formulas: `x(t)` and `y(t)` that are
/// plain numbers (read in the document's unit) get the old unit, proven at
/// points along the range of `t`; formulas that are lengths are kept. A
/// formula whose dimension changes along `t` is refused.
fn size_formula_curves(
    sketch: &mut craft3d_doc::Sketch,
    variables: &Evaluation,
    from: LengthUnit,
    feature: &str,
) -> Result<usize, String> {
    let mut rewritten = 0usize;
    let symbol = from.symbol();
    let factor = from.as_expr_unit().factor();
    for entity in &mut sketch.entities {
        let id = entity.id;
        let Geometry::Formula {
            x,
            y,
            from: t0,
            to: t1,
            ..
        } = &mut entity.geometry
        else {
            continue;
        };
        let range = |e: &Expr, name: &str| {
            variables.evaluate(e.source()).map_err(|err| {
                format!(
                    "`{feature}` {id}.{name}: `{}` does not evaluate: {err}",
                    e.source()
                )
            })
        };
        let (mut start, mut end) = (range(t0, "from")?, range(t1, "to")?);
        // As regeneration reads them: an end that is exactly zero takes the
        // other's dimension.
        if start.dimension != end.dimension {
            if start.value == 0.0 {
                start = Quantity::new(0.0, end.dimension);
            } else if end.value == 0.0 {
                end = Quantity::new(0.0, start.dimension);
            }
        }
        if start.dimension != end.dimension {
            return Err(format!(
                "`{feature}` {id}: the range of t has ends of two dimensions"
            ));
        }
        let samples: Vec<Quantity> = (0..=8)
            .map(|i| {
                let s = f64::from(i) / 8.0;
                Quantity::new(start.value + (end.value - start.value) * s, start.dimension)
            })
            .collect();
        for (name, formula) in [("x", x), ("y", y)] {
            let source = formula.source().to_string();
            let what = format!("`{feature}` {id}.{name}");
            let at = |text: &str, t: Quantity| -> Result<Quantity, String> {
                let parsed = craft3d_expr::Expr::parse(text)
                    .map_err(|e| format!("{what}: `{text}`: {e}"))?;
                variables
                    .evaluate_bound(&parsed, craft3d_doc::FORMULA_PARAMETER, t)
                    .map_err(|e| format!("{what}: `{text}` at t = {}: {e}", t.value))
            };
            let plain: Vec<Quantity> = samples
                .iter()
                .map(|&t| at(&source, t))
                .collect::<Result<_, _>>()?;
            if plain.iter().all(|q| q.dimension == Dimension::LENGTH) {
                continue;
            }
            if !plain.iter().all(|q| q.dimension.is_dimensionless()) {
                return Err(format!(
                    "{what}: `{source}` is not a plain number or a length all along t"
                ));
            }
            let written = format!("({}) * 1 {symbol}", source.trim());
            for (&t, was) in samples.iter().zip(&plain) {
                let got = at(&written, t)?;
                let want = was.value * factor;
                if got.dimension != Dimension::LENGTH
                    || (got.value - want).abs() > 1e-12 * want.abs().max(f64::MIN_POSITIVE)
                {
                    return Err(format!(
                        "{what}: `{source}` could not be written with its unit (`{written}`)"
                    ));
                }
            }
            *formula = Expr::new(written);
            rewritten = rewritten.saturating_add(1);
        }
    }
    Ok(rewritten)
}

/// Rewrites `doc` (already in the unit `to`) so that it is as large as it
/// was in `from`; the number of expressions given a unit. `outputs` are the
/// document's outputs as computed before the change, which the bounds and
/// the measures' expressions may use.
fn keep_the_sizes(
    doc: &mut Document,
    from: LengthUnit,
    to: LengthUnit,
    outputs: Option<&Resolved>,
) -> Result<(usize, Vec<Option<i32>>), String> {
    let k = factor(from, to);
    // Per assertion, the power of the length its measured value is to (0
    // for a mass, a count, a yes or no), `None` where it is not known.
    let mut powers: Vec<Option<i32>> = Vec::new();
    let variables = doc
        .variable_table()
        .map_err(|e| format!("its variables: {e}"))?
        .evaluate();
    let values = Values {
        variables: &variables,
        outputs,
    };
    let mut rewritten = 0usize;
    for feature in &mut doc.features {
        let circular_step = matches!(
            &feature.kind,
            FeatureKind::Pattern(p) if matches!(p.layout, PatternLayout::Circular { .. })
        );
        // What each expression is, by its parameter's name: sketch
        // constraints by their kind.
        let named: Vec<(String, String, Option<bool>)> = feature
            .exprs()
            .into_iter()
            .map(|(name, expr)| {
                let role = match &feature.kind {
                    FeatureKind::Sketch(s) if name.ends_with(".value") => s
                        .constraints
                        .iter()
                        .find(|c| format!("{}.value", c.id) == name)
                        .map(|c| !matches!(c.kind, ConstraintKind::Angle { .. })),
                    _ if circular_step && name == "step" => Some(false),
                    _ => is_length(&name),
                };
                (name, expr.source().to_string(), role)
            })
            .collect();
        let feature_name = feature.name.clone();
        let exprs = feature.exprs_mut();
        if exprs.len() != named.len()
            || exprs
                .iter()
                .zip(&named)
                .any(|(e, (_, s, _))| e.source() != s)
        {
            return Err(format!(
                "the expressions of `{feature_name}` could not be told apart (a bug in 3DCraft, please report it)"
            ));
        }
        for (expr, (name, source, role)) in exprs.into_iter().zip(named) {
            let quantity = values.eval(&source).map_err(|e| {
                format!("`{feature_name}` {name}: `{source}` does not evaluate: {e}")
            })?;
            if !quantity.dimension.is_dimensionless() {
                continue;
            }
            match role {
                Some(false) => {}
                Some(true) => {
                    *expr = Expr::new(
                        with_unit(&values, &source, from, Dimension::LENGTH)
                            .map_err(|e| format!("`{feature_name}` {name}: {e}"))?,
                    );
                    rewritten = rewritten.saturating_add(1);
                }
                None => {
                    return Err(format!(
                        "`{feature_name}` {name}: `{source}` is a plain number, and whether it is a length is not known here"
                    ));
                }
            }
        }
        if let FeatureKind::Sketch(sketch) = &mut feature.kind {
            rewritten = rewritten.saturating_add(size_formula_curves(
                sketch,
                &variables,
                from,
                &feature_name,
            )?);
            for entity in &mut sketch.entities {
                match &mut entity.geometry {
                    Geometry::Point { x, y } => {
                        *x *= k;
                        *y *= k;
                    }
                    Geometry::Circle { radius, .. } => *radius *= k,
                    _ => {}
                }
            }
            for place in &mut sketch.image_places {
                place.at = place.at.map(|v| v * k);
                match &mut place.map {
                    Some(PlaceMap::Shift { offset }) => *offset = offset.map(|v| v * k),
                    Some(PlaceMap::Turn { at, .. }) => *at = at.map(|v| v * k),
                    _ => {}
                }
            }
        }
    }
    let mut sizer = Sizer {
        values,
        from,
        rewritten,
    };
    for set in &mut doc.sets {
        let what = format!("the set `{}`", set.name);
        let query: Value = serde_json::from_str(&set.query).map_err(|e| format!("{what}: {e}"))?;
        let sized = sizer.query(&query, &what)?;
        if sized != query {
            set.query = serde_json::to_string(&sized).map_err(|e| format!("{what}: {e}"))?;
            if set.query.len() > craft3d_doc::MAX_SET_QUERY_BYTES {
                return Err(format!(
                    "{what} would be longer than {} bytes with its units",
                    craft3d_doc::MAX_SET_QUERY_BYTES
                ));
            }
        }
    }
    for output in &mut doc.outputs {
        // An output's expression is computed in base units: the same
        // number in either unit. A measured one's subjects and lengths are
        // read in the document's unit.
        if let OutputDef::Measure(text) = &mut output.def {
            let what = format!("the output `{}`", output.name);
            if let Some(sized) = sizer.measure(text, &what)? {
                *text = sized;
            }
        }
    }
    for assertion in &mut doc.assertions {
        let name = assertion.name.clone();
        let what = format!("the assertion `{name}`");
        let measure: Value = serde_json::from_str(&assertion.measure).unwrap_or(Value::Null);
        let kind = measure.get("measure").and_then(Value::as_str).unwrap_or("");
        // `dim_of` knows measures by their name and reads any other as a
        // length: a value is told apart here, before it.
        let dim = match (kind, crate::assert_cmds::dim_of(kind)) {
            // A value's bounds are in the document's units of its
            // expression's dimension, which the outputs it uses tell.
            ("value", _) | (_, Dim::Of(_)) => {
                let source = measure.get("expr").and_then(Value::as_str).unwrap_or("");
                match sizer.values.eval(source) {
                    Ok(q) => Some(q.dimension),
                    Err(e) => {
                        let bounds: Vec<&Expr> = match &assertion.expect {
                            Expect::Is(_) => Vec::new(),
                            Expect::Equal { value, tol } => {
                                std::iter::once(value).chain(tol.as_ref()).collect()
                            }
                            Expect::Within { min, max } => {
                                min.as_ref().into_iter().chain(max.as_ref()).collect()
                            }
                        };
                        let plain_bound = bounds.iter().any(|b| {
                            sizer
                                .values
                                .eval(b.source())
                                .is_ok_and(|q| q.dimension.is_dimensionless())
                        });
                        if plain_bound {
                            return Err(format!(
                                "{what} measures `{source}`, whose dimension is not known ({e}), so a plain number in its bounds could be a length or not; write its bounds with their unit"
                            ));
                        }
                        None
                    }
                }
            }
            (_, Dim::Length(power)) => Some(Dimension::new(power, 0)),
            (_, Dim::Mass | Dim::Count | Dim::Flag) => None,
        };
        if let Some(sized) = sizer.measure(&assertion.measure, &what)? {
            assertion.measure = sized;
        }
        // The power of the length unit its numbers are in: a force's are
        // in N and a stress's in MPa whatever the length unit (0).
        let shown_power = |d: Dimension| d.default_unit(from.as_expr_unit()).length_power;
        powers.push(match (kind, crate::assert_cmds::dim_of(kind)) {
            ("value", _) | (_, Dim::Of(_) | Dim::Length(_)) => dim.map(shown_power),
            // A mass is in kilograms whatever the length unit, as long as a
            // material's density does not depend on it (it is kept in
            // kg/m^3); a count and a yes or no have no size.
            (_, Dim::Mass | Dim::Count | Dim::Flag) => Some(0),
        });
        let Some(dim) = dim.filter(|d| shown_power(*d) != 0) else {
            continue;
        };
        let bounds: Vec<&mut Expr> = match &mut assertion.expect {
            Expect::Is(_) => Vec::new(),
            Expect::Equal { value, tol } => std::iter::once(value).chain(tol.as_mut()).collect(),
            Expect::Within { min, max } => min.as_mut().into_iter().chain(max.as_mut()).collect(),
        };
        for bound in bounds {
            let source = bound.source().to_string();
            let quantity = sizer
                .values
                .eval(&source)
                .map_err(|e| format!("{what}: `{source}` does not evaluate: {e}"))?;
            if quantity.dimension.is_dimensionless() {
                *bound = Expr::new(
                    with_unit(&sizer.values, &source, from, dim)
                        .map_err(|e| format!("{what}: {e}"))?,
                );
                sizer.rewritten = sizer.rewritten.saturating_add(1);
            }
        }
    }
    let rewritten = sizer.rewritten;
    doc.tolerance.linear *= k;
    doc.tolerance.loosest = doc.tolerance.loosest.map(|l| l * k);
    if !craft3d_doc::Tolerance::linear_in_range(doc.tolerance.linear) {
        return Err(format!(
            "the tolerance would be {:e} {}, outside the range of a modelling tolerance",
            doc.tolerance.linear,
            to.symbol()
        ));
    }
    Ok((rewritten, powers))
}

/// The proof that every named set holds what it held: the same items
/// (kind, name and feature), in the same order, in `doc_after` regenerated
/// as `after` as in `doc_before` regenerated as `before`; a set that could
/// not be read before must not be readable after either.
fn same_sets(
    doc_before: &Document,
    before: &craft3d_regen::Regen,
    doc_after: &Document,
    after: &craft3d_regen::Regen,
) -> Result<(), String> {
    for set in &doc_before.sets {
        let was = crate::select_cmds::set_items(doc_before, before, &set.name);
        let now = crate::select_cmds::set_items(doc_after, after, &set.name);
        let same = match (&was, &now) {
            (Ok(a), Ok(b)) => a == b,
            (Err(_), Err(_)) => true,
            _ => false,
        };
        if !same {
            let count = |r: &Result<Vec<_>, String>| {
                r.as_ref().map_or_else(
                    |e| format!("an error ({e})"),
                    |v| format!("{} items", v.len()),
                )
            };
            return Err(format!(
                "the set `{}` holds {} before and {} in the new unit, or not the same ones",
                set.name,
                count(&was),
                count(&now)
            ));
        }
    }
    Ok(())
}

/// `doc` with every feature built: no rollback marker, nothing
/// suppressed, no condition. What the proof of keeping the sizes
/// regenerates, so that a feature it would not otherwise build is proven
/// too.
fn everything(doc: &Document) -> Document {
    let mut all = doc.clone();
    all.rollback = None;
    for feature in &mut all.features {
        feature.suppressed = false;
        feature.when = None;
    }
    all
}

/// Whether a query or subject holds a number (or a string that is one)
/// other than an instance's, which would be read in the document's unit.
fn has_plain_number(value: &Value, depth: usize) -> bool {
    if depth > 64 {
        return true;
    }
    match value {
        Value::Number(_) => true,
        Value::String(text) => text.trim().parse::<f64>().is_ok(),
        Value::Array(items) => items.iter().any(|v| has_plain_number(v, depth + 1)),
        Value::Object(map) => map
            .iter()
            .any(|(key, v)| key != "instance" && has_plain_number(v, depth + 1)),
        Value::Bool(_) | Value::Null => false,
    }
}

/// The assertions and outputs of `doc` as checked against it, in a
/// document of their own beside the open ones (the active one put back
/// whatever happens): what the change is proven against.
fn checked_alone(
    session: &mut Session,
    doc: Document,
) -> Result<(Vec<Checked>, std::sync::Arc<Resolved>), CommandError> {
    let active = session.active;
    session.documents.push(OpenDocument::new(doc, None));
    session.active = session.documents.len().saturating_sub(1);
    let checked = session.checks_and_outputs(Wait::All);
    session.documents.pop();
    session.active = active;
    checked
}

/// The proof that the assertions and outputs did not change: every
/// assertion has the status it had, every output the value (in base
/// units, so the same in either unit) and the dimension it had.
fn same_checks(
    before: &(Vec<Checked>, std::sync::Arc<Resolved>),
    after: &(Vec<Checked>, std::sync::Arc<Resolved>),
    powers: &[Option<i32>],
    k: f64,
) -> Result<(), String> {
    if before.0.len() != after.0.len() {
        return Err("the assertions are not the same in the new unit".into());
    }
    for (i, (was, now)) in before.0.iter().zip(&after.0).enumerate() {
        if was.state.status != now.state.status {
            return Err(format!(
                "the assertion `{}` is {} in the new unit and was {}",
                now.state.name,
                now.state.status.name(),
                was.state.status.name()
            ));
        }
        // What it measured, in the document's unit: as large, so its
        // number by the factor to the power of its dimension (a failed or
        // not proven one too, whose status alone would not tell).
        let power = powers.get(i).copied().flatten();
        let same = match (&was.value, &now.value, power) {
            (Value::Number(a), Value::Number(b), Some(p)) => match (a.as_f64(), b.as_f64()) {
                (Some(a), Some(b)) => {
                    let a = a * k.powi(p);
                    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()) + 1e-12
                }
                _ => false,
            },
            // A number whose dimension is not known cannot be held to a
            // size: not proven.
            (Value::Number(_), Value::Number(_), None) => false,
            (Value::Bool(a), Value::Bool(b), _) => a == b,
            // Nothing measured either time (an error, a note in words).
            (Value::Null, Value::Null, _) | (Value::String(_), Value::String(_), _) => true,
            // A number one time and not the other, or a yes or no.
            _ => false,
        };
        if !same {
            return Err(format!(
                "the assertion `{}` measured {} before and {} in the new unit",
                now.state.name, was.value, now.value
            ));
        }
    }
    for (name, was) in before.1.entries() {
        let now = after.1.get(name);
        let same = match (was, now) {
            (OutputValue::Known(a), Some(OutputValue::Known(b))) => {
                a.dimension == b.dimension
                    && (a.value - b.value).abs()
                        <= 1e-6 * a.value.abs().max(b.value.abs()).max(f64::MIN_POSITIVE)
            }
            (OutputValue::Unknown(_), Some(OutputValue::Unknown(_))) => true,
            _ => false,
        };
        if !same {
            return Err(format!(
                "the output `{name}` is {was:?} before and {now:?} in the new unit"
            ));
        }
    }
    Ok(())
}

/// The proof of keeping the sizes: `after` (the document in the new unit,
/// whose numbers are `k` times as large) has every feature with the
/// status it had in `before`, and every body the volume and area it
/// had, by the powers of `k`.
fn same_sizes(
    before: &craft3d_regen::Regen,
    after: &craft3d_regen::Regen,
    doc: &Document,
    k: f64,
) -> Result<(), String> {
    let called = |id| {
        doc.feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone())
    };
    for (was, now) in before.features.iter().zip(&after.features) {
        if was.id != now.id
            || std::mem::discriminant(&was.status) != std::mem::discriminant(&now.status)
        {
            return Err(format!(
                "`{}` is built otherwise in the new unit",
                called(now.id)
            ));
        }
    }
    if before.features.len() != after.features.len() {
        return Err("the history is not the same in the new unit".into());
    }
    let mut bodies_after = after.bodies();
    for (id, body) in before.bodies() {
        let Some((other, scaled)) = bodies_after.next() else {
            return Err(format!("`{}` has no body in the new unit", called(id)));
        };
        if other != id {
            return Err(format!("`{}` has no body in the new unit", called(id)));
        }
        let near =
            |a: f64, b: f64| (a - b).abs() <= 1e-7 * a.abs().max(b.abs()).max(f64::MIN_POSITIVE);
        let (Ok(was), Ok(now)) = (
            craft3d_topo::mass_properties(body),
            craft3d_topo::mass_properties(scaled),
        ) else {
            if body.faces.len() == scaled.faces.len() {
                continue;
            }
            return Err(format!("`{}` has other faces in the new unit", called(id)));
        };
        if !near(was.volume * k.powi(3), now.volume) || !near(was.area * k.powi(2), now.area) {
            return Err(format!(
                "`{}` has another size in the new unit (volume {} for {})",
                called(id),
                now.volume,
                was.volume * k.powi(3)
            ));
        }
        // Where it is, not only how large: its centroid and its box, by
        // the factor, within a part in 10^7 of its size.
        let (Ok(box_was), Ok(box_now)) = (body.bbox(), scaled.bbox()) else {
            return Err(format!(
                "`{}` cannot be boxed to prove where it is",
                called(id)
            ));
        };
        let size = box_now.diagonal();
        let same_place = |a: Vec3, b: Vec3| (a * k - b).length() <= 1e-7 * size;
        let moved = match (box_was.corners(), box_now.corners()) {
            (Some((lo, hi)), Some((lo2, hi2))) => !same_place(lo, lo2) || !same_place(hi, hi2),
            (None, None) => false,
            _ => true,
        } || match (was.centroid, now.centroid) {
            (Some(a), Some(b)) => !same_place(a, b),
            (None, None) => false,
            _ => true,
        };
        if moved {
            return Err(format!(
                "`{}` is in another place in the new unit",
                called(id)
            ));
        }
    }
    if bodies_after.next().is_some() {
        return Err("a body is in the new unit that was not before".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;

    fn block_at(x: &str) -> Document {
        let mut s = Session::new(Workspace::none());
        let run = |s: &mut Session, id: &str, p: Value| {
            s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
        };
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "feature.block",
            json!({"size": [10, 20, 30], "position": [x, "0", "0"]}),
        );
        (**s.document().unwrap().history.current()).clone()
    }

    /// The proof sees a body that is as large as it was but in another
    /// place: a block moved by 1 has the same volume and area.
    #[test]
    fn a_body_in_another_place_is_not_proven() {
        let was = block_at("0");
        let same = block_at("0");
        let moved = block_at("1");
        let regen = |d: &Document| craft3d_regen::regenerate(&everything(d));
        assert!(same_sizes(&regen(&was), &regen(&same), &same, 1.0).is_ok());
        let e = same_sizes(&regen(&was), &regen(&moved), &moved, 1.0).unwrap_err();
        assert!(e.contains("another place"), "{e}");
        // At a factor: the block 25.4 times as large is proven only where
        // its place is 25.4 times as far too.
        let mut inches = block_at("0");
        // Its position and size, the first six expressions.
        for e in inches.features[0].exprs_mut().into_iter().take(6) {
            let v: f64 = e.source().parse().unwrap();
            *e = Expr::new(format!("{}", v * 25.4));
        }
        assert!(same_sizes(&regen(&was), &regen(&inches), &inches, 25.4).is_ok());
    }
    /// A failed assertion keeps its status whatever its number is: the
    /// proof also holds its measured value to the factor to the power of
    /// its dimension, so a number left unscaled is caught.
    #[test]
    fn a_failed_assertion_measured_at_the_wrong_size_is_not_proven() {
        let mut s = Session::new(Workspace::none());
        for (id, p) in [
            ("file.new", json!({"units": "mm"})),
            ("feature.block", json!({"size": [10, 20, 30], "name": "B"})),
            (
                "assert.add",
                json!({"name": "small", "measure": "volume", "of": "B", "max": 1000}),
            ),
        ] {
            s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        let was = s.checks_and_outputs(Wait::All).unwrap();
        assert_eq!(was.0[0].state.status.name(), "failed");
        let v = was.0[0].value.as_f64().unwrap();
        assert!((v - 6000.0).abs() < 1e-9, "{v}");
        let k: f64 = 0.1;
        let mut scaled = was.clone();
        scaled.0[0].value = json!(v * k.powi(3));
        assert!(same_checks(&was, &scaled, &[Some(3)], k).is_ok());
        let mut unscaled = was.clone();
        unscaled.0[0].value = json!(v * k);
        let e = same_checks(&was, &unscaled, &[Some(3)], k).unwrap_err();
        assert!(e.contains("small") && e.contains("measured"), "{e}");
    }

    /// A named set that holds other items in the new unit is not proven.
    #[test]
    fn a_set_that_holds_other_items_is_not_proven() {
        let mut s = Session::new(Workspace::none());
        for (id, p) in [
            ("file.new", json!({"units": "mm"})),
            ("feature.block", json!({"size": [10, 20, 30], "name": "B"})),
            (
                "set.define",
                json!({"name": "big", "kind": "face", "where": {"area": {"min": 500}}}),
            ),
        ] {
            s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        let doc = (**s.document().unwrap().history.current()).clone();
        let regen = craft3d_regen::regenerate(&doc);
        assert!(same_sets(&doc, &regen, &doc, &regen).is_ok());
        let mut other = doc.clone();
        other.sets[0].query = other.sets[0].query.replace("500", "100");
        let e = same_sets(&doc, &regen, &other, &regen).unwrap_err();
        assert!(e.contains("big"), "{e}");
    }
}
