//! What a click would select, under the pointer: the pick where the
//! pointer rests over the viewport ([`UiState::hovered`], shaped as the
//! last click's [`UiState::picked`]). The control channel places the
//! pointer without a click (`ui.pick {x, y, hover: true}`), so pictures can
//! show what the pointer lights up.
//!
//! [`UiState::hovered`]: crate::state::UiState::hovered
//! [`UiState::picked`]: crate::state::UiState::picked

use super::{Shell, edge_ref, named_vertex};
use crate::state::{CameraState, EdgeRef, FaceRef, Picked, PickedEntity, PickedKind, VertexRef};
use crate::viewport;
use craft3d_engine::{PickKind, Scene, SceneNear, ScenePointed, SelectionKind};
use craft3d_render::Section;
use serde_json::{Value, json};
// The browser has no clock in `std` (`std::time::Instant::now` panics
// there).
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

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

/// Seconds between two picks under a moving pointer: at most 30 a second.
pub const HOVER_EVERY: f64 = 0.033;
/// A pick under the pointer that has cost more than this many
/// milliseconds (the median of the last ones) waits until the pointer has
/// rested [`HOVER_REST`] seconds, and nothing is outlined while it moves.
pub const HOVER_QUICK_MS: f64 = 4.0;
pub const HOVER_REST: f64 = 0.080;
/// A pick under the pointer that has cost more than this many milliseconds
/// (the median of the last ones) on the model shown stops the hover there:
/// the window would stall a frame at every rest. `ui.pick` with `hover`
/// still places one. Measured again on a model of another size.
pub const HOVER_MAX_MS: f64 = 16.0;
/// Most segments of a face's boundary outlined under the pointer.
pub const MAX_HOVER_SEGMENTS: usize = 20_000;
/// How many of the last picks their cost is the median of.
const COSTS: usize = 15;

/// What a pick under the pointer was made for: the place, the camera, the
/// scene and the section, the viewport's size. Another one asks again.
#[derive(Clone, Debug, PartialEq)]
struct HoverKey {
    at: [u64; 2],
    camera: CameraState,
    scene: (usize, u64, (f64, Option<f64>)),
    section: Option<Section>,
    size: [u32; 2],
    /// What is selected and lit by the last click: a thing already chosen
    /// is not outlined.
    selection: u64,
    lit: (Option<FaceRef>, Option<EdgeRef>, Option<VertexRef>),
}

/// Where the pointer rests over the viewport, and what was found there.
#[derive(Debug, Default)]
pub(super) struct Hover {
    /// How many picks under the pointer were made: the `serial` of
    /// [`crate::state::UiState::hovered`].
    serial: u64,
    /// Where the pointer rests, in viewport pixels; `None` while it is not
    /// over the viewport.
    at: Option<[f64; 2]>,
    /// The pointer was placed over the control channel (`ui.pick` with
    /// `hover`): it stays there until the mouse moves over the viewport.
    placed: bool,
    /// The mouse over the viewport as the last frame had it, in points,
    /// and when it last moved (the input's time, in seconds).
    pointer: Option<egui::Pos2>,
    moved: f64,
    /// When the last pick ran.
    last: f64,
    found_for: Option<HoverKey>,
    /// The outline drawn for it, and a number that moves when it changes
    /// (the frame's key).
    marks: viewport::Marks,
    drawn: u64,
    /// What the last picks cost, in milliseconds, and the size of the
    /// model they were made on (its triangles).
    costs: Vec<f64>,
    costs_for: Option<usize>,
    /// How many picks the frames made under the pointer.
    picks: u64,
    /// What lies under the pointer is selected or lit already: not
    /// outlined.
    chosen: bool,
}

impl Hover {
    /// The median cost of the last picks, in milliseconds; 0 before any.
    fn cost(&self) -> f64 {
        let mut costs = self.costs.clone();
        costs.sort_by(f64::total_cmp);
        costs.get(costs.len() / 2).copied().unwrap_or(0.0)
    }

