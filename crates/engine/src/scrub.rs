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
//! [`max_states`], and [`STATE_BODY_BUDGET`] of bodies beyond the
//! document's own, the least recently shown going first), and its scenes
//! are kept by the session's scene cache like those of any other state,
//! so going back to a step seen before costs nothing. When the document's
//! cache has let go of bodies, a state takes those the state shown last
//! derived again, so a step forward derives one body at most. Every state
//! is meshed at the tolerances of the document as it is
//! (`Session::scrubbed_sizes`), so the bodies the steps share keep their
//! meshes.
//!
//! The scrub is view state of an open document, like its selection: not
//! saved, not undone. It is let go of when the features of the history
//! are no longer the same ones in the same order (a feature added,
//! deleted or moved, a delete and an add in one batch, another document
//! read into it): the history it was a position in is another one. An
//! edit that keeps the features (a parameter, a colour, hiding) keeps it.
//!
//! What a scrubbed view does not follow: a viewport click selects nothing
//! (the window says to go to the end first), because selecting resolves
//! names against the document as it is and the face clicked may be
//! another one there; `view.pick` says `"scrubbed": true`. A section on a
//! datum plane is placed where the document has the plane, and
//! `view.replay` replays the document's own history, not the scrub.

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
    "Shows the model in the view as it was after one feature of the history, as if the rollback marker stood there, without changing the document: not its rollback marker, not its undo history, not its dirty state. feature names the feature (null shows the document as it is again); step is its place instead (1 is the first feature). The viewport, view.render, view.pick and the window's screenshots show that state; document.inspect, measures and exports still see the document itself. New features still go where the real marker says: history.rollback moves it (\"roll back here\"), and history.rollback with null takes it away. The scrub is let go of when the features of the history change (one added, deleted or moved). While scrubbed, a click in the window's viewport does not select (go to the end first: selections name the document as it is), and view.pick says \"scrubbed\": true; a section on a datum plane uses the plane of the document as it is, and view.replay replays the document's own history. Without parameters it changes nothing. Returns where the view is: the feature (null when it shows the document as it is), its step, the number of steps, and the real rollback marker.",
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

/// Most bytes of bodies the regenerations of the scrubbed states keep
/// together beyond those the document's own regeneration holds (each
/// step of a chain of booleans has a body of its own, as big as all
/// before it). Beyond it the regenerations of the least recently shown
/// states go first; their scenes stay in the scene cache (within its own
/// budget), so showing them again costs nothing, and a regeneration let
/// go of is made again, the same bits, when something asks for it.
/// The cache of the state shown last (where the next state takes the
/// bodies the document's cache let go of) keeps bodies within the same
/// budget, so while scrubbed a document holds at most its own cache's
/// bodies and about twice this.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const STATE_BODY_BUDGET: usize = 1 << 30;
#[cfg(target_arch = "wasm32")]
pub(crate) const STATE_BODY_BUDGET: usize = 1 << 28;

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
    /// About how many bytes the bodies of `regen` take that the
    /// document's own regeneration does not hold.
    bytes: usize,
}

/// Where an open document's view is in its history.
#[derive(Clone, Default)]
pub(crate) struct Scrub {
    /// The feature the view shows the model after, and the
    /// [`features_key`] of the history when it was set.
    at: Option<(FeatureId, u64)>,
    /// The states shown so far, the most recently shown last.
    states: Vec<State>,
    /// What building the regeneration of the state shown last took
    /// ([`Session::scrubbed_regeneration`]).
    last: ScrubRegeneration,
    /// The sizes of the document as it is that the viewer's tolerances
    /// of every scrubbed state are fractions of, with the document they
    /// are of (`scene.rs`, `Session::scrubbed_sizes`).
    pub(crate) sizes: Option<(Arc<Document>, ShownSizes)>,
    /// Most bytes of bodies the states keep ([`STATE_BODY_BUDGET`] unless
    /// set: `Session::set_scrub_budget`), and the bodies the cache of the
    /// last state keeps; kept when the scrub is let go.
    pub(crate) budget: Option<usize>,
    /// The regeneration cache of the state shown last, with the document
    /// it is a state of: where the next state takes the bodies from that
    /// the document's own cache has let go of.
    playback: Option<(Arc<Document>, craft3d_regen::RegenCache)>,
}

