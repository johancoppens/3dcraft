//! Features that make their body out of the body of an earlier feature:
//! a transform, a mirror, a pattern.
//!
//! The source feature's result stays in the list (names still resolve
//! against it); it is marked as taken, so its body is no longer one of the
//! document's bodies, unless the feature keeps it.

use crate::{FeatureResult, Names, Status, angle, length, number};
use craft3d_doc::{
    Document, Expr, Feature, FeatureId, FeatureKind, MAX_NAME_DEPTH, PatternLayout, PersistentName,
};
use craft3d_expr::Evaluation;
use craft3d_math::{Transform, Vec3};
use craft3d_topo::Body;
use std::f64::consts::TAU;

// TEMPORARY, see pending.rs.
use crate::pending::{BodyExt, TransformExt, merge};

/// Most instances one pattern makes.
pub const MAX_PATTERN_INSTANCES: u64 = 10_000;

/// Most faces a pattern's body has, over all instances.
pub const MAX_PATTERN_FACES: usize = 1_000_000;

/// What a derived feature made.
pub(crate) struct Derived {
    pub body: Body,
    /// The names of its entities, when the source had names.
    pub names: Option<Names>,
    /// `instances[i]` maps the source body onto instance `i`.
    pub instances: Vec<Transform>,
}

/// The body a feature is made from: the source has to exist, come
/// earlier, have succeeded, have a body, and still own it.
fn source_body<'a>(
    doc: &Document,
    earlier: &'a [FeatureResult],
    id: FeatureId,
) -> Result<&'a FeatureResult, String> {
    let Some(source) = doc.feature(id) else {
        return Err(format!(
            "source: there is no feature {id} (was it deleted?)"
        ));
    };
    let name = &source.name;
    let Some(outcome) = earlier.iter().find(|r| r.id == id) else {
        return Err(format!(
            "source: `{name}` comes after this feature in the history; move it before (feature.move)"
        ));
    };
    match &outcome.status {
        Status::Ok => {}
        Status::Suppressed => return Err(format!("source: `{name}` is suppressed")),
        Status::RolledBack => return Err(format!("source: `{name}` is rolled back")),
        Status::Failed(_) => return Err(format!("source: `{name}` failed; fix it first")),
    }
    if outcome.body.is_none() {
        return Err(format!(
            "source: `{name}` is a {} and has no body",
            source.kind.type_name()
        ));
    }
    if let Some(taker) = outcome.consumed_by {
        let taker = doc
            .feature(taker)
            .map_or_else(|| taker.to_string(), |f| f.name.clone());
        return Err(format!(
            "source: the body of `{name}` already went into `{taker}`; use that feature as the source, or give `{taker}` keep"
        ));
    }
    Ok(outcome)
}

fn lengths(
    doc: &Document,
    variables: &Evaluation,
    name: &str,
    exprs: &[Expr; 3],
) -> Result<Vec3, String> {
    let [x, y, z] = exprs;
    let v = Vec3::new(
        length(doc, variables, &format!("{name}.x"), x)?,
        length(doc, variables, &format!("{name}.y"), y)?,
        length(doc, variables, &format!("{name}.z"), z)?,
    );
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{name} is not finite"))
    }
}

fn direction(variables: &Evaluation, name: &str, exprs: &[Expr; 3]) -> Result<Vec3, String> {
    let [x, y, z] = exprs;
    let v = Vec3::new(
        number(variables, &format!("{name}.x"), x)?,
        number(variables, &format!("{name}.y"), y)?,
        number(variables, &format!("{name}.z"), z)?,
    );
    match v.normalized() {
        Some(unit) if v.is_finite() => Ok(unit),
        _ => Err(format!(
            "{name} must be a direction, but it is ({}, {}, {})",
            v.x, v.y, v.z
        )),
    }
}

/// How many instances: a whole number from 1 to the cap.
fn count(variables: &Evaluation, expr: &Expr) -> Result<u64, String> {
    let value = number(variables, "count", expr)?;
    let whole = value.round();
    if !(value.is_finite() && (value - whole).abs() <= 1e-9 && whole >= 1.0) {
        return Err(format!(
            "count must be a whole number of at least 1, but `{}` is {value}",
            expr.source()
        ));
    }
    if whole > MAX_PATTERN_INSTANCES as f64 {
        return Err(format!(
            "count: `{}` is {whole}; a pattern has at most {MAX_PATTERN_INSTANCES} instances",
            expr.source()
        ));
    }
    // Whole, at least 1 and at most the cap: the cast is exact.
    Ok(whole as u64)
}

