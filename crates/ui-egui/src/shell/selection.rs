//! The selection as the window shows and changes it (docs/decisions.md,
//! 2026-10-21, B1; docs/design/selection.md). The selection itself is the
//! session's (`select.get`, `select.set`): the window draws it and turns
//! clicks into `select.set` requests, so an agent and the person share
//! one list of names.
//!
//! - A click replaces the selection with what is under it (a vertex, else
//!   an edge, else a face), Shift+click toggles it, a double click takes
//!   the whole body, a click on nothing clears it.
//! - Clicking again on the same spot (or Alt+click) selects the next face
//!   behind the previous one along the ray.
//! - A right click lists every face under the cursor ("select other"):
//!   hovering an entry lights it up, clicking selects it.
//! - Ctrl (or Cmd) and a drag draws a box: the bodies wholly inside it
//!   are selected, and of the others the faces wholly inside it; with
//!   Shift too they are added.
//! - Esc clears it; a click in the feature list selects that feature.

use super::Shell;
use crate::state::{CameraState, EdgeRef, FaceRef, VertexRef};
use crate::theme::{color, color_alpha};
use crate::viewport::{self, Marks};
use craft3d_engine::{
    Scene, SceneBody, ScenePick, ScenePointed, SelectionItem, SelectionKind, SelectionView,
    Session, selection_summary,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Most line segments the outlines of selected bodies and faces add to
/// what is drawn over the picture: past it, the outlines of further items
/// are left out (their faces are still lit).
pub const MAX_OUTLINE_SEGMENTS: usize = 40_000;
/// How near a second click must land to the first, in pixels, to go on
/// to the next face behind.
pub const SAME_SPOT: f64 = 3.0;
/// Most entries the "select other" list shows.
pub const MAX_OTHER: usize = 24;
/// Most selection requests waiting for the session.
const MAX_PENDING: usize = 64;

/// The window's side of the selection.
#[derive(Default)]
pub(crate) struct Selecting {
    /// The session's selection as of the last sync.
    pub(crate) view: SelectionView,
    /// What `faces` and `marks` were worked out for: the selection's
    /// serial, the scene's revision and its sampling.
    drawn_for: Option<(u64, u64, (f64, Option<f64>))>,
    /// The ids in the scene's mesh of the faces lit as selected.
    pub(crate) faces: Vec<u32>,
    /// The outlines of selected bodies and faces, selected edges and
    /// vertices: drawn over the picture, dimmed where hidden.
    pub(crate) marks: Marks,
    /// Moves whenever what is drawn for the selection changes.
    pub(crate) drawn: u64,
    /// `select.set` requests made by the mouse, run when the session is
    /// here (at the end of the frame).
    pending: Vec<Value>,
    /// The last click, for going on to the next face behind it.
    last_click: Option<Click>,
    /// The "select other" list, while it is open.
    pub(crate) menu: Option<OtherMenu>,
    /// The face of the list's entry under the pointer: lit as selected.
    pub(crate) hover: Option<u32>,
    /// The box being dragged.
    pub(crate) boxing: Option<Boxing>,
    /// Why the last request from the mouse was refused.
    pub(crate) problem: Option<String>,
    /// Whether the highlight (`state.highlight*`) is what the last click
    /// lit as it selected it: it then goes when the selection changes
    /// otherwise (an agent's `select.set`, Esc), so a face no longer
    /// selected is not left lit in the selection's colour.
    lit: bool,
    /// The selection's serial after the window's own requests last ran:
    /// a change to it is the click's own, not one to take the light off.
    own: Option<u64>,
}

/// A click in the viewport, remembered for the next one.
struct Click {
    at: [f64; 2],
    camera: CameraState,
    revision: u64,
    /// Every face under it, front to back; worked out at the second
    /// click on the spot.
    hits: Option<Vec<ScenePick>>,
    /// Which of them the last click selected.
    index: usize,
}

/// The "select other" list.
pub(crate) struct OtherMenu {
    /// Where it opens, in points.
    pub(crate) at: egui::Pos2,
    pub(crate) hits: Vec<ScenePick>,
    /// Opened with Shift held: a click toggles rather than replaces.
    pub(crate) shift: bool,
}

/// A box being dragged, in points.
#[derive(Clone, Copy)]
pub(crate) struct Boxing {
    pub(crate) from: egui::Pos2,
    pub(crate) to: egui::Pos2,
    pub(crate) add: bool,
}

/// What `items` light up in `scene`: the face ids, and the outlines and
/// marks drawn over the picture.
fn drawn(scene: &Scene, items: &[SelectionItem]) -> (Vec<u32>, Marks) {
    let mut faces: BTreeSet<u32> = BTreeSet::new();
    let mut marks = Marks::default();
    let mut room = MAX_OUTLINE_SEGMENTS;
    // The ids of a body's faces run up to the next body's first.
    let last_id = scene
        .mesh
        .face_ids
        .iter()
        .max()
        .map_or(0, |&id| id.saturating_add(1));
    let range = |index: usize| {
        let first = scene.bodies.get(index).map_or(0, |b| b.first_face_id);
        let end = scene
            .bodies
            .get(index.saturating_add(1))
            .map_or(last_id, |b| b.first_face_id);
        first..end.max(first)
    };
    let mut outline = |body: &SceneBody, wanted: &dyn Fn(usize) -> bool, marks: &mut Marks| {
        for (k, on) in body.edge_faces.iter().enumerate() {
            if !on.iter().any(|&face| wanted(face)) {
                continue;
            }
            let Some(polyline) = body.edges.get(k) else {
                continue;
            };
            for pair in polyline.windows(2) {
                if room == 0 {
                    return;
                }
                if let [a, b] = *pair {
                    marks.lines.push([a, b]);
                    room -= 1;
                }
            }
        }
    };
    for item in items {
        // A feature that made faces of its own lights those, wherever
        // they are now (a fillet's round, not the whole part); one that
        // made none (a boolean) lights its body, below.
        if item.kind == SelectionKind::Feature {
            let made = scene.feature_face_ids(&item.feature);
            if !made.is_empty() {
                faces.extend(made.iter().copied());
                // One outline per body, of the faces made in it: each
                // edge once, however many of its faces were made.
                for (index, body) in scene.bodies.iter().enumerate() {
                    let ids = range(index);
                    let mine: BTreeSet<usize> = made
                        .iter()
                        .filter(|id| ids.contains(id))
                        .filter_map(|id| usize::try_from(id - ids.start).ok())
                        .collect();
                    if !mine.is_empty() {
                        outline(body, &|face| mine.contains(&face), &mut marks);
                    }
                }
                continue;
            }
        }
        match item.kind {
            SelectionKind::Body | SelectionKind::Feature => {
                let Some(index) = scene.bodies.iter().position(|b| b.feature == item.feature)
                else {
                    continue;
                };
                faces.extend(range(index));
                if let Some(body) = scene.bodies.get(index) {
                    outline(body, &|_| true, &mut marks);
                }
            }
            SelectionKind::Face => {
                let Ok(found) = scene.faces_named(Some(&item.feature), &item.name) else {
                    continue;
                };
                for &id in &found.ids {
                    faces.insert(id);
                    if let Some((body, index)) = scene.face_of(id) {
                        outline(body, &|face| face == index, &mut marks);
                    }
                }
            }
            SelectionKind::Edge => {
                let edge = EdgeRef {
                    feature: Some(item.feature.clone()),
                    edge: Some(item.name.clone()),
                    between: Vec::new(),
                };
                marks = marks.with(viewport::edge_marks(Some(scene), &edge));
            }
            SelectionKind::Vertex => {
                let vertex = VertexRef {
                    feature: Some(item.feature.clone()),
                    vertex: Some(item.name.clone()),
                    corner: Vec::new(),
                };
                marks = marks.with(viewport::vertex_marks(Some(scene), &vertex));
            }
            // A sketch is drawn as the selected feature's; a datum by its
            // row in the tree.
            SelectionKind::Sketch | SelectionKind::Datum => {}
        }
    }
    (faces.into_iter().collect(), marks)
}

/// The item a click on `found` points at: the vertex, else the edge, else
/// the face, each when it has a persistent name.
fn clicked_item(found: &ScenePointed) -> Option<Value> {
    let named = |kind: &str, feature: &str, name: Option<&String>| {
        name.map(|name| json!({"kind": kind, "name": name, "feature": feature}))
    };
    found
        .vertex
        .as_ref()
        .and_then(|v| named("vertex", &v.feature, v.name.as_ref()))
        .or_else(|| {
            found
                .edge
                .as_ref()
                .filter(|_| found.vertex.is_none())
                .and_then(|e| named("edge", &e.feature, e.name.as_ref()))
        })
        .or_else(|| {
            found
                .face
                .as_ref()
                .and_then(|f| named("face", &f.feature, f.face.as_ref()))
        })
}

fn face_item(hit: &ScenePick) -> Option<Value> {
    hit.face
        .as_ref()
        .map(|name| json!({"kind": "face", "name": name, "feature": hit.feature}))
}

impl Shell {
    /// The selection as of the last sync, as `select.get` gives it.
    pub fn selection(&self) -> &SelectionView {
        &self.select.view
    }

    /// What is drawn for the selection: moves whenever it changes.
    pub(crate) fn selection_drawn(&self) -> u64 {
        self.select.drawn
    }

    /// The marks of the selection, drawn over the picture.
    pub(crate) fn selection_marks(&self) -> &Marks {
        &self.select.marks
    }

    /// The faces lit as selected, the hovered entry of the list included.
    pub(crate) fn selection_faces(&self) -> impl Iterator<Item = u32> + '_ {
        self.select.faces.iter().copied().chain(self.select.hover)
    }

    /// Asks for `select.set` with `params`, run when the session is here.
    pub(crate) fn queue_select(&mut self, params: Value) {
        if self.select.pending.len() < MAX_PENDING {
            self.select.pending.push(params);
        }
    }

    /// Runs the requests the mouse made. True when there were any.
    pub(crate) fn apply_selection_requests(&mut self, session: &mut Session) -> bool {
        let pending = std::mem::take(&mut self.select.pending);
        for params in &pending {
            match session.execute("select.set", params) {
                Ok(_) => self.select.problem = None,
                Err(e) => self.select.problem = Some(e.message().to_string()),
            }
        }
        if !pending.is_empty() {
            self.select.own = Some(session.selection_serial());
        }
        !pending.is_empty()
    }

    /// Brings what is drawn for the selection up to date with the
    /// session's selection and `scene`. When the selection changed, the
    /// selected feature (the tree's row, the sketch drawn as selected)
    /// follows its first body, feature, sketch or datum.
    pub(crate) fn sync_selection(&mut self, session: &mut Session, scene: &Scene) {
        // The first look at a document's selection after its tab was
        // switched to is no change of it: the list keeps where that tab
        // had it scrolled.
        let after_switch = std::mem::take(&mut self.tab_switched);
        // Copied only when it changed: it may hold thousands of names.
        let serial = session.selection_serial();
        let changed = serial != self.select.view.serial;
        if changed {
            // Changed by something else than the last click: what that
            // click lit is no longer what is selected.
            if self.select.lit && self.select.own != Some(serial) {
                self.clear_click_light();
            }
            let view = session.selection();
            self.state.selected_feature = view
                .items
                .iter()
                .find(|item| !item.kind.is_entity())
                .map(|item| item.feature.clone());
            self.select.view = view;
        }
        let key = (self.select.view.serial, scene.revision, scene.sampling());
        if self.select.drawn_for != Some(key) {
            let (faces, marks) = drawn(scene, &self.select.view.items);
            self.select.faces = faces;
            self.select.marks = marks;
            self.select.drawn = self.select.drawn.wrapping_add(1);
            self.select.drawn_for = Some(key);
            // A list of the faces of another state of the model.
            self.select.menu = None;
            self.select.hover = None;
            // Which feature made it, as the model is now.
            self.mark_made_selection(scene);
        }
        if changed && !after_switch {
            self.scroll_tree_to_selection();
        }
    }

    /// Takes off the light the last click put on what it selected.
    fn clear_click_light(&mut self) {
        self.state.highlight = None;
        self.state.highlight_edge = None;
        self.state.highlight_vertex = None;
        self.select.lit = false;
    }

    /// The highlight is now set by something else than a click (an
    /// agent's `ui.highlight`): it stays when the selection changes.
    pub(crate) fn highlight_set_elsewhere(&mut self) {
        self.select.lit = false;
    }

    /// Without a document there is nothing to draw.
    pub(crate) fn forget_selection(&mut self) {
        if self.select.drawn_for.is_some() || !self.select.view.items.is_empty() {
            self.select = super::selection::Selecting::default();
            self.select.drawn = 1;
        }
    }

    /// A click in the viewport at (`x`, `y`) in its pixels, on `found`
    /// (what `pick` found there), with Shift, Alt and a double click as
    /// given: what it selects is asked for (see the module).
    pub(crate) fn click_select(
        &mut self,
        scene: Option<&Scene>,
        x: f64,
        y: f64,
        found: &ScenePointed,
        (shift, alt, double): (bool, bool, bool),
    ) {
        self.select.menu = None;
        self.select.hover = None;
        if self.refused_while_scrubbed() {
            return;
        }
        // A click lights what it selects; a toggle lights nothing.
        self.select.lit = !shift;
        let mode = if shift { "toggle" } else { "replace" };
        let Some(scene) = scene else {
            return;
        };
        if double && let Some(face) = &found.face {
            self.queue_select(
                json!({"items": [{"kind": "body", "feature": face.feature}], "mode": mode}),
            );
            self.select.last_click = None;
            return;
        }
        let camera = self.state.camera;
        let same = self.select.last_click.as_ref().is_some_and(|last| {
            last.revision == scene.revision
                && last.camera == camera
                && (last.at[0] - x).hypot(last.at[1] - y) <= SAME_SPOT
        });
        if alt || same {
            let index = if same {
                self.select.last_click.as_ref().map_or(0, |l| l.index + 1)
            } else {
                1
            };
            let hits = match self.select.last_click.take().filter(|_| same) {
                Some(Click {
                    hits: Some(hits), ..
                }) => hits,
                _ => self.hits_at(scene, x, y),
            };
            if hits.is_empty() {
                if !shift {
                    self.queue_select(json!({"mode": "clear"}));
                }
                return;
            }
            let index = index % hits.len();
            if let Some(hit) = hits.get(index) {
                if let Some(item) = face_item(hit) {
                    self.queue_select(json!({"items": [item], "mode": mode}));
                }
                // The face gone through to is the one pointed at.
                self.state.highlight = hit.face.as_ref().map(|face| FaceRef {
                    feature: Some(hit.feature.clone()),
                    face: face.clone(),
                });
                self.state.highlight_edge = None;
                self.state.highlight_vertex = None;
            }
            self.select.last_click = Some(Click {
                at: [x, y],
                camera,
                revision: scene.revision,
                hits: Some(hits),
                index,
            });
            return;
        }
        match clicked_item(found) {
            Some(item) => self.queue_select(json!({"items": [item], "mode": mode})),
            None if !shift => self.queue_select(json!({"mode": "clear"})),
            None => {}
        }
        if shift {
            // A toggle: what is lit is the selection, not the last click.
            self.state.highlight = None;
            self.state.highlight_edge = None;
            self.state.highlight_vertex = None;
        }
        self.select.last_click = Some(Click {
            at: [x, y],
            camera,
            revision: scene.revision,
            hits: None,
            index: 0,
        });
    }

    /// Every face under (`x`, `y`) of the viewport, front to back.
    pub(crate) fn hits_at(&self, scene: &Scene, x: f64, y: f64) -> Vec<ScenePick> {
        let [width, height] = self.size;
        scene
            .pick_all_cut(
                &self.state.camera.to_render(),
                width,
                height,
                x,
                y,
                self.section().as_ref(),
            )
            .unwrap_or_default()
    }

    /// Opens the "select other" list at `at` (points) for the faces
    /// under (`x`, `y`) of the viewport's pixels.
    pub(crate) fn open_other_menu(
        &mut self,
        scene: Option<&Scene>,
        x: f64,
        y: f64,
        at: egui::Pos2,
        shift: bool,
    ) {
        let Some(scene) = scene else {
            return;
        };
        let mut hits = self.hits_at(scene, x, y);
        hits.truncate(MAX_OTHER);
        self.select.hover = None;
        self.select.menu = (!hits.is_empty()).then_some(OtherMenu { at, hits, shift });
    }

    /// The faces of the open "select other" list, front to back, as
    /// (feature, face name): what a click on entry `k` selects.
    pub fn other_menu(&self) -> Option<Vec<(String, Option<String>)>> {
        self.select.menu.as_ref().map(|menu| {
            menu.hits
                .iter()
                .map(|hit| (hit.feature.clone(), hit.face.clone()))
                .collect()
        })
    }

    /// A drag with Ctrl (or Cmd): a box. True while the drag is one, so
    /// that the camera does not move with it.
    pub(crate) fn box_drag(&mut self, ui: &egui::Ui, response: &egui::Response) -> bool {
        if response.drag_started() {
            let (command, shift, primary) = ui.input(|i| {
                (
                    i.modifiers.command,
                    i.modifiers.shift,
                    i.pointer.button_down(egui::PointerButton::Primary),
                )
            });
            let from = ui.input(|i| i.pointer.press_origin());
            if command
                && primary
                && let Some(from) = from.or_else(|| response.interact_pointer_pos())
            {
                self.select.boxing = Some(Boxing {
                    from,
                    to: from,
                    add: shift,
                });
            }
        }
        let Some(mut boxing) = self.select.boxing else {
            return false;
        };
        if let Some(at) = response.interact_pointer_pos() {
            boxing.to = at;
            self.select.boxing = Some(boxing);
        }
        if response.drag_stopped() || !response.dragged() {
            self.select.boxing = None;
            self.finish_box(boxing);
        }
        true
    }

    /// While the timeline shows an earlier step, the viewport selects
    /// nothing (decided 2026-10-23): a selection names the document as it
    /// is, where what was clicked may be another face or not there at
    /// all. Says so where a refused click does. True when refused.
    fn refused_while_scrubbed(&mut self) -> bool {
        let Some(step) = self.scrubbed_step() else {
            return false;
        };
        self.select.problem = Some(format!(
            "showing step {step} of the timeline: go To end to select in the view"
        ));
        true
    }

    /// Selects what lies wholly inside the box that was dragged.
    fn finish_box(&mut self, boxing: Boxing) {
        if self.refused_while_scrubbed() {
            return;
        }
        let (Some(rect), Some(scene)) = (self.rect, self.shown_scene.clone()) else {
            return;
        };
        let [width, height] = self.size;
        let pixel = |at: egui::Pos2| {
            let across = |from: f32, to: f32, at: f32, pixels: u32| {
                f64::from(at - from) / f64::from((to - from).max(f32::MIN_POSITIVE))
                    * f64::from(pixels)
            };
            (
                across(rect.left(), rect.right(), at.x, width),
                across(rect.top(), rect.bottom(), at.y, height),
            )
        };
        let (x0, y0) = pixel(boxing.from);
        let (x1, y1) = pixel(boxing.to);
        let found = scene
            .inside_rect(
                &self.state.camera.to_render(),
                width,
                height,
                [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)],
            )
            .unwrap_or_default();
        let items: Vec<Value> = found
            .iter()
            .filter_map(|&(body, face)| {
                let body = scene.bodies.get(body)?;
                Some(match face {
                    None => json!({"kind": "body", "feature": body.feature}),
                    Some(face) => {
                        json!({"kind": "face", "name": body.face_names.get(face)?, "feature": body.feature})
                    }
                })
            })
            .take(craft3d_engine::MAX_SELECTION)
            .collect();
        match (items.is_empty(), boxing.add) {
            (true, true) => {}
            (true, false) => self.queue_select(json!({"mode": "clear"})),
            (false, add) => self.queue_select(json!({
                "items": items,
                "mode": if add { "add" } else { "replace" },
            })),
        }
    }

    /// Esc over the viewport: closes the list, or clears the selection.
    pub(crate) fn escape_selection(&mut self) {
        if self.select.menu.take().is_some() {
            self.select.hover = None;
        } else if !self.select.view.items.is_empty() {
            self.queue_select(json!({"mode": "clear"}));
            if self.select.lit {
                self.clear_click_light();
            }
        }
    }

    /// Draws the box being dragged and the "select other" list.
    pub(crate) fn paint_selecting(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        if let Some(boxing) = self.select.boxing {
            let area = egui::Rect::from_two_pos(boxing.from, boxing.to);
            let painter = ui.painter();
            painter.rect_filled(
                area,
                0.0,
                color_alpha(tokens.face_highlight, tokens.select_box_opacity),
            );
            painter.rect_stroke(
                area,
                0.0,
                egui::Stroke::new(tokens.select_box_width, color(tokens.face_highlight)),
                egui::StrokeKind::Inside,
            );
        }
        let Some(menu) = &self.select.menu else {
            return;
        };
        let mut hovered = None;
        let mut chosen = None;
        let mut mate = None;
        let shift = menu.shift;
        // Two faces of two instances of an assembly selected: the mates
        // that join them come first (`mate.add`).
        let pair = self.mate_pair();
        let mut mate_hits = Vec::new();
        let response = egui::Area::new(egui::Id::new("craft3d-select-other"))
            .order(egui::Order::Foreground)
            .fixed_pos(menu.at)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if let Some((a, b)) = &pair {
                        ui.label(egui::RichText::new("Mate the selected faces").strong())
                            .on_hover_text(format!("{a} and {b}"));
                        for (kind, words) in super::joints::MATE_ENTRIES {
                            let entry = ui.selectable_label(false, words);
                            mate_hits.push((kind, entry.rect));
                            if entry.clicked() {
                                mate = Some(kind);
                            }
                        }
                        ui.separator();
                    }
                    ui.label(egui::RichText::new("Select other").strong());
                    for (k, hit) in menu.hits.iter().enumerate() {
                        let face = hit.face.as_deref().unwrap_or("face");
                        let text = format!("{}. {face} · {}", k + 1, hit.body);
                        let entry = ui.selectable_label(false, text);
                        if entry.hovered() {
                            hovered = Some(hit.face_id);
                        }
                        if entry.clicked() {
                            chosen = Some(k);
                        }
                    }
                });
            })
            .response;
        self.joints.mate_hits = mate_hits;
        if let (Some(kind), Some((a, b))) = (mate, pair) {
            self.timeline
                .queue("mate.add", json!({"kind": kind, "a": a, "b": b}));
            self.select.menu = None;
            self.select.hover = None;
            self.select.drawn = self.select.drawn.wrapping_add(1);
            ui.ctx().request_repaint();
            return;
        }
        if self.select.hover != hovered {
            self.select.hover = hovered;
            self.select.drawn = self.select.drawn.wrapping_add(1);
        }
        if let Some(k) = chosen {
            let item = self
                .select
                .menu
                .as_ref()
                .and_then(|menu| menu.hits.get(k))
                .and_then(face_item);
            if let Some(item) = item {
                let mode = if shift { "toggle" } else { "replace" };
                self.queue_select(json!({"items": [item], "mode": mode}));
            }
            self.select.menu = None;
            self.select.hover = None;
            self.select.drawn = self.select.drawn.wrapping_add(1);
        } else if ui.input(|i| i.pointer.any_pressed())
            && !response.contains_pointer()
            && ui.input(|i| !i.pointer.button_pressed(egui::PointerButton::Secondary))
        {
            // A click elsewhere closes it.
            self.select.menu = None;
            self.select.hover = None;
            self.select.drawn = self.select.drawn.wrapping_add(1);
        }
    }

    /// The status bar's words for the selection: "3 bodies, 2 faces
    /// selected", and how many dropped out; `None` when nothing is
    /// selected or dropped.
    pub(crate) fn selection_status(&self) -> Option<(String, bool)> {
        let view = &self.select.view;
        let dropped = view.dropped.len();
        if view.items.is_empty() && dropped == 0 && self.select.problem.is_none() {
            return None;
        }
        let mut text = if view.items.is_empty() {
            "nothing selected".to_string()
        } else {
            format!("{} selected", selection_summary(&view.items))
        };
        if dropped > 0 {
            text = format!("{text} · {dropped} gone from the selection");
        }
        if let Some(problem) = &self.select.problem {
            text = format!("{text} · {problem}");
        }
        Some((text, dropped > 0 || self.select.problem.is_some()))
    }
}