impl Scrub {
    /// Lets go of the scrub and its states, keeping the budget.
    fn reset(&mut self) {
        *self = Scrub {
            budget: self.budget,
            ..Scrub::default()
        };
    }

    /// Lets go of the regenerations of the least recently shown states
    /// until those kept are within the budget; the state shown last keeps
    /// its own.
    fn trim(&mut self) {
        let budget = self.budget.unwrap_or(STATE_BODY_BUDGET);
        let mut held = self
            .states
            .iter()
            .fold(0usize, |n, state| n.saturating_add(state.bytes));
        let last = self.states.len().saturating_sub(1);
        for (at, state) in self.states.iter_mut().enumerate() {
            if held <= budget || at >= last {
                break;
            }
            if state.regen.take().is_some() {
                held = held.saturating_sub(state.bytes);
                state.bytes = 0;
            }
        }
    }
}

/// The sizes of a model that the relative tolerances of a viewer's
/// scenes are fractions of: the quick first picture's (the diagonal of
/// the box around the bodies) and the display scene's (the side of a
/// square with the area of the first picture's mesh).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShownSizes {
    pub preview: f64,
    pub display: f64,
}

/// What the regeneration of the state the view is scrubbed to took, as
/// [`Session::scrubbed_regeneration`] gives it: for benchmarks and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrubRegeneration {
    /// It was a state kept before: nothing was built.
    pub kept: bool,
    /// Features rebuilt and reused.
    pub rebuilt: usize,
    pub reused: usize,
    /// Bodies derived again that a cache had let go of.
    pub rederived: usize,
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

/// The caption of one step of the history, as the window's history player
/// shows it over the viewport and `view.replay` writes it into its
/// subtitles: `Step 3/10 · Plate · block` (the step's place, the number of
/// steps, the feature's name and its type).
pub fn step_caption(place: usize, count: usize, name: &str, kind: &str) -> String {
    format!("Step {place}/{count} · {name} · {kind}")
}

