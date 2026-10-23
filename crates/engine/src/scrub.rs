//! Scrubbing through the history: the view shows the model as it was after
//! one feature, as if the rollback marker stood there, while the document
//! does not change (not its marker, its undo history, its revision nor its
//! dirty state). `view.scrub` sets it; `history.rollback` is what moves
//! the real marker.
//!
//! The state shown is a copy of the document with the marker on that
//! feature, regenerated from a copy of the document's own regeneration
//! cache: everything up to the feature is what the document has built
//! already and is reused, so a step costs a comparison per feature and
//! the meshing of the bodies no earlier scene had (the session's store of
//! meshes keeps every body it meshed). Each state is kept, with its
//! regeneration, for as long as the document stays as it is (up to
//! [`max_states`], the least recently shown going first), and its scenes
//! are kept by the session's scene cache like those of any other state,
//! so going back to a step seen before costs nothing.
//!
//! The scrub is view state of an open document, like its selection: not
//! saved, not undone. It is let go of when the document gains or loses a
//! feature (the history it was a position in is another one); an edit
//! that keeps the features (a parameter, a colour, hiding) keeps it.

use crate::commands::{Run, fail, feature_ref, spec};
use crate::params::Params;
use crate::{CommandSpec, OpenDocument, Session};
use craft3d_doc::{Document, FeatureId};
use craft3d_regen::Regen;
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "view.scrub",
    "Scrub History",
    r#"{"feature": id or name or null, "step": integer}"#,
    "Shows the model in the view as it was after one feature of the history, as if the rollback marker stood there, without changing the document: not its rollback marker, not its undo history, not its dirty state. feature names the feature (null shows the document as it is again); step is its place instead (1 is the first feature). The viewport, view.render, view.pick and the window's screenshots show that state; document.inspect, measures and exports still see the document itself. New features still go where the real marker says: history.rollback moves it (\"roll back here\"), and history.rollback with null takes it away. The scrub is let go of when a feature is added or deleted. Without parameters it changes nothing. Returns where the view is: the feature (null when it shows the document as it is), its step, the number of steps, and the real rollback marker.",
    doc: true,
    view_scrub
)];

/// Most copies of features the scrubbed states of a document keep
/// together: each state is a copy of the document.
const MAX_FEATURE_COPIES: usize = 100_000;

/// Most scrubbed states kept for a document of `features` features: every
/// step of a history of a few hundred features (a scrub back and forth
/// over all of it costs nothing the second time), fewer for a longer one.
pub(crate) fn max_states(features: usize) -> usize {
    (MAX_FEATURE_COPIES / features.max(1)).clamp(16, 512)
}

/// One state the view was scrubbed to.
#[derive(Clone)]
struct State {
    /// The document it is a state of.
    base: Arc<Document>,
    at: FeatureId,
    /// `base` with its rollback marker on `at`.
    doc: Arc<Document>,
    /// Its regeneration, once a scene asked for it.
    regen: Option<Arc<Regen>>,
}

/// Where an open document's view is in its history.
#[derive(Clone, Default)]
pub(crate) struct Scrub {
    /// The feature the view shows the model after, and how many features
    /// the document had when it was set.
    at: Option<(FeatureId, usize)>,
    /// The states shown so far, the most recently shown last.
    states: Vec<State>,
}

/// Where the view of the active document is in its history, as
/// [`Session::scrub_position`] gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrubPosition {
    /// The feature the model is shown after, as `F3`.
    pub feature: String,
    /// Its place in the history: 1 for the first feature.
    pub step: usize,
    /// How many features the history has.
    pub steps: usize,
}

impl OpenDocument {
    /// The feature the view is scrubbed to, if the scrub still holds: the
    /// feature is there and the history has as many features as when the
    /// scrub was set. Lets go of one that does not.
    pub(crate) fn scrubbed_to(&mut self) -> Option<FeatureId> {
        let (at, count) = self.scrub.at?;
        let doc = self.history.current();
        if doc.features.len() != count || doc.feature(at).is_none() {
            self.scrub = Scrub::default();
            return None;
        }
        Some(at)
    }