/// The maps from the source body onto each instance.
fn instances(
    doc: &Document,
    variables: &Evaluation,
    kind: &FeatureKind,
) -> Result<Vec<Transform>, String> {
    let degenerate = |what: &str| format!("{what} does not give a finite motion");
    match kind {
        FeatureKind::Transform(t) => {
            let shift = lengths(doc, variables, "translate", &t.translate)?;
            let turn = angle(doc, variables, "angle", &t.angle)?;
            if !turn.is_finite() {
                return Err("angle is not finite".to_string());
            }
            let mut map = Transform::IDENTITY;
            // No turn: the axis is not looked at, so it need not be valid.
            if turn != 0.0 {
                let pivot = lengths(doc, variables, "pivot", &t.pivot)?;
                let axis = direction(variables, "axis", &t.axis)?;
                map = Transform::rotation_about(pivot, axis, turn)
                    .ok_or_else(|| degenerate("the rotation"))?;
            }
            Ok(vec![map.then(&Transform::translation(shift))])
        }
        FeatureKind::Mirror(m) => {
            let point = lengths(doc, variables, "point", &m.point)?;
            let normal = direction(variables, "normal", &m.normal)?;
            Ok(vec![
                Transform::mirror(point, normal).ok_or_else(|| degenerate("the mirror plane"))?,
            ])
        }
        FeatureKind::Pattern(p) => {
            let n = count(variables, &p.count)?;
            let mut maps = Vec::new();
            match &p.layout {
                PatternLayout::Linear { step } => {
                    let step = lengths(doc, variables, "step", step)?;
                    if n > 1 && step.length() <= doc.tolerance.linear {
                        return Err(format!(
                            "step is ({}, {}, {}): the instances would lie on top of each other",
                            step.x, step.y, step.z
                        ));
                    }
                    for i in 0..n {
                        let shift = step * i as f64;
                        if !shift.is_finite() {
                            return Err(degenerate("the step"));
                        }
                        maps.push(Transform::translation(shift));
                    }
                }
                PatternLayout::Circular {
                    point,
                    axis,
                    angle: step,
                } => {
                    let step = angle(doc, variables, "angle", step)?;
                    if !step.is_finite() {
                        return Err("angle is not finite".to_string());
                    }
                    if n > 1 {
                        if step.abs() <= doc.tolerance.angular {
                            return Err(
                                "angle is zero: the instances would lie on top of each other"
                                    .to_string(),
                            );
                        }
                        // Evenly round a full circle is `count * angle =
                        // 360 deg`; more than that and an instance lands
                        // on an earlier one.
                        let total = step.abs() * n as f64;
                        if total > TAU * (1.0 + 1e-12) {
                            return Err(format!(
                                "count times angle is {} deg, more than a full turn: instances would lie on top of each other",
                                total.to_degrees()
                            ));
                        }
                    }
                    let point = lengths(doc, variables, "point", point)?;
                    let axis = direction(variables, "axis", axis)?;
                    for i in 0..n {
                        maps.push(if i == 0 {
                            Transform::IDENTITY
                        } else {
                            Transform::rotation_about(point, axis, step * i as f64)
                                .ok_or_else(|| degenerate("the rotation"))?
                        });
                    }
                }
            }
            Ok(maps)
        }
        other => Err(format!("a {} is not made from a body", other.type_name())),
    }
}

/// The names of instance `instance`: every entity is the copy of the one
/// it is the image of.
fn copied(
    feature: FeatureId,
    instance: u64,
    names: &[PersistentName],
    out: &mut Vec<PersistentName>,
) -> Result<(), String> {
    for name in names {
        if name.depth() >= MAX_NAME_DEPTH {
            return Err(format!(
                "more than {MAX_NAME_DEPTH} features in a row each make their body from the one before"
            ));
        }
        out.push(PersistentName::copy(feature, instance, name.clone()));
    }
    Ok(())
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: &Feature,
    earlier: &[FeatureResult],
) -> Result<Derived, String> {
    let Some((source_id, _)) = feature.kind.source() else {
        return Err(format!(
            "a {} is not made from a body",
            feature.kind.type_name()
        ));
    };
    let source = source_body(doc, earlier, source_id)?;
    let Some(body) = &source.body else {
        return Err("source: it has no body".to_string());
    };
    let maps = instances(doc, variables, &feature.kind)?;
    if body.faces.len().saturating_mul(maps.len()) > MAX_PATTERN_FACES {
        return Err(format!(
            "{} instances of a body with {} faces are more than the {MAX_PATTERN_FACES} faces one body may have",
            maps.len(),
            body.faces.len()
        ));
    }
    let mut placed = Vec::with_capacity(maps.len());
    for map in &maps {
        placed.push(body.transformed(map).map_err(|e| e.to_string())?);
    }
    let merged = match placed.len() {
        1 => placed.pop(),
        _ => {
            let parts: Vec<&Body> = placed.iter().collect();
            Some(merge(&parts).map_err(|e| e.to_string())?.0)
        }
    };
    let Some(result) = merged else {
        return Err("there is no instance".to_string());
    };
    // `transformed` keeps the order of entities and `merge` puts the
    // bodies one after the other, so the names of instance i follow those
    // of instance i - 1 in the same order as in the source.
    let names = match &source.names {
        None => None,
        Some(names) => {
            let mut out = Names::default();
            for instance in 0..maps.len() as u64 {
                copied(feature.id, instance, &names.faces, &mut out.faces)?;
                copied(feature.id, instance, &names.edges, &mut out.edges)?;
                copied(feature.id, instance, &names.vertices, &mut out.vertices)?;
            }
            Some(out)
        }
    };
    Ok(Derived {
        body: result,
        names,
        instances: maps,
    })
}
