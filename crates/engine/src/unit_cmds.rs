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
//! in the new unit has every body the size it had (volume and area by
//! the factor's powers) and every feature the status it had, or the
//! change is refused and the document left as it was.

use crate::commands::{Run, edit, fail, spec};
use crate::params::Params;
use crate::verify_cmds::Dim;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::sketch::{ConstraintKind, Geometry, PlaceMap};
use craft3d_doc::{Document, Expect, Expr, FeatureKind, LengthUnit, PatternLayout};
use craft3d_expr::{Dimension, Evaluation};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "document.setUnit",
    "Set Length Unit",
    r#"{"units": "um|mm|cm|m|in|ft", "keep": "numbers|sizes"}"#,
    "Changes the length unit of the active document (one undo step). A plain number read as a length is in the document's unit, so keep says what stays: numbers keeps every number as written and changes only the unit (a block of size 10 is then 10 of the new unit; a value written with a unit, 10 mm, keeps its size, and so does the tolerance's number); sizes keeps the model as large as it was: every plain number read as a length gets the old unit written on it (10 becomes 10 mm, w / 2 becomes (w / 2) * 1 mm; variables are left as they are, their uses get the unit), assertions' bounds likewise, the tolerance and the sketches' points and circles are converted, and the result is proven: regenerated in the new unit, every body has the volume and area it had and every feature the status it had, or the change is refused and the document stays as it was. sizes refuses what it cannot convert for certain: outputs, formula curves, numbers in sets and subjects, measures with expressions of their own, a parameter whose expression does not evaluate. Returns {units, was, keep, rewritten: the expressions given a unit}.",
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
        rewritten = keep_the_sizes(&mut changed, from, to)
            .map_err(|e| CommandError::new(format!("{ID}: keep sizes: {e}")))?;
        let before = session.document_mut()?.regen();
        let after = craft3d_regen::regenerate(&changed);
        same_sizes(&before, &after, &changed, factor(from, to)).map_err(|e| {
            CommandError::new(format!(
                "{ID}: keep sizes: not proven, so nothing was changed: {e}"
            ))
        })?;
    }
    let label = format!("Set length unit to {}", to.symbol());
    edit(session, ID, &label, |doc| {
        *doc = changed;
        Ok(())
    })?;
    Ok(json!({"units": to.symbol(), "was": from.symbol(), "keep": keep, "rewritten": rewritten}))
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

/// `source`, read as a length to the power `power` in the unit `unit`,
/// written with that unit, and proven to be that: a plain number gets the
/// unit after it, anything else is multiplied by one of it.
fn with_unit(
    values: &Evaluation,
    source: &str,
    unit: LengthUnit,
    power: i32,
) -> Result<String, String> {
    let symbol = unit.symbol();
    let plain = values
        .evaluate(source)
        .map_err(|e| format!("`{source}` does not evaluate: {e}"))?;
    let trimmed = source.trim();
    let written = match (power, trimmed.parse::<f64>()) {
        (1, Ok(n)) if n.is_finite() => format!("{trimmed} {symbol}"),
        (1, _) => format!("({trimmed}) * 1 {symbol}"),
        (k, _) => format!("({trimmed}) * (1 {symbol})^{k}"),
    };
    let got = values
        .evaluate(&written)
        .map_err(|e| format!("`{written}` does not evaluate: {e}"))?;
    let want = plain.value * unit.as_expr_unit().factor().powi(power);
    let close = (got.value - want).abs() <= 1e-12 * want.abs().max(f64::MIN_POSITIVE);
    if got.dimension != Dimension::new(power, 0) || !close {
        return Err(format!(
            "`{source}` could not be written with its unit (`{written}`)"
        ));
    }
    Ok(written)
}

