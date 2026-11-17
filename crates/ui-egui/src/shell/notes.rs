//! The notes pinned to the model in the view (the developer's design of
//! 2026-11-17, `plan/viewer-tools`, section 1; `docs/guide/window.md` ›
//! *Notes*): each note's marker, a small numbered pin at its place
//! (`at` of `note.list`, a detached note at its stored point, drawn in
//! grey), a done one a quiet tick; the chosen note's text beside its
//! marker; the field a note is written in.
//!
//! - **Writing a note:** with one body, face, edge or vertex selected,
//!   **N** (or *Note here…* in the list under the right mouse button)
//!   opens one field at that place; Enter adds the note (`note.add`, as
//!   the person: `Session::by_person`), Escape leaves. The point is the
//!   click's on that entity (the last click when it was on it; the front
//!   face under the right click), else the entity's middle.
//! - **Markers:** drawn in the accent, small; behind the bodies (a ray
//!   from the eye through the marker meets a face nearer than it by more
//!   than a hundredth of the model's size) faint and not clickable, as
//!   the mates' markers behind a face are. A click on a marker chooses its
//!   note (`notes.selected`) and shows its text. Thinned for many notes:
//!   one marker per cell of `Tokens::note_cell` points (the chosen note,
//!   else an agent's open question, else an open note, else a done one,
//!   lowest id first), with how many more it stands for; at most
//!   [`MAX_MARKERS`] cells drawn. What is behind a face is looked up once
//!   per camera, document state and size, for the markers drawn only.
//!
//! The window only views: every change is a command, one undo step.

use super::Shell;
use super::views::FlyTo;
use crate::camera::Projector;
use crate::theme::{color, color_alpha};
use craft3d_engine::{Scene, SelectionKind, Session};
use craft3d_math::Vec3;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The key that writes a note on what is selected.
pub const NOTE_KEY: egui::Key = egui::Key::N;

/// Most markers drawn: cells beyond these (by their notes' ids) are not
/// drawn, so a document with a thousand notes costs no more per frame
/// than one with this many.
pub const MAX_MARKERS: usize = 400;

/// One note as the window shows it, read from `note.list`.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteRow {
    pub id: u64,
    pub text: String,
    pub done: bool,
    /// `person`, `agent` or `unknown`, and the agent's name.
    pub author: String,
    pub agent: Option<String>,
    pub time_ms: u64,
    /// `attached`, `byPlace`, `lost`, `hidden`, `detached`, or empty
    /// where the places were not read (the bodies are built elsewhere).
    pub attachment: String,
    /// Where it is, in words: "face of Rib", "by place, near Round",
    /// "place gone, near Round", "detached".
    pub place: String,
    /// What a click flies to: the entity now (kind, name, feature).
    pub entity: Option<(String, String, String)>,
    /// Where its marker goes: `at`, else the stored point of a detached
    /// note or one whose place is gone; `None` while it is hidden.
    pub marker: Option<[f64; 3]>,
    pub reply: Option<String>,
    /// Who closed it: `person`, `agent` (with the name) or `unknown`.
    pub closed_by: Option<String>,
}

impl NoteRow {
    /// An open note an agent wrote: its question for the person.
    pub fn question(&self) -> bool {
        !self.done && self.author == "agent"
    }

    /// Off its entity: detached, or held by place where nothing is any
    /// more. Drawn grey at its own point.
    pub fn astray(&self) -> bool {
        matches!(self.attachment.as_str(), "detached" | "lost")
    }

    /// Who, in a few words: "you", "Claude Code", "unknown".
    pub fn who(&self) -> String {
        match (self.author.as_str(), &self.agent) {
            ("person", _) => "you".to_string(),
            ("agent", Some(name)) => name.clone(),
            ("agent", None) => "an agent".to_string(),
            _ => "unknown".to_string(),
        }
    }
}