    fn note_cost(&mut self, milliseconds: f64) {
        if !milliseconds.is_finite() {
            return;
        }
        if self.costs.len() >= COSTS {
            self.costs.remove(0);
        }
        self.costs.push(milliseconds);
    }
}

/// Whether two picks name the same thing to select: the same vertex, or
/// edge, of the same body (whatever face lies under the position), or the
/// same face.
fn same_thing(a: &Picked, b: &Picked) -> bool {
    let entity = |e: &Option<PickedEntity>| e.as_ref().map(|e| (e.feature.clone(), e.index));
    a.kind == b.kind
        && match a.kind {
            Some(PickedKind::Vertex) => entity(&a.vertex) == entity(&b.vertex),
            Some(PickedKind::Edge) => entity(&a.edge) == entity(&b.edge),
            _ => a.feature == b.feature && a.face == b.face && a.face_index == b.face_index,
        }
}

/// The outline of what `found` points at in `scene`: the vertex as a dot,
/// the edge along its chords, a face by the edges that bound it (at most
/// [`MAX_HOVER_SEGMENTS`] segments). Nothing for the cap of the section,
/// which a click does not select.
fn outline_of(scene: &Scene, found: &ScenePointed) -> viewport::Marks {
    let mut marks = viewport::Marks::default();
    match found.kind() {
        Some(PickKind::Vertex) => {
            marks
                .hover_points
                .extend(found.vertex.as_ref().map(|near| near.point));
        }
        Some(PickKind::Edge) => {
            if let Some(edge) = found.edge.as_ref().and_then(edge_ref) {
                marks.hover_edges = viewport::edge_marks(Some(scene), &edge).lines;
            }
        }
        Some(PickKind::Face) => {
            let Some((body, index)) = found
                .face
                .as_ref()
                .and_then(|face| scene.face_of(face.face_id))
            else {
                return marks;
            };
            let mut room = MAX_HOVER_SEGMENTS;
            'edges: for (k, on) in body.edge_faces.iter().enumerate() {
                if !on.contains(&index) {
                    continue;
                }
                let Some(polyline) = body.edges.get(k) else {
                    continue;
                };
                for pair in polyline.windows(2) {
                    if room == 0 {
                        break 'edges;
                    }
                    if let [a, b] = *pair {
                        marks.hover_lines.push([a, b]);
                        room -= 1;
                    }
                }
            }
        }
        Some(PickKind::Section) | None => {}
    }
    marks
}

fn seconds(s: f64) -> std::time::Duration {
    std::time::Duration::try_from_secs_f64(s.clamp(0.0, 1.0)).unwrap_or_default()
}

impl Shell {
    /// `ui.pick` with `hover`: the pointer placed at (`x`, `y`), pixels of
    /// the viewport, without a click. What lies there becomes
    /// [`crate::state::UiState::hovered`] (`None` on nothing) and is
    /// outlined, as the mouse resting there makes it; it stays until the
    /// mouse moves over the viewport. `radius` as [`Self::pick`] takes it.
    pub(crate) fn hover_place(
        &mut self,
        scene: Option<&Scene>,
        x: f64,
        y: f64,
        radius: Option<f64>,
    ) -> Result<ScenePointed, String> {
        let found = self.pick(scene, x, y, None, radius, false)?;
        self.hover.at = Some([x, y]);
        self.hover.placed = true;
        self.hover.found_for = scene.map(|scene| self.hover_key(scene, [x, y]));
        // On something drawn over the model that takes the click (a
        // note's pin): nothing of the model is outlined.
        if self
            .hover_point()
            .is_some_and(|at| self.taken_elsewhere(at))
        {
            self.clear_hovered();
        } else {
            self.take_hovered(scene, &found, x, y);
        }
        Ok(found)
    }