/// Rewrites `doc` (already in the unit `to`) so that it is as large as it
/// was in `from`; the number of expressions given a unit.
fn keep_the_sizes(doc: &mut Document, from: LengthUnit, to: LengthUnit) -> Result<usize, String> {
    let k = factor(from, to);
    if !doc.outputs.is_empty() {
        return Err("the document has outputs, whose values are in its unit (owed)".into());
    }
    let values = doc
        .variable_table()
        .map_err(|e| format!("its variables: {e}"))?
        .evaluate();
    let mut rewritten = 0usize;
    for feature in &mut doc.features {
        if !feature.kind.formula_exprs().is_empty() {
            return Err(format!(
                "`{}` has a formula curve, whose numbers are lengths in the unit (owed)",
                feature.name
            ));
        }
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
            let quantity = values.evaluate(&source).map_err(|e| {
                format!("`{feature_name}` {name}: `{source}` does not evaluate: {e}")
            })?;
            if !quantity.dimension.is_dimensionless() {
                continue;
            }
            match role {
                Some(false) => {}
                Some(true) => {
                    *expr = Expr::new(
                        with_unit(&values, &source, from, 1)
                            .map_err(|e| format!("`{feature_name}` {name}: {e}"))?,
                    );
                    rewritten = rewritten.saturating_add(1);
                }
                None => {
                    return Err(format!(
                        "`{feature_name}` {name}: `{source}` is a plain number, and whether it is a length is not known here (owed)"
                    ));
                }
            }
        }
        if let FeatureKind::Sketch(sketch) = &mut feature.kind {
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
    for set in &doc.sets {
        if has_plain_number(&serde_json::from_str(&set.query).unwrap_or(Value::Null), 0) {
            return Err(format!(
                "the set `{}` has a number in its query, in the unit (owed: write it with its unit)",
                set.name
            ));
        }
    }
    for assertion in &mut doc.assertions {
        let measure: Value = serde_json::from_str(&assertion.measure).unwrap_or(Value::Null);
        let kind = measure.get("measure").and_then(Value::as_str).unwrap_or("");
        let own_numbers = measure.as_object().is_some_and(|m| {
            m.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "measure" | "of" | "from" | "to" | "axis" | "expr"
                )
            })
        });
        let subjects_numbered = ["of", "from", "to"]
            .iter()
            .any(|key| measure.get(*key).is_some_and(|v| has_plain_number(v, 0)));
        if own_numbers || subjects_numbered {
            return Err(format!(
                "the assertion `{}` has numbers of its own in what it measures (owed)",
                assertion.name
            ));
        }
        let power = match crate::assert_cmds::dim_of(kind) {
            Dim::Length(power) => power,
            Dim::Of(_) => {
                return Err(format!(
                    "the assertion `{}` measures a value, whose bounds are in the unit (owed)",
                    assertion.name
                ));
            }
            Dim::Mass | Dim::Count | Dim::Flag => continue,
        };
        let name = assertion.name.clone();
        let bounds: Vec<&mut Expr> = match &mut assertion.expect {
            Expect::Is(_) => Vec::new(),
            Expect::Equal { value, tol } => std::iter::once(value).chain(tol.as_mut()).collect(),
            Expect::Within { min, max } => min.as_mut().into_iter().chain(max.as_mut()).collect(),
        };
        for bound in bounds {
            let source = bound.source().to_string();
            let quantity = values.evaluate(&source).map_err(|e| {
                format!("the assertion `{name}`: `{source}` does not evaluate: {e}")
            })?;
            if quantity.dimension.is_dimensionless() {
                *bound = Expr::new(
                    with_unit(&values, &source, from, power)
                        .map_err(|e| format!("the assertion `{name}`: {e}"))?,
                );
                rewritten = rewritten.saturating_add(1);
            }
        }
    }
    doc.tolerance.linear *= k;
    doc.tolerance.loosest = doc.tolerance.loosest.map(|l| l * k);
    if !craft3d_doc::Tolerance::linear_in_range(doc.tolerance.linear) {
        return Err(format!(
            "the tolerance would be {:e} {}, outside the range of a modelling tolerance",
            doc.tolerance.linear,
            to.symbol()
        ));
    }
    Ok(rewritten)
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
    }
    if bodies_after.next().is_some() {
        return Err("a body is in the new unit that was not before".into());
    }
    Ok(())
}
