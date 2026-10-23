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
        // A flight or a turn under way is put aside as where it was going.
        if let Some(turn) = self.turn.take() {
            self.state.camera = turn.to;
            if turn.to.auto_fit
                && let Some(scene) = &self.shown_scene
            {
                self.framed = scene.bbox;
            }
        }
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
        self.tab_switched = true;
        // The session's selection is the document's: read it afresh.
        let drawn = self.select.drawn.wrapping_add(1);
        self.select = super::selection::Selecting::default();
        self.select.drawn = drawn;
    }

    /// The document whose view the state holds, by its id.
    pub fn viewing(&self) -> Option<u64> {
        self.viewing
    }

    /// What a flight to `to` frames in `scene`, or why there is none: the
    /// box around its points ([`Scene::feature_points`],
    /// [`Scene::items_points`], read once), and whether the section cuts
    /// every one of them away (farther than the cut's snap into the half
    /// it takes away).
    fn fly_target(&self, scene: &Scene, to: &FlyTo) -> Result<Target, String> {
        let section = self.section().and_then(|section| {
            let normal = section.normal.normalized()?;
            Some((section.origin, normal))
        });
        let snap = craft3d_render::SECTION_SNAP * scene.bbox.diagonal();
        let mut bbox = Bbox3::EMPTY;
        let mut kept = false;
        let mut visit = |point: Vec3| {
            bbox.add(point);
            if let Some((origin, normal)) = section
                && (point - origin).dot(normal) <= snap
            {
                kept = true;
            }
        };
        match to {
            FlyTo::Feature(feature) => scene.feature_points(feature, &mut visit),
            FlyTo::Selection => {
                if self.select.view.items.is_empty() {
                    return Err("nothing is selected".to_string());
                }
                scene.items_points(&self.select.view.items, &mut visit);
            }
        }
        if bbox.is_empty() {
            return Err(match to {
                FlyTo::Feature(feature) => format!(
                    "nothing of `{}` is shown to frame (a feature that failed, or made no face of its own)",
                    feature.chars().take(64).collect::<String>()
                ),
                FlyTo::Selection => "nothing of the selection is shown to frame".to_string(),
            });
        }
        Ok(Target {
            bbox,
            cut_away: section.is_some() && !kept,
        })
    }

    /// Makes what a flight to `to` frames visible: a hidden sketch or
    /// datum is shown, and a section that cuts away all of it is switched
    /// off.
    fn reveal(&mut self, scene: &Scene, to: &FlyTo, cut_away: bool) {
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
        if cut_away {
            self.state.section.on = false;
            self.section_cache = None;
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
        let target = self.fly_target(scene, to)?;
        self.fly_with(scene, to, &target, started);
        Ok(())
    }

    /// [`Self::fly_to`] with its target worked out.
    fn fly_with(&mut self, scene: &Scene, to: &FlyTo, target: &Target, started: Option<f64>) {
        self.reveal(scene, to, target.cut_away);
        let from = self.state.camera;
        let mut camera = from;
        camera.fit(&at_least(&target.bbox, scene), self.aspect());
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

    /// F at egui's time `now`: flies to the selection, or frames
    /// everything at once when nothing is selected (or nothing of it is
    /// shown). What is selected is looked at once.
    pub(crate) fn fit_key(&mut self, scene: Option<&Scene>, now: f64) {
        let to = FlyTo::Selection;
        let target = scene
            .filter(|_| !self.select.view.items.is_empty())
            .and_then(|scene| Some((scene, self.fly_target(scene, &to).ok()?)));
        match target {
            Some((scene, target)) => self.fly_with(scene, &to, &target, Some(now)),
            None => self.fit_to(scene),
        }
    }

    /// The feature that made the first face, edge or vertex selected,
    /// whose row the list marks.
    pub fn made_selection(&self) -> Option<&str> {
        self.made_selection.as_deref()
    }

    /// Which feature made the first face, edge or vertex selected (by
    /// its persistent name), in `scene`: worked out again whenever the
    /// selection or the model changes.
    pub(crate) fn mark_made_selection(&mut self, scene: &Scene) {
        self.made_selection = self
            .select
            .view
            .items
            .iter()
            .find(|item| item.kind.is_entity())
            .and_then(|item| scene.made_by(&item.name));
    }

    /// After the selection changed: the row of the feature it is (a
    /// feature or a body selected) or that made what it is is scrolled
    /// into view in the list.
    pub(crate) fn scroll_tree_to_selection(&mut self) {
        self.tree_scroll = self
            .state
            .selected_feature
            .clone()
            .or_else(|| self.made_selection.clone());
    }
}

/// What a flight frames.
struct Target {
    bbox: Bbox3,
    /// The section cuts all of it away.
    cut_away: bool,
}

/// Smallest side of the box a flight frames, as a fraction of the
/// diagonal of the model's box: a vertex, a short edge or a tiny face is
/// framed with some of what is around it, not at the camera's nearest.
pub const MIN_FLY_EXTENT: f64 = 0.02;

/// `bbox` grown about its centre to at least [`MIN_FLY_EXTENT`] of the
/// model's diagonal along every axis.
fn at_least(bbox: &Bbox3, scene: &Scene) -> Bbox3 {
    let (Some(center), Some((lo, hi))) = (bbox.center(), bbox.corners()) else {
        return *bbox;
    };
    let side = MIN_FLY_EXTENT * scene.bbox.diagonal();
    if !(side.is_finite() && side > 0.0) {
        return *bbox;
    }
    let half = side * 0.5;
    let grow = |lo: f64, hi: f64, c: f64| {
        if hi - lo < side {
            (c - half, c + half)
        } else {
            (lo, hi)
        }
    };
    let (x0, x1) = grow(lo.x, hi.x, center.x);
    let (y0, y1) = grow(lo.y, hi.y, center.y);
    let (z0, z1) = grow(lo.z, hi.z, center.z);
    Bbox3::from_points([Vec3::new(x0, y0, z0), Vec3::new(x1, y1, z1)])
}
