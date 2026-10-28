//! Angles written as plain numbers with `pi` in them.
//!
//! A plain number where an angle is expected (a revolve's `angle`, a
//! pattern's, an angle constraint's value) is in the document's angle
//! unit, degrees by default, and `pi` is a plain number: so `2 * pi / n`
//! there is a few degrees, not a turn. Files rely on that meaning, so it
//! stays; `document.check` warns where it is likely a mistake: a parameter
//! regeneration reads as an angle, in a document whose angle unit is
//! degrees, whose value is a plain number and whose expression, or a
//! variable it uses (followed to any depth), has a `pi` that is neither
//! given a unit (`pi rad`) nor inside `sin`, `cos` or `tan`
//! ([`craft3d_expr::Expr::has_bare_pi`]). A warning never changes a
//! value.

use std::collections::{BTreeMap, BTreeSet};

use craft3d_doc::{ConstraintKind, Document, FeatureId, FeatureKind};
use serde_json::{Value, json};

/// Whether regeneration reads the parameter called `name` as an angle:
/// `angle` and `<part>.angle` (a primitive's turn, a revolve, a transform,
/// a pattern, a draft, a chamfer's `size.angle`, a flange's, a plate
/// joint's, a sketch pattern's), and a fold's crease angles.
fn is_angle_parameter(name: &str) -> bool {
    name == "angle" || name.ends_with(".angle") || name.starts_with("crease.")
}

/// The variable through which `source` has a bare `pi`: `Some("")` when
/// `source` itself has one, `Some(name)` for the first variable it uses
/// (directly or through others) that does, `None` when none does. Each
/// variable is looked at once, so a cycle ends.
fn bare_pi_in(source: &str, variables: &BTreeMap<&str, &str>) -> Option<String> {
    let mut seen = BTreeSet::new();
    let mut todo = vec![(String::new(), source.to_string())];
    while let Some((name, text)) = todo.pop() {
        let Ok(parsed) = craft3d_expr::Expr::parse(&text) else {
            continue;
        };
        if parsed.has_bare_pi() {
            return Some(name);
        }
        for used in parsed.variables().into_iter().rev() {
            if seen.insert(used.to_string())
                && let Some(expr) = variables.get(used)
            {
                todo.push((used.to_string(), (*expr).to_string()));
            }
        }
    }
    None
}

/// The warnings of `document.check` (`kind: "plainAngle"`), for every
/// feature or only for `only`.
pub(crate) fn plain_angles(doc: &Document, only: Option<FeatureId>) -> Vec<Value> {
    if doc.units.angle != craft3d_doc::AngleUnit::Degree {
        return Vec::new();
    }
    let Some(values) = doc.variable_table().ok().map(|t| t.evaluate()) else {
        return Vec::new();
    };
    let variables: BTreeMap<&str, &str> = doc
        .variables
        .iter()
        .map(|v| (v.name.as_str(), v.expr.source()))
        .collect();
    let mut out = Vec::new();
    for feature in &doc.features {
        if only.is_some_and(|id| id != feature.id) {
            continue;
        }
        let mut angles: Vec<(String, &craft3d_doc::Expr)> = feature
            .exprs()
            .into_iter()
            .filter(|(name, _)| is_angle_parameter(name))
            .collect();
        if let FeatureKind::Sketch(sketch) = &feature.kind {
            for c in &sketch.constraints {
                if let ConstraintKind::Angle { value, .. } = &c.kind {
                    angles.push((format!("{}.value", c.id), value));
                }
            }
        }
        for (parameter, expr) in angles {
            let source = expr.source();
            let Ok(value) = values.evaluate(source) else {
                continue;
            };
            if !value.dimension.is_dimensionless() {
                continue;
            }
            let Some(through) = bare_pi_in(source, &variables) else {
                continue;
            };
            let via = if through.is_empty() {
                String::new()
            } else {
                format!(" (through `{through}`)")
            };
            out.push(json!({
                "kind": "plainAngle",
                "feature": feature.id.to_string(),
                "name": feature.name,
                "parameter": parameter,
                "expr": source,
                "warning": format!(
                    "{parameter} of {} is `{source}`, a plain number with pi in it{via}, which an angle takes in degrees: {} degrees; if radians are meant, give pi its unit (`2 * pi rad / n`) or write degrees (`360 deg / n`)",
                    feature.name,
                    value.value
                ),
            }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_bare_pi_is_followed_through_variables_and_cycles_end() {
        let variables: BTreeMap<&str, &str> = [
            ("n", "6"),
            ("turn", "2 * pi / n"),
            ("half", "turn / 2"),
            ("whole", "2 * pi rad / n"),
            ("a", "b + 1"),
            ("b", "a + 1"),
        ]
        .into_iter()
        .collect();
        assert_eq!(bare_pi_in("pi / 3", &variables), Some(String::new()));
        assert_eq!(bare_pi_in("half * 2", &variables), Some("turn".into()));
        assert_eq!(bare_pi_in("whole / 1 rad", &variables), None);
        assert_eq!(bare_pi_in("360 / n", &variables), None);
        assert_eq!(bare_pi_in("a * 2", &variables), None);
        assert_eq!(bare_pi_in("sin(pi / 6) * 60", &variables), None);
        assert!(is_angle_parameter("angle"));
        assert!(is_angle_parameter("size.angle"));
        assert!(is_angle_parameter("crease.e7"));
        assert!(!is_angle_parameter("step"));
        assert!(!is_angle_parameter("e7.from"));
    }
}
