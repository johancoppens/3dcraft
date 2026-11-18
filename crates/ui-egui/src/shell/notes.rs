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
//!   the person: `Session::by_person`), Shift+Enter starts a new line in
//!   it (the field grows), Escape leaves. The field stops at the longest
//!   text a note takes and counts near it; when `note.add` refuses, the
//!   field stays open with its text and says why. The point is the
//!   click's on that entity (the last click when it was on it; the front
//!   face under the right click), else the entity's middle.
//! - **Markers:** small pins in `Tokens::note_*`, the viewport's own
//!   colours (the viewport is not themed); behind the bodies (a ray
//!   from the eye through the marker meets a face nearer than it by more
//!   than a hundredth of the model's size) faint and not clickable, as
//!   the mates' markers behind a face are. A click on a marker chooses its
//!   note (`notes.selected`) and shows its text. Thinned for many notes
//!   by a rule that does not depend on the view ([`lay_out_pins`]): the
//!   notes taken in a fixed order (the chosen note, agents' open
//!   questions, other open notes, done ones; by id), each a pin unless an
//!   earlier pin is within `Tokens::note_cell` points, which then stands
//!   for it too; at most [`MAX_MARKERS`] pins, the Notes tab saying how
//!   many notes in view have none. What is behind a face is looked up
//!   once per camera, document state and size, for the pins drawn only.
//! - **Pictures:** `ui.screenshot` draws the same pins into its picture
//!   (`pin_pixels.rs`), laid out for the picture's size.
//!
//! The window only views: every change is a command, one undo step.

use super::Shell;
use super::views::FlyTo;
use crate::camera::Projector;
use crate::theme::{Rgb, Tokens, color, color_alpha};
use craft3d_engine::{Scene, SelectionKind, Session};
use craft3d_math::Vec3;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

mod pin_pixels;

/// The key that writes a note on what is selected.
pub const NOTE_KEY: egui::Key = egui::Key::N;

/// Most pins drawn: notes after these (in the order of
/// [`lay_out_pins`]) and not near one of them have none, so a document
/// with a thousand notes costs no more per frame than one with this many.
pub const MAX_MARKERS: usize = 400;

/// The key under which the side panel runs the note being written, so
/// the field learns whether it was added.
const NOTE_FIELD: &str = "note field";

/// The count of a note's text shows from this many code points before
/// the longest it may be.
const COUNT_FROM: usize = 200;

/// What a pin shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinKind {
    /// An open note: a head in `note_open` with its number.
    Open,
    /// An agent's open question: a ring in `note_open` with a "?".
    Question,
    /// A done note: a small ring in `note_done` with a tick.
    Done,
    /// A note off its entity (detached, or held by place where nothing
    /// is any more), at its stored point: a ring in `note_astray` with
    /// its number.
    Astray,
}

impl PinKind {
    /// What `row`'s pin shows.
    pub fn of(row: &NoteRow) -> Self {
        if row.done {
            PinKind::Done
        } else if row.astray() {
            PinKind::Astray
        } else if row.question() {
            PinKind::Question
        } else {
            PinKind::Open
        }
    }

    /// Its colour.
    pub fn ink(self, tokens: &Tokens) -> Rgb {
        match self {
            PinKind::Open | PinKind::Question => tokens.note_open,
            PinKind::Done => tokens.note_done,
            PinKind::Astray => tokens.note_astray,
        }
    }
}

/// A pin as the view draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub id: u64,
    /// The place it stands on, in points from the view's top left.
    pub at: egui::Pos2,
    pub kind: PinKind,
    /// A face is nearer the eye than its place: drawn faint, not clicked.
    pub behind: bool,
    /// The chosen note's pin.
    pub chosen: bool,
    /// How many more notes near it it stands for.
    pub more: usize,
}

impl Pin {
    /// The centre of its head, for a pin standing at `at` (points): above
    /// the place, on a short stalk.
    pub fn head(at: egui::Pos2, radius: f32) -> egui::Pos2 {
        at - egui::vec2(0.0, 2.2 * radius)
    }
}