    /// What the pointer's outline was drawn for: moves when it changes.
    pub(crate) fn hover_drawn(&self) -> u64 {
        self.hover.drawn
    }

    /// The outline of what a click would select under the pointer, drawn
    /// over the picture with the other marks.
    pub(crate) fn hover_marks(&self) -> viewport::Marks {
        if self.hover_fresh() {
            self.hover.marks.clone()
        } else {
            viewport::Marks::default()
        }
    }

    fn hover_key(&self, scene: &Scene, [x, y]: [f64; 2]) -> HoverKey {
        HoverKey {
            at: [x.to_bits(), y.to_bits()],
            camera: self.state.camera,
            scene: (
                std::ptr::from_ref(scene).addr(),
                scene.revision,
                scene.sampling(),
            ),
            section: self.section(),
            size: self.size,
            selection: self.selection_drawn(),
            lit: (
                self.state.highlight.clone(),
                self.state.highlight_edge.clone(),
                self.state.highlight_vertex.clone(),
            ),
        }
    }

    /// Whether what `found` points at is selected already, or lit by the
    /// last click: a person who has just clicked a thing needs no outline
    /// of what a click would select there.
    fn already_chosen(&self, found: &ScenePointed) -> bool {
        let items = &self.select.view.items;
        let has = |kind: SelectionKind, feature: &str, name: Option<&str>| {
            name.is_some_and(|name| {
                items
                    .iter()
                    .any(|i| i.kind == kind && i.feature == feature && i.name == name)
            })
        };
        match found.kind() {
            Some(PickKind::Vertex) => found.vertex.as_ref().is_some_and(|near| {
                has(SelectionKind::Vertex, &near.feature, near.name.as_deref())
                    || named_vertex(&near.feature, near.name.as_deref(), &near.faces)
                        .is_some_and(|v| self.state.highlight_vertex.as_ref() == Some(&v))
            }),
            Some(PickKind::Edge) => found.edge.as_ref().is_some_and(|near| {
                has(SelectionKind::Edge, &near.feature, near.name.as_deref())
                    || edge_ref(near)
                        .is_some_and(|e| self.state.highlight_edge.as_ref() == Some(&e))
            }),
            Some(PickKind::Face) => found.face.as_ref().is_some_and(|hit| {
                self.select.faces.binary_search(&hit.face_id).is_ok()
                    || hit.face.as_ref().is_some_and(|face| {
                        self.state.highlight.as_ref().is_some_and(|lit| {
                            &lit.face == face
                                && lit.feature.as_deref().is_none_or(|f| f == hit.feature)
                        })
                    })
            }),
            Some(PickKind::Section) | None => false,
        }
    }

    /// Whether what the hover found was found on the scene shown now (an
    /// edit or another tab since leaves it stale until the next frame).
    fn hover_fresh(&self) -> bool {
        let shown = self.shown_scene_arc();
        match (&self.hover.found_for, shown.as_deref()) {
            (Some(key), Some(scene)) => {
                key.scene.0 == std::ptr::from_ref(scene).addr() && key.scene.1 == scene.revision
            }
            _ => false,
        }
    }

    /// How many picks the frames made under the pointer (`ui.pick` with
    /// `hover` not counted).
    pub fn hover_picks(&self) -> u64 {
        self.hover.picks
    }

    /// Takes `milliseconds` as what the last picks under the pointer cost
    /// on the model shown now: for tests of the ceiling
    /// ([`HOVER_MAX_MS`]), which a clock cannot hold still.
    pub fn note_hover_costs(&mut self, milliseconds: &[f64]) {
        self.hover.costs.clear();
        for &ms in milliseconds {
            self.hover.note_cost(ms);
        }
        self.hover.costs_for = self.shown_scene_arc().map(|s| s.mesh.triangles.len());
    }

