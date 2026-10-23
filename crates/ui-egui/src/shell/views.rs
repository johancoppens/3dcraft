//! The view of each document tab, and flying the camera to a feature or
//! to the selection.
//!
//! - Every open document keeps its own view ([`TabView`]: the camera, the
//!   section, the see-through, what is highlighted and picked). The
//!   active one's is in `Shell::state`, the others are kept aside by the
//!   document's id (`DocumentStatus::id`), and switching tabs (a click,
//!   `file.activate`, closing one) swaps them before anything else reads
//!   the state. A document opened or created starts framed, iso,
//!   perspective.
//! - A double click on a row of the feature list, or F with something
//!   selected, flies the camera in [`FLY_SECONDS`] (eased) to frame what
//!   the feature made or what is selected, from the direction it looks
//!   from now. What the flight is aimed at is made visible: a hidden
//!   sketch or datum is shown again, and a section that cuts away all of
//!   it is switched off (one that leaves some of it in view stays: the
//!   person put it there). The flight is view state only: nothing in the
//!   document changes, and `ui.fit` with `feature` or `selection` frames
//!   the same at once, for an agent.

use super::{Shell, Turn};
use crate::state::TabView;
use craft3d_engine::{Scene, SelectionKind, Session};
use craft3d_math::{Bbox3, Vec3};
use std::collections::BTreeSet;

/// How long a flight to a feature or to the selection takes, in seconds.
pub const FLY_SECONDS: f64 = 0.3;

/// What a flight frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FlyTo {
    /// What the feature (by id or name) made ([`Scene::feature_box`]).
    Feature(String),
    /// What is selected ([`Scene::items_box`]).
    Selection,
}

impl Shell {
    /// Keeps the view with its document: when the session's active
    /// document is not the one `state` holds the view of, that view is
    /// put aside under its document and the active document's is taken
    /// out (a fresh one for a document not seen before). Run first in
    /// every sync, so whatever reads the state next reads the active
    /// tab's.
    pub(crate) fn follow_active_tab(&mut self, session: &Session) {
        let now = session.document_status().map(|status| status.id);
        if now == self.viewing {
            return;
        }
        let open: BTreeSet<u64> = session
            .document_statuses()
            .iter()
            .map(|status| status.id)
            .collect();
        if let Some(before) = self.viewing.filter(|id| open.contains(id)) {
            self.views
                .insert(before, (TabView::of(&self.state), self.framed));
        }
        self.views.retain(|id, _| open.contains(id));
        match now.and_then(|id| self.views.remove(&id)) {
            Some((view, framed)) => {
                view.put(&mut self.state);
                self.framed = framed;
            }
            // From no document at all, what was set meanwhile (`ui.set`
            // before `file.open`) is the new document's.
            None if self.viewing.is_none() => self.framed = Bbox3::EMPTY,
            None => {
                TabView::default().put(&mut self.state);
                self.framed = Bbox3::EMPTY;
            }
        }
        self.viewing = now;
        // What belonged to the other document's view.
        self.turn = None;
        self.drag_from = None;
        self.cube_drag_from = None;
        self.zoom_anchor = None;
        self.typing = None;
        self.section_cache = None;
        self.section_lost = None;
        self.fly = None;
        self.made_selection = None;
        self.tree_scroll = None;
        // The session's selection is the document's: read it afresh.
        let drawn = self.select.drawn.wrapping_add(1);
        self.select = super::selection::Selecting::default();
        self.select.drawn = drawn;
    }

    /// The document whose view the state holds, by its id.
    pub fn viewing(&self) -> Option<u64> {
        self.viewing
    }

    /// The box a flight to `to` frames in `scene`, or why there is none.
    fn fly_box(&self, scene: &Scene, to: &FlyTo) -> Result<Bbox3, String> {
        match to {
            FlyTo::Feature(feature) => {
                let bbox = scene.feature_box(feature);
                if bbox.is_empty() {
                    Err(format!(
                        "nothing of `{}` is shown to frame (a feature that failed, or made no face of its own)",
                        feature.chars().take(64).collect::<String>()
                    ))
                } else {
                    Ok(bbox)
                }
            }
            FlyTo::Selection => {
                let items = &self.select.view.items;
                if items.is_empty() {
                    return Err("nothing is selected".to_string());
                }
                let bbox = scene.items_box(items);
                if bbox.is_empty() {
                    Err("nothing of the selection is shown to frame".to_string())
                } else {
                    Ok(bbox)
                }
            }
        }
    }

