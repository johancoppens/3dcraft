//! Questions about the bodies of the document beyond `document.inspect`.

use crate::commands::{Run, fail, feature_ref, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::PersistentName;
use craft3d_math::Vec3;
use serde_json::{Value, json};

/// Most edges one answer lists.
const MAX_EDGES: usize = 2_000;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("document.edges", "Edges", r#"{"feature": id or name, "between": [face name, face name]}"#,
        "Lists the edges of a feature's body, each with its own name (name, which no other edge of the body has; after a boolean it is made of the faces the edge lies between, \"F7:between[face name,face name]\", with the faces near it after | and a number when several edges lie between the same two; aliases: the name an operand of a boolean gave the edge, when it is still one edge), the names of the faces it lies between, its curve kind and its end points. With between: only the edges between those two faces (names from document.inspect, faceNames; an alias from faceAliases names the same face). That is how an edge is referred to: by the faces it separates, which keeps meaning the same edge when the model changes upstream, also through booleans. There can be several, or none. The same name twice gives the seams of that face and the cuts between its pieces.", doc: true, document_edges),
];

fn vec3(v: Vec3) -> Value {
    json!([v.x, v.y, v.z])
}

fn document_edges(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "document.edges";
    let p = Params::new(ID, params, &["feature", "between"])?;
    let feature = feature_ref(session.document()?.history.current(), &p, ID, "feature")?;
    let between = match p.get("between") {
        None => None,
        Some(Value::Array(items)) => {
            let [a, b] = items.as_slice() else {
                return fail(format!("{ID}: `between` takes two face names"));
            };
            let name = |value: &Value| -> Result<PersistentName, CommandError> {
                let Some(text) = value.as_str() else {
                    return fail(format!(
                        "{ID}: `between` takes two face names such as \"F2:side(e7)\""
                    ));
                };
                let name: PersistentName = text
                    .parse()
                    .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
                if !name.is_face() {
                    return fail(format!("{ID}: {name} is not a face"));
                }
                Ok(name)
            };
            Some((name(a)?, name(b)?))
        }
        Some(_) => return fail(format!("{ID}: `between` takes two face names")),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let made_by = doc
        .feature(feature)
        .map_or_else(|| feature.to_string(), |f| f.name.clone());
    let Some(outcome) = regen.feature(feature) else {
        return fail(format!("{ID}: there is no feature {feature}"));
    };
    let Some(body) = outcome.own_body() else {
        return fail(match outcome.consumed_by {
            Some(taker) => format!(
                "{ID}: the body of `{made_by}` went into {taker}; ask for the edges of that feature"
            ),
            None => format!("{ID}: `{made_by}` has no body"),
        });
    };
    let Some(names) = &outcome.names else {
        return fail(format!("{ID}: the faces of `{made_by}` have no names"));
    };
    let users = craft3d_regen::faces_of_edges(body);
    let wanted: Option<Vec<u32>> = match &between {
        None => None,
        Some((a, b)) => {
            for name in [a, b] {
                if craft3d_regen::faces_named(names, name).is_empty() {
                    return fail(format!(
                        "{ID}: `{made_by}` has no face {name} (document.inspect lists its faceNames)"
                    ));
                }
            }
            Some(
                craft3d_regen::edges_between(body, names, a, b)
                    .into_iter()
                    .map(|edge| edge.0)
                    .collect(),
            )
        }
    };
    let mut edges = Vec::new();
    let mut total = 0_usize;
    for (index, edge) in body.edges.iter().enumerate() {
        let id = u32::try_from(index).unwrap_or(u32::MAX);
        if wanted.as_ref().is_some_and(|list| !list.contains(&id)) {
            continue;
        }
        total = total.saturating_add(1);
        if edges.len() >= MAX_EDGES {
            continue;
        }
        let faces: Vec<String> = users
            .get(index)
            .into_iter()
            .flatten()
            .filter_map(|face| names.faces.get(face.0 as usize))
            .map(ToString::to_string)
            .collect();
        let point = |vertex| body.vertex(vertex).map_or(Value::Null, |v| vec3(v.point));
        let mut entry = json!({
            "faces": faces,
            "curve": edge.curve.kind(),
            "start": point(edge.start),
            "end": point(edge.end),
            "closed": edge.start == edge.end,
        });
        if let (Some(map), Some(name)) = (entry.as_object_mut(), names.edges.get(index)) {
            map.insert("name".into(), name.to_string().into());
            let aliases: Vec<String> = names
                .edge_aliases_of(index)
                .iter()
                .map(ToString::to_string)
                .collect();
            if !aliases.is_empty() {
                map.insert("aliases".into(), aliases.into());
            }
        }
        edges.push(entry);
    }
    Ok(json!({
        "feature": feature.to_string(),
        "name": made_by,
        "count": total,
        "edges": edges,
    }))
}