/// The rows of `listed` (`note.list`, or `Session::notes_json`).
pub fn rows_of(listed: &Value) -> Vec<NoteRow> {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let point = |v: Option<&Value>| -> Option<[f64; 3]> {
        let a = v?.as_array()?;
        let c = |k: usize| a.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        Some([c(0)?, c(1)?, c(2)?])
    };
    let Some(notes) = listed.get("notes").and_then(Value::as_array) else {
        return Vec::new();
    };
    notes
        .iter()
        .filter_map(|n| {
            let id = n.get("id").and_then(Value::as_u64)?;
            let attachment = text(n, "attachment").unwrap_or_default();
            let on = n.get("on");
            let kind = on.and_then(|o| text(o, "kind")).unwrap_or_default();
            let made = on
                .and_then(|o| o.get("origin"))
                .and_then(|o| text(o, "name"))
                .or_else(|| on.and_then(|o| text(o, "body")))
                .unwrap_or_default();
            let place = match attachment.as_str() {
                "byPlace" => format!("by place, near {made}"),
                "lost" => format!("place gone, near {made}"),
                "detached" => "detached".to_string(),
                "hidden" => format!("{kind} of {made}, hidden for now"),
                _ if kind == "body" => format!("body {made}"),
                _ => format!("{kind} of {made}"),
            };
            // What it is now: the first piece, else what it was pinned to.
            let now = n
                .get("now")
                .and_then(Value::as_array)
                .and_then(|list| list.first());
            let entity = match attachment.as_str() {
                "attached" => now
                    .or(on)
                    .and_then(|e| Some((text(e, "kind")?, text(e, "name")?, text(e, "feature")?))),
                _ => None,
            };
            let marker = match attachment.as_str() {
                "attached" | "byPlace" => point(n.get("at")),
                "detached" | "lost" => point(n.get("point")),
                _ => None,
            };
            let done = n.get("done").filter(|d| d.is_object());
            Some(NoteRow {
                id,
                text: text(n, "text").unwrap_or_default(),
                done: done.is_some(),
                author: text(n, "author").unwrap_or_default(),
                agent: text(n, "agent"),
                time_ms: n.get("timeMs").and_then(Value::as_u64).unwrap_or(0),
                attachment,
                place,
                entity,
                marker,
                reply: done.and_then(|d| text(d, "reply")),
                closed_by: done.map(|d| match (text(d, "author").as_deref(), text(d, "agent")) {
                    (Some("agent"), Some(name)) => name,
                    (Some("person"), _) => "you".to_string(),
                    _ => "unknown".to_string(),
                }),
            })
        })
        .collect()
}

/// The field a note is being written in.
#[derive(Clone, Debug, PartialEq)]
pub struct Writing {
    /// What it goes on: `on` and `feature` as `note.add` takes them.
    pub on: String,
    pub feature: Option<String>,
    /// The click's point on it, when there was one.
    pub point: Option<[f64; 3]>,
    pub text: String,
    /// Where the field is shown, in points.
    pub at: egui::Pos2,
    /// The field takes the keyboard in its first frame.
    pub focus: bool,
}

/// What is behind a face is looked up for this camera, state and size.
type SeenKey = (crate::state::CameraState, u64, [u32; 2], Option<(u64, u64)>);

/// The notes' memory between frames.
#[derive(Clone, Debug, Default)]
pub struct NoteMemory {
    /// What `note.list` said, and for which document and revision.
    pub rows: Arc<Vec<NoteRow>>,
    pub(crate) read_for: Option<(u64, u64)>,
    pub writing: Option<Writing>,
    /// The markers drawn in the last frame that a click chooses: each
    /// note's id and its centre, in points.
    pub markers: Vec<(u64, egui::Pos2)>,
    /// How many markers the last frame drew, and how many notes they
    /// stood for.
    pub drawn: (usize, usize),
    /// Which drawn markers are behind a face, by note id.
    seen: Option<(SeenKey, BTreeMap<u64, bool>)>,
    /// A note's text being changed in the tab: its id and the text.
    pub(crate) editing: Option<(u64, String)>,
    pub(crate) edit_focus: bool,
    /// Where the last frame drew *Note here…* in the "select other" list.
    pub(crate) here_hit: Option<egui::Rect>,
}

