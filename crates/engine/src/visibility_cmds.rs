//! `feature.visible`: hiding the body of a feature, a sketch or a datum in
//! the view, and showing it again (`craft3d_doc::visibility`).
//!
//! What is hidden is left out of every scene ([`crate::Scene`]: the
//! viewport, the window's screenshots, `view.render`, `view.pick`) and of
//! nothing else: regeneration does not read it, so no body, status or
//! measure changes, and exports write every body (a view setting is not
//! a choice of what to export; `file.export` with `bodies` is). It is
//! kept in the document and saved with it, and like a colour it is one
//! undo step.

use crate::commands::{Run, edit, fail, feature_ref, spec};
use crate::params::Params;
use crate::{CommandSpec, Session};
use craft3d_doc::FeatureId;
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "feature.visible",
    "Show or Hide Feature",
    r#"{"feature": id or name or [id or name], "visible": boolean}"#,
    "Hides features in the view (visible: false) or shows them again (visible: true): the body a feature makes, a sketch's curves or a datum leave the viewport, view.render, view.pick and the window's screenshots. Nothing else changes: unlike feature.suppress the feature is still built, later features are still made from it, document.inspect still measures its body, and file.export still writes it (name the bodies to export with bodies). document.inspect marks a hidden feature with \"hidden\": true. Kept in the document and saved with it; one undo step. Returns each feature with whether it is visible now, and the ids of all hidden features.",
    doc: true,
    feature_visible
)];

/// Most features one call names.
const MAX_FEATURES_AT_ONCE: usize = 10_000;

fn feature_visible(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.visible";
    let p = Params::new(ID, params, &["feature", "visible"])?;
    let visible = p.bool("visible")?.ok_or_else(|| p.missing("visible"))?;
    let doc = session.document()?.history.current();
    let given: Vec<&Value> = match p.get("feature") {
        None => return Err(p.missing("feature")),
        Some(Value::Array(items)) if items.is_empty() => {
            return fail(format!("{ID}: `feature` is an empty list"));
        }
        Some(Value::Array(items)) if items.len() > MAX_FEATURES_AT_ONCE => {
            return fail(format!(
                "{ID}: at most {MAX_FEATURES_AT_ONCE} features at once, got {}",
                items.len()
            ));
        }
        Some(Value::Array(items)) => items.iter().collect(),
        Some(one) => vec![one],
    };
    let mut ids: Vec<FeatureId> = Vec::with_capacity(given.len());
    for item in given {
        let one = json!({ "feature": item });
        let p = Params::new(ID, &one, &["feature"])?;
        let id = feature_ref(doc, &p, ID, "feature")?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let label = match (visible, ids.as_slice()) {
        (true, [one]) => format!("Show {one}"),
        (false, [one]) => format!("Hide {one}"),
        (true, _) => format!("Show {} features", ids.len()),
        (false, _) => format!("Hide {} features", ids.len()),
    };
    edit(session, ID, &label, |doc| {
        for id in &ids {
            doc.set_hidden(*id, !visible)?;
        }
        Ok(())
    })?;
    let doc = session.document()?.history.current();
    let features: Vec<Value> = ids
        .iter()
        .map(|id| {
            let name = doc.feature(*id).map(|f| f.name.clone()).unwrap_or_default();
            json!({ "id": id.to_string(), "name": name, "visible": !doc.is_hidden(*id) })
        })
        .collect();
    let hidden: Vec<String> = doc.hidden.iter().map(ToString::to_string).collect();
    Ok(json!({ "features": features, "hidden": hidden }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use crate::{Session, Workspace};
    use serde_json::json;

    fn session_with_block() -> Session {
        let mut session = Session::new(Workspace::none());
        session.execute("file.new", &json!({})).unwrap();
        session
            .execute(
                "feature.block",
                &json!({"size": ["10 mm", "10 mm", "10 mm"], "name": "Box"}),
            )
            .unwrap();
        session
            .execute(
                "feature.cylinder",
                &json!({"radius": "2 mm", "height": "20 mm", "name": "Pin", "position": ["30 mm", "0 mm", "0 mm"]}),
            )
            .unwrap();
        session
    }

    #[test]
    fn hiding_takes_the_body_out_of_the_scene_only() {
        let mut session = session_with_block();
        let before = session.scene(None).unwrap().unwrap();
        assert_eq!(before.bodies.len(), 2);
        let out = session
            .execute(
                "feature.visible",
                &json!({"feature": "Pin", "visible": false}),
            )
            .unwrap();
        assert_eq!(out["features"][0]["visible"], false);
        assert_eq!(out["hidden"], json!(["F2"]));
        let scene = session.scene(None).unwrap().unwrap();
        assert_eq!(scene.bodies.len(), 1);
        assert_eq!(scene.bodies[0].name, "Box");
        // The row is still there, marked hidden.
        assert_eq!(scene.features.len(), 2);
        assert!(scene.features[1].hidden && !scene.features[0].hidden);
        // The model is not: both bodies are measured and exported.
        let inspect = session.execute("document.inspect", &json!({})).unwrap();
        assert_eq!(inspect["bodies"].as_array().unwrap().len(), 2);
        assert_eq!(inspect["features"][1]["hidden"], true);
        assert!(inspect["features"][0].get("hidden").is_none());
        // One undo step brings it back.
        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(session.scene(None).unwrap().unwrap().bodies.len(), 2);
        session.execute("edit.redo", &json!({})).unwrap();
        session
            .execute(
                "feature.visible",
                &json!({"feature": ["F2", "Box"], "visible": true}),
            )
            .unwrap();
        assert_eq!(session.scene(None).unwrap().unwrap().bodies.len(), 2);
    }

    #[test]
    fn bad_params_are_errors() {
        let mut session = session_with_block();
        for params in [
            json!({}),
            json!({"feature": "Box"}),
            json!({"visible": false}),
            json!({"feature": "Nope", "visible": false}),
            json!({"feature": [], "visible": false}),
            json!({"feature": [true], "visible": false}),
            json!({"feature": "Box", "visible": "no"}),
            json!({"feature": 99, "visible": false}),
            json!({"feature": -1, "visible": false}),
            json!({"feature": "Box", "visible": false, "extra": 1}),
        ] {
            assert!(
                session.execute("feature.visible", &params).is_err(),
                "{params}"
            );
        }
        let mut empty = Session::new(Workspace::none());
        assert!(
            empty
                .execute("feature.visible", &json!({"feature": 1, "visible": false}))
                .is_err()
        );
    }
}