    /// Takes `found` at (`x`, `y`) as what lies under the pointer, and
    /// outlines it when it is another thing than before.
    fn take_hovered(&mut self, scene: Option<&Scene>, found: &ScenePointed, x: f64, y: f64) {
        let next = if found.kind().is_some() {
            self.hover.serial = self.hover.serial.saturating_add(1);
            Some(picked_of(found, x, y, self.hover.serial))
        } else {
            None
        };
        let same = match (&self.state.hovered, &next) {
            (Some(a), Some(b)) => same_thing(a, b),
            (None, None) => true,
            _ => false,
        };
        let chosen = next.is_some() && self.already_chosen(found);
        if !same || chosen != self.hover.chosen {
            self.hover.marks = match scene {
                Some(scene) if !chosen => outline_of(scene, found),
                _ => viewport::Marks::default(),
            };
            self.hover.chosen = chosen;
            self.hover.drawn = self.hover.drawn.wrapping_add(1);
        }
        self.state.hovered = next;
    }

    /// Nothing under the pointer: nothing outlined.
    fn clear_hovered(&mut self) {
        let had = self.state.hovered.take().is_some();
        if had || !self.hover.marks.is_empty() {
            self.hover.marks = viewport::Marks::default();
            self.hover.drawn = self.hover.drawn.wrapping_add(1);
        }
        self.hover.found_for = None;
    }

    /// Where the pointer rests, in points of the window: the mouse's, or
    /// the place `ui.pick` with `hover` put it (pixels of the viewport,
    /// once the viewport was drawn). Kept whatever lies there, the model or
    /// something drawn over it (a note's pin reads it to light up).
    pub(crate) fn hover_point(&self) -> Option<egui::Pos2> {
        let ([x, y], rect) = self.hover.at.zip(self.rect)?;
        let [width, height] = self.size;
        let across = |from: f32, length: f32, at: f64, pixels: u32| {
            from + (at / f64::from(pixels.max(1)) * f64::from(length)) as f32
        };
        Some(egui::pos2(
            across(rect.left(), rect.width(), x, width),
            across(rect.top(), rect.height(), y, height),
        ))
    }

    /// Whether a click at `at` (points) is taken by something drawn over
    /// the model: the section's handle, a note's pin, the dimension's
    /// number, a mate's marker.
    fn taken_elsewhere(&self, at: egui::Pos2) -> bool {
        let reach = self.tokens.note_radius + 3.0;
        self.section_handle()
            .is_some_and(|place| place.part_at(at).is_some() || place.group.contains(at))
            || self.pin_at(at).is_some()
            || self
                .dimension_place()
                .is_some_and(|place| place.number.expand(2.0).contains(at))
            || self
                .mate_marker_hits()
                .iter()
                .any(|(_, c)| c.distance(at) <= reach)
    }