impl Shell {
    /// The notes as the window last read them.
    pub fn note_rows(&self) -> &[NoteRow] {
        &self.notes.rows
    }

    /// The markers drawn in the last frame (note id, centre in points),
    /// and how many notes they stood for.
    pub fn note_markers(&self) -> (&[(u64, egui::Pos2)], usize) {
        (&self.notes.markers, self.notes.drawn.1)
    }

    /// Where the last frame drew *Note here…* in the list under the right
    /// mouse button, when it was offered.
    pub fn note_here_entry(&self) -> Option<egui::Rect> {
        self.notes.here_hit
    }

    /// The field a note is being written in, when one is open.
    pub fn note_writing(&self) -> Option<&Writing> {
        self.notes.writing.as_ref()
    }

    /// Types `text` into the open field (for tests and an agent showing
    /// the person around); `submit` adds the note as Enter does.
    pub fn type_note(&mut self, text: &str, submit: bool) {
        if let Some(writing) = &mut self.notes.writing {
            writing.text = text.to_string();
        }
        if submit {
            self.submit_note();
        }
    }

    /// How many open notes agents left: their questions for the person.
    pub fn notes_questions(&self) -> usize {
        self.notes.rows.iter().filter(|r| r.question()).count()
    }

    /// Reads the notes when the document changed: with their places
    /// (`note.list`) where the bodies are here, else as kept
    /// (`Session::notes_json`, the web page's worker builds them).
    pub(crate) fn sync_notes(&mut self, session: &mut Session, key: (u64, u64), bodies_here: bool) {
        if self.notes.read_for == Some(key) {
            return;
        }
        self.notes.read_for = Some(key);
        let listed = if bodies_here {
            session
                .execute("note.list", &json!({}))
                .unwrap_or_else(|_| session.notes_json())
        } else {
            session.notes_json()
        };
        self.notes.rows = Arc::new(rows_of(&listed));
        self.notes.seen = None;
        if let Some(selected) = self.state.notes.selected
            && !self.notes.rows.iter().any(|r| r.id == selected)
        {
            self.state.notes.selected = None;
        }
        if let Some((id, _)) = &self.notes.editing
            && !self.notes.rows.iter().any(|r| r.id == *id)
        {
            self.notes.editing = None;
        }
    }

    /// Shows the Notes tab (the agent pill's click while agents' questions
    /// are open).
    pub fn show_notes_tab(&mut self) {
        self.state.panels.side_tab = crate::state::SideTab::Notes;
        self.state.panels.tree = true;
    }

    /// Whether *Note here…* and **N** can write a note: one body, face,
    /// edge or vertex selected, in a part.
    pub(crate) fn note_here_offered(&self) -> bool {
        !self.is_assembly_shown()
            && matches!(
                self.selection().items.as_slice(),
                [one] if matches!(
                    one.kind,
                    SelectionKind::Body | SelectionKind::Face | SelectionKind::Edge | SelectionKind::Vertex
                )
            )
    }

