//! Blends: features that replace edges of a body by faces: a chamfer
//! (flat or conical faces) and a fillet (faces tangent to both their
//! neighbours).
//!
//! An edge is given as the two faces it lies between, by their persistent
//! names from `document.inspect` or `document.edges`. Whether the faces
//! exist, meet in an edge and leave room for the distance is found out by
//! regeneration, like every other parameter: the feature is stored and
//! reports `failed` with the reason.

use crate::commands::{Run, add_feature, fail, spec};
use crate::datum_cmds::between_value;
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    Chamfer, ChamferSize, Expr, FeatureKind, Fillet, MAX_CHAMFER_EDGES, MAX_FILLET_EDGES,
    PersistentName,
};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.chamfer", "Chamfer", r#"{"edges": [[face name, face name], ...], "distance": expr, "second": expr, "angle": expr, "chain": boolean = false, "name": string = automatic}"#,
        "Adds a feature that replaces edges of a body by flat or conical faces (feature.fillet makes rounded ones): the two faces next to an edge are cut back by a distance each, measured along the face from the edge, and a new face joins the two cut lines. An outside edge loses material, an inside edge gains it. Each edge is the two faces it lies between, by their names (document.edges lists every edge of a body with them under faces); all are edges of one body, which goes into this feature, and such a pair means every edge between those two faces (a name of an earlier body of the same chain, such as \"F1:face(0)\" after that body went into later features, is followed to the newest body before this feature). Three forms: distance alone (the same along both faces); distance and second (distance along the first face of each pair, second along the other); distance and angle (distance along the first face, and the new face leaves that face at angle). chain takes with each edge the edges that continue it smoothly (the whole rim of a slot from one of its edges). Works on straight edges between flat faces and on circular edges between a flat face, a cylinder or a cone about the circle's axis (the end of a rod, the mouth of a hole); other edges fail with their kind. Where three chamfered edges meet the corner is a flat triangle through the points where the cut lines cross; two meet in a mitre. A distance too large for a face fails with the edge, the face and the largest distance that fits. Faces keep their names with a prefix, \"F9:copy(0)/...\"; the new face of the edge at place k of edges is \"F9:chamfer(k)\" (\"F9:chamfer(k,0)\", \"F9:chamfer(k,1)\" when the pair gave several edges), a corner \"F9:chamferCorner(i,j,k)\". Change it with feature.edit: edges, distance, second, angle, chain.", doc: true, feature_chamfer),
    spec!("feature.fillet", "Fillet", r#"{"edges": [[face name, face name], ...], "radius": expr, "chain": boolean = false, "name": string = automatic}"#,
        "Adds a feature that rounds edges of a body with one radius: each edge is replaced by the face a ball of that radius touches as it rolls along the edge against both faces, which runs into both without a crease. An outside edge loses material, an inside edge gains it. Each edge is the two faces it lies between, by their names (document.edges lists every edge of a body with them under faces), in either order; all are edges of one body, which goes into this feature, and such a pair means every edge between those two faces (a name of an earlier body of the same chain, such as \"F1:face(0)\" after that body went into later features, is followed to the newest body before this feature). chain takes with each edge the edges that continue it smoothly (the whole rim of a slot from one of its edges). Works on straight edges between flat faces at any angle (the new face is a piece of a cylinder) and on circular edges between a flat face, a cylinder or a cone about the circle's axis (the end of a rod, the mouth of a hole, the foot of a boss: a piece of a torus); other edges fail with their kind. Where three straight filleted edges meet the corner is a piece of a ball of the same radius; two that meet at a corner whose third edge stays sharp run out into each other. Not built, and refused with the reason: three fillets at a corner with a circular edge or a fourth edge, an outside and an inside fillet that meet, a radius of half a rod's or more on its rim. A radius too large for a face fails with the edge, the face and the largest radius that fits. Faces keep their names with a prefix, \"F9:copy(0)/...\"; the new face of the edge at place k of edges is \"F9:fillet(k)\" (\"F9:fillet(k,0)\", \"F9:fillet(k,1)\" when the pair gave several edges or a chain), the ball in a corner \"F9:filletCorner(i,j,k)\". Change it with feature.edit: edges, radius, chain.", doc: true, feature_fillet),
];

