//! Blends: features that replace edges of a body by faces: a chamfer
//! (flat or conical faces) and a fillet (faces tangent to both their
//! neighbours).
//!
//! An edge is given as the two faces it lies between, by their persistent
//! names from `document.inspect` or `document.edges` (every edge between
//! them), or by its own name from `document.edges` or `view.pick` (that
//! one edge), followed by the face its first distance is measured along
//! when that matters. Whether the faces and edges exist, meet and leave
//! room for the distance is found out by regeneration, like every other
//! parameter: the feature is stored and reports `failed` with the reason.

use crate::commands::{Run, add_feature, fail, spec};
use crate::datum_cmds::{between_value, face_value, name_value};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::identity;
use craft3d_doc::{
    Chamfer, ChamferSize, EdgeRef, Expr, FeatureKind, Fillet, FilletFade, MAX_CHAMFER_EDGES,
    MAX_FILLET_EDGES, MAX_FILLET_FADES, PersistentName,
};
use serde_json::Value;
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.chamfer", "Chamfer", r#"{"edges": [[face name, face name] or edge name or [edge name, face name], ...], "distance": expr, "second": expr, "angle": expr, "chain": boolean = false, "name": string = automatic, "when": expr = always on}"#,
        "Adds a feature that replaces edges of a body by flat or conical faces (feature.fillet makes rounded ones): the two faces next to an edge are cut back by a distance each, measured along the face from the edge, and a new face joins the two cut lines. An outside edge loses material, an inside edge gains it. Each edge is the two faces it lies between, by their names (document.edges lists every edge of a body with them under faces), and such a pair means every edge between those two faces; or one edge by its own name (document.edges lists it under name, view.pick gives it), which picks one of several edges between the same two faces, written [edge name, face name] to say which of its faces the first distance is measured along (needed with second or angle); a name is taken only when every part of it is of a kind proven to stay with its edge (a primitive's or an import's entities, a sweep's by sketch entities, copies, names by faces with no number at the end); any other (a number after its brackets, \"...](1)\", a blend's face such as \"fillet(0,1)\" or \"chamfer(0)\", a glyph of a text, a developable's face, a plate's rim, a lump a split chose by place) is refused, since an edit could move it to another edge, and the refusal says what names that edge instead. All are edges of one body, which goes into this feature; a name of an earlier body of the same chain, such as \"F1:face(0)\" after that body went into later features, is followed to the newest body before this feature, an edge's name to the edge each later feature kept of it. Three forms: distance alone (the same along both faces); distance and second (distance along the first face of each pair, second along the other); distance and angle (distance along the first face, and the new face leaves that face at angle). chain takes with each edge the edges that continue it smoothly (the whole rim of a slot from one of its edges). Works on straight edges between flat faces, or between a flat face and a cylinder along the edge (the cut edge of a halved tube), and on circular edges between a flat face, a cylinder or a cone about the circle's axis (the end of a rod, the mouth of a hole); any other edge (next to a cone, between lofted or spline faces) gets a ruled spline face within the document's tolerance when each of its ends is a corner of three faces (or it is closed, as the mouth of a slanted hole) and no other chamfered edge ends there, and fails with the reason otherwise. Where three chamfered edges meet the corner is a flat triangle through the points where the cut lines cross; two meet in a mitre. A distance too large for a face fails with the edge, the face and the distances shown not to fit (from one up to the one asked, one just below fitting). Faces keep their names with a prefix, \"F9:copy(0)/...\"; the new face of the edge at place k of edges is \"F9:chamfer(k)\" (\"F9:chamfer(k,0)\", \"F9:chamfer(k,1)\" when the pair gave several edges), a corner \"F9:chamferCorner(i,j,k)\". Change it with feature.edit: edges, distance, second, angle, chain.", doc: true, names: ["edges"], feature_chamfer),
    spec!("feature.fillet", "Fillet", r#"{"edges": [[face name, face name] or edge name or [edge name, face name], ...], "radius": expr, "chain": boolean = false, "fade": [{at: vertex name, over: expr, back: expr = 0}, ...] = none, "name": string = automatic, "when": expr = always on}"#,
        "Adds a feature that rounds edges of a body with one radius: each edge is replaced by the face a ball of that radius touches as it rolls along the edge against both faces, which runs into both without a crease. An outside edge loses material, an inside edge gains it. Each edge is the two faces it lies between, by their names (document.edges lists every edge of a body with them under faces), in either order, and such a pair means every edge between those two faces; or one edge by its own name (document.edges lists it under name, view.pick gives it), which picks one of several edges between the same two faces ([edge name, face name], as feature.chamfer takes it, is the same edge: the face must be one of its two and says nothing more here); a name is taken only when every part of it is of a kind proven to stay with its edge (a primitive's or an import's entities, a sweep's by sketch entities, copies, names by faces with no number at the end); any other (a number after its brackets, \"...](1)\", a blend's face such as \"fillet(0,1)\" or \"chamfer(0)\", a glyph of a text, a developable's face, a plate's rim, a lump a split chose by place) is refused, since an edit could move it to another edge, and the refusal says what names that edge instead. All are edges of one body, which goes into this feature; a name of an earlier body of the same chain, such as \"F1:face(0)\" after that body went into later features, is followed to the newest body before this feature, an edge's name to the edge each later feature kept of it. chain takes with each edge the edges that continue it smoothly (the whole rim of a slot from one of its edges). Works on straight edges between flat faces at any angle, or next to a cylinder along the edge (the cut edge of a halved tube, the tip of a two-arc blade; the new face is a piece of a cylinder), and on circular edges between a flat face, a cylinder or a cone about the circle's axis (the end of a rod, the mouth of a hole, the foot of a boss: a piece of a torus); any other edge (next to a cone, between lofted or spline faces) gets a spline face within the document's tolerance, tangent to both faces, when each of its ends is a corner of three faces (or it is closed, as the mouth of a slanted hole) and no other filleted edge ends there, and fails with the reason otherwise. Where three straight filleted edges meet the corner is a piece of a ball of the same radius; two that meet at a corner whose third edge stays sharp run out into each other. Not built, and refused with the reason: three fillets at a corner with a circular edge or a fourth edge, an outside and an inside fillet that meet, a radius of half a rod's or more on its rim. A radius too large for a face fails with the edge, the face and the radii shown not to fit (from one up to the one asked, one just below fitting). Faces keep their names with a prefix, \"F9:copy(0)/...\"; the new face of the edge at place k of edges is \"F9:fillet(k)\" (\"F9:fillet(k,0)\", \"F9:fillet(k,1)\" when the pair gave several edges or a chain), the ball in a corner \"F9:filletCorner(i,j,k)\". fade ends the fillet in a point on its edge (a hull's stem rounded from the deck down and dying out above the forefoot): per end, at is the vertex there by its name (document.edges lists the vertices of a body), over the length along the edge over which the radius falls smoothly to nothing, back how far short of the vertex the face closes in its point (0: in the vertex; the edge beyond stays sharp); the face stays tangent to both faces all the way. A faded edge is rounded on its own: an open edge, no chain, no other rounded edge ending in its vertices, its other end a corner of three faces or a fade, over at least the radius (the least that builds is said otherwise); the result lists under fades each fade's tip (the point), vertex (its name), over, back and the edge's length. Change it with feature.edit: edges, radius, chain, fade (null for one radius all along).", doc: true, names: ["edges", "fade"], feature_fillet),
];