/// How pins are laid out ([`lay_out_pins`]): how near (in points at the
/// view's zoom) a note shares an earlier pin, the pins' radius, the note
/// chosen, and where the view's own controls are (their plates, the view
/// cube, the triad: in points from the view's top left), under which no
/// pin is drawn.
#[derive(Clone, Copy, Debug)]
pub struct PinRule<'a> {
    pub cell: f32,
    pub radius: f32,
    pub selected: Option<u64>,
    pub covered: &'a [egui::Rect],
}

/// The pins of a view, and how many notes they leave without one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PinLayout {
    pub pins: Vec<Pin>,
    /// Notes whose place is in the view.
    pub in_view: usize,
    /// Notes the pins stand for: their own and those near them.
    pub stood_for: usize,
}

/// Lays out the pins of `rows` in a view of `size` pixels, `scale`
/// pixels a point, seen by `camera`. The notes whose place is in the view
/// are taken in an order that does not depend on the view (the chosen
/// note, agents' open questions, the other open notes, done ones; by id),
/// each drawn as a pin unless an earlier pin's place is nearer to it in
/// the model than `cell` points are at the view's zoom (rounded up to a
/// power of two model units, so a small zoom keeps it), which then stands
/// for it too; at most [`MAX_MARKERS`] pins, a note after them near none
/// of them left without. Which notes have pins depends on the places and
/// the zoom alone: turning the view changes it only where notes come into
/// or leave the view. A note whose pin would stand under one of the
/// view's own controls (`rule.covered`) has none (review-pins 2b: at a
/// thousand notes the pins ran under the toolbar, the cube and the triad).
/// `behind` says whether a pin's place (pixels across, down, and depth) is
/// behind a face; asked for the pins only.
pub fn lay_out_pins(
    rows: &[NoteRow],
    camera: &craft3d_render::Camera,
    size: [u32; 2],
    scale: f64,
    rule: &PinRule<'_>,
    behind: &mut dyn FnMut(u64, [f64; 3]) -> bool,
) -> PinLayout {
    let PinRule {
        cell,
        radius,
        selected,
        covered,
    } = *rule;
    let mut layout = PinLayout::default();
    let [w, h] = size;
    let Some(projector) = Projector::new(camera, f64::from(w.max(1)), f64::from(h.max(1))) else {
        return layout;
    };
    let scale = if scale.is_finite() {
        scale.max(0.1)
    } else {
        1.0
    };
    let (width, height) = (f64::from(w) / scale, f64::from(h) / scale);
    let mut placed: Vec<(&NoteRow, Vec3, egui::Pos2, [f64; 3])> = Vec::new();
    for row in rows {
        let Some(p) = row.marker else {
            continue;
        };
        let place = Vec3::new(p[0], p[1], p[2]);
        let Some(at) = projector.project(place) else {
            continue;
        };
        let (x, y) = (at.x / scale, at.y / scale);
        if (0.0..=width).contains(&x) && (0.0..=height).contains(&y) {
            placed.push((
                row,
                place,
                egui::pos2(x as f32, y as f32),
                [at.x, at.y, at.depth],
            ));
        }
    }
    layout.in_view = placed.len();
    // A pin spans from its place up its stalk to the top of its head.
    let r = if radius.is_finite() {
        radius.max(0.0)
    } else {
        0.0
    };
    placed.retain(|(_, _, pos, _)| {
        let span = egui::Rect::from_min_max(
            egui::pos2(pos.x - r, pos.y - 3.2 * r),
            egui::pos2(pos.x + r, pos.y),
        );
        !covered.iter().any(|c| c.intersects(span))
    });
    placed.sort_by_key(|(row, ..)| (Some(row.id) != selected, !row.question(), row.done, row.id));
    let near = model_cell(camera, &projector, f64::from(cell) * scale);
    let bucket = |p: Vec3, q: f64| -> (i64, i64, i64) {
        (
            (p.x / q).floor() as i64,
            (p.y / q).floor() as i64,
            (p.z / q).floor() as i64,
        )
    };
    // The places of the pins laid out so far, and the pins by the cube of
    // the model cell their place is in.
    let mut places: Vec<Vec3> = Vec::new();
    let mut buckets: BTreeMap<(i64, i64, i64), Vec<usize>> = BTreeMap::new();
    for (row, place, pos, pixel) in &placed {
        let mut nearest: Option<(f64, usize)> = None;
        if let Some(q) = near {
            let (bx, by, bz) = bucket(*place, q);
            for dx in -1..=1_i64 {
                for dy in -1..=1_i64 {
                    for dz in -1..=1_i64 {
                        let key = (
                            bx.saturating_add(dx),
                            by.saturating_add(dy),
                            bz.saturating_add(dz),
                        );
                        for &k in buckets.get(&key).into_iter().flatten() {
                            let Some(there) = places.get(k) else {
                                continue;
                            };
                            let d = there.distance(*place);
                            if d < q && nearest.is_none_or(|(best, _)| d < best) {
                                nearest = Some((d, k));
                            }
                        }
                    }
                }
            }
        }
        match nearest {
            Some((_, k)) => {
                if let Some(pin) = layout.pins.get_mut(k) {
                    pin.more += 1;
                }
                layout.stood_for += 1;
            }
            None if layout.pins.len() < MAX_MARKERS => {
                if let Some(q) = near {
                    buckets
                        .entry(bucket(*place, q))
                        .or_default()
                        .push(layout.pins.len());
                }
                places.push(*place);
                layout.pins.push(Pin {
                    id: row.id,
                    at: *pos,
                    kind: PinKind::of(row),
                    behind: behind(row.id, *pixel),
                    chosen: Some(row.id) == selected,
                    more: 0,
                });
                layout.stood_for += 1;
            }
            None => {}
        }
    }
    layout
}