    /// Makes what a flight to `to` frames visible: a hidden sketch or
    /// datum is shown, and a section that cuts away all of `bbox` is
    /// switched off.
    fn reveal(&mut self, scene: &Scene, to: &FlyTo, bbox: &Bbox3) {
        let features: Vec<&str> = match to {
            FlyTo::Feature(feature) => vec![feature.as_str()],
            FlyTo::Selection => self
                .select
                .view
                .items
                .iter()
                .filter(|item| !item.kind.is_entity() && item.kind != SelectionKind::Body)
                .map(|item| item.feature.as_str())
                .collect(),
        };
        for feature in features {
            let named = |id: &str, name: &str| id == feature || name == feature;
            if scene.sketches.iter().any(|s| named(&s.feature, &s.name)) {
                self.state.show.sketches = true;
            }
            if scene.datums.iter().any(|d| named(&d.feature, &d.name)) {
                self.state.show.datums = true;
            }
        }
        if let Some(section) = self.section()
            && let Some((lo, hi)) = bbox.corners()
        {
            let cut_away = [lo.x, hi.x].iter().all(|&x| {
                [lo.y, hi.y].iter().all(|&y| {
                    [lo.z, hi.z]
                        .iter()
                        .all(|&z| (Vec3::new(x, y, z) - section.origin).dot(section.normal) > 0.0)
                })
            });
            if cut_away {
                self.state.section.on = false;
                self.section_cache = None;
            }
        }
    }

    /// Frames what `to` names: at once (`started` is `None`: `ui.fit`), or
    /// as a flight that began at egui's time `started`. The camera keeps
    /// its direction and stops following the model as it grows. An error
    /// changes nothing.
    pub(crate) fn fly_to(
        &mut self,
        scene: &Scene,
        to: &FlyTo,
        started: Option<f64>,
    ) -> Result<(), String> {
        let bbox = self.fly_box(scene, to)?;
        self.reveal(scene, to, &bbox);
        let from = self.state.camera;
        let mut camera = from;
        camera.fit(&bbox, self.aspect());
        camera.auto_fit = false;
        match started {
            Some(started) => {
                self.turn = Some(Turn {
                    from,
                    to: camera,
                    started,
                    last: from,
                    seconds: FLY_SECONDS,
                });
            }
            None => {
                self.turn = None;
                self.state.camera = camera;
            }
        }
        Ok(())
    }

    /// Asks for a flight, made in the viewport's next frame, where the
    /// scene is at hand.
    pub(crate) fn request_fly(&mut self, to: FlyTo) {
        self.fly = Some(to);
    }

    /// Makes the flight asked for, if any; a flight that has nothing to
    /// frame says why where a refused click does.
    pub(crate) fn start_requested_fly(&mut self, scene: Option<&Scene>, now: f64) {
        let Some(to) = self.fly.take() else {
            return;
        };
        let Some(scene) = scene else {
            return;
        };
        match self.fly_to(scene, &to, Some(now)) {
            Ok(()) => self.select.problem = None,
            Err(why) => self.select.problem = Some(why),
        }
    }

    /// F: flies to the selection, or frames everything at once when
    /// nothing is selected (or nothing of it is shown).
    pub(crate) fn fit_key(&mut self, scene: Option<&Scene>) {
        let shown = scene.is_some_and(|scene| {
            !self.select.view.items.is_empty()
                && !scene.items_box(&self.select.view.items).is_empty()
        });
        if shown {
            self.request_fly(FlyTo::Selection);
        } else {
            self.fit_to(scene);
        }
    }

    /// The feature that made the first face, edge or vertex selected,
    /// whose row the list marks.
    pub fn made_selection(&self) -> Option<&str> {
        self.made_selection.as_deref()
    }

    /// After the selection changed: the row of the feature it is (a
    /// feature or a body selected) or that made what it is (a face, an
    /// edge or a vertex, by its persistent name) is marked and scrolled
    /// into view in the list.
    pub(crate) fn follow_selection_in_tree(&mut self, scene: &Scene) {
        self.made_selection = self
            .select
            .view
            .items
            .iter()
            .find(|item| item.kind.is_entity())
            .and_then(|item| scene.made_by(&item.name));
        self.tree_scroll = self
            .state
            .selected_feature
            .clone()
            .or_else(|| self.made_selection.clone());
    }
}