/// The edges of a chamfer: pairs of face names, or edges by their names.
pub(crate) fn edges_value(command: &str, value: &Value) -> Result<Vec<EdgeRef>, CommandError> {
    edges_of(command, value, "chamfer", MAX_CHAMFER_EDGES)
}

/// The forms an edge of a blend is given in, for what is refused.
const EDGE_FORMS: &str = "each edge is two face names [face, face] (every edge between them; document.edges lists them under faces), or one edge's own name \"edge\" (document.edges lists it under name, view.pick gives it), or [edge, face]: that edge, its first distance measured along that face";

/// One edge of a blend: `[face, face]`, `"edge"` or `["edge", "face"]`
/// (the first name of a list says which).
fn edge_value(command: &str, value: &Value) -> Result<EdgeRef, CommandError> {
    match value {
        Value::String(_) => {
            let edge = name_value(command, "edges", value, "an edge's name")?;
            if !edge.is_edge() {
                return fail(format!(
                    "{command}: `edges`: {edge} is not an edge; {EDGE_FORMS}"
                ));
            }
            Ok(EdgeRef::Edge { edge, first: None })
        }
        Value::Array(items) => {
            let [lead, second] = items.as_slice() else {
                return fail(format!(
                    "{command}: `edges` holds a list of {} names; {EDGE_FORMS}",
                    items.len()
                ));
            };
            let lead = name_value(command, "edges", lead, "a face or an edge name")?;
            if !lead.is_edge() {
                return Ok(EdgeRef::Faces(between_value(command, "edges", value)?));
            }
            Ok(EdgeRef::Edge {
                edge: lead,
                first: Some(face_value(command, "edges", second)?),
            })
        }
        _ => fail(format!(
            "{command}: `edges` must be a list of edges; {EDGE_FORMS}"
        )),
    }
}