/// The edges of a chamfer: pairs of face names.
pub(crate) fn edges_value(
    command: &str,
    value: &Value,
) -> Result<Vec<[PersistentName; 2]>, CommandError> {
    edges_of(command, value, "chamfer", MAX_CHAMFER_EDGES)
}

/// The edges of a blend (`noun`: a chamfer or a fillet): pairs of face
/// names, `most` at most.
fn edges_of(
    command: &str,
    value: &Value,
    noun: &str,
    most: usize,
) -> Result<Vec<[PersistentName; 2]>, CommandError> {
    let Some(items) = value.as_array() else {
        return fail(format!(
            "{command}: `edges` must be a list of edges, each two face names [face, face] (document.edges lists them under faces)"
        ));
    };
    if items.is_empty() {
        return fail(format!(
            "{command}: `edges` is empty; give at least one edge"
        ));
    }
    if items.len() > most {
        return fail(format!(
            "{command}: `edges` has {} edges; one {noun} takes {most} at most",
            items.len()
        ));
    }
    let mut edges: Vec<[PersistentName; 2]> = Vec::with_capacity(items.len());
    for item in items {
        let pair = between_value(command, "edges", item)?;
        let [a, b] = &pair;
        if a == b {
            return fail(format!(
                "{command}: `edges`: {a} is given twice; an edge lies between two different faces"
            ));
        }
        let again = edges
            .iter()
            .any(|[c, d]| (c == a && d == b) || (c == b && d == a));
        if again {
            return fail(format!(
                "{command}: `edges`: the edge between {a} and {b} is given twice"
            ));
        }
        // Faces named after different features can be faces of one body:
        // a boolean's operands, or a body before and after a blend, whose
        // names regeneration follows to the newest body (regen `follow`).
        // Whether they are is found out there, like every other reference.
        edges.push(pair);
    }
    Ok(edges)
}

/// Faces of one blend or face operation may be named after different
/// features when those are faces of one body: the operands of a boolean,
/// a body before and after a blend, whose names regeneration follows to
/// the newest body (regen `follow`). Names whose bodies never came
/// together (as the document stands) are refused at once.
pub(crate) fn of_one_body(
    command: &str,
    session: &mut Session,
    kind: &FeatureKind,
) -> Result<(), CommandError> {
    let (key, noun, names): (&str, &str, Vec<&PersistentName>) = match kind {
        FeatureKind::Chamfer(c) => ("edges", "chamfer", c.edges.iter().flatten().collect()),
        FeatureKind::Fillet(f) => ("edges", "fillet", f.edges.iter().flatten().collect()),
        FeatureKind::OffsetFace(o) => ("faces", "offset", o.faces.iter().collect()),
        FeatureKind::DeleteFace(d) => ("faces", "deletion", d.faces.iter().collect()),
        FeatureKind::MoveFace(m) => ("faces", "move", m.faces.iter().collect()),
        FeatureKind::Thicken(t) => ("faces", "layer", t.faces.iter().collect()),
        FeatureKind::Shell(s) => ("open", "shell", s.open.iter().collect()),
        _ => return Ok(()),
    };
    let Some(first) = names.first() else {
        return Ok(());
    };
    if names.iter().all(|name| name.feature == first.feature) {
        return Ok(());
    }
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let newest = |name: &PersistentName| craft3d_regen::newest_body(&regen.features, name.feature);
    let body = newest(first);
    let Some(other) = names.iter().find(|name| newest(name) != body) else {
        return Ok(());
    };
    let called = |id| {
        doc.feature(id).map_or_else(
            || format!("{id}, which this document does not have"),
            |f| format!("`{}`", f.name),
        )
    };
    fail(format!(
        "{command}: `{key}`: {first} is a face of {} and {other} of {}, which are not one body; the faces of one {noun} are of one body",
        called(first.feature),
        called(other.feature)
    ))
}