    /// Opens the field to write a note on the one item selected; `front`
    /// is what was under the right click (feature, face, point).
    pub(crate) fn start_note(&mut self, front: Option<(String, Option<String>, Vec3)>) {
        if !self.note_here_offered() {
            return;
        }
        let Some(item) = self.selection().items.first().cloned() else {
            return;
        };
        let picked = self.state.picked.clone().unwrap_or_default();
        let point = match item.kind {
            SelectionKind::Face | SelectionKind::Body => front
                .filter(|(feature, face, _)| {
                    *feature == item.feature
                        && (item.kind == SelectionKind::Body || face.as_deref() == Some(&item.name))
                })
                .map(|(_, _, p)| [p.x, p.y, p.z])
                .or_else(|| {
                    let on = picked.feature.as_deref() == Some(item.feature.as_str())
                        && (item.kind == SelectionKind::Body
                            || picked.face.as_deref() == Some(item.name.as_str()));
                    picked.point.filter(|_| on)
                }),
            SelectionKind::Edge => picked
                .edge
                .as_ref()
                .filter(|e| e.feature == item.feature && e.name.as_deref() == Some(&item.name))
                .map(|e| e.point),
            SelectionKind::Vertex => picked
                .vertex
                .as_ref()
                .filter(|v| v.feature == item.feature && v.name.as_deref() == Some(&item.name))
                .map(|v| v.point),
            _ => None,
        };
        let (on, feature) = match item.kind {
            SelectionKind::Body => (item.feature.clone(), None),
            _ => (item.name.clone(), Some(item.feature.clone())),
        };
        let at = point
            .and_then(|p| self.note_screen(p))
            .or_else(|| self.rect.map(|r| r.center()))
            .unwrap_or(egui::pos2(100.0, 100.0));
        self.notes.writing = Some(Writing {
            on,
            feature,
            point,
            text: String::new(),
            at,
            focus: true,
        });
    }

    /// Adds the note being written (`note.add`, as the person), when it
    /// says something; closes the field either way.
    fn submit_note(&mut self) {
        let Some(writing) = self.notes.writing.take() else {
            return;
        };
        if writing.text.trim().is_empty() {
            return;
        }
        let mut params = json!({"text": writing.text, "on": writing.on});
        if let Some(map) = params.as_object_mut() {
            if let Some(feature) = writing.feature {
                map.insert("feature".into(), feature.into());
            }
            if let Some(point) = writing.point {
                map.insert("point".into(), json!(point));
            }
        }
        self.side_queue("note.add", params, None);
    }

    /// Chooses note `id`, and flies to what it is on (its entity
    /// selected), when it is on something.
    pub(crate) fn choose_note(&mut self, id: u64, fly: bool) {
        self.state.notes.selected = Some(id);
        if !fly {
            return;
        }
        let entity = self
            .notes
            .rows
            .iter()
            .find(|r| r.id == id)
            .and_then(|r| r.entity.clone());
        if let Some((kind, name, feature)) = entity {
            let item = if kind == "body" {
                json!({"kind": "body", "feature": feature})
            } else {
                json!({"kind": kind, "name": name, "feature": feature})
            };
            self.queue_select(json!({ "items": [item] }));
            self.request_fly(FlyTo::Selection);
        }
    }

    /// Where model point `p` is on screen, in points.
    fn note_screen(&self, p: [f64; 3]) -> Option<egui::Pos2> {
        let rect = self.rect?;
        let [w, h] = self.size;
        let projector = Projector::new(
            &self.state.camera.to_render(),
            f64::from(w.max(1)),
            f64::from(h.max(1)),
        )?;
        let at = projector.project(Vec3::new(p[0], p[1], p[2]))?;
        let scale = f64::from(self.pixels_per_point.max(0.1));
        Some(egui::pos2(
            rect.left() + (at.x / scale) as f32,
            rect.top() + (at.y / scale) as f32,
        ))
    }

    /// A click at `at` (points) on a marker drawn in the last frame:
    /// chooses its note. True when it did.
    pub(crate) fn click_note_marker(&mut self, at: egui::Pos2) -> bool {
        let reach = self.tokens.note_radius + 3.0;
        let hit = self
            .notes
            .markers
            .iter()
            .filter(|(_, c)| c.distance(at) <= reach)
            .min_by(|a, b| a.1.distance(at).total_cmp(&b.1.distance(at)))
            .map(|(id, _)| *id);
        match hit {
            Some(id) => {
                self.choose_note(id, false);
                true
            }
            None => false,
        }
    }