/// The model length `pixels` span at the camera's target, rounded up to a
/// power of two (so a small zoom keeps it, and turning about the target
/// does not change it); `None` for a view without one.
fn model_cell(camera: &craft3d_render::Camera, projector: &Projector, pixels: f64) -> Option<f64> {
    let right = (camera.target - camera.eye).cross(camera.up).normalized()?;
    let a = projector.project(camera.target)?;
    let b = projector.project(camera.target + right)?;
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let per_unit = (dx * dx + dy * dy).sqrt();
    if !(per_unit.is_finite() && per_unit > 0.0 && pixels.is_finite() && pixels > 0.0) {
        return None;
    }
    let wanted = pixels / per_unit;
    let mut q = 1.0_f64;
    for _ in 0..1100 {
        if q >= wanted {
            break;
        }
        q *= 2.0;
    }
    for _ in 0..1100 {
        if q * 0.5 < wanted {
            break;
        }
        q *= 0.5;
    }
    (q.is_finite() && q > 0.0).then_some(q)
}

/// Whether a place (pixels across and down in a view of `size`, and its
/// depth) is behind a face of `scene`: a face (or the section's cap)
/// under it nearer the eye by more than `slack`.
fn pin_behind(
    scene: Option<&Scene>,
    camera: &craft3d_render::Camera,
    size: [u32; 2],
    at: [f64; 3],
    section: Option<&craft3d_render::Section>,
    slack: f64,
) -> bool {
    let [w, h] = size;
    let [x, y, depth] = at;
    let front = scene
        .and_then(|scene| scene.pick_cut(camera, w, h, x, y, section).ok())
        .and_then(|(face, cap)| face.map(|f| f.depth).or(cap.map(|c| c.depth)))
        .filter(|d| d.is_finite());
    front.is_some_and(|f| f + slack < depth)
}