    /// The state of the document the view shows: the document itself, or
    /// the copy of it with the marker where the view is scrubbed to.
    pub(crate) fn shown(&mut self) -> Arc<Document> {
        let current = Arc::clone(self.history.current());
        let Some(at) = self.scrubbed_to() else {
            return current;
        };
        let last = current.features.last().map(|f| f.id);
        if current.rollback == Some(at) || (current.rollback.is_none() && last == Some(at)) {
            return current;
        }
        // States of another document state can no longer be shown.
        self.scrub
            .states
            .retain(|state| Arc::ptr_eq(&state.base, &current));
        if let Some(found) = self.scrub.states.iter().position(|s| s.at == at) {
            let state = self.scrub.states.remove(found);
            let doc = Arc::clone(&state.doc);
            self.scrub.states.push(state);
            return doc;
        }
        let mut copy = Document::clone(&current);
        copy.rollback = Some(at);
        let doc = Arc::new(copy);
        if self.scrub.states.len() >= max_states(current.features.len()) {
            self.scrub.states.remove(0);
        }
        self.scrub.states.push(State {
            base: current,
            at,
            doc: Arc::clone(&doc),
            regen: None,
        });
        doc
    }

    /// The regeneration of [`Self::shown`].
    pub(crate) fn shown_regen(&mut self) -> Arc<Regen> {
        let doc = self.shown();
        if Arc::ptr_eq(&doc, self.history.current()) {
            return self.regen();
        }
        if let Some(regen) = self
            .scrub
            .states
            .iter()
            .find(|s| Arc::ptr_eq(&s.doc, &doc))
            .and_then(|s| s.regen.clone())
        {
            return regen;
        }
        // From what the document built: everything up to the feature is
        // reused, what comes after it is rolled back. On a copy of the
        // cache, so the document's own keeps the later features.
        self.regen();
        let mut cache = self.cache.clone();
        let regen = Arc::new(craft3d_regen::regenerate_cached(&doc, &mut cache));
        if let Some(state) = self
            .scrub
            .states
            .iter_mut()
            .find(|s| Arc::ptr_eq(&s.doc, &doc))
        {
            state.regen = Some(Arc::clone(&regen));
        }
        regen
    }
}

impl Session {
    /// Where the view of the active document is scrubbed to in its
    /// history (`view.scrub`); `None` when it shows the document as it is
    /// or no document is open.
    pub fn scrub_position(&mut self) -> Option<ScrubPosition> {
        let open = self.documents.get_mut(self.active)?;
        let at = open.scrubbed_to()?;
        let doc = open.history.current();
        let index = doc.features.iter().position(|f| f.id == at)?;
        Some(ScrubPosition {
            feature: at.to_string(),
            step: index.saturating_add(1),
            steps: doc.features.len(),
        })
    }
}

impl Session {
    /// The active document's history as a timeline shows it: how many
    /// features it has, and the place of its rollback marker (1 for the
    /// first feature) when it has one. `None` without a document.
    pub fn history_marker(&self) -> Option<(usize, Option<usize>)> {
        let doc = self.documents.get(self.active)?.history.current();
        let marker = doc
            .rollback
            .and_then(|id| doc.features.iter().position(|f| f.id == id))
            .map(|index| index.saturating_add(1));
        Some((doc.features.len(), marker))
    }
}