    /// The notes in the view: the markers, the chosen note's text, the
    /// field a note is written in, and **N**.
    pub(super) fn paint_notes(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        scene: Option<&Scene>,
    ) {
        // N writes a note on what is selected.
        if self.notes.writing.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(NOTE_KEY) && i.modifiers.is_none())
            && self.note_here_offered()
        {
            self.start_note(None);
        }
        self.notes.markers.clear();
        self.notes.drawn = (0, 0);
        if !self.notes.rows.is_empty() && !self.state.notes.hide_markers {
            self.paint_markers(ui, rect, scene);
        }
        self.writing_field(ui);
    }

    fn paint_markers(&mut self, ui: &egui::Ui, rect: egui::Rect, scene: Option<&Scene>) {
        let tokens = self.tokens;
        let [w, h] = self.size;
        let Some(projector) = Projector::new(
            &self.state.camera.to_render(),
            f64::from(w.max(1)),
            f64::from(h.max(1)),
        ) else {
            return;
        };
        let scale = f64::from(self.pixels_per_point.max(0.1));
        let selected = self.state.notes.selected;
        // Each note's place on screen and its depth.
        let mut placed: Vec<(&NoteRow, egui::Pos2, f64, [f64; 3])> = Vec::new();
        for row in self.notes.rows.iter() {
            let Some(p) = row.marker else {
                continue;
            };
            let Some(at) = projector.project(Vec3::new(p[0], p[1], p[2])) else {
                continue;
            };
            let pos = egui::pos2(
                rect.left() + (at.x / scale) as f32,
                rect.top() + (at.y / scale) as f32,
            );
            if rect.contains(pos) {
                placed.push((row, pos, at.depth, [at.x, at.y, at.depth]));
            }
        }
        // One marker per cell: the chosen note, else a question, else an
        // open note, else a done one; lowest id first.
        let rank = |row: &NoteRow| (Some(row.id) != selected, !row.question(), row.done, row.id);
        let cell = tokens.note_cell.max(4.0);
        let mut cells: BTreeMap<(i64, i64), (usize, usize)> = BTreeMap::new();
        for (k, (row, pos, _, _)) in placed.iter().enumerate() {
            let key = (
                ((pos.x - rect.left()) / cell).floor() as i64,
                ((pos.y - rect.top()) / cell).floor() as i64,
            );
            let entry = cells.entry(key).or_insert((k, 0));
            entry.1 += 1;
            if placed
                .get(entry.0)
                .is_some_and(|(best, ..)| rank(row) < rank(best))
            {
                entry.0 = k;
            }
        }
        let mut shown: Vec<(usize, usize)> = cells.into_values().collect();
        shown.sort_by_key(|(k, _)| placed.get(*k).map(|(row, ..)| rank(row)));
        shown.truncate(MAX_MARKERS);
        // Behind a face: looked up once per camera, state and size.
        let key: SeenKey = (
            self.state.camera,
            scene.map_or(0, |s| s.revision),
            self.size,
            self.notes.read_for,
        );
        if self.notes.seen.as_ref().is_none_or(|(k, _)| *k != key) {
            let section = self.section();
            let camera = self.state.camera.to_render();
            let slack = scene.map_or(0.0, |s| s.bbox.diagonal() * 0.01);
            let mut seen = BTreeMap::new();
            for (k, _) in &shown {
                let Some((row, _, depth, pixel)) = placed.get(*k) else {
                    continue;
                };
                let front = scene
                    .and_then(|scene| {
                        scene
                            .pick_cut(&camera, w, h, pixel[0], pixel[1], section.as_ref())
                            .ok()
                    })
                    .and_then(|(face, cap)| face.map(|f| f.depth).or(cap.map(|c| c.depth)))
                    .filter(|d| d.is_finite());
                let behind = front.is_some_and(|f| f + slack < *depth);
                seen.insert(row.id, behind);
            }
            self.notes.seen = Some((key, seen));
        }
        let seen = self
            .notes
            .seen
            .as_ref()
            .map(|(_, s)| s.clone())
            .unwrap_or_default();
        let painter = ui.painter_at(rect);
        let r = tokens.note_radius;
        let mut hits = Vec::new();
        let mut stood_for = 0;
        let mut bubble = None;
        for (k, count) in &shown {
            let Some((row, pos, _, _)) = placed.get(*k) else {
                continue;
            };
            stood_for += count;
            let behind = seen.get(&row.id).copied().unwrap_or(false);
            let alpha = if behind { 70 } else { 255 };
            let detached = row.astray();
            let ink = if row.done {
                tokens.ok
            } else if detached {
                tokens.text_dim
            } else {
                tokens.accent
            };
            // The pin: a dot at the place, a short stalk, the head above.
            let head = *pos - egui::vec2(0.0, 2.2 * r);
            painter.circle_filled(*pos, 1.5, color_alpha(ink, alpha));
            painter.line_segment(
                [*pos, head + egui::vec2(0.0, r)],
                egui::Stroke::new(1.0, color_alpha(ink, alpha)),
            );
            let chosen = Some(row.id) == selected;
            if row.done {
                // A quiet tick in a small ring.
                let small = r * 0.75;
                painter.circle_filled(head, small, color_alpha(tokens.panel_fill, alpha));
                painter.circle_stroke(head, small, egui::Stroke::new(1.2, color_alpha(ink, alpha)));
                painter.line_segment(
                    [
                        head + egui::vec2(-0.45 * small, 0.0),
                        head + egui::vec2(-0.1 * small, 0.35 * small),
                    ],
                    egui::Stroke::new(1.2, color_alpha(ink, alpha)),
                );
                painter.line_segment(
                    [
                        head + egui::vec2(-0.1 * small, 0.35 * small),
                        head + egui::vec2(0.45 * small, -0.35 * small),
                    ],
                    egui::Stroke::new(1.2, color_alpha(ink, alpha)),
                );
            } else if row.question() || detached {
                // An agent's question, or a note whose place is gone: a
                // ring with the number.
                painter.circle_filled(head, r, color_alpha(tokens.panel_fill, alpha));
                painter.circle_stroke(head, r, egui::Stroke::new(1.5, color_alpha(ink, alpha)));
                painter.text(
                    head,
                    egui::Align2::CENTER_CENTER,
                    if detached {
                        row.id.to_string()
                    } else {
                        "?".to_string()
                    },
                    egui::FontId::proportional(r * 1.4),
                    color_alpha(ink, alpha),
                );
            } else {
                painter.circle_filled(head, r, color_alpha(ink, alpha));
                painter.text(
                    head,
                    egui::Align2::CENTER_CENTER,
                    row.id.to_string(),
                    egui::FontId::proportional(r * 1.3),
                    color_alpha(tokens.panel_fill, alpha),
                );
            }
            if chosen {
                painter.circle_stroke(
                    head,
                    r + 2.5,
                    egui::Stroke::new(1.0, color_alpha(ink, alpha)),
                );
                bubble = Some((row.text.clone(), row.reply.clone(), head));
            }
            if *count > 1 {
                painter.text(
                    head + egui::vec2(r + 2.0, -r),
                    egui::Align2::LEFT_TOP,
                    format!("+{}", count - 1),
                    egui::FontId::proportional(r * 1.2),
                    color_alpha(tokens.viewport_text, alpha),
                );
            }
            if !behind {
                hits.push((row.id, head));
            }
        }
        self.notes.markers = hits;
        self.notes.drawn = (shown.len(), stood_for);
        // The chosen note's text beside its marker, on a plate.
        if let Some((text, reply, head)) = bubble {
            let width = tokens.note_text_width;
            let mut job = egui::text::LayoutJob::default();
            job.wrap.max_width = width;
            job.append(
                &text,
                0.0,
                egui::TextFormat::simple(
                    egui::FontId::proportional(13.0),
                    color(tokens.plate_text),
                ),
            );
            if let Some(reply) = reply {
                job.append(
                    &format!("\n{reply}"),
                    0.0,
                    egui::TextFormat::simple(
                        egui::FontId::proportional(12.0),
                        color(tokens.plate_text_dim),
                    ),
                );
            }
            let galley = painter.layout_job(job);
            let at = head + egui::vec2(r + 6.0, -galley.size().y * 0.5);
            let plate = egui::Rect::from_min_size(at, galley.size()).expand(6.0);
            painter.rect_filled(plate, tokens.radius, color(tokens.plate_fill));
            painter.galley(at, galley, color(tokens.plate_text));
        }
    }

    /// The field a note is written in, while one is open: Enter adds the
    /// note, Escape leaves.
    fn writing_field(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let Some(writing) = &mut self.notes.writing else {
            return;
        };
        let mut submit = false;
        let mut leave = false;
        egui::Area::new(egui::Id::new("craft3d-note-field"))
            .order(egui::Order::Foreground)
            .fixed_pos(writing.at + egui::vec2(tokens.note_radius + 6.0, -12.0))
            .show(ui.ctx(), |ui| {
                ui.style_mut().visuals = tokens.plate_visuals();
                egui::Frame::new()
                    .fill(color(tokens.plate_fill))
                    .corner_radius(tokens.radius)
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut writing.text)
                                .hint_text("Note…")
                                .desired_width(tokens.note_text_width),
                        );
                        if writing.focus {
                            field.request_focus();
                            writing.focus = false;
                        }
                        if field.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                submit = true;
                            } else {
                                leave = true;
                            }
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            leave = true;
                        }
                    });
            });
        if submit {
            self.submit_note();
        } else if leave {
            self.notes.writing = None;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn rows_say_where_who_and_whether_done() {
        let listed = json!({"notes": [
            {"id": 1, "text": "thicker", "state": "open", "author": "person", "agent": null,
             "timeMs": 5, "attachment": "attached",
             "on": {"kind": "face", "name": "F1:face(1)", "feature": "F1", "body": "Plate",
                    "origin": {"feature": "F1", "name": "Plate", "type": "block"}},
             "at": [1.0, 2.0, 3.0], "point": [1.0, 2.0, 3.0]},
            {"id": 2, "text": "why?", "state": "open", "author": "agent", "agent": "Claude Code",
             "attachment": "byPlace",
             "on": {"kind": "face", "name": null, "pinnedAs": "F2:fillet(0)", "feature": "F2",
                    "origin": {"feature": "F2", "name": "Round"}},
             "at": [0.0, 0.0, 0.0], "point": [0.0, 0.0, 0.0]},
            {"id": 3, "text": "gone", "state": "done", "author": "unknown", "attachment": "detached",
             "on": {"kind": "face", "name": "F4:face(5)", "feature": "F4"}, "point": [9.0, 9.0, 9.0],
             "done": {"author": "agent", "agent": "Claude Code", "reply": "removed"}},
            {"id": 4, "text": "this corner", "state": "open", "author": "person", "attachment": "lost",
             "on": {"kind": "edge", "name": null, "pinnedAs": "F2:fillet(1)", "feature": "F2",
                    "origin": {"feature": "F2", "name": "Round"}},
             "point": [4.0, 5.0, 6.0]},
            "nonsense"
        ]});
        let rows = rows_of(&listed);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[3].place, "place gone, near Round");
        assert_eq!(rows[3].marker, Some([4.0, 5.0, 6.0]), "at its own point");
        assert!(rows[3].astray() && rows[2].astray() && !rows[1].astray());
        assert_eq!(rows[3].entity, None);
        assert_eq!(rows[0].place, "face of Plate");
        assert_eq!(rows[0].who(), "you");
        assert_eq!(
            rows[0].entity,
            Some(("face".into(), "F1:face(1)".into(), "F1".into()))
        );
        assert_eq!(rows[1].place, "by place, near Round");
        assert!(rows[1].question());
        assert_eq!(rows[1].entity, None, "never an entity it cannot prove");
        assert_eq!(rows[2].place, "detached");
        assert_eq!(rows[2].marker, Some([9.0, 9.0, 9.0]));
        assert_eq!(rows[2].closed_by.as_deref(), Some("Claude Code"));
        assert_eq!(rows[2].reply.as_deref(), Some("removed"));
        assert!(rows_of(&json!({})).is_empty());
    }
}
