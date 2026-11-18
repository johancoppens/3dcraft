//! The notes pinned to the model in the view (the developer's design of
//! 2026-11-17, `plan/viewer-tools`, section 1; the UX round of 2026-11-18,
//! N1 to N7; `docs/guide/window.md` › *Notes*): each note's pin at its
//! place (`at` of `note.list`, a note off its entity at its stored point,
//! in grey), a done one a quiet tick; the chosen note's text beside its
//! pin; the field a note is written in.
//!
//! - **Writing a note:** three ways in. **N** writes on the one body,
//!   face, edge or vertex selected, and says why not when it cannot
//!   (nothing selected, more than one, an assembly, a Jig in use: a hint in
//!   the status bar). *Note here…* heads the list under the right mouse
//!   button (a press and hold on a touch screen) whenever the pointer is on
//!   a face, an edge or a vertex of a part: it writes on *that* thing, at
//!   the point under the pointer, and selects it; in an assembly or a Jig
//!   in use it is greyed, with the reason under it. The Notes tab's *Note
//!   on selection (N)* does what N does. The field follows its place as
//!   the camera moves; Enter adds the note (`note.add`, as the person:
//!   `Session::by_person`), Shift+Enter starts a new line in it (the field
//!   grows), Escape leaves; opening a second field while one holds text
//!   moves the text into it. The field stops at the longest text a note
//!   takes and counts near it; when `note.add` refuses, the field stays open
//!   with its text and says why (in a person's words; the command's own as
//!   the hover).
//! - **Pins:** heads of `Tokens::note_radius` with the note's number: a
//!   dark plate for a note that waits for an agent, the agent's indigo for
//!   one that waits for the person, a green tick for a done one, a grey
//!   ring for one off its entity; the chosen one ringed in amber, the one
//!   under the pointer ringed fainter, with the pointing hand and, after
//!   `Tokens::note_hover_delay`, its first line beside it. Behind the
//!   bodies (a ray from the eye through the place meets a face nearer than
//!   it by more than a hundredth of the model's size), or in the half a
//!   section cuts away, faint and not clickable, as the mates' markers
//!   behind a face are; the chosen note's pin in the half cut away is drawn
//!   in full. A click on a pin chooses its note, shows its text beside the
//!   pin and, when the side panel is in sight, opens the Notes tab with its
//!   row in view; a click on empty space un-chooses. Thinned for many notes
//!   by a rule that does not depend on the view ([`lay_out_pins`]): the
//!   notes taken in a fixed order (the chosen note, those that wait for the
//!   person, other open notes, done ones; by id), each a pin unless an
//!   earlier pin is within `Tokens::note_cell` points, which then stands
//!   for it too; at most [`MAX_MARKERS`] pins, the Notes tab saying how
//!   many notes in view have none. What is behind a face is looked up once
//!   per camera, document state, section and size, for the pins drawn
//!   only. What is drawn per frame is bounded: at most [`MAX_MARKERS`]
//!   pins of a fixed size, one text beside the chosen pin (three lines of
//!   it, two of its last reply) and one first line beside the pin under
//!   the pointer, whatever the zoom.
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

/// How many lines of the chosen note's text, and of its last reply, show
/// beside its pin.
const BUBBLE_LINES: usize = 3;
const BUBBLE_REPLY_LINES: usize = 2;

/// Why a note cannot be written here: in an assembly, and in a Jig in
/// use (N6).
pub const IN_ASSEMBLY: &str = "Notes are written in a part. Open this part to note it.";
pub const IN_JIG: &str = "A Jig's notes are written in its model: File › Open for editing.";
/// Why **N** writes nothing: nothing selected, or more than one thing.
pub const SELECT_FIRST: &str = "Select a face, an edge or a vertex first, then press N";
pub const SELECT_ONE: &str = "A note goes on one thing: select one, then press N";

