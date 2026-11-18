//! What a click would select, under the pointer: the pick where the
//! pointer rests over the viewport ([`UiState::hovered`], shaped as the
//! last click's [`UiState::picked`]). The control channel places the
//! pointer without a click (`ui.pick {x, y, hover: true}`), so pictures can
//! show what the pointer lights up.
//!
//! [`UiState::hovered`]: crate::state::UiState::hovered
//! [`UiState::picked`]: crate::state::UiState::picked

use super::Shell;
use crate::state::{Picked, PickedEntity, PickedKind};
use craft3d_engine::{PickKind, Scene, SceneNear, ScenePointed};
use serde_json::Value;

/// The pick `found` at (`x`, `y`), as [`UiState::picked`] and
/// [`UiState::hovered`] hold it: the face under the position (or, on the
/// cap of the section, the body cut there and the point of the plane),
/// and the vertex, or else the edge, it points at.
///
/// [`UiState::picked`]: crate::state::UiState::picked
/// [`UiState::hovered`]: crate::state::UiState::hovered
pub(super) fn picked_of(found: &ScenePointed, x: f64, y: f64, serial: u64) -> Picked {
    let hit = found.face.as_ref();
    let entity = |near: &SceneNear| PickedEntity {
        feature: near.feature.clone(),
        body: near.body.clone(),
        name: near.name.clone(),
        index: u64::try_from(near.index).unwrap_or(u64::MAX),
        faces: near.faces.clone(),
        point: [near.point.x, near.point.y, near.point.z],
    };
    // On the cap of the section: the body that is cut there and the point
    // of the plane, and no face.
    let cap = found.section.as_ref().filter(|_| hit.is_none());
    Picked {
        serial,
        x,
        y,
        feature: hit
            .map(|hit| hit.feature.clone())
            .or_else(|| cap.and_then(|cap| cap.feature.clone())),
        name: hit
            .map(|hit| hit.body.clone())
            .or_else(|| cap.and_then(|cap| cap.body.clone())),
        face: hit.and_then(|hit| hit.face.clone()),
        face_index: hit.and_then(|hit| u64::try_from(hit.face_index).ok()),
        point: hit
            .map(|hit| hit.point)
            .or(cap.map(|cap| cap.point))
            .map(|p| [p.x, p.y, p.z]),
        kind: found.kind().map(|kind| match kind {
            PickKind::Face => PickedKind::Face,
            PickKind::Edge => PickedKind::Edge,
            PickKind::Vertex => PickedKind::Vertex,
            PickKind::Section => PickedKind::Section,
        }),
        edge: found
            .edge
            .as_ref()
            .filter(|_| found.vertex.is_none())
            .map(entity),
        vertex: found.vertex.as_ref().map(entity),
    }
}

/// Where the pointer rests over the viewport, and what was found there.
#[derive(Debug, Default)]
pub(super) struct Hover {
    /// How many picks under the pointer were made: the `serial` of
    /// [`crate::state::UiState::hovered`].
    serial: u64,
}

impl Shell {
    /// `ui.pick` with `hover`: the pointer placed at (`x`, `y`), pixels of
    /// the viewport, without a click. What lies there becomes
    /// [`crate::state::UiState::hovered`] (`None` on nothing), as the
    /// mouse resting there makes it; it stays until the mouse moves over
    /// the viewport. `radius` as [`Self::pick`] takes it.
    pub(crate) fn hover_place(
        &mut self,
        scene: Option<&Scene>,
        x: f64,
        y: f64,
        radius: Option<f64>,
    ) -> Result<ScenePointed, String> {
        let found = self.pick(scene, x, y, None, radius, false)?;
        self.set_hovered(&found, x, y);
        Ok(found)
    }

    /// Takes `found` at (`x`, `y`) as what lies under the pointer.
    fn set_hovered(&mut self, found: &ScenePointed, x: f64, y: f64) {
        self.state.hovered = if found.kind().is_some() {
            self.hover.serial = self.hover.serial.saturating_add(1);
            Some(picked_of(found, x, y, self.hover.serial))
        } else {
            None
        };
    }

    /// `ui.get` › `hovered`: what a click would select where the pointer
    /// rests, shaped as `state.picked`; null when nothing is under it.
    pub(crate) fn hovered_json(&self) -> Value {
        self.state
            .hovered
            .as_ref()
            .and_then(|hovered| serde_json::to_value(hovered).ok())
            .unwrap_or(Value::Null)
    }
}