    /// Once a frame, from the viewport: where the mouse rests over the
    /// model with no button down and the camera still, what a click would
    /// select there becomes [`crate::state::UiState::hovered`] and is
    /// outlined in the selection's colour. The pick runs at most once per
    /// [`HOVER_EVERY`] and only when the place, the camera, the scene or
    /// the section changed; one that costs more than [`HOVER_QUICK_MS`]
    /// waits until the pointer has rested [`HOVER_REST`]. Nothing is
    /// outlined while the camera moves, a button is down, a finger
    /// touches, the "select other" list is open, the view shows a step of
    /// the timeline (where a click selects nothing), or the pointer is on
    /// something drawn over the model that takes the click.
    pub(super) fn hover_frame(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        scene: Option<&Scene>,
        moving: bool,
    ) {
        let now = ui.input(|i| i.time);
        let (pointer, down, touch) =
            ui.input(|i| (i.pointer.hover_pos(), i.pointer.any_down(), i.any_touches()));
        let over = pointer.filter(|at| response.hovered() && rect.contains(*at));
        if let Some(at) = over {
            if self.hover.pointer != Some(at) {
                self.hover.pointer = Some(at);
                self.hover.moved = now;
                self.hover.placed = false;
                let [width, height] = self.size;
                let across = |from: f32, to: f32, at: f32, pixels: u32| {
                    f64::from(at - from) / f64::from((to - from).max(f32::MIN_POSITIVE))
                        * f64::from(pixels)
                };
                self.hover.at = Some([
                    across(rect.left(), rect.right(), at.x, width),
                    across(rect.top(), rect.bottom(), at.y, height),
                ]);
            }
        } else if !self.hover.placed {
            self.hover.pointer = None;
            self.hover.at = None;
        }
        let blocked = moving
            || touch
            || self.touch_gesture
            || self.select.menu.is_some()
            || self.scrubbed_step().is_some()
            || self
                .hover_point()
                .is_some_and(|at| self.taken_elsewhere(at));
        let (Some(scene), Some(at), false) = (scene, self.hover.at, blocked) else {
            self.clear_hovered();
            return;
        };
        // A button down: what is outlined holds, no new pick (a click
        // does not flicker; a drag clears it as the camera moves).
        if down {
            return;
        }
        // The costs measured on a model of another size do not hold.
        let triangles = scene.mesh.triangles.len();
        let other = self.hover.costs_for.is_none_or(|n| {
            let ratio = triangles as f64 / n.max(1) as f64;
            !(0.5..=2.0).contains(&ratio)
        });
        if other {
            self.hover.costs.clear();
            self.hover.costs_for = Some(triangles);
        }
        // Too dear on this model: no hover but one an agent placed.
        if self.hover.cost() > HOVER_MAX_MS && !self.hover.placed {
            self.clear_hovered();
            return;
        }
        let key = self.hover_key(scene, at);
        if self.hover.found_for.as_ref() == Some(&key) {
            return;
        }
        let rested = now - self.hover.moved;
        if self.hover.cost() > HOVER_QUICK_MS && rested < HOVER_REST && !self.hover.placed {
            // Too dear to follow the pointer: asked once it rests.
            self.clear_hovered();
            ui.ctx().request_repaint_after(seconds(HOVER_REST - rested));
            return;
        }
        let since = now - self.hover.last;
        if since < HOVER_EVERY {
            ui.ctx().request_repaint_after(seconds(HOVER_EVERY - since));
            return;
        }
        let [x, y] = at;
        let started = Instant::now();
        let found = self.pick(Some(scene), x, y, None, None, false);
        self.hover
            .note_cost(started.elapsed().as_secs_f64() * 1000.0);
        self.hover.picks = self.hover.picks.saturating_add(1);
        self.hover.last = now;
        self.hover.found_for = Some(key);
        match found {
            Ok(found) => self.take_hovered(Some(scene), &found, x, y),
            Err(_) => self.clear_hovered(),
        }
    }

    /// `ui.get` › `hovered`: what a click would select where the pointer
    /// rests, shaped as `state.picked`; null when nothing is under it.
    pub(crate) fn hovered_json(&self) -> Value {
        // On a note's pin: the note, whose pin a click chooses.
        if let Some(id) = self.hover_point().and_then(|at| self.pin_at(at)) {
            let [x, y] = self.hover.at.unwrap_or_default();
            return json!({"kind": "note", "id": id, "x": x, "y": y, "serial": self.hover.serial});
        }
        if !self.hover_fresh() {
            return Value::Null;
        }
        self.state
            .hovered
            .as_ref()
            .and_then(|hovered| serde_json::to_value(hovered).ok())
            .unwrap_or(Value::Null)
    }

    /// The note whose pin, as the last frame drew it, is within reach of
    /// `at` (points): the nearest.
    fn pin_at(&self, at: egui::Pos2) -> Option<u64> {
        let reach = self.tokens.note_radius + 3.0;
        self.notes
            .markers
            .iter()
            .filter(|(_, c)| c.distance(at) <= reach)
            .min_by(|a, b| a.1.distance(at).total_cmp(&b.1.distance(at)))
            .map(|(id, _)| *id)
    }
}