/// What a pin shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinKind {
    /// An open note that waits for an agent: a dark plate with its
    /// number.
    ForAgent,
    /// An open note that waits for the person (the last word on it is an
    /// agent's): an indigo head with its number.
    ForYou,
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
        } else if row.for_you() {
            PinKind::ForYou
        } else {
            PinKind::ForAgent
        }
    }

    /// Its colour.
    pub fn ink(self, tokens: &Tokens) -> Rgb {
        match self {
            PinKind::ForAgent => tokens.note_for_agent,
            PinKind::ForYou => tokens.note_for_you,
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
    /// A face is nearer the eye than its place, or its place is in the
    /// half a section cuts away: drawn faint, not clicked.
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

/// The section's plane as the pins see it: a point of it, its unit normal
/// into the half that is cut away, and how far beyond it a place must be
/// to count as cut away (a note on a face in the plane is not).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PinCut {
    pub origin: Vec3,
    pub normal: Vec3,
    pub slack: f64,
}

impl PinCut {
    /// Whether `place` lies in the half that is cut away.
    pub fn cuts(&self, place: Vec3) -> bool {
        let side = (place - self.origin).dot(self.normal);
        side.is_finite() && side > self.slack
    }
}

/// How pins are laid out ([`lay_out_pins`]): how near (in points at the
/// view's zoom) a note shares an earlier pin, the pins' radius, the note
/// chosen, where the view's own controls are (their plates, the view
/// cube, the triad: in points from the view's top left), under which no
/// pin is drawn, and the section, whose cut-away half holds no full pin.
#[derive(Clone, Copy, Debug)]
pub struct PinRule<'a> {
    pub cell: f32,
    pub radius: f32,
    pub selected: Option<u64>,
    pub covered: &'a [egui::Rect],
    pub cut: Option<PinCut>,
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
/// note, those that wait for the person, the other open notes, done ones;
/// by id), each drawn as a pin unless an earlier pin's place is nearer to
/// it in the model than `cell` points are at the view's zoom (rounded up
/// to a power of two model units, so a small zoom keeps it), which then
/// stands for it too; at most [`MAX_MARKERS`] pins, a note after them near
/// none of them left without. Which notes have pins depends on the places
/// and the zoom alone: turning the view changes it only where notes come
/// into or leave the view. A note whose pin would stand under one of the
/// view's own controls (`rule.covered`) has none (review-pins 2b: at a
/// thousand notes the pins ran under the toolbar, the cube and the triad).
/// `behind` says whether a pin's place (pixels across, down, and depth) is
/// behind a face; asked for the pins only, and not for a pin whose place
/// the section cuts away (`rule.cut`), which is faint unless chosen.
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
        cut,
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
    placed.sort_by_key(|(row, ..)| (Some(row.id) != selected, !row.for_you(), row.done, row.id));
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
                let chosen = Some(row.id) == selected;
                // In the half the section cuts away a pin would hang in
                // the air (N7): faint, unless its note is chosen.
                let faint = if cut.is_some_and(|c| c.cuts(*place)) {
                    !chosen
                } else {
                    behind(row.id, *pixel)
                };
                layout.pins.push(Pin {
                    id: row.id,
                    at: *pos,
                    kind: PinKind::of(row),
                    behind: faint,
                    chosen,
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

/// The pins' view of `section`: cut away beyond a hundredth of the
/// model's size (`size`, its diagonal) past the plane.
fn pin_cut(section: Option<&craft3d_render::Section>, size: f64) -> Option<PinCut> {
    let section = section?;
    let normal = section.normal.normalized()?;
    let slack = if size.is_finite() { size * 0.01 } else { 0.0 };
    (section.origin.is_finite() && normal.is_finite()).then_some(PinCut {
        origin: section.origin,
        normal,
        slack,
    })
}

/// The number written on a head of radius `r`, and the size it is
/// written at: `Tokens::note_number_size` at the pin's own radius, smaller
/// for three digits and more, so it stays inside.
fn number_on(tokens: &Tokens, id: u64, r: f32) -> (String, f32) {
    let words = id.to_string();
    let base = tokens.note_number_size * r / tokens.note_radius.max(1.0);
    let digits = words.chars().count().max(1) as f32;
    let size = if digits > 2.0 {
        base * 2.0 / digits
    } else {
        base
    };
    (words, size)
}

/// Paints a pin's head of `kind`, for note `id`, about `centre` with
/// radius `r`, at `alpha` (0 to 255): the view's pins and the Notes tab's
/// rows draw the same heads.
pub(crate) fn paint_head(
    painter: &egui::Painter,
    tokens: &Tokens,
    kind: PinKind,
    id: u64,
    centre: egui::Pos2,
    r: f32,
    alpha: u8,
) {
    let faded = |c: Rgb| color_alpha(c, alpha);
    let ink = kind.ink(tokens);
    let (number, size) = number_on(tokens, id, r);
    let font = egui::FontId::proportional(size.max(1.0));
    // Every head carries a light edge just outside it, so it reads on a
    // body of any colour, as the dimension line wears its casing (the
    // verdict of 2026-11-18: the indigo head vanished on a slate-blue
    // part).
    painter.circle_stroke(
        centre,
        head_outer(kind, r) + 0.5,
        egui::Stroke::new(1.0, color_alpha(tokens.note_rim, edge_alpha(tokens, alpha))),
    );
    match kind {
        PinKind::Done => {
            // A quiet tick in a small ring.
            let small = r * 0.75;
            let stroke = egui::Stroke::new(1.5, faded(ink));
            painter.circle_filled(centre, small, faded(tokens.note_fill));
            painter.circle_stroke(centre, small, stroke);
            let knee = centre + egui::vec2(-0.1 * small, 0.35 * small);
            painter.line_segment([centre + egui::vec2(-0.45 * small, 0.0), knee], stroke);
            painter.line_segment(
                [knee, centre + egui::vec2(0.45 * small, -0.35 * small)],
                stroke,
            );
        }
        PinKind::Astray => {
            // A note whose place is gone: a grey ring with its number.
            painter.circle_filled(centre, r, faded(tokens.note_fill));
            painter.circle_stroke(centre, r, egui::Stroke::new(2.0, faded(ink)));
            painter.text(
                centre,
                egui::Align2::CENTER_CENTER,
                number,
                font,
                faded(ink),
            );
        }
        PinKind::ForAgent | PinKind::ForYou => {
            painter.circle_filled(centre, r, faded(ink));
            painter.text(
                centre,
                egui::Align2::CENTER_CENTER,
                number,
                font,
                faded(tokens.note_ink),
            );
        }
    }
}

/// How far out a head of `kind` and radius `r` is painted (its ring's
/// stroke included): its light edge lies just outside.
pub(crate) fn head_outer(kind: PinKind, r: f32) -> f32 {
    match kind {
        PinKind::Done => r * 0.75 + 0.75,
        PinKind::Astray => r + 1.0,
        PinKind::ForAgent | PinKind::ForYou => r,
    }
}

/// The light edge's alpha on a head drawn at `alpha`.
fn edge_alpha(tokens: &Tokens, alpha: u8) -> u8 {
    (u16::from(tokens.note_edge_opacity) * u16::from(alpha) / 255) as u8
}

/// Paints `pin` with `painter`, standing at `pos` (points), `hovered`
/// when the pointer is on it; the centre of its head.
fn paint_pin(
    painter: &egui::Painter,
    tokens: &Tokens,
    pin: &Pin,
    pos: egui::Pos2,
    hovered: bool,
) -> egui::Pos2 {
    let r = tokens.note_radius;
    let alpha = if pin.behind { 70 } else { 255 };
    let ink = pin.kind.ink(tokens);
    let faded = |c: Rgb| color_alpha(c, alpha);
    let head = Pin::head(pos, r);
    // The pin: a dot at the place, a short stalk, the head above; the dot
    // and the stalk with the head's light edge under them.
    let edge = color_alpha(tokens.note_rim, edge_alpha(tokens, alpha));
    let foot = head + egui::vec2(0.0, head_outer(pin.kind, r));
    painter.circle_filled(pos, 2.5, edge);
    painter.line_segment([pos, foot], egui::Stroke::new(3.0, edge));
    painter.circle_filled(pos, 1.5, faded(ink));
    painter.line_segment([pos, foot], egui::Stroke::new(1.0, faded(ink)));
    paint_head(painter, tokens, pin.kind, pin.id, head, r, alpha);
    // Chosen: a 2 pt amber ring 3 pt outside the head; under the pointer,
    // that ring fainter.
    let ring = if pin.chosen {
        Some(alpha)
    } else if hovered {
        Some(tokens.note_rim_opacity)
    } else {
        None
    };
    if let Some(ring) = ring {
        painter.circle_stroke(
            head,
            r + 4.0,
            egui::Stroke::new(2.0, color_alpha(tokens.note_chosen, ring)),
        );
    }
    if pin.more > 0 {
        painter.text(
            head + egui::vec2(r + 5.0, -r),
            egui::Align2::LEFT_TOP,
            format!("+{}", pin.more),
            egui::FontId::proportional(r * 1.2),
            faded(tokens.viewport_text),
        );
    }
    head
}

/// Whose turn it is on an open note (`note.list` › `waitingFor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// The last word on it is an agent's: it waits for the person.
    You,
    /// It waits for an agent.
    Agent,
}

/// A reply on a note, as the window shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    /// Who gave it, as [`NoteRow::who`] says it.
    pub who: String,
    pub text: String,
    pub time_ms: u64,
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
    /// an entity of the feature that made it: "near an edge of Bookend"
    /// (its name is not proven to stay with what it was pinned to, so not
    /// that); "its place is gone · was near an edge of Bookend"; "its face
    /// is gone"; "face of Rib · hidden".
    pub place: String,
    /// What the place's words say when pointed at: why it is held by
    /// place, gone or hidden.
    pub place_hover: Option<String>,
    /// What a click flies to: the entity now (kind, name, feature).
    pub entity: Option<(String, String, String)>,
    /// Where its marker goes: `at`, else the stored point of a detached
    /// note or one whose place is gone; `None` while it is hidden.
    pub marker: Option<[f64; 3]>,
    /// The reply it was closed with, who closed it (as [`NoteRow::who`]
    /// says it) and when.
    pub reply: Option<String>,
    pub closed_by: Option<String>,
    pub closed_ms: u64,
    /// Its replies, in order.
    pub replies: Vec<Reply>,
    /// Whose turn it is while it is open.
    pub waiting: Option<Waiting>,
}

/// An agent in a few words: its name ("Claude Code"), "an agent" for one
/// without a name. An agent that calls itself "you" never reads as the
/// person: "you (agent)" (review-pins 2b).
pub fn agent_words(name: Option<&str>) -> String {
    match name {
        Some(name) if name.trim().eq_ignore_ascii_case("you") => format!("{name} (agent)"),
        Some(name) => name.to_string(),
        None => "an agent".to_string(),
    }
}

/// Who by an author's word and agent's name, in a few words: "you" for
/// the person, an agent as [`agent_words`] says it, "someone" for a
/// channel that says no one.
fn who_words(author: Option<&str>, agent: Option<&str>) -> String {
    match author {
        Some("person") => "you".to_string(),
        Some("agent") => agent_words(agent),
        _ => "someone".to_string(),
    }
}

impl NoteRow {
    /// An open note that waits for the person: the last word on it is an
    /// agent's.
    pub fn for_you(&self) -> bool {
        !self.done && self.waiting == Some(Waiting::You)
    }

    /// Off its entity: detached, or held by place where nothing is any
    /// more. Drawn grey at its own point.
    pub fn astray(&self) -> bool {
        matches!(self.attachment.as_str(), "detached" | "lost")
    }

    /// Who wrote it, in a few words: "you" for the person alone; an agent
    /// as [`agent_words`] says it ("Claude Code"); "someone".
    pub fn who(&self) -> String {
        who_words(Some(self.author.as_str()), self.agent.as_deref())
    }

    /// Its thread after its text, in order: its replies, with the reply it
    /// was closed with among them at its time (who, text, when, and
    /// whether it is the closing's).
    pub fn thread(&self) -> Vec<(&str, &str, u64, bool)> {
        let mut lines: Vec<(&str, &str, u64, bool)> = self
            .replies
            .iter()
            .map(|r| (r.who.as_str(), r.text.as_str(), r.time_ms, false))
            .collect();
        if let (Some(text), Some(who)) = (&self.reply, &self.closed_by) {
            let at = lines
                .iter()
                .position(|(_, _, time, _)| *time > self.closed_ms)
                .unwrap_or(lines.len());
            lines.insert(at, (who.as_str(), text.as_str(), self.closed_ms, true));
        }
        lines
    }

    /// Whether the person wrote it (only their own note's text is theirs
    /// to change in the tab).
    pub fn yours(&self) -> bool {
        self.author == "person"
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
            let on = n.get("on").filter(|o| o.is_object());
            let kind = on.and_then(|o| text(o, "kind")).unwrap_or_default();
            let made = on
                .and_then(|o| o.get("origin"))
                .and_then(|o| text(o, "name"))
                .or_else(|| on.and_then(|o| text(o, "body")))
                .unwrap_or_default();
            // Held by place: what holds it, an entity of the feature that
            // made it, never the name it was pinned by (the round it
            // spoke of may be a chamfer now).
            let (one, thing) = match kind.as_str() {
                "edge" => ("an edge", "edge"),
                "vertex" => ("a vertex", "vertex"),
                "body" => ("the body", "part"),
                _ => ("a face", "face"),
            };
            let place = match attachment.as_str() {
                "byPlace" => format!("near {one} of {made}"),
                "lost" => format!("its place is gone · was near {one} of {made}"),
                "detached" if on.is_none() => "its place is gone".to_string(),
                "detached" => format!("its {thing} is gone"),
                "hidden" if kind == "body" => format!("body {made} · hidden"),
                "hidden" => format!("{kind} of {made} · hidden"),
                _ if kind == "body" => format!("body {made}"),
                _ => format!("{kind} of {made}"),
            };
            let place_hover = match attachment.as_str() {
                "byPlace" => Some(format!(
                    "The {thing} this note was written on has no lasting name; the note is kept at its place."
                )),
                "lost" | "detached" | "hidden" => text(n, "why"),
                _ => None,
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
            let who_of = |r: &Value| who_words(text(r, "author").as_deref(), text(r, "agent").as_deref());
            let replies = n
                .get("replies")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|r| {
                            Some(Reply {
                                who: who_of(r),
                                text: text(r, "text")?,
                                time_ms: r.get("timeMs").and_then(Value::as_u64).unwrap_or(0),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let author = text(n, "author").unwrap_or_default();
            let waiting = match n.get("waitingFor").and_then(Value::as_str) {
                Some("you") => Some(Waiting::You),
                Some("agent") => Some(Waiting::Agent),
                // A list without it (none known): an agent's open note
                // waits for the person.
                _ if done.is_none() && n.get("waitingFor").is_none() => Some(if author == "agent" {
                    Waiting::You
                } else {
                    Waiting::Agent
                }),
                _ => None,
            };
            Some(NoteRow {
                id,
                text: text(n, "text").unwrap_or_default(),
                done: done.is_some(),
                author,
                agent: text(n, "agent"),
                time_ms: n.get("timeMs").and_then(Value::as_u64).unwrap_or(0),
                attachment,
                place,
                place_hover,
                entity,
                marker,
                reply: done.and_then(|d| text(d, "reply")),
                closed_by: done.map(who_of),
                closed_ms: done
                    .and_then(|d| d.get("timeMs"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                replies,
                waiting,
            })
        })
        .collect()
}

/// A command's refusal in a person's words (N6): the command's own
/// sentence names commands and is for agents (it goes to the hover).
pub fn person_words(why: &str) -> String {
    if why.contains("notes, as many as it may") || why.contains("at most 1000 notes") {
        return "This document holds 1000 notes, as many as it may: delete some that are done."
            .to_string();
    }
    if why.contains("bytes of text together") {
        return "The notes hold too much text together: shorten this one, or delete some that are done."
            .to_string();
    }
    if why.contains("replies already") {
        return "This note has 20 replies, as many as a note takes.".to_string();
    }
    if why.contains("this is a Jig") {
        return IN_JIG.to_string();
    }
    if why.contains("not yet in an assembly") {
        return IN_ASSEMBLY.to_string();
    }
    // The command's name before its sentence is the agents'.
    let plain = match why.split_once(": ") {
        Some((head, rest)) if head.starts_with("note.") && !head.contains(' ') => rest,
        _ => why,
    };
    let mut words = plain.to_string();
    if let Some(first) = words.get(..1) {
        let upper = first.to_uppercase();
        words.replace_range(..1, &upper);
    }
    words
}

/// What a note is written on, as `note.add` takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteTarget {
    /// A body, a face, an edge or a vertex.
    pub kind: SelectionKind,
    /// Its persistent name; empty for a body.
    pub name: String,
    /// The feature whose body has it (`F3`).
    pub feature: String,
    /// The point on it, when one is known.
    pub point: Option<[f64; 3]>,
}

impl NoteTarget {
    /// What is pointed at at a place of the view (`pick`'s answer): the
    /// vertex, else the edge, else the face, each with its point; a face
    /// of a body whose faces have no names, the body. `None` for nothing.
    pub fn pointed(found: &craft3d_engine::ScenePointed) -> Option<Self> {
        let near = |kind: SelectionKind, n: &craft3d_engine::SceneNear| {
            n.name.as_ref().map(|name| NoteTarget {
                kind,
                name: name.clone(),
                feature: n.feature.clone(),
                point: Some([n.point.x, n.point.y, n.point.z]),
            })
        };
        found
            .vertex
            .as_ref()
            .and_then(|v| near(SelectionKind::Vertex, v))
            .or_else(|| {
                found
                    .edge
                    .as_ref()
                    .filter(|_| found.vertex.is_none())
                    .and_then(|e| near(SelectionKind::Edge, e))
            })
            .or_else(|| {
                found.face.as_ref().map(|f| {
                    let point = Some([f.point.x, f.point.y, f.point.z]);
                    match &f.face {
                        Some(name) => NoteTarget {
                            kind: SelectionKind::Face,
                            name: name.clone(),
                            feature: f.feature.clone(),
                            point,
                        },
                        None => NoteTarget {
                            kind: SelectionKind::Body,
                            name: String::new(),
                            feature: f.feature.clone(),
                            point,
                        },
                    }
                })
            })
    }

    /// It as an item of `select.set`.
    pub fn item(&self) -> Value {
        match self.kind {
            SelectionKind::Body => json!({"kind": "body", "feature": self.feature}),
            kind => json!({"kind": kind.name(), "name": self.name, "feature": self.feature}),
        }
    }
}

/// The field a note is being written in.
#[derive(Clone, Debug, PartialEq)]
pub struct Writing {
    /// What it goes on: `on` and `feature` as `note.add` takes them.
    pub on: String,
    pub feature: Option<String>,
    /// The click's point on it, when there was one: the field follows it
    /// as the camera moves.
    pub point: Option<[f64; 3]>,
    pub text: String,
    /// Where the field is shown when it has no point, in points.
    pub at: egui::Pos2,
    /// The field takes the keyboard in its first frame.
    pub focus: bool,
    /// Sent to be added (`note.add`): the field waits for the answer.
    pub sent: bool,
    /// Why `note.add` refused it, in a person's words: said at the field,
    /// the text kept; the command's own sentence as its hover.
    pub error: Option<String>,
    pub error_hover: Option<String>,
}

/// What is behind a face is looked up for this camera, state, section
/// and size.
type SeenKey = (
    crate::state::CameraState,
    u64,
    [u32; 2],
    Option<(u64, u64)>,
    Option<[u64; 6]>,
);

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
    /// The reply being written on the chosen row: its note and its text,
    /// kept until another row is chosen; the field takes the keyboard in
    /// the next frame.
    pub(crate) reply: Option<(u64, String)>,
    pub(crate) reply_focus: bool,
    /// The tab's last act on a note (a reply, Done, Open again, Delete)
    /// waits for its answer: on which note, and whether it carried the
    /// reply field's text.
    pub(crate) act: Option<(u64, bool)>,
    /// Why the last act was refused, under its row until the next act: the
    /// note, the person's words and the command's own.
    pub(crate) act_error: Option<(u64, String, String)>,
    /// The pin under the pointer, and since when (egui's time).
    pub(crate) hovered: Option<(u64, f64)>,
    /// The row to bring into view in the Notes tab (a pin was clicked).
    pub(crate) scroll_to: Option<u64>,
    /// When the sentence for the agent was copied (egui's time).
    pub(crate) copied: Option<f64>,
    /// Where the last frame drew the link *Reply* beside the chosen pin.
    pub(crate) reply_link: Option<egui::Rect>,
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
    /// mouse button, when it was offered (greyed or not).
    pub fn note_here_entry(&self) -> Option<egui::Rect> {
        self.notes.here_hit
    }

    /// Where the last frame drew the link *Reply* beside the chosen pin.
    pub fn note_reply_link(&self) -> Option<egui::Rect> {
        self.notes.reply_link
    }

    /// The pin under the pointer in the last frame, when there was one.
    pub fn note_hovered(&self) -> Option<u64> {
        self.notes.hovered.map(|(id, _)| id)
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

    /// How many open notes wait for the person: the last word on each is
    /// an agent's.
    pub fn notes_for_you(&self) -> usize {
        self.notes.rows.iter().filter(|r| r.for_you()).count()
    }

    /// How many open notes wait for an agent.
    pub fn notes_for_agent(&self) -> usize {
        self.notes
            .rows
            .iter()
            .filter(|r| !r.done && !r.for_you())
            .count()
    }

    /// `ui.get` › `notes`: how many open notes wait for the person and
    /// for an agent (N2: an agent that looks sees what waits for it).
    pub(crate) fn notes_counts_json(&self) -> Value {
        json!({"forYou": self.notes_for_you(), "forAgent": self.notes_for_agent()})
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
        let known = |id: u64| self.notes.rows.iter().any(|r| r.id == id);
        if let Some(selected) = self.state.notes.selected
            && !known(selected)
        {
            self.state.notes.selected = None;
        }
        if self
            .notes
            .editing
            .as_ref()
            .is_some_and(|(id, _)| !known(*id))
        {
            self.notes.editing = None;
        }
        if self.notes.reply.as_ref().is_some_and(|(id, _)| !known(*id)) {
            self.notes.reply = None;
        }
    }

    /// Shows the Notes tab (the agent pill's click while notes wait for
    /// the person).
    pub fn show_notes_tab(&mut self) {
        self.state.panels.side_tab = crate::state::SideTab::Notes;
        self.state.panels.tree = true;
        if self.narrow() {
            self.state.panels.drawer = true;
        }
    }

    /// Whether the side panel is in sight: beside the view, or the
    /// drawer open in the narrow layout.
    fn side_in_sight(&self) -> bool {
        if self.narrow() {
            self.state.panels.drawer
        } else {
            self.state.panels.tree
        }
    }

    /// Why a note cannot be written in the document shown, whatever is
    /// selected: an assembly, a Jig in use (N6).
    pub(crate) fn note_place_refusal(&self) -> Option<&'static str> {
        if self.is_assembly_shown() {
            Some(IN_ASSEMBLY)
        } else if self.jig_in_use() {
            Some(IN_JIG)
        } else {
            None
        }
    }

    /// Why **N** (and the tab's *Note on selection*) writes no note on
    /// what is selected now; `None` when it can: one body, face, edge or
    /// vertex selected, in a part.
    pub(crate) fn note_refusal(&self) -> Option<&'static str> {
        if let Some(why) = self.note_place_refusal() {
            return Some(why);
        }
        match self.selection().items.as_slice() {
            [] => Some(SELECT_FIRST),
            [one]
                if matches!(
                    one.kind,
                    SelectionKind::Body
                        | SelectionKind::Face
                        | SelectionKind::Edge
                        | SelectionKind::Vertex
                ) =>
            {
                None
            }
            [_] => Some(SELECT_FIRST),
            _ => Some(SELECT_ONE),
        }
    }

    /// Whether **N** can write a note: one body, face, edge or vertex
    /// selected, in a part.
    pub(crate) fn note_here_offered(&self) -> bool {
        self.note_refusal().is_none()
    }

    /// **N**, or the tab's *Note on selection*: the field on the one thing
    /// selected, or the reason it cannot be, as a hint.
    pub(crate) fn note_on_selection(&mut self) {
        match self.note_refusal() {
            None => self.start_note(None),
            Some(why) => self.hint(why, super::hints::HINT_SECONDS),
        }
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
        self.open_note_field(NoteTarget {
            kind: item.kind,
            name: item.name.clone(),
            feature: item.feature.clone(),
            point,
        });
    }

    /// *Note here…* on `target`, the thing pointed at: selects it and
    /// opens the field on it, at its point (N5); in an assembly or a Jig
    /// in use, the reason instead.
    pub(crate) fn note_here(&mut self, target: NoteTarget) {
        if let Some(why) = self.note_place_refusal() {
            self.hint(why, super::hints::HINT_SECONDS);
            return;
        }
        self.queue_select(json!({"items": [target.item()], "mode": "replace"}));
        self.open_note_field(target);
    }

    /// Opens the field on `target`; the text of a field already open goes
    /// with it.
    fn open_note_field(&mut self, target: NoteTarget) {
        let (on, feature) = match target.kind {
            SelectionKind::Body => (target.feature.clone(), None),
            _ => (target.name.clone(), Some(target.feature.clone())),
        };
        let at = target
            .point
            .and_then(|p| self.note_screen(p))
            .or_else(|| self.rect.map(|r| r.center()))
            .unwrap_or(egui::pos2(100.0, 100.0));
        let text = self
            .notes
            .writing
            .take()
            .map(|w| w.text)
            .unwrap_or_default();
        self.notes.writing = Some(Writing {
            on,
            feature,
            point: target.point,
            text,
            at,
            focus: true,
            sent: false,
            error: None,
            error_hover: None,
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
        writing.error_hover = None;
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
                    writing.error = Some(person_words(&why));
                    writing.error_hover = Some(why);
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
        let mut covered: Vec<egui::Rect> = self
            .controls_layout
            .as_ref()
            .map(|l| {
                l.parts()
                    .into_iter()
                    .map(|(_, r)| r.translate(-origin))
                    .collect()
            })
            .unwrap_or_default();
        // The section's plate beside its cut is a control of the view too
        // (the pictures of 2026-11-18: a pin stood over its buttons).
        if let Some(handle) = self.section_handle() {
            covered.push(handle.group.translate(-origin));
        }
        covered
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
        let model = scene.map_or(0.0, |s| s.bbox.diagonal());
        let slack = model * 0.01;
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
            cut: pin_cut(section.as_ref(), model),
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
    /// selected), when it is on something. A reply being written on
    /// another note is let go.
    pub(crate) fn choose_note(&mut self, id: u64, fly: bool) {
        if self.state.notes.selected != Some(id) {
            self.notes.act_error = None;
        }
        self.state.notes.selected = Some(id);
        if self.notes.reply.as_ref().is_some_and(|(at, _)| *at != id) {
            self.notes.reply = None;
        }
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

    /// Opens the Notes tab on note `id`'s row, when the side panel is in
    /// sight (a click on its pin, N4); `reply`: its reply field takes the
    /// keyboard, the drawer opened on a phone (the link *Reply*).
    fn open_note_row(&mut self, id: u64, reply: bool) {
        if reply {
            if self.narrow() {
                self.state.panels.drawer = true;
            } else {
                self.state.panels.tree = true;
            }
            if self.notes.reply.as_ref().is_none_or(|(at, _)| *at != id) {
                self.notes.reply = Some((id, String::new()));
            }
            self.notes.reply_focus = true;
        } else if !self.side_in_sight() {
            return;
        }
        self.state.panels.side_tab = crate::state::SideTab::Notes;
        self.notes.scroll_to = Some(id);
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

    /// The marker drawn in the last frame whose head is within reach of
    /// `at` (points), the nearest.
    fn marker_at(&self, at: egui::Pos2) -> Option<u64> {
        let reach = self.tokens.note_reach;
        self.notes
            .markers
            .iter()
            .filter(|(_, c)| c.distance(at) <= reach)
            .min_by(|a, b| a.1.distance(at).total_cmp(&b.1.distance(at)))
            .map(|(id, _)| *id)
    }

    /// A click at `at` (points) on a marker drawn in the last frame:
    /// chooses its note and opens its row in the Notes tab when the side
    /// panel is in sight; on the link *Reply* beside the chosen pin, puts
    /// the keyboard in its reply field. True when it did.
    pub(crate) fn click_note_marker(&mut self, at: egui::Pos2) -> bool {
        if let (Some(link), Some(id)) = (self.notes.reply_link, self.state.notes.selected)
            && link.contains(at)
        {
            self.open_note_row(id, true);
            return true;
        }
        match self.marker_at(at) {
            Some(id) => {
                self.choose_note(id, false);
                self.open_note_row(id, false);
                true
            }
            None => false,
        }
    }

    /// The notes in the view: the markers, the chosen one's text, the
    /// field a note is written in, and **N**.
    pub(super) fn paint_notes(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        scene: Option<&Scene>,
    ) {
        // N writes a note on what is selected, or says why not.
        if self.notes.writing.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(NOTE_KEY) && i.modifiers.is_none())
        {
            self.note_on_selection();
        }
        self.notes.markers.clear();
        self.notes.drawn = (0, 0);
        self.notes.in_view = 0;
        self.notes.reply_link = None;
        if !self.notes.rows.is_empty() && !self.state.notes.hide_markers {
            self.paint_markers(ui, rect, scene);
        } else {
            self.notes.hovered = None;
        }
        self.note_answered();
        self.writing_field(ui);
    }

    fn paint_markers(&mut self, ui: &egui::Ui, rect: egui::Rect, scene: Option<&Scene>) {
        let tokens = self.tokens;
        let size = self.size;
        let camera = self.state.camera.to_render();
        let scale = f64::from(self.pixels_per_point.max(0.1));
        let section = self.section();
        let model = scene.map_or(0.0, |s| s.bbox.diagonal());
        let cut = pin_cut(section.as_ref(), model);
        // Behind a face: looked up once per camera, state, section and
        // size.
        let plane = cut.map(|c| {
            [
                c.origin.x.to_bits(),
                c.origin.y.to_bits(),
                c.origin.z.to_bits(),
                c.normal.x.to_bits(),
                c.normal.y.to_bits(),
                c.normal.z.to_bits(),
            ]
        });
        let key: SeenKey = (
            self.state.camera,
            scene.map_or(0, |s| s.revision),
            size,
            self.notes.read_for,
            plane,
        );
        let mut seen = match self.notes.seen.take() {
            Some((known, seen)) if known == key => seen,
            _ => BTreeMap::new(),
        };
        let slack = model * 0.01;
        let rows = Arc::clone(&self.notes.rows);
        let covered = self.controls_covered();
        let rule = PinRule {
            cell: tokens.note_cell,
            radius: tokens.note_radius,
            selected: self.state.notes.selected,
            covered: &covered,
            cut,
        };
        let layout = lay_out_pins(&rows, &camera, size, scale, &rule, &mut |id, at| {
            *seen
                .entry(id)
                .or_insert_with(|| pin_behind(scene, &camera, size, at, section.as_ref(), slack))
        });
        self.notes.seen = Some((key, seen));
        let origin = rect.min.to_vec2();
        // The pin under the pointer (none while a button is held: a drag
        // turns the view).
        // The pointer egui knows, else one placed without a mouse (`ui.pick`
        // with `hover`: where it rests, kept apart from what it points at).
        let placed = self.hover_point();
        let pointer = ui
            .input(|i| (!i.pointer.any_down()).then(|| i.pointer.hover_pos()))
            .flatten()
            .filter(|p| rect.contains(*p))
            .or(placed);
        let reach = tokens.note_reach;
        let hovered = pointer.and_then(|p| {
            layout
                .pins
                .iter()
                .filter(|pin| !pin.behind)
                .map(|pin| (pin.id, Pin::head(pin.at + origin, tokens.note_radius)))
                .filter(|(_, head)| head.distance(p) <= reach)
                .min_by(|a, b| a.1.distance(p).total_cmp(&b.1.distance(p)))
                .map(|(id, _)| id)
        });
        let now = ui.input(|i| i.time);
        self.notes.hovered = match (hovered, self.notes.hovered) {
            (Some(id), Some((was, since))) if was == id => Some((id, since)),
            (Some(id), _) => Some((id, now)),
            (None, _) => None,
        };
        let painter = ui.painter_at(rect);
        let mut hits = Vec::new();
        let mut bubble = None;
        let mut first_line = None;
        for pin in &layout.pins {
            let on = hovered == Some(pin.id);
            let head = paint_pin(&painter, &tokens, pin, pin.at + origin, on);
            if pin.chosen {
                bubble = rows.iter().find(|r| r.id == pin.id).map(|r| (r, head));
            } else if on {
                first_line = rows.iter().find(|r| r.id == pin.id).map(|r| (r, head));
            }
            if !pin.behind {
                hits.push((pin.id, head));
            }
        }
        self.notes.markers = hits;
        self.notes.drawn = (layout.pins.len(), layout.stood_for);
        self.notes.in_view = layout.in_view;
        if hovered.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // The chosen note's text beside its pin, on a plate.
        if let Some((row, head)) = bubble {
            self.notes.reply_link =
                paint_bubble(&painter, &tokens, row, head, rect, self.agents_now());
            if let Some(link) = self.notes.reply_link
                && pointer.is_some_and(|p| link.contains(p))
            {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        // The first line of the note under the pointer, once it rested.
        if let (Some((row, head)), Some((_, since))) = (first_line, self.notes.hovered) {
            let delay = f64::from(tokens.note_hover_delay.max(0.0));
            let rested = now - since;
            if rested >= delay {
                paint_first_line(&painter, &tokens, row, head, rect);
            } else {
                ui.ctx().request_repaint_after(
                    std::time::Duration::try_from_secs_f64(delay - rested).unwrap_or_default(),
                );
            }
        }
    }

    /// The field a note is written in, while one is open: Enter adds the
    /// note, Shift+Enter starts a new line in it (the field grows),
    /// Escape leaves. It stands at its place as the camera moves, stops at
    /// the longest text a note takes, counts the code points near it, and
    /// says why `note.add` refused the note, keeping the text. A click
    /// elsewhere leaves an empty field, and keeps one with text.
    fn writing_field(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        // Where its place is now: it follows the camera.
        let place = self
            .notes
            .writing
            .as_ref()
            .and_then(|w| w.point)
            .and_then(|p| self.note_screen(p));
        let Some(writing) = &mut self.notes.writing else {
            return;
        };
        if let Some(place) = place {
            writing.at = place;
        }
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
                                .hint_text("What should change here?")
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
                            writing.error_hover = None;
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
                            let said = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(why)
                                        .small()
                                        .color(color(tokens.plate_text)),
                                )
                                .wrap(),
                            );
                            if let Some(hover) = &writing.error_hover {
                                said.on_hover_text(hover);
                            }
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

/// A plate of `size` beside a pin's head, on its right where the view has
/// room, else on its left; within `view`.
fn beside(head: egui::Pos2, r: f32, size: egui::Vec2, view: egui::Rect) -> egui::Rect {
    let pad = 6.0;
    let right = egui::Rect::from_min_size(head + egui::vec2(r + 6.0 + pad, -size.y * 0.5), size);
    let at = if right.right() + pad > view.right() {
        egui::Rect::from_min_size(
            head + egui::vec2(-(r + 6.0 + pad) - size.x, -size.y * 0.5),
            size,
        )
    } else {
        right
    };
    let dy = if at.top() - pad < view.top() {
        view.top() - (at.top() - pad)
    } else if at.bottom() + pad > view.bottom() {
        view.bottom() - (at.bottom() + pad)
    } else {
        0.0
    };
    at.translate(egui::vec2(0.0, dy))
}

/// A galley of `text` at most `width` wide and `rows` lines long (cut
/// with "…").
fn lines(
    painter: &egui::Painter,
    text: &str,
    size: f32,
    ink: egui::Color32,
    width: f32,
    rows: usize,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat::simple(egui::FontId::proportional(size), ink),
    );
    job.wrap.max_width = width;
    job.wrap.max_rows = rows;
    painter.layout_job(job)
}

/// The chosen note's text beside its pin, on a plate (N4): the note in
/// three lines at most, its last reply dim, who and when; on a note that
/// waits for the person, the link *Reply*, whose place it returns.
fn paint_bubble(
    painter: &egui::Painter,
    tokens: &Tokens,
    row: &NoteRow,
    head: egui::Pos2,
    view: egui::Rect,
    now: u64,
) -> Option<egui::Rect> {
    let width = tokens.note_text_width;
    let text = lines(
        painter,
        &row.text,
        13.0,
        color(tokens.plate_text),
        width,
        BUBBLE_LINES,
    );
    let last = row
        .thread()
        .last()
        .map(|(who, text, _, _)| format!("“{text}” · {who}"));
    let reply = last.map(|said| {
        lines(
            painter,
            &said,
            12.0,
            color(tokens.plate_text_dim),
            width,
            BUBBLE_REPLY_LINES,
        )
    });
    let mut meta = row.who();
    if row.time_ms > 0 {
        meta.push_str(&format!(
            " · {}",
            super::sidepanel::agent::ago(row.time_ms, now)
        ));
    }
    let meta = lines(painter, &meta, 11.0, color(tokens.plate_text_dim), width, 1);
    let link = row.for_you().then(|| {
        lines(
            painter,
            "Reply",
            12.0,
            color(tokens.plate_info_accent),
            width,
            1,
        )
    });
    let gap = 3.0;
    let galleys: Vec<&Arc<egui::Galley>> =
        [Some(&text), reply.as_ref(), Some(&meta), link.as_ref()]
            .into_iter()
            .flatten()
            .collect();
    let inner = egui::vec2(
        galleys.iter().map(|g| g.size().x).fold(0.0, f32::max),
        galleys.iter().map(|g| g.size().y).sum::<f32>()
            + gap * (galleys.len().saturating_sub(1)) as f32,
    );
    let plate = beside(head, tokens.note_radius, inner, view);
    painter.rect_filled(
        plate.expand(6.0),
        tokens.radius,
        color_alpha(tokens.plate_fill, tokens.plate_opacity),
    );
    let mut y = plate.top();
    let mut link_rect = None;
    for galley in [Some(text), reply, Some(meta)].into_iter().flatten() {
        let h = galley.size().y;
        painter.galley(
            egui::pos2(plate.left(), y),
            galley,
            color(tokens.plate_text),
        );
        y += h + gap;
    }
    if let Some(galley) = link {
        let rect = egui::Rect::from_min_size(egui::pos2(plate.left(), y), galley.size());
        painter.galley(rect.min, galley, color(tokens.plate_info_accent));
        // A finger's reach about the word.
        link_rect = Some(rect.expand(4.0));
    }
    link_rect
}

/// The first line of the note under the pointer, beside its pin (N4).
fn paint_first_line(
    painter: &egui::Painter,
    tokens: &Tokens,
    row: &NoteRow,
    head: egui::Pos2,
    view: egui::Rect,
) {
    let first = row.text.lines().next().unwrap_or_default();
    let galley = lines(
        painter,
        first,
        13.0,
        color(tokens.plate_text),
        tokens.note_text_width,
        1,
    );
    let plate = beside(head, tokens.note_radius, galley.size(), view);
    painter.rect_filled(
        plate.expand(5.0),
        tokens.radius,
        color_alpha(tokens.plate_fill, tokens.plate_opacity),
    );
    painter.galley(plate.min, galley, color(tokens.plate_text));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn rows_say_where_who_and_whose_turn_it_is() {
        let listed = json!({"notes": [
            {"id": 1, "text": "thicker", "state": "open", "author": "person", "agent": null,
             "timeMs": 5, "attachment": "attached", "waitingFor": "agent", "replies": [],
             "on": {"kind": "face", "name": "F1:face(1)", "feature": "F1", "body": "Plate",
                    "origin": {"feature": "F1", "name": "Plate", "type": "block"}},
             "at": [1.0, 2.0, 3.0], "point": [1.0, 2.0, 3.0]},
            {"id": 2, "text": "why?", "state": "open", "author": "agent", "agent": "Claude Code",
             "attachment": "byPlace", "waitingFor": "you",
             "on": {"kind": "edge", "name": null, "pinnedAs": "F1:edge(7)", "feature": "F3",
                    "body": "Chamfer", "origin": {"feature": "F1", "name": "Bookend"}},
             "at": [0.0, 0.0, 0.0], "point": [0.0, 0.0, 0.0]},
            {"id": 3, "text": "gone", "state": "done", "author": "unknown", "attachment": "detached",
             "waitingFor": null, "why": "the body of `Lug` is gone",
             "on": {"kind": "face", "name": "F4:face(5)", "feature": "F4"}, "point": [9.0, 9.0, 9.0],
             "done": {"author": "person", "agent": null, "reply": "ok", "timeMs": 20},
             "replies": [{"author": "agent", "agent": "Claude Code", "timeMs": 9,
                          "text": "removed it"},
                         {"author": "agent", "agent": null, "timeMs": 30, "text": "checked"}]},
            {"id": 4, "text": "this corner", "state": "open", "author": "person", "attachment": "lost",
             "waitingFor": "agent",
             "on": {"kind": "vertex", "name": null, "pinnedAs": "F2:fillet(1)", "feature": "F2",
                    "origin": {"feature": "F2", "name": "Round"}},
             "point": [4.0, 5.0, 6.0]},
            {"id": 5, "text": "over the channel", "state": "open", "author": "agent", "agent": null,
             "attachment": "attached", "waitingFor": "agent",
             "replies": [{"author": "person", "timeMs": 7, "text": "9 mm"}],
             "on": {"kind": "face", "name": "F1:face(2)", "feature": "F1", "body": "Plate"},
             "at": [1.0, 1.0, 1.0], "point": [1.0, 1.0, 1.0]},
            {"id": 6, "text": "hidden", "state": "open", "author": "person", "attachment": "hidden",
             "waitingFor": "agent",
             "on": {"kind": "face", "name": "F1:face(2)", "feature": "F1", "body": "Plate"},
             "point": [1.0, 1.0, 1.0]},
            "nonsense"
        ]});
        let rows = rows_of(&listed);
        assert_eq!(rows.len(), 6);
        // The words of place (N3): held by place, what holds it, not the
        // round it spoke of; its place gone; its face gone; hidden.
        assert_eq!(rows[1].place, "near an edge of Bookend");
        assert!(
            rows[1]
                .place_hover
                .as_deref()
                .is_some_and(|h| h.contains("no lasting name")),
            "{:?}",
            rows[1].place_hover
        );
        assert_eq!(
            rows[3].place,
            "its place is gone · was near a vertex of Round"
        );
        assert_eq!(rows[3].marker, Some([4.0, 5.0, 6.0]), "at its own point");
        assert!(rows[3].astray() && rows[2].astray() && !rows[1].astray());
        assert_eq!(rows[3].entity, None);
        assert_eq!(rows[0].place, "face of Plate");
        assert_eq!(rows[5].place, "face of Plate · hidden");
        assert_eq!(rows[0].who(), "you");
        assert_eq!(
            rows[0].entity,
            Some(("face".into(), "F1:face(1)".into(), "F1".into()))
        );
        assert!(rows[1].for_you() && !rows[0].for_you());
        assert_eq!(rows[1].who(), "Claude Code");
        assert_eq!(rows[1].entity, None, "never an entity it cannot prove");
        assert_eq!(rows[2].place, "its face is gone");
        assert_eq!(
            rows[2].place_hover.as_deref(),
            Some("the body of `Lug` is gone")
        );
        assert_eq!(rows[2].marker, Some([9.0, 9.0, 9.0]));
        assert_eq!(rows[2].who(), "someone");
        // Closed by the person with a reply, replied to by agents before
        // and after: the closing's reply among them at its time.
        assert_eq!(rows[2].closed_by.as_deref(), Some("you"));
        let thread: Vec<(&str, &str)> = rows[2]
            .thread()
            .iter()
            .map(|(who, text, _, _)| (*who, *text))
            .collect();
        assert_eq!(
            thread,
            vec![
                ("Claude Code", "removed it"),
                ("you", "ok"),
                ("an agent", "checked")
            ]
        );
        // An agent's note the person answered: the agent's turn.
        assert!(!rows[4].for_you());
        assert_eq!(rows[4].who(), "an agent");
        assert_eq!(rows[4].replies[0].who, "you");
        // An agent that calls itself "you" never reads as the person
        // (review-pins 2b).
        let you = rows_of(&json!({"notes": [
            {"id": 1, "text": "x", "author": "agent", "agent": "you", "attachment": "attached",
             "done": {"author": "agent", "agent": "You"}}
        ]}));
        assert_eq!(you[0].who(), "you (agent)");
        assert_eq!(you[0].closed_by.as_deref(), Some("You (agent)"));
        assert_eq!(PinKind::of(&rows[1]), PinKind::ForYou);
        assert_eq!(PinKind::of(&rows[4]), PinKind::ForAgent);
        assert_eq!(PinKind::of(&rows[2]), PinKind::Done);
        assert_eq!(PinKind::of(&rows[3]), PinKind::Astray);
        assert_eq!(PinKind::of(&rows[0]), PinKind::ForAgent);
        assert!(rows_of(&json!({})).is_empty());
    }

    /// A refusal in a person's words (N6): the two a person can meet in
    /// the spec's words, a command's name taken off the front, the rest as
    /// it is.
    #[test]
    fn refusals_read_in_a_persons_words() {
        assert_eq!(
            person_words(
                "note.add: the document holds 1000 notes, as many as it may; delete some first (note.delete with state: done takes away those done)"
            ),
            "This document holds 1000 notes, as many as it may: delete some that are done."
        );
        assert!(
            person_words("note.reply: the notes of a document hold at most 2097152 bytes of text together (so it always saves)")
                .starts_with("The notes hold too much text together")
        );
        assert_eq!(
            person_words("note.add: a note's text must not hold the control character U+0007"),
            "A note's text must not hold the control character U+0007"
        );
        assert_eq!(
            person_words("note.delete: this is a Jig: only its parameters change here (param.set)"),
            IN_JIG
        );
        assert_eq!(person_words(""), "");
    }

    /// The pins of many notes close together: the same notes keep their
    /// pins when the view turns a little; open notes and those for the
    /// person before done ones; at most `MAX_MARKERS`, the rest counted.
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
                place_hover: None,
                entity: None,
                marker: Some([i * 1.5, j * 1.5, 0.0]),
                reply: None,
                closed_by: None,
                closed_ms: 0,
                replies: Vec::new(),
                waiting: (k % 3 != 0).then_some(if k % 7 == 0 {
                    Waiting::You
                } else {
                    Waiting::Agent
                }),
            });
        }
        let rule = |covered: &'static [egui::Rect]| PinRule {
            cell: 22.0,
            radius: 9.0,
            selected: None,
            covered,
            cut: None,
        };
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
                &rule(&[]),
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
        static LEFT: [egui::Rect; 1] = [egui::Rect {
            min: egui::Pos2 { x: 0.0, y: 0.0 },
            max: egui::Pos2 { x: 640.0, y: 800.0 },
        }];
        static ALL: [egui::Rect; 1] = [egui::Rect {
            min: egui::Pos2 { x: 0.0, y: 0.0 },
            max: egui::Pos2 {
                x: 1280.0,
                y: 800.0,
            },
        }];
        let under = |covered: &'static [egui::Rect]| {
            lay_out_pins(
                &rows,
                &camera.to_render(),
                [1280, 800],
                1.0,
                &rule(covered),
                &mut |_, _| false,
            )
        };
        let half = under(&LEFT);
        assert_eq!(half.in_view, 1000);
        assert!(
            half.stood_for < a.stood_for,
            "{} {}",
            half.stood_for,
            a.stood_for
        );
        assert!(
            half.pins.iter().all(|p| p.at.x - 9.0 > 640.0),
            "no pin under the control"
        );
        let none = under(&ALL);
        assert!(none.pins.is_empty());
        assert_eq!(none.in_view, 1000);
    }

    /// N7: a pin whose place the section cuts away is faint and not
    /// clicked, unless its note is chosen; one on the kept side, or in the
    /// plane itself, is as the ray says.
    #[test]
    fn a_pin_in_the_half_cut_away_is_faint_unless_chosen() {
        let row = |id: u64, x: f64| NoteRow {
            id,
            text: String::new(),
            done: false,
            author: "person".into(),
            agent: None,
            time_ms: 0,
            attachment: "attached".into(),
            place: String::new(),
            place_hover: None,
            entity: None,
            marker: Some([x, 0.0, 0.0]),
            reply: None,
            closed_by: None,
            closed_ms: 0,
            replies: Vec::new(),
            waiting: Some(Waiting::Agent),
        };
        let rows = vec![row(1, -10.0), row(2, 10.0), row(3, 0.0)];
        let camera = crate::state::CameraState {
            target: [0.0, 0.0, 0.0],
            distance: 100.0,
            yaw: 0.0,
            pitch: -30.0,
            ..crate::state::CameraState::default()
        };
        let cut = PinCut {
            origin: Vec3::new(0.0, 0.0, 0.0),
            normal: Vec3::new(1.0, 0.0, 0.0),
            slack: 0.5,
        };
        let lay = |selected: Option<u64>| {
            lay_out_pins(
                &rows,
                &camera.to_render(),
                [800, 600],
                1.0,
                &PinRule {
                    cell: 1.0,
                    radius: 9.0,
                    selected,
                    covered: &[],
                    cut: Some(cut),
                },
                &mut |_, _| false,
            )
        };
        let faint = |l: &PinLayout, id: u64| l.pins.iter().find(|p| p.id == id).unwrap().behind;
        let plain = lay(None);
        assert_eq!(plain.pins.len(), 3);
        assert!(!faint(&plain, 1), "kept side");
        assert!(faint(&plain, 2), "cut away");
        assert!(!faint(&plain, 3), "in the plane");
        let chosen = lay(Some(2));
        assert!(!faint(&chosen, 2), "chosen: drawn in full");
        assert!(!cut.cuts(Vec3::new(f64::NAN, 0.0, 0.0)));
    }
}