/// Paints `pin` with `painter`, standing at `pos` (points); the centre of
/// its head.
fn paint_pin(painter: &egui::Painter, tokens: &Tokens, pin: &Pin, pos: egui::Pos2) -> egui::Pos2 {
    let r = tokens.note_radius;
    let alpha = if pin.behind { 70 } else { 255 };
    let ink = pin.kind.ink(tokens);
    let faded = |c: Rgb| color_alpha(c, alpha);
    let head = Pin::head(pos, r);
    // The pin: a dot at the place, a short stalk, the head above.
    painter.circle_filled(pos, 1.5, faded(ink));
    painter.line_segment(
        [pos, head + egui::vec2(0.0, r)],
        egui::Stroke::new(1.0, faded(ink)),
    );
    match pin.kind {
        PinKind::Done => {
            // A quiet tick in a small ring.
            let small = r * 0.75;
            let stroke = egui::Stroke::new(1.2, faded(ink));
            painter.circle_filled(head, small, faded(tokens.note_fill));
            painter.circle_stroke(head, small, stroke);
            let knee = head + egui::vec2(-0.1 * small, 0.35 * small);
            painter.line_segment([head + egui::vec2(-0.45 * small, 0.0), knee], stroke);
            painter.line_segment(
                [knee, head + egui::vec2(0.45 * small, -0.35 * small)],
                stroke,
            );
        }
        PinKind::Question | PinKind::Astray => {
            // An agent's question, or a note whose place is gone: a ring
            // with "?" or the number.
            painter.circle_filled(head, r, faded(tokens.note_fill));
            painter.circle_stroke(head, r, egui::Stroke::new(1.5, faded(ink)));
            let words = if pin.kind == PinKind::Astray {
                pin.id.to_string()
            } else {
                "?".to_string()
            };
            painter.text(
                head,
                egui::Align2::CENTER_CENTER,
                words,
                egui::FontId::proportional(r * 1.4),
                faded(ink),
            );
        }
        PinKind::Open => {
            painter.circle_filled(head, r, faded(ink));
            painter.text(
                head,
                egui::Align2::CENTER_CENTER,
                pin.id.to_string(),
                egui::FontId::proportional(r * 1.3),
                faded(tokens.note_ink),
            );
        }
    }
    if pin.chosen {
        painter.circle_stroke(head, r + 2.5, egui::Stroke::new(1.0, faded(ink)));
    }
    if pin.more > 0 {
        painter.text(
            head + egui::vec2(r + 2.0, -r),
            egui::Align2::LEFT_TOP,
            format!("+{}", pin.more),
            egui::FontId::proportional(r * 1.2),
            faded(tokens.viewport_text),
        );
    }
    head
}

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
    /// Where it is, in words: "face of Rib"; held by place, what holds it,
    /// an entity of the feature that made it: "by place: near an edge of
    /// Bookend" (its name is not proven to stay with what it was pinned
    /// to, so not that); "place gone: was near an edge of Bookend";
    /// "detached".
    pub place: String,
    /// What a click flies to: the entity now (kind, name, feature).
    pub entity: Option<(String, String, String)>,
    /// Where its marker goes: `at`, else the stored point of a detached
    /// note or one whose place is gone; `None` while it is hidden.
    pub marker: Option<[f64; 3]>,
    pub reply: Option<String>,
    /// Who closed it, as [`NoteRow::who`] says it.
    pub closed_by: Option<String>,
    /// The replies given after it was closed (kept when it was opened
    /// again): who, and what.
    pub later: Vec<(String, String)>,
}