/// The edges of a blend (`noun`: a chamfer or a fillet): pairs of face
/// names or edges by their own names, `most` at most.
fn edges_of(
    command: &str,
    value: &Value,
    noun: &str,
    most: usize,
) -> Result<Vec<EdgeRef>, CommandError> {
    let Some(items) = value.as_array() else {
        return fail(format!(
            "{command}: `edges` must be a list of edges; {EDGE_FORMS}"
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
    let mut edges: Vec<EdgeRef> = Vec::with_capacity(items.len());
    for item in items {
        let edge = edge_value(command, item)?;
        if let EdgeRef::Faces([a, b]) = &edge
            && a == b
        {
            return fail(format!(
                "{command}: `edges`: {a} is given twice; an edge lies between two different faces"
            ));
        }
        if edges.iter().any(|earlier| earlier.same_as(&edge)) {
            return fail(format!(
                "{command}: `edges`: {} is given twice",
                edge.describe()
            ));
        }
        // Faces and edges named after different features can be of one
        // body: a boolean's operands, or a body before and after a blend,
        // whose names regeneration follows to the newest body (regen
        // `follow`). Whether they are is found out there, like every
        // other reference; so is whether an edge given by its name is
        // also one of the edges a pair gives.
        edges.push(edge);
    }
    Ok(edges)
}

/// An edge of a chamfer or a fillet given by a name with anything not
/// proven to identify its entity (`craft3d_doc::identity::unproven`: a
/// part of a kind not on the list, at any depth, or a split by place its
/// bodies grew from), or with a face so named, is refused at once, as
/// regeneration refuses it in a file: an edit could move it to another
/// entity without a failure. The reason says what names that edge
/// instead, from the body the name is of (`craft3d_regen::numbered_edge`).
/// A pair is taken as it always was.
pub(crate) fn numbered_refused(
    command: &str,
    session: &mut Session,
    kind: &FeatureKind,
) -> Result<(), CommandError> {
    let (noun, edges) = match kind {
        FeatureKind::Chamfer(c) => ("chamfer", &c.edges),
        FeatureKind::Fillet(f) => ("fillet", &f.edges),
        _ => return Ok(()),
    };
    let open = session.document_mut()?;
    let doc = Arc::clone(open.history.current());
    // Reference by reference, the edge before the face given with it, as
    // regeneration says it.
    let mut found = None;
    for (index, reference) in edges.iter().enumerate() {
        let EdgeRef::Edge { edge, first } = reference else {
            continue;
        };
        if identity::unproven(&doc, edge).is_some() {
            found = Some((index, edge));
            break;
        }
        if let Some(face) = first
            .as_ref()
            .filter(|face| identity::unproven(&doc, face).is_some())
        {
            return fail(format!(
                "{command}: `edges.{index}`: {}",
                craft3d_regen::numbered_face(&doc, face, noun)
            ));
        }
    }
    // A vertex a fillet fades at, the same way: an edit could move the
    // fade to another end.
    if found.is_none()
        && let FeatureKind::Fillet(fillet) = kind
        && let Some((index, fade)) = fillet
            .fade
            .iter()
            .enumerate()
            .find(|(_, fade)| identity::unproven(&doc, &fade.at).is_some())
        && let Some(why) = identity::unproven(&doc, &fade.at)
    {
        return fail(format!(
            "{command}: `fade.{index}.at`: {}; a fillet does not fade at it: give the vertex by a name that is proven (document.edges lists the vertices of a body), or leave the fade out",
            why.reason(&doc, &fade.at)
        ));
    }
    let Some((index, name)) = found else {
        return Ok(());
    };
    let regen = open.regen_needing(craft3d_regen::Need::These(&[name.feature]));
    let at = regen
        .feature(name.feature)
        .and_then(|r| Some((r.body.as_deref()?, r.names.as_deref()?)));
    fail(format!(
        "{command}: `edges.{index}`: {}",
        craft3d_regen::numbered_edge(&doc, name, noun, at)
    ))
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
        FeatureKind::Chamfer(c) => (
            "edges",
            "chamfer",
            c.edges.iter().flat_map(EdgeRef::names).collect(),
        ),
        FeatureKind::Fillet(f) => (
            "edges",
            "fillet",
            f.edges.iter().flat_map(EdgeRef::names).collect(),
        ),
        FeatureKind::OffsetFace(o) => ("faces", "offset", o.faces.iter().collect()),
        FeatureKind::DeleteFace(d) => ("faces", "deletion", d.faces.iter().collect()),
        FeatureKind::MoveFace(m) => ("faces", "move", m.faces.iter().collect()),
        FeatureKind::Thicken(t) => ("faces", "layer", t.faces.iter().collect()),
        FeatureKind::Plate(p) => ("faces", "plate", p.faces.iter().collect()),
        FeatureKind::Draft(d) => ("faces", "draft", d.faces.iter().collect()),
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
    // The bodies an edge's way can end in (`edge_way`, the ones no later
    // feature took): one, unless the way branches into the splits of a
    // body. With one, that is the body the edge is of, gone or not, and
    // nothing is followed. Following into splits reads the names of the
    // edges of the bodies on the way, which go with a body the cache let
    // go of: only then are those bodies derived again (regeneration
    // derives them anyway when it builds the feature), so the answer is
    // the one a full cache gives.
    let ends = |regen: &craft3d_regen::Regen, way: &[craft3d_doc::FeatureId]| {
        way.iter()
            .copied()
            .filter(|id| {
                regen
                    .features
                    .iter()
                    .find(|r| r.id == *id)
                    .is_some_and(|r| r.consumed_by.is_none())
            })
            .collect::<Vec<_>>()
    };
    let branching: Vec<craft3d_doc::FeatureId> = {
        let regen = open.regen();
        let doc = open.history.current();
        let mut ways: Vec<craft3d_doc::FeatureId> = names
            .iter()
            .filter(|name| name.is_edge())
            .map(|name| craft3d_regen::edge_way(doc, &regen.features, name))
            .filter(|way| ends(&regen, way).len() > 1)
            .flatten()
            .collect();
        ways.sort_unstable();
        ways.dedup();
        ways
    };
    let regen = if branching.is_empty() {
        open.regen()
    } else {
        open.regen_needing(craft3d_regen::Need::These(&branching))
    };
    let doc = open.history.current();
    // The body a face or an edge is of now: where the name follows to
    // (after a split, the one split that has it); where it cannot be
    // followed, the last feature the body went into, and regeneration
    // says what is wrong.
    let newest = |name: &PersistentName| {
        if name.is_edge() {
            let way = craft3d_regen::edge_way(doc, &regen.features, name);
            if let [only] = ends(&regen, &way).as_slice() {
                return *only;
            }
        }
        let followed = if name.is_edge() {
            craft3d_regen::follow_edge(doc, &regen.features, name, key)
        } else {
            craft3d_regen::follow_face(doc, &regen.features, name, key)
        };
        followed.ok().flatten().map_or_else(
            || craft3d_regen::newest_body(&regen.features, name.feature),
            |full| full.feature,
        )
    };
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
    let what = |name: &PersistentName| if name.is_edge() { "an edge" } else { "a face" };
    let all = if key == "edges" {
        "faces and edges"
    } else {
        "faces"
    };
    fail(format!(
        "{command}: `{key}`: {first} is {} of {} and {other} {} of {}, which are not one body; the {all} of one {noun} are of one body",
        what(first),
        called(first.feature),
        what(other),
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
    numbered_refused(ID, session, &kind)?;
    of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
}

/// What a fade is given as, for what is refused.
const FADE_FORM: &str = "each fade is {\"at\": the vertex at the end of an edge the fillet rounds, by its name (document.edges lists the vertices of a body), \"over\": the length along the edge over which the radius falls to nothing, \"back\": how far short of the vertex the fillet ends in its point (0 when left out)}";

/// A length of a fade: an expression, a string or a finite number.
fn fade_length(command: &str, key: &str, value: &Value) -> Result<Expr, CommandError> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Ok(Expr::new(text.clone())),
        Value::Number(number) => match number.as_f64().filter(|x| x.is_finite()) {
            Some(x) => Ok(Expr::new(x.to_string())),
            None => fail(format!(
                "{command}: `{key}` must be an expression, such as \"15 mm\""
            )),
        },
        _ => fail(format!(
            "{command}: `{key}` must be an expression, such as \"15 mm\""
        )),
    }
}

/// The fades of a fillet: a list of `{at, over, back}`, `null` or `[]`
/// for none. Each `at` a vertex's name, given once.
pub(crate) fn fade_value(command: &str, value: &Value) -> Result<Vec<FilletFade>, CommandError> {
    let items = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(items) => items,
        _ => {
            return fail(format!(
                "{command}: `fade` must be a list of fades; {FADE_FORM}"
            ));
        }
    };
    if items.len() > MAX_FILLET_FADES {
        return fail(format!(
            "{command}: `fade` has {} fades; one fillet takes {MAX_FILLET_FADES} at most (one per end of an edge)",
            items.len()
        ));
    }
    let mut out: Vec<FilletFade> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let key = |field: &str| format!("fade.{index}.{field}");
        let Value::Object(map) = item else {
            return fail(format!(
                "{command}: `fade.{index}` must be an object; {FADE_FORM}"
            ));
        };
        if let Some(other) = map
            .keys()
            .find(|k| !matches!(k.as_str(), "at" | "over" | "back"))
        {
            return fail(format!(
                "{command}: `fade.{index}` has no parameter `{other}`; {FADE_FORM}"
            ));
        }
        let Some(at) = map.get("at").filter(|v| !v.is_null()) else {
            return fail(format!("{command}: `fade.{index}` needs `at`; {FADE_FORM}"));
        };
        let at = name_value(command, &key("at"), at, "a vertex's name")?;
        if !at.is_vertex() {
            return fail(format!(
                "{command}: `{}`: {at} is not a vertex; a fade is at a vertex that ends an edge the fillet rounds (document.edges lists the vertices of a body under vertices)",
                key("at")
            ));
        }
        let Some(over) = map.get("over").filter(|v| !v.is_null()) else {
            return fail(format!(
                "{command}: `fade.{index}` needs `over`; {FADE_FORM}"
            ));
        };
        let over = fade_length(command, &key("over"), over)?;
        let back = match map.get("back").filter(|v| !v.is_null()) {
            Some(back) => Some(fade_length(command, &key("back"), back)?),
            None => None,
        };
        if let Some(earlier) = out.iter().position(|f| f.at == at) {
            return fail(format!(
                "{command}: `{}`: the fillet already fades at {at} (fade.{earlier}); give one fade per end of an edge",
                key("at")
            ));
        }
        out.push(FilletFade { at, over, back });
    }
    Ok(out)
}

/// `feature.edit` for a fillet: `edges` and `fade` (`radius` is a plain
/// expression and `chain` a plain flag, set where the others are). A
/// `fade` of `null` or `[]` takes every fade away.
pub(crate) fn edit_fillet(p: &Params, fillet: &mut Fillet) -> Result<(), CommandError> {
    if let Some(value) = p.get("edges") {
        fillet.edges = edges_of("feature.edit", value, "fillet", MAX_FILLET_EDGES)?;
    }
    if p.has("fade") {
        fillet.fade = fade_value("feature.edit", p.get("fade").unwrap_or(&Value::Null))?;
    }
    Ok(())
}

fn feature_fillet(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.fillet";
    let p = Params::new(ID, params, &["edges", "radius", "chain", "fade", "name"])?;
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
    let fade = match p.get("fade") {
        Some(value) => fade_value(ID, value)?,
        None => Vec::new(),
    };
    let kind = FeatureKind::Fillet(Fillet {
        edges,
        radius: Expr::new(radius),
        chain,
        fade,
    });
    numbered_refused(ID, session, &kind)?;
    of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
}

/// Where a fillet's fades end, as the result and `document.inspect` give
/// them: per fade the vertex it is at, the place of the edge's reference,
/// the tip (the point the face closes in) and the name of its vertex,
/// over, back and the edge's length, in the document's unit.
pub(crate) fn fades_json(reports: &[craft3d_regen::FadeReport]) -> Value {
    Value::Array(
        reports
            .iter()
            .map(|r| {
                serde_json::json!({
                    "at": r.at.to_string(),
                    "edge": r.edge,
                    "tip": [r.tip.x, r.tip.y, r.tip.z],
                    "vertex": r.vertex.as_ref().map(ToString::to_string),
                    "over": r.over,
                    "back": r.back,
                    "length": r.length,
                })
            })
            .collect(),
    )
}