/// The size of a chamfer from `distance`, `second` and `angle`, on top of
/// `current` when there is one: what is not given stays, and the form is
/// the one the given parameters name.
pub(crate) fn size_value(
    p: &Params,
    command: &str,
    current: Option<&ChamferSize>,
) -> Result<Option<ChamferSize>, CommandError> {
    let distance = p.expr("distance")?;
    let second = p.expr("second")?;
    let angle = p.expr("angle")?;
    if second.is_some() && angle.is_some() {
        return fail(format!(
            "{command}: give `second` (two distances) or `angle` (a distance and an angle), not both"
        ));
    }
    let kept = current.map(|size| match size {
        ChamferSize::Equal { distance }
        | ChamferSize::TwoDistances { distance, .. }
        | ChamferSize::DistanceAngle { distance, .. } => distance.clone(),
    });
    let Some(first) = distance.map(Expr::new).or(kept) else {
        return Err(p.missing("distance"));
    };
    Ok(match (second, angle, current) {
        (Some(second), _, _) => Some(ChamferSize::TwoDistances {
            distance: first,
            second: Expr::new(second),
        }),
        (_, Some(angle), _) => Some(ChamferSize::DistanceAngle {
            distance: first,
            angle: Expr::new(angle),
        }),
        (None, None, None) => Some(ChamferSize::Equal { distance: first }),
        (None, None, Some(_)) if !p.has("distance") => None,
        (None, None, Some(size)) => Some(match size {
            ChamferSize::Equal { .. } => ChamferSize::Equal { distance: first },
            ChamferSize::TwoDistances { second, .. } => ChamferSize::TwoDistances {
                distance: first,
                second: second.clone(),
            },
            ChamferSize::DistanceAngle { angle, .. } => ChamferSize::DistanceAngle {
                distance: first,
                angle: angle.clone(),
            },
        }),
    })
}

/// `feature.edit` for a chamfer: `edges`, `distance`, `second`, `angle`
/// (`chain` is a plain flag, set where the others are).
pub(crate) fn edit(p: &Params, chamfer: &mut Chamfer) -> Result<(), CommandError> {
    const ID: &str = "feature.edit";
    if let Some(value) = p.get("edges") {
        chamfer.edges = edges_value(ID, value)?;
    }
    if let Some(size) = size_value(p, ID, Some(&chamfer.size))? {
        chamfer.size = size;
    }
    Ok(())
}

fn feature_chamfer(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.chamfer";
    let p = Params::new(
        ID,
        params,
        &["edges", "distance", "second", "angle", "chain", "name"],
    )?;
    // The document has to be there before anything is said of the
    // parameters.
    session.document()?;
    let Some(value) = p.get("edges") else {
        return Err(p.missing("edges"));
    };
    let edges = edges_value(ID, value)?;
    let Some(size) = size_value(&p, ID, None)? else {
        return Err(p.missing("distance"));
    };
    let chain = p.bool("chain")?.unwrap_or(false);
    let kind = FeatureKind::Chamfer(Chamfer { edges, size, chain });
    of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
}

/// `feature.edit` for a fillet: `edges` (`radius` is a plain expression
/// and `chain` a plain flag, set where the others are).
pub(crate) fn edit_fillet(p: &Params, fillet: &mut Fillet) -> Result<(), CommandError> {
    if let Some(value) = p.get("edges") {
        fillet.edges = edges_of("feature.edit", value, "fillet", MAX_FILLET_EDGES)?;
    }
    Ok(())
}

fn feature_fillet(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.fillet";
    let p = Params::new(ID, params, &["edges", "radius", "chain", "name"])?;
    // The document has to be there before anything is said of the
    // parameters.
    session.document()?;
    let Some(value) = p.get("edges") else {
        return Err(p.missing("edges"));
    };
    let edges = edges_of(ID, value, "fillet", MAX_FILLET_EDGES)?;
    let Some(radius) = p.expr("radius")? else {
        return Err(p.missing("radius"));
    };
    let chain = p.bool("chain")?.unwrap_or(false);
    let kind = FeatureKind::Fillet(Fillet {
        edges,
        radius: Expr::new(radius),
        chain,
    });
    of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
}