/// An agent in a few words, never as the person reads: "Claude Code
/// (agent)" whatever it calls itself (an agent named "you" is "you
/// (agent)"), "an agent" for one without a name.
pub fn agent_words(name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{name} (agent)"),
        None => "an agent".to_string(),
    }
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

    /// Who, in a few words: "you" for the person alone; an agent as
    /// [`agent_words`] says it ("Claude Code (agent)"); "unknown".
    pub fn who(&self) -> String {
        match self.author.as_str() {
            "person" => "you".to_string(),
            "agent" => agent_words(self.agent.as_deref()),
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
            // Held by place: what holds it, an entity of the feature that
            // made it, never the name it was pinned by (the round it
            // spoke of may be a chamfer now).
            let one = match kind.as_str() {
                "edge" => "an edge",
                "vertex" => "a corner",
                "body" => "the body",
                _ => "a face",
            };
            let place = match attachment.as_str() {
                "byPlace" => format!("by place: near {one} of {made}"),
                "lost" => format!("place gone: was near {one} of {made}"),
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
            let who_of = |r: &Value| match text(r, "author").as_deref() {
                Some("person") => "you".to_string(),
                Some("agent") => agent_words(text(r, "agent").as_deref()),
                _ => "unknown".to_string(),
            };
            let later = n
                .get("later")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|r| Some((who_of(r), text(r, "reply")?)))
                        .collect()
                })
                .unwrap_or_default();
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
                closed_by: done.map(who_of),
                later,
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
    /// Sent to be added (`note.add`): the field waits for the answer.
    pub sent: bool,
    /// Why `note.add` refused it: said at the field, the text kept.
    pub error: Option<String>,
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
    /// How many notes' places the last frame had in the view.
    pub in_view: usize,
    /// Which drawn markers are behind a face, by note id.
    seen: Option<(SeenKey, BTreeMap<u64, bool>)>,
    /// A note's text being changed in the tab: its id and the text.
    pub(crate) editing: Option<(u64, String)>,
    pub(crate) edit_focus: bool,
    /// The changed text was sent (`note.edit`): its field waits for the
    /// answer; and why it was refused, said under the field.
    pub(crate) edit_sent: bool,
    pub(crate) edit_error: Option<String>,
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

    /// How many pins the last frame drew (those behind a face too).
    pub fn note_pins_drawn(&self) -> usize {
        self.notes.drawn.0
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
            sent: false,
            error: None,
        });
    }

    /// Adds the note being written (`note.add`, as the person), when it
    /// says something, else closes the field. The field stays until the
    /// answer: closed when the note was added, open with its text and the
    /// reason when it was refused.
    fn submit_note(&mut self) {
        let Some(writing) = &mut self.notes.writing else {
            return;
        };
        if writing.sent {
            return;
        }
        if writing.text.trim().is_empty() {
            self.notes.writing = None;
            return;
        }
        writing.sent = true;
        writing.error = None;
        let mut params = json!({"text": writing.text, "on": writing.on});
        if let Some(map) = params.as_object_mut() {
            if let Some(feature) = &writing.feature {
                map.insert("feature".into(), feature.clone().into());
            }
            if let Some(point) = writing.point {
                map.insert("point".into(), json!(point));
            }
        }
        self.side_queue("note.add", params, Some(NOTE_FIELD.to_string()));
    }

    /// What the side panel made of the note sent: closes the field when it
    /// was added, keeps it open with its text and the reason when it was
    /// refused; nothing while it waits.
    fn note_answered(&mut self) {
        if !self.notes.writing.as_ref().is_some_and(|w| w.sent) {
            return;
        }
        match self.side_outcome(NOTE_FIELD) {
            None => {}
            Some(Ok(())) => self.notes.writing = None,
            Some(Err(why)) => {
                if let Some(writing) = &mut self.notes.writing {
                    writing.sent = false;
                    writing.error = Some(why);
                    writing.focus = true;
                }
            }
        }
    }

    /// Reads the notes for the document as it is now (`ui.screenshot`
    /// between frames).
    pub(crate) fn sync_notes_now(&mut self, session: &mut Session) {
        let Some(doc) = session.document_status().map(|s| s.id) else {
            return;
        };
        let revision = session.document_revision().unwrap_or(0);
        let bodies_here = !session.regen_elsewhere() || session.regenerated_here();
        self.sync_notes(session, (doc, revision), bodies_here);
    }

    /// Where the view's own controls were drawn in the last frame (the
    /// plates, the view cube, the triad), in points from the view's top
    /// left: no pin goes under them.
    fn controls_covered(&self) -> Vec<egui::Rect> {
        let Some(view) = self.rect else {
            return Vec::new();
        };
        let origin = view.min.to_vec2();
        self.controls_layout
            .as_ref()
            .map(|l| {
                l.parts()
                    .into_iter()
                    .map(|(_, r)| r.translate(-origin))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Draws the notes' pins into `rgb`, a picture of the view of `size`
    /// pixels at `scale` pixels a point (`ui.screenshot`), as the window
    /// draws them over the view: while they are shown, laid out for that
    /// picture ([`lay_out_pins`]). The chosen note's text beside its pin is
    /// not drawn. How many pins it drew.
    pub(crate) fn draw_note_pins(
        &self,
        scene: Option<&Scene>,
        rgb: &mut [u8],
        size: [u32; 2],
        scale: f64,
    ) -> usize {
        if self.notes.rows.is_empty() || self.state.notes.hide_markers {
            return 0;
        }
        let camera = self.state.camera.to_render();
        let section = self.section();
        let slack = scene.map_or(0.0, |s| s.bbox.diagonal() * 0.01);
        // At the view's own size, the same pins as the window: none under
        // its controls; at another, the controls are not where they were.
        let covered = if size == self.size {
            self.controls_covered()
        } else {
            Vec::new()
        };
        let rule = PinRule {
            cell: self.tokens.note_cell,
            radius: self.tokens.note_radius,
            selected: self.state.notes.selected,
            covered: &covered,
        };
        let layout = lay_out_pins(
            &self.notes.rows,
            &camera,
            size,
            scale,
            &rule,
            &mut |_, at| pin_behind(scene, &camera, size, at, section.as_ref(), slack),
        );
        pin_pixels::draw_pins(rgb, size, &layout.pins, &self.tokens, scale as f32);
        layout.pins.len()
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
        self.notes.in_view = 0;
        if !self.notes.rows.is_empty() && !self.state.notes.hide_markers {
            self.paint_markers(ui, rect, scene);
        }
        self.note_answered();
        self.writing_field(ui);
    }

    fn paint_markers(&mut self, ui: &egui::Ui, rect: egui::Rect, scene: Option<&Scene>) {
        let tokens = self.tokens;
        let size = self.size;
        let camera = self.state.camera.to_render();
        let scale = f64::from(self.pixels_per_point.max(0.1));
        // Behind a face: looked up once per camera, state and size.
        let key: SeenKey = (
            self.state.camera,
            scene.map_or(0, |s| s.revision),
            size,
            self.notes.read_for,
        );
        let mut seen = match self.notes.seen.take() {
            Some((known, seen)) if known == key => seen,
            _ => BTreeMap::new(),
        };
        let section = self.section();
        let slack = scene.map_or(0.0, |s| s.bbox.diagonal() * 0.01);
        let rows = Arc::clone(&self.notes.rows);
        let covered = self.controls_covered();
        let rule = PinRule {
            cell: tokens.note_cell,
            radius: tokens.note_radius,
            selected: self.state.notes.selected,
            covered: &covered,
        };
        let layout = lay_out_pins(&rows, &camera, size, scale, &rule, &mut |id, at| {
            *seen
                .entry(id)
                .or_insert_with(|| pin_behind(scene, &camera, size, at, section.as_ref(), slack))
        });
        self.notes.seen = Some((key, seen));
        let painter = ui.painter_at(rect);
        let origin = rect.min.to_vec2();
        let mut hits = Vec::new();
        let mut bubble = None;
        for pin in &layout.pins {
            let head = paint_pin(&painter, &tokens, pin, pin.at + origin);
            if pin.chosen {
                bubble = rows
                    .iter()
                    .find(|r| r.id == pin.id)
                    .map(|r| (r.text.clone(), r.reply.clone(), head));
            }
            if !pin.behind {
                hits.push((pin.id, head));
            }
        }
        self.notes.markers = hits;
        self.notes.drawn = (layout.pins.len(), layout.stood_for);
        self.notes.in_view = layout.in_view;
        // The chosen note's text beside its marker, on a plate.
        if let Some((text, reply, head)) = bubble {
            let r = tokens.note_radius;
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
    /// note, Shift+Enter starts a new line in it (the field grows),
    /// Escape leaves. It stops at the longest text a note takes, counts
    /// the code points near it, and says why `note.add` refused the note,
    /// keeping the text. A click elsewhere leaves an empty field, and
    /// keeps one with text.
    fn writing_field(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let Some(writing) = &mut self.notes.writing else {
            return;
        };
        let mut submit = false;
        let mut leave = false;
        let field_id = egui::Id::new("craft3d-note-field-text");
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
                        ui.set_max_width(tokens.note_text_width + 12.0);
                        // Enter adds the note, taken before the field sees
                        // it; Shift+Enter is the field's: a new line.
                        let focused = ui.memory(|m| m.has_focus(field_id));
                        let enter = focused
                            && ui.input_mut(|i| {
                                !i.modifiers.shift
                                    && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            });
                        let field = ui.add_enabled(
                            !writing.sent,
                            egui::TextEdit::multiline(&mut writing.text)
                                .id(field_id)
                                .hint_text("Note…")
                                .return_key(egui::KeyboardShortcut::new(
                                    egui::Modifiers::SHIFT,
                                    egui::Key::Enter,
                                ))
                                .desired_rows(1)
                                .desired_width(tokens.note_text_width)
                                .char_limit(craft3d_engine::MAX_NOTE_CHARS),
                        );
                        if writing.focus {
                            field.request_focus();
                            writing.focus = false;
                        }
                        if field.changed() {
                            writing.error = None;
                        }
                        let count = writing.text.chars().count();
                        if count + COUNT_FROM >= craft3d_engine::MAX_NOTE_CHARS {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{count} / {}",
                                    craft3d_engine::MAX_NOTE_CHARS
                                ))
                                .small()
                                .color(color(tokens.plate_text_dim)),
                            );
                        }
                        if let Some(why) = &writing.error {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(why)
                                        .small()
                                        .color(color(tokens.plate_text)),
                                )
                                .wrap(),
                            );
                        }
                        if enter {
                            submit = true;
                        } else if field.lost_focus() && writing.text.trim().is_empty() {
                            leave = true;
                        }
                        // Escape, the grammar's first step: an empty field
                        // closes; one with text gives up the keys and stays,
                        // and the next Escape closes it
                        // (`Shell::escape_step`).
                        if ui.input(|i| i.key_pressed(egui::Key::Escape))
                            && writing.text.trim().is_empty()
                        {
                            leave = true;
                        }
                    });
            });
        if leave {
            self.notes.writing = None;
        } else if submit {
            self.submit_note();
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
             "on": {"kind": "edge", "name": null, "pinnedAs": "F1:edge(7)", "feature": "F3",
                    "body": "Chamfer", "origin": {"feature": "F1", "name": "Bookend"}},
             "at": [0.0, 0.0, 0.0], "point": [0.0, 0.0, 0.0]},
            {"id": 3, "text": "gone", "state": "done", "author": "unknown", "attachment": "detached",
             "on": {"kind": "face", "name": "F4:face(5)", "feature": "F4"}, "point": [9.0, 9.0, 9.0],
             "done": {"author": "person", "agent": null, "reply": null},
             "later": [{"author": "agent", "agent": "Claude Code", "timeMs": 9,
                        "reply": "removed it"},
                       {"author": "agent", "agent": null, "reply": "checked"}]},
            {"id": 4, "text": "this corner", "state": "open", "author": "person", "attachment": "lost",
             "on": {"kind": "edge", "name": null, "pinnedAs": "F2:fillet(1)", "feature": "F2",
                    "origin": {"feature": "F2", "name": "Round"}},
             "point": [4.0, 5.0, 6.0]},
            {"id": 5, "text": "over the channel", "state": "open", "author": "agent", "agent": null,
             "attachment": "attached",
             "on": {"kind": "face", "name": "F1:face(2)", "feature": "F1", "body": "Plate"},
             "at": [1.0, 1.0, 1.0], "point": [1.0, 1.0, 1.0]},
            "nonsense"
        ]});
        let rows = rows_of(&listed);
        assert_eq!(rows.len(), 5);
        // Held by place: what holds it, not the round it spoke of.
        assert_eq!(rows[1].place, "by place: near an edge of Bookend");
        assert_eq!(rows[3].place, "place gone: was near an edge of Round");
        assert_eq!(rows[3].marker, Some([4.0, 5.0, 6.0]), "at its own point");
        assert!(rows[3].astray() && rows[2].astray() && !rows[1].astray());
        assert_eq!(rows[3].entity, None);
        assert_eq!(rows[0].place, "face of Plate");
        assert_eq!(rows[0].who(), "you");
        assert_eq!(
            rows[0].entity,
            Some(("face".into(), "F1:face(1)".into(), "F1".into()))
        );
        assert!(rows[1].question());
        assert_eq!(rows[1].entity, None, "never an entity it cannot prove");
        assert_eq!(rows[2].place, "detached");
        assert_eq!(rows[2].marker, Some([9.0, 9.0, 9.0]));
        // Closed by the person, then replied to by agents.
        assert_eq!(rows[2].closed_by.as_deref(), Some("you"));
        assert_eq!(rows[2].reply, None);
        assert_eq!(
            rows[2].later,
            vec![
                ("Claude Code (agent)".to_string(), "removed it".to_string()),
                ("an agent".to_string(), "checked".to_string())
            ]
        );
        // An agent without a name: its question all the same.
        assert!(rows[4].question());
        assert_eq!(rows[4].who(), "an agent");
        // An agent that calls itself "you" never reads as the person
        // (review-pins 2b).
        let you = rows_of(&json!({"notes": [
            {"id": 1, "text": "x", "author": "agent", "agent": "you", "attachment": "attached",
             "done": {"author": "agent", "agent": "you"}}
        ]}));
        assert_eq!(you[0].who(), "you (agent)");
        assert_eq!(you[0].closed_by.as_deref(), Some("you (agent)"));
        assert_eq!(PinKind::of(&rows[4]), PinKind::Question);
        assert_eq!(PinKind::of(&rows[2]), PinKind::Done);
        assert_eq!(PinKind::of(&rows[3]), PinKind::Astray);
        assert_eq!(PinKind::of(&rows[0]), PinKind::Open);
        assert!(rows_of(&json!({})).is_empty());
    }

    /// The pins of many notes close together: the same notes keep their
    /// pins when the view turns a little; open notes and questions before
    /// done ones; at most `MAX_MARKERS`, the rest counted.
    #[test]
    fn pins_are_laid_out_by_a_rule_the_view_does_not_change() {
        let mut rows = Vec::new();
        for k in 0..1000u64 {
            let (i, j) = ((k % 40) as f64, (k / 40) as f64);
            rows.push(NoteRow {
                id: k + 1,
                text: format!("note {k}"),
                done: k % 3 == 0,
                author: if k % 7 == 0 { "agent" } else { "person" }.to_string(),
                agent: None,
                time_ms: 0,
                attachment: "attached".to_string(),
                place: String::new(),
                entity: None,
                marker: Some([i * 1.5, j * 1.5, 0.0]),
                reply: None,
                closed_by: None,
                later: Vec::new(),
            });
        }
        let lay_out = |yaw: f64| {
            let camera = crate::state::CameraState {
                target: [30.0, 18.0, 0.0],
                distance: 200.0,
                yaw,
                pitch: -60.0,
                ..crate::state::CameraState::default()
            };
            lay_out_pins(
                &rows,
                &camera.to_render(),
                [1280, 800],
                1.0,
                &PinRule {
                    cell: 22.0,
                    radius: 7.0,
                    selected: None,
                    covered: &[],
                },
                &mut |_, _| false,
            )
        };
        let a = lay_out(30.0);
        assert!(
            a.pins.len() <= MAX_MARKERS && a.pins.len() > 10,
            "{}",
            a.pins.len()
        );
        assert_eq!(a.in_view, 1000, "every note in view");
        assert!(a.stood_for <= a.in_view);
        let ids = |l: &PinLayout| {
            l.pins
                .iter()
                .map(|p| p.id)
                .collect::<std::collections::BTreeSet<u64>>()
        };
        // Turned half a degree, or ten: the same notes have pins.
        for yaw in [30.5, 40.0] {
            let b = lay_out(yaw);
            assert_eq!(b.in_view, 1000, "every note in view at {yaw}");
            assert_eq!(ids(&a), ids(&b), "turned to {yaw}");
        }
        // Done notes only where no open one stands near.
        let first_done = a.pins.iter().position(|p| p.kind == PinKind::Done);
        let last_open = a.pins.iter().rposition(|p| p.kind != PinKind::Done);
        if let (Some(d), Some(o)) = (first_done, last_open) {
            assert!(d > o, "open pins are laid out first");
        }
        // Under the view's controls no pin stands (review-pins 2b): a
        // control over the left half leaves its notes without, counted
        // in view; one over the whole view, none at all.
        let camera = crate::state::CameraState {
            target: [30.0, 18.0, 0.0],
            distance: 200.0,
            yaw: 30.0,
            pitch: -60.0,
            ..crate::state::CameraState::default()
        };
        let under = |covered: &[egui::Rect]| {
            lay_out_pins(
                &rows,
                &camera.to_render(),
                [1280, 800],
                1.0,
                &PinRule {
                    cell: 22.0,
                    radius: 7.0,
                    selected: None,
                    covered,
                },
                &mut |_, _| false,
            )
        };
        let left = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(640.0, 800.0));
        let half = under(&[left]);
        assert_eq!(half.in_view, 1000);
        assert!(
            half.stood_for < a.stood_for,
            "{} {}",
            half.stood_for,
            a.stood_for
        );
        assert!(
            half.pins.iter().all(|p| p.at.x - 7.0 > 640.0),
            "no pin under the control"
        );
        let all = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 800.0));
        let none = under(&[all]);
        assert!(none.pins.is_empty());
        assert_eq!(none.in_view, 1000);
    }
}