/// The features of a history, by id and in order, as one number (FNV-1a
/// over the count and the ids): a scrub holds while it stays the same.
pub(crate) fn features_key(doc: &Document) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let count = u64::try_from(doc.features.len()).unwrap_or(u64::MAX);
    for word in std::iter::once(count).chain(doc.features.iter().map(|f| f.id.0)) {
        for byte in word.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

impl OpenDocument {
    /// The feature the view is scrubbed to, if the scrub still holds: the
    /// feature is there and the history has the same features in the same
    /// order as when the scrub was set. Lets go of one that does not.
    pub(crate) fn scrubbed_to(&mut self) -> Option<FeatureId> {
        let (at, key) = self.scrub.at?;
        let doc = self.history.current();
        if features_key(doc) != key || doc.feature(at).is_none() {
            self.scrub.reset();
            return None;
        }
        Some(at)
    }

    /// Scrubs the view to `at` (`None`: the document as it is).
    pub(crate) fn scrub_to(&mut self, at: Option<FeatureId>) {
        let key = features_key(self.history.current());
        self.scrub.at = at.map(|id| (id, key));
    }

    /// The state of the document the view shows below its motion
    /// (`motion.rs`): the document itself, or the copy of it with the
    /// marker where the view is scrubbed to.
    pub(crate) fn scrubbed(&mut self) -> Arc<Document> {
        let current = Arc::clone(self.history.current());
        // States of another document state can no longer be shown: let go
        // of them (and of the old document each holds) at once.
        self.scrub
            .states
            .retain(|state| Arc::ptr_eq(&state.base, &current));
        if self
            .scrub
            .sizes
            .as_ref()
            .is_some_and(|(of, _)| !Arc::ptr_eq(of, &current))
        {
            self.scrub.sizes = None;
        }
        if self
            .scrub
            .playback
            .as_ref()
            .is_some_and(|(of, _)| !Arc::ptr_eq(of, &current))
        {
            self.scrub.playback = None;
        }
        let Some(at) = self.scrubbed_to() else {
            return current;
        };
        let last = current.features.last().map(|f| f.id);
        if current.rollback == Some(at) || (current.rollback.is_none() && last == Some(at)) {
            return current;
        }
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
            bytes: 0,
        });
        doc
    }

    /// The regeneration of [`Self::scrubbed`].
    pub(crate) fn scrubbed_regen(&mut self) -> Arc<Regen> {
        let doc = self.scrubbed();
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
            self.scrub.last = ScrubRegeneration {
                kept: true,
                ..ScrubRegeneration::default()
            };
            return regen;
        }
        // From what the document built: everything up to the feature is
        // reused, what comes after it is rolled back. On a copy of the
        // cache, so the document's own keeps the later features.
        let own = self.regen();
        // Tried without the bodies (`commands_elsewhere.rs`): not built.
        if self.forbid_regen.is_some() {
            self.forbid_regen = Some(true);
            return Arc::new(Regen::default());
        }
        let current = Arc::clone(self.history.current());
        // From a copy of the document's own cache, which has every body
        // unless it let go of some to stay within its budget (a model
        // whose bodies take more than that): then the bodies the cache of
        // the state shown last built or derived again are taken from it,
        // so a step forward derives one body, not the chain before it.
        let mut cache = self.cache.clone();
        cache.set_body_budget(self.scrub.budget.unwrap_or(STATE_BODY_BUDGET));
        if let Some((_, last)) = self
            .scrub
            .playback
            .as_ref()
            .filter(|(of, _)| Arc::ptr_eq(of, &current))
            && self.cache.bodies_held().1 > 0
        {
            cache.take_bodies_from(last);
        }
        self.work.regenerated();
        let regen = Arc::new(craft3d_regen::regenerate_cached(&doc, &mut cache));
        let (rebuilt, reused) = cache.last_run();
        self.scrub.last = ScrubRegeneration {
            kept: false,
            rebuilt,
            reused,
            rederived: cache.last_run_rederived(),
        };
        // The next state, a step further, finds the bodies this one built
        // or derived again in it.
        self.scrub.playback = Some((Arc::clone(&current), cache));
        // What its bodies take beyond the document's own: those are the
        // ones a state keeps alive.
        let shared: std::collections::BTreeSet<*const craft3d_topo::Body> = own
            .features
            .iter()
            .filter_map(|f| f.body.as_ref().map(Arc::as_ptr))
            .collect();
        let bytes = regen
            .features
            .iter()
            .filter_map(|f| f.body.as_ref())
            .filter(|body| !shared.contains(&Arc::as_ptr(body)))
            .fold(0usize, |n, body| {
                n.saturating_add(craft3d_regen::body_bytes(body))
            });
        if let Some(at) = self
            .scrub
            .states
            .iter()
            .position(|s| Arc::ptr_eq(&s.doc, &doc))
            && let Some(state) = self.scrub.states.get_mut(at)
        {
            state.regen = Some(Arc::clone(&regen));
            state.bytes = bytes;
            // The most recently shown last: trimming keeps it.
            let state = self.scrub.states.remove(at);
            self.scrub.states.push(state);
            self.scrub.trim();
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
    /// Sets the most bytes of bodies the regenerations of the scrubbed
    /// states of the active document keep beyond the document's own
    /// ([`STATE_BODY_BUDGET`] by default). For tests.
    pub fn set_scrub_budget(&mut self, bytes: usize) {
        if let Some(open) = self.documents.get_mut(self.active) {
            open.scrub.budget = Some(bytes);
        }
    }

    /// About how many bytes of bodies the scrubbed states of the active
    /// document keep beyond the document's own, and how many states keep
    /// their regeneration.
    pub fn scrub_bytes(&self) -> Option<(usize, usize)> {
        let open = self.documents.get(self.active)?;
        Some(
            open.scrub
                .states
                .iter()
                .fold((0, 0), |(bytes, kept), state| {
                    (
                        bytes.saturating_add(state.bytes),
                        kept + usize::from(state.regen.is_some()),
                    )
                }),
        )
    }

    /// Builds the regeneration of the state the active document's view is
    /// scrubbed to, as a scene would, and says what that took; `None`
    /// when the view shows the document as it is (or none is open). For
    /// benchmarks and tests.
    pub fn scrubbed_regeneration(&mut self) -> Option<ScrubRegeneration> {
        let open = self.documents.get_mut(self.active)?;
        open.scrubbed_to()?;
        open.scrubbed_regen();
        Some(open.scrub.last)
    }

    /// Regenerates the state the active document's view is scrubbed to
    /// from nothing and compares it with the one the view shows, built
    /// from what earlier states and the document had built. `None` when
    /// the view is not scrubbed. A body that went into a later feature
    /// and that a cache let go of (it is derived again when asked for) is
    /// left out of the comparison, with the names of its edges and
    /// vertices (the names of its faces are compared); everything else must be the same bits.
    /// For tests: the two must never differ.
    pub fn scrubbed_matches_a_full_one(&mut self) -> Option<bool> {
        let open = self.documents.get_mut(self.active)?;
        open.scrubbed_to()?;
        let doc = open.scrubbed();
        let held = open.scrubbed_regen();
        let mut fresh = craft3d_regen::regenerate(&doc);
        for (mine, theirs) in held.features.iter().zip(fresh.features.iter_mut()) {
            if mine.body.is_none() && mine.consumed_by.is_some() {
                // Without its edges' and vertices' names (`RegenCache`).
                theirs.body = None;
                theirs.names = theirs
                    .names
                    .as_deref()
                    .map(|names| Arc::new(names.without_entities()));
            }
        }
        Some(*held == fresh)
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
        open.scrub_to(target);
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
    fn a_delete_and_an_add_in_one_go_let_go_of_the_scrub() {
        let mut s = session();
        s.execute("view.scrub", &json!({"step": 1})).unwrap();
        // Nothing looks at the scrub between the two (as in a batch): the
        // count is the same afterwards, the features are not.
        s.execute("feature.delete", &json!({"feature": "Join"}))
            .unwrap();
        s.execute("feature.sphere", &json!({"radius": "1 mm"}))
            .unwrap();
        assert!(s.scrub_position().is_none());
        // A move keeps the count and is let go of too.
        s.execute("view.scrub", &json!({"step": 1})).unwrap();
        s.execute("feature.move", &json!({"feature": "Pin", "position": 0}))
            .unwrap();
        assert!(s.scrub_position().is_none());
    }

    #[test]
    fn the_states_of_an_old_document_go_when_the_view_is_back_at_the_end() {
        let mut s = session();
        s.execute("view.scrub", &json!({"step": 1})).unwrap();
        s.scene(None).unwrap();
        s.execute("view.scrub", &json!({"feature": null})).unwrap();
        s.execute(
            "feature.visible",
            &json!({"feature": "Box", "visible": false}),
        )
        .unwrap();
        s.scene(None).unwrap();
        assert!(s.documents[s.active].scrub.states.is_empty());
    }

    #[test]
    fn every_scrubbed_state_is_the_one_a_fresh_regeneration_gives() {
        for budget in [None, Some(0)] {
            let mut s = session();
            if let Some(bytes) = budget {
                s.set_scrub_budget(bytes);
            }
            s.scene(None).unwrap();
            for step in [1, 2, 3, 2, 1, 2] {
                s.execute("view.scrub", &json!({"step": step})).unwrap();
                s.scene(None).unwrap();
                if step < 3 {
                    assert_eq!(
                        s.scrubbed_matches_a_full_one(),
                        Some(true),
                        "step {step}, budget {budget:?}"
                    );
                }
            }
            // Their bodies are the document's own: they cost nothing
            // beyond it, so both keep their regeneration.
            assert_eq!(s.scrub_bytes().unwrap(), (0, 2));
        }
    }

    #[test]
    fn with_bodies_let_go_of_a_step_forward_goes_on_from_the_last_state() {
        for scrub_budget in [None, Some(0)] {
            let mut s = session();
            // The document's cache keeps no body that went into a later
            // feature: the block and the pin are derived again when a
            // state needs them.
            s.set_body_budget(0);
            if let Some(bytes) = scrub_budget {
                s.set_scrub_budget(bytes);
            }
            s.execute("feature.sphere", &json!({"radius": "1 mm"}))
                .unwrap();
            s.scene(None).unwrap();
            assert!(s.bodies_held().unwrap().1 > 0);
            s.execute("view.scrub", &json!({"step": 1})).unwrap();
            let first = s.scrubbed_regeneration().unwrap();
            assert!(first.rederived > 0, "{first:?}");
            assert_eq!(s.scrubbed_matches_a_full_one(), Some(true));
            for step in [2, 3] {
                s.execute("view.scrub", &json!({"step": step})).unwrap();
                let took = s.scrubbed_regeneration().unwrap();
                // At most its own body derived again (the document's cache
                // let go of it), from the last state's: never the chain.
                assert!(took.rederived <= 1, "step {step}: {took:?}");
                assert_eq!(s.scrubbed_matches_a_full_one(), Some(true));
            }
            // Back: a state kept is a lookup, within the budget.
            s.execute("view.scrub", &json!({"step": 1})).unwrap();
            let back = s.scrubbed_regeneration().unwrap();
            assert_eq!(back.kept, scrub_budget.is_none(), "{back:?}");
            assert_eq!(s.scrubbed_matches_a_full_one(), Some(true));
            let (bytes, kept) = s.scrub_bytes().unwrap();
            if scrub_budget.is_some() {
                assert!(kept <= 1 || bytes == 0, "{bytes} bytes in {kept} states");
            } else {
                assert!(bytes > 0);
            }
        }
    }

    #[test]
    fn stepping_forward_builds_only_the_new_feature_and_back_builds_nothing() {
        let mut s = session();
        s.scene(None).unwrap();
        s.execute("view.scrub", &json!({"step": 1})).unwrap();
        let first = s.scrubbed_regeneration().unwrap();
        assert!(!first.kept);
        assert_eq!(first.rederived, 0);
        s.execute("view.scrub", &json!({"step": 2})).unwrap();
        assert!(!s.scrubbed_regeneration().unwrap().kept);
        s.execute("view.scrub", &json!({"step": 1})).unwrap();
        assert!(s.scrubbed_regeneration().unwrap().kept);
        s.execute("view.scrub", &json!({"feature": null})).unwrap();
        assert!(s.scrubbed_regeneration().is_none());
    }

    #[test]
    fn a_scrubbed_state_is_meshed_as_the_document_as_it_is() {
        let mut s = Session::new(Workspace::none());
        s.execute("file.new", &json!({})).unwrap();
        s.execute(
            "feature.sphere",
            &json!({"radius": "2 mm", "name": "Ball", "position": ["-40 mm", "0 mm", "0 mm"]}),
        )
        .unwrap();
        s.execute(
            "feature.block",
            &json!({"size": ["60 mm", "60 mm", "60 mm"], "name": "Box"}),
        )
        .unwrap();
        let quality = crate::SceneQuality::display();
        let end = s.scene_with(quality).unwrap().unwrap();
        s.execute("view.scrub", &json!({"step": 1})).unwrap();
        let step = s.scene_with(quality).unwrap().unwrap();
        assert_eq!(step.bodies.len(), 1);
        // The same sampling as the end, so the ball keeps its mesh.
        assert_eq!(step.sampling(), end.sampling());
        assert!(std::sync::Arc::ptr_eq(
            &step.bodies[0].mesh,
            &end.bodies[0].mesh
        ));
        let preview = s.scene_with(crate::SceneQuality::Preview).unwrap().unwrap();
        s.execute("view.scrub", &json!({"feature": null})).unwrap();
        let end_preview = s.scene_with(crate::SceneQuality::Preview).unwrap().unwrap();
        assert_eq!(preview.sampling(), end_preview.sampling());
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