fn view_scrub(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "view.scrub";
    let p = Params::new(ID, params, &["feature", "step"])?;
    if p.has("feature") && p.has("step") {
        return fail(format!("{ID}: give `feature` or `step`, not both"));
    }
    let open = session.document_mut()?;
    let doc = open.history.current();
    let count = doc.features.len();
    let target: Option<Option<FeatureId>> = if p.has("step") {
        if count == 0 {
            return fail(format!(
                "{ID}: the document has no features to scrub through"
            ));
        }
        let step = p
            .integer("step", 1, u64::try_from(count).unwrap_or(u64::MAX))?
            .ok_or_else(|| p.missing("step"))?;
        let index = usize::try_from(step)
            .ok()
            .and_then(|s| s.checked_sub(1))
            .ok_or_else(|| crate::CommandError::new(format!("{ID}: bad `step`")))?;
        Some(doc.features.get(index).map(|f| f.id))
    } else {
        match (p.has("feature"), p.get("feature")) {
            (false, _) => None,
            (true, None) => Some(None),
            (true, Some(_)) => Some(Some(feature_ref(doc, &p, ID, "feature")?)),
        }
    };
    if let Some(target) = target {
        open.scrub.at = target.map(|id| (id, count));
    }
    let rollback = open.history.current().rollback.map(|id| id.to_string());
    let position = session.scrub_position();
    Ok(json!({
        "feature": position.as_ref().map(|p| p.feature.clone()),
        "step": position.as_ref().map(|p| p.step),
        "steps": count,
        "rollback": rollback,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use crate::{Session, Workspace};
    use serde_json::json;

    /// A block, a cylinder unioned into it, and a sketch.
    fn session() -> Session {
        let mut s = Session::new(Workspace::none());
        s.execute("file.new", &json!({})).unwrap();
        for (id, params) in [
            (
                "feature.block",
                json!({"size": ["10 mm", "10 mm", "10 mm"], "name": "Box"}),
            ),
            (
                "feature.cylinder",
                json!({"radius": "2 mm", "height": "20 mm", "name": "Pin", "position": ["5 mm", "5 mm", "0 mm"]}),
            ),
            (
                "feature.boolean",
                json!({"op": "union", "target": "Box", "tool": "Pin", "name": "Join"}),
            ),
        ] {
            s.execute(id, &params).unwrap();
        }
        s
    }

    #[test]
    fn scrubbing_shows_earlier_states_without_touching_the_document() {
        let mut s = session();
        let revision = s.document_revision();
        let end = s.scene(None).unwrap().unwrap();
        assert_eq!(end.bodies.len(), 1);
        let out = s.execute("view.scrub", &json!({"step": 2})).unwrap();
        assert_eq!(out["feature"], "F2");
        assert_eq!(out["step"], 2);
        assert_eq!(out["steps"], 3);
        assert_eq!(out["rollback"], json!(null));
        // After the cylinder: the block and the cylinder, apart.
        let scene = s.scene(None).unwrap().unwrap();
        assert_eq!(scene.bodies.len(), 2);
        assert_eq!(scene.features[2].state, crate::FeatureState::RolledBack);
        // The document did not change.
        assert_eq!(s.document_revision(), revision);
        let inspect = s.execute("document.inspect", &json!({})).unwrap();
        assert_eq!(inspect["bodies"].as_array().unwrap().len(), 1);
        assert_eq!(inspect["rollback"], json!(null));
        // The first step, then back to the end.
        s.execute("view.scrub", &json!({"feature": "Box"})).unwrap();
        assert_eq!(s.scene(None).unwrap().unwrap().bodies.len(), 1);
        assert_eq!(s.scrub_position().unwrap().step, 1);
        s.execute("view.scrub", &json!({"feature": null})).unwrap();
        assert!(s.scrub_position().is_none());
        let back = s.scene(None).unwrap().unwrap();
        assert_eq!(back.bodies.len(), 1);
        assert_eq!(back.mesh, end.mesh);
        // A state seen before is the same scene again, not built anew.
        s.execute("view.scrub", &json!({"step": 2})).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &s.scene(None).unwrap().unwrap().mesh,
            &scene.mesh
        ));
        // Without parameters nothing changes.
        let out = s.execute("view.scrub", &json!({})).unwrap();
        assert_eq!(out["step"], 2);
    }

    #[test]
    fn the_scrub_matches_a_real_rollback() {
        let mut s = session();
        s.execute("view.scrub", &json!({"step": 2})).unwrap();
        let scrubbed = s.scene(None).unwrap().unwrap();
        s.execute("view.scrub", &json!({"feature": null})).unwrap();
        s.execute("history.rollback", &json!({"feature": "Pin"}))
            .unwrap();
        let rolled = s.scene(None).unwrap().unwrap();
        assert_eq!(scrubbed.mesh, rolled.mesh);
        assert_eq!(scrubbed.features, rolled.features);
    }

    #[test]
    fn adding_a_feature_lets_go_of_the_scrub_and_hiding_keeps_it() {
        let mut s = session();
        s.execute("view.scrub", &json!({"step": 1})).unwrap();
        s.execute(
            "feature.visible",
            &json!({"feature": "Pin", "visible": false}),
        )
        .unwrap();
        assert_eq!(s.scrub_position().unwrap().step, 1);
        s.execute(
            "feature.sphere",
            &json!({"radius": "1 mm", "position": ["20 mm", "0 mm", "0 mm"]}),
        )
        .unwrap();
        assert!(s.scrub_position().is_none());
        assert_eq!(s.scene(None).unwrap().unwrap().bodies.len(), 2);
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = session();
        for params in [
            json!({"step": 0}),
            json!({"step": 4}),
            json!({"step": -1}),
            json!({"step": "two"}),
            json!({"step": 1.5}),
            json!({"feature": "Nope"}),
            json!({"feature": 99}),
            json!({"feature": true}),
            json!({"feature": "Box", "step": 1}),
            json!({"to": 1}),
        ] {
            assert!(s.execute("view.scrub", &params).is_err(), "{params}");
        }
        let mut empty = Session::new(Workspace::none());
        assert!(empty.execute("view.scrub", &json!({"step": 1})).is_err());
        empty.execute("file.new", &json!({})).unwrap();
        assert!(empty.execute("view.scrub", &json!({"step": 1})).is_err());
        assert!(empty.execute("view.scrub", &json!({})).is_ok());
    }
}
