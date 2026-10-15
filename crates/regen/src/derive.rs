//! Features that make their body out of the body of an earlier feature:
//! a transform, a mirror, a pattern.
//!
//! The source feature's result stays in the list (names still resolve
//! against it); it is marked as taken, so its body is no longer one of the
//! document's bodies, unless the feature keeps it.

use crate::cause::FIX_IT_FIRST;
use crate::{Contributors, FeatureResult, Kept, Names, Status, angle, length, number};
use craft3d_doc::{Document, Expr, Feature, FeatureId, FeatureKind, PatternLayout, PersistentName};
use craft3d_expr::Evaluation;
use craft3d_math::{Transform, Vec3};
use craft3d_topo::Body;
use std::f64::consts::TAU;

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
pub(crate) fn source_body<'a>(
    doc: &Document,
    earlier: &'a [FeatureResult],
    id: FeatureId,
    role: &str,
) -> Result<&'a FeatureResult, String> {
    let Some(source) = doc.feature(id) else {
        return Err(format!(
            "{role}: there is no feature {id} (was it deleted?)"
        ));
    };
    let name = &source.name;
    let Some(outcome) = earlier.iter().find(|r| r.id == id) else {
        return Err(format!(
            "{role}: `{name}` {}",
            crate::comes_after(doc, earlier, id)
        ));
    };
    match &outcome.status {
        Status::Ok => {}
        Status::Suppressed => return Err(format!("{role}: `{name}` is suppressed")),
        Status::RolledBack => return Err(format!("{role}: `{name}` is rolled back")),
        Status::Failed(_) => return Err(format!("{role}: `{name}` {FIX_IT_FIRST}")),
    }
    if outcome.body.is_none() {
        return Err(format!(
            "{role}: `{name}` is a {} and has no body",
            source.kind.type_name()
        ));
    }
    if let Some(taker) = outcome.consumed_by {
        let taker = doc
            .feature(taker)
            .map_or_else(|| taker.to_string(), |f| f.name.clone());
        return Err(format!(
            "{role}: the body of `{name}` already went into `{taker}`; use that feature as the source, or give `{taker}` keep"
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
    earlier: &[FeatureResult],
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
            // In a datum plane, or in the plane through a point with a
            // normal.
            let (point, normal) = match m.plane {
                Some(plane) => {
                    let frame = crate::datum::datum_plane(doc, earlier, plane, "plane")?;
                    (frame.origin(), frame.z())
                }
                None => (
                    lengths(doc, variables, "point", &m.point)?,
                    direction(variables, "normal", &m.normal)?,
                ),
            };
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
                    about,
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
                    // About a datum axis, or about the line through a
                    // point along a direction.
                    let (point, axis) = match about {
                        Some(about) => {
                            let line = crate::datum::datum_axis(doc, earlier, *about, "axis")?;
                            (line.origin, line.direction)
                        }
                        None => (
                            lengths(doc, variables, "point", point)?,
                            direction(variables, "axis", axis)?,
                        ),
                    };
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
pub(crate) fn copied(
    feature: FeatureId,
    instance: u64,
    names: &[PersistentName],
    out: &mut Vec<PersistentName>,
) -> Result<(), String> {
    for name in names {
        let copy = PersistentName::copy(feature, instance, name.clone());
        copy.check().map_err(crate::too_long)?;
        out.push(copy);
    }
    Ok(())
}

/// The aliases of instance `instance`, whose entities start at `first`
/// in the body: the copies of the aliases of the entities they are the
/// images of.
pub(crate) fn copied_aliases(
    feature: FeatureId,
    instance: u64,
    aliases: &std::collections::BTreeMap<usize, Vec<PersistentName>>,
    first: usize,
    out: &mut std::collections::BTreeMap<usize, Vec<PersistentName>>,
) {
    for (index, list) in aliases {
        let copies: Vec<PersistentName> = list
            .iter()
            .map(|name| PersistentName::copy(feature, instance, name.clone()))
            .filter(|copy| copy.check().is_ok())
            .collect();
        if !copies.is_empty() {
            out.insert(first.saturating_add(*index), copies);
        }
    }
}

/// Whether any two instances of a pattern come near each other: touch or
/// share volume, as far as their boxes can tell.
///
/// Every instance is the first one moved by a multiple of one step, so
/// two instances are near exactly when the first is near some other one:
/// it is enough to look at the first against each of the others.
fn instances_meet(doc: &Document, placed: &[Body]) -> Result<bool, String> {
    let Some((first, others)) = placed.split_first() else {
        return Ok(false);
    };
    let slack = doc.tolerance.linear;
    let corners = |body: &Body| {
        body.bbox()
            .map_err(|e| e.to_string())
            .map(|bbox| bbox.corners())
    };
    let Some((lo, hi)) = corners(first)? else {
        return Ok(false);
    };
    for other in others {
        let Some((other_lo, other_hi)) = corners(other)? else {
            continue;
        };
        let apart =
            |a_lo: f64, a_hi: f64, b_lo: f64, b_hi: f64| a_hi < b_lo - slack || b_hi < a_lo - slack;
        if !(apart(lo.x, hi.x, other_lo.x, other_hi.x)
            || apart(lo.y, hi.y, other_lo.y, other_hi.y)
            || apart(lo.z, hi.z, other_lo.z, other_hi.z))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The instances joined into one body by the kernel's union, one after
/// the other: where they touch or overlap they become one piece, and no
/// face is left inside the material. Returns the body and, for each of
/// its faces, the instances and the faces of the source it has a piece
/// of: one pair for a face that is a piece of one instance's face, more
/// for a face the union merged from the faces of several (three bars end
/// to end: one top), the one it is named after first; and for each of its
/// edges and vertices the instance and the entity of the source it is,
/// when it is one through every union.
fn joined(doc: &Document, placed: &[Body]) -> Result<(Body, Contributors, Kept, Kept), String> {
    use craft3d_kernel::boolean::{
        BooleanOp, EdgeSource, FaceSource, Operand, VertexSource, boolean,
    };
    let tolerance = craft3d_math::Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let Some((first, others)) = placed.split_first() else {
        return Err("there is no instance".to_string());
    };
    let mut body = first.clone();
    let mut sources: Contributors = (0..body.faces.len()).map(|face| vec![(0, face)]).collect();
    let mut edges: Kept = (0..body.edges.len()).map(|edge| Some((0, edge))).collect();
    let mut vertices: Kept = (0..body.vertices.len()).map(|v| Some((0, v))).collect();
    // An entity of the body so far is what it already was; one of the new
    // instance is its own; one the union made is nobody's.
    let through = |so_far: &Kept, operand: Operand, index: u32, instance: u64| match operand {
        Operand::A => so_far.get(index as usize).copied().flatten(),
        Operand::B => Some((instance, index as usize)),
    };
    for (index, other) in others.iter().enumerate() {
        let instance = index.saturating_add(1) as u64;
        let result = boolean(&body, other, BooleanOp::Union, &tolerance).map_err(|e| {
            format!("instance {instance} touches or overlaps the ones before it and cannot be joined to them: {e}")
        })?;
        let mut next = Contributors::with_capacity(result.faces.len());
        for (face, source) in result.faces.iter().enumerate() {
            // A face of the body so far brings every instance it already
            // had a piece of; a face of the new instance is its own. The
            // kernel's source comes first, then what it merged in.
            let mut pairs = Vec::new();
            let mut add = |from: &FaceSource| -> Result<(), String> {
                let face = from.face.0 as usize;
                let earlier: &[(u64, usize)] = match from.operand {
                    Operand::A => sources
                        .get(face)
                        .ok_or("the union lost track of a face".to_string())?,
                    Operand::B => &[(instance, face)],
                };
                for pair in earlier {
                    if !pairs.contains(pair) {
                        pairs.push(*pair);
                    }
                }
                Ok(())
            };
            add(source)?;
            for from in result.merged.get(face).into_iter().flatten() {
                add(from)?;
            }
            next.push(pairs);
        }
        edges = result
            .edges
            .iter()
            .map(|source| match source {
                EdgeSource::Kept { operand, edge } => through(&edges, *operand, edge.0, instance),
                EdgeSource::Intersection { .. } => None,
            })
            .collect();
        vertices = result
            .vertices
            .iter()
            .map(|source| match source {
                VertexSource::Kept { operand, vertex } => {
                    through(&vertices, *operand, vertex.0, instance)
                }
                VertexSource::New { .. } => None,
            })
            .collect();
        body = result.body;
        sources = next;
    }
    Ok((body, sources, edges, vertices))
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
    let source = source_body(doc, earlier, source_id, "source")?;
    let Some(body) = &source.body else {
        return Err("source: it has no body".to_string());
    };
    let maps = instances(doc, variables, &feature.kind, earlier)?;
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
    // A pattern whose instances touch or overlap is joined by the kernel:
    // put side by side in one body they would leave faces inside the
    // material, a solid that passes through itself.
    if matches!(feature.kind, FeatureKind::Pattern(_)) && instances_meet(doc, &placed)? {
        let (result, contributors, edges, vertices) = joined(doc, &placed)?;
        // Every instance is a copy of the one source. As after a boolean,
        // edges and vertices are named by the faces they lie on (and keep
        // the name they have as a copy when the union left them whole),
        // and a face merged from the faces of several instances answers
        // to each instance's name for it.
        let mut names =
            Names::copied_faces(feature.id, &contributors, |_| source.names.as_deref())?;
        if let Some(names) = &mut names {
            names.name_entities(
                feature.id,
                &result,
                doc.tolerance.linear,
                &edges,
                &vertices,
                |_| source.names.as_deref(),
            );
        }
        // Each instance is the source moved, which does not pass through
        // itself: what is looked at is where instances meet.
        let roles = crate::selfcheck::roles_of(&contributors, &[]);
        let faces = names
            .as_ref()
            .map_or(&[][..], |names| names.faces.as_slice());
        crate::selfcheck::crossing_near(&result, faces, doc.tolerance.linear, &roles).refused(
            "count",
            "pattern",
            "the union of the instances did not find where they meet there, and the result is refused rather than handed on",
        )?;
        return Ok(Derived {
            body: result,
            names,
            instances: maps,
        });
    }
    let merged = match placed.len() {
        1 => placed.pop(),
        _ => {
            let parts: Vec<&Body> = placed.iter().collect();
            Some(craft3d_topo::merge(&parts).map_err(|e| e.to_string())?.0)
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
                // What an entity answers to besides its own name, its
                // copy answers to as a copy.
                copied_aliases(
                    feature.id,
                    instance,
                    &names.aliases,
                    out.faces.len(),
                    &mut out.aliases,
                );
                copied_aliases(
                    feature.id,
                    instance,
                    &names.edge_aliases,
                    out.edges.len(),
                    &mut out.edge_aliases,
                );
                copied_aliases(
                    feature.id,
                    instance,
                    &names.vertex_aliases,
                    out.vertices.len(),
                    &mut out.vertex_aliases,
                );
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
