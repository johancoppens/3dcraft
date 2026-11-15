//! What's new: after an update the start screen shows a card *New in
//! <version>* once, and Help › What's new shows the same notes again at
//! any time (on the start screen as that card, over a document in a
//! window).
//!
//! The card is never a dialog in the way: it stands on the start screen,
//! under New and Open and above the examples, and only there. A document
//! opened from the command line, by an agent or by a test is never
//! covered; the card waits until the start screen comes up. A
//! presentation other than the whole window (Embed's `view` and `form`)
//! shows none.
//!
//! **What it says.** First what changes an answer, under a heading of its
//! own: the entries the changelog files under *Results that changed* (a
//! measure, a proof, a check or a body that an earlier version got
//! wrong). Someone who relied on such an answer must look again. Then
//! what is new, then the rest (what else changed, what was fixed, the
//! known limitations), folded away until asked for. A title unfolds to
//! what the changelog says under it.
//!
//! **Where the notes come from.** The host hands them over
//! ([`Shell::set_whats_new`]): the sections of the changelog the program
//! carries (`craft3d_engine::release_notes`) that are new to this person
//! ([`WhatsNew::for_start`]). The shell keeps no record of what was seen:
//! it says when the person closed the notes
//! ([`Shell::take_whats_new_closed`]) and the host remembers the version
//! where its preferences live. Nothing is fetched.

use super::Shell;
use crate::kit::{Button, Dialog, Glyph, Kit, centred_left, glyphs, sane};
use crate::theme::color;
use craft3d_engine::changelog::{Changelog, Entry, Kind, Section, Seen, Version};
use std::collections::BTreeSet;
use std::sync::Arc;

/// The Help menu's item.
pub const MENU_ITEM: &str = "What's new";
/// The heading over the entries that change an answer, in plain words.
pub const RESULTS_TITLE: &str = "Results that may differ from before";
/// What that heading means for the person, under it.
pub const RESULTS_HINT: &str = "A measure, a proof or a check that an earlier version gave can come out differently now, and a body it built wrong is built right or refused. If you relied on one of these, check it again.";
/// The heading over what is new.
pub const NEW_TITLE: &str = "New";
/// The folded rest.
pub const REST_TITLE: &str = "Also changed and fixed";
/// What unfolds the card while it is low.
pub const LOW_SHOW: &str = "Show";

/// Where the card stands on the start screen, for it to know whether it
/// and the first row of examples under it are wholly in sight.
#[derive(Clone, Copy, Debug)]
pub(super) struct CardPlace {
    /// The height the window gives the start screen: the notes' list
    /// takes a share of it at most.
    pub room: f32,
    /// The top of the start screen's content, where the card's own top
    /// is measured from (it moves with the scrolling).
    pub top: f32,
    /// How much of the content is in sight below that top when it is not
    /// scrolled.
    pub sight: f32,
    /// From the card's bottom to the bottom of the first row of examples,
    /// as the last frame laid them out; `None` before the first.
    pub below: Option<f32>,
}

/// The notes the window can show: what is new to the person at this
/// start, or the running version's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhatsNew {
    /// This program's version, which the card's title names.
    pub version: String,
    /// Newest first; what is not released yet, in a build between two
    /// releases, before the versions.
    pub sections: Vec<Section>,
    /// True after an update: the card comes up by itself, once.
    pub offered: bool,
}

/// One line of the notes: where its entry stands (section, entry) and
/// the entry.
type Line<'a> = ((usize, usize), &'a Section, &'a Entry);

impl WhatsNew {
    /// What a program of version `running` that carries `notes` shows a
    /// person of whom `seen` is known: after an update everything since
    /// the version seen, offered by itself; otherwise the running
    /// version's own notes, for the Help menu. `None` when there is
    /// nothing to show at all.
    pub fn for_start(notes: &Changelog, running: Version, seen: Seen) -> Option<Self> {
        let (sections, offered) = match notes.offer(running, seen) {
            Some(sections) => (sections, true),
            None => (notes.of(running), false),
        };
        if sections.is_empty() {
            return None;
        }
        Some(Self {
            version: running.to_string(),
            sections: sections.into_iter().cloned().collect(),
            offered,
        })
    }

    /// The card's and the window's title.
    pub fn title(&self) -> String {
        format!("New in {}", self.version)
    }

    /// The line under the title: one release's own summary, or which
    /// releases the notes cover.
    pub fn subtitle(&self) -> String {
        match self.sections.as_slice() {
            [only] if !only.summary.is_empty() => only.summary.clone(),
            [_] | [] => String::new(),
            sections => {
                let labels: Vec<String> = sections
                    .iter()
                    .map(|section| match section.version {
                        Some(version) => version.to_string(),
                        None => "what is not released yet".to_string(),
                    })
                    .collect();
                let (last, first) = match labels.split_last() {
                    Some((last, first)) => (last.as_str(), first.join(", ")),
                    None => ("", String::new()),
                };
                format!("What changed in {first} and {last}.")
            }
        }
    }

    fn lines(&self, wanted: impl Fn(Kind) -> bool) -> Vec<Line<'_>> {
        let mut lines = Vec::new();
        for kind in Kind::ALL {
            if !wanted(kind) {
                continue;
            }
            for (s, section) in self.sections.iter().enumerate() {
                for (e, entry) in section.entries.iter().enumerate() {
                    if entry.kind == kind {
                        lines.push(((s, e), section, entry));
                    }
                }
            }
        }
        lines
    }

    /// The entries that change an answer, newest version first.
    pub fn results(&self) -> Vec<Line<'_>> {
        self.lines(|kind| kind == Kind::Results)
    }

    /// What is new.
    pub fn added(&self) -> Vec<Line<'_>> {
        self.lines(|kind| kind == Kind::Added)
    }

    /// The rest, kind by kind.
    pub fn rest(&self) -> Vec<Line<'_>> {
        self.lines(|kind| !matches!(kind, Kind::Results | Kind::Added))
    }

    /// What the groups of the kinds `wanted` say of themselves in the
    /// changelog (the text under a group's heading that is not an entry):
    /// (the group's heading, the text), kind by kind, newest version
    /// first, each text once.
    pub fn said(&self, wanted: impl Fn(Kind) -> bool) -> Vec<(&str, &str)> {
        let mut said: Vec<(&str, &str)> = Vec::new();
        for kind in Kind::ALL {
            if !wanted(kind) {
                continue;
            }
            for section in &self.sections {
                for note in section.notes_of(kind) {
                    let one = (note.heading.as_str(), note.text.as_str());
                    if !said.contains(&one) {
                        said.push(one);
                    }
                }
            }
        }
        said
    }

    /// The one line of the card while it is low: how many results may
    /// differ (empty when none does), and how much else there is.
    pub fn low_line(&self) -> (String, String) {
        let lead = match self.results().len() {
            0 => String::new(),
            1 => "1 result may differ from before".to_string(),
            n => format!("{n} results may differ from before"),
        };
        let mut others = Vec::new();
        match self.added().len() {
            0 => {}
            n => others.push(format!("{n} new")),
        }
        match self.rest().len() {
            0 => {}
            n => others.push(format!("{n} changed and fixed")),
        }
        let tail = match (lead.is_empty(), others.is_empty()) {
            (_, true) => String::new(),
            (true, false) => others.join(", "),
            (false, false) => format!(", {}", others.join(", ")),
        };
        (lead, tail)
    }

    /// `ui.get`'s `whatsNew`: `low` is true while the card is drawn low
    /// (its title and one line), in a window too low for the whole of it.
    pub fn json(&self, shown: Option<&'static str>, low: bool) -> serde_json::Value {
        serde_json::json!({
            "version": self.version,
            "offered": self.offered,
            "shown": shown,
            "low": low,
            "versions": self.sections.iter().map(|section| match section.version {
                Some(version) => version.to_string(),
                None => "unreleased".to_string(),
            }).collect::<Vec<_>>(),
            "changesAnswers": self.results().len(),
            "new": self.added().len(),
            "rest": self.rest().len(),
        })
    }
}

/// What the notes remember between frames.
#[derive(Default)]
pub(crate) struct NewsMemory {
    notes: Option<Arc<WhatsNew>>,
    /// The person closed the card or the window, until the host takes it.
    closed: bool,
    /// The entries unfolded to their text: (section, entry).
    unfolded: BTreeSet<(usize, usize)>,
    /// Where the last frame drew the notes: `card` or `window` (the
    /// whole), `close`, `list`, `results`, `new`, `rest`, and
    /// `entry:<section>:<entry>` for each line.
    hits: Vec<(String, egui::Rect)>,
    /// Which of the two the last frame drew.
    shown: Option<&'static str>,
    /// The card was drawn whole in this frame, and in the one before:
    /// once whole it stays whole while it is shown, whatever is unfolded
    /// in it.
    whole: bool,
    whole_last: bool,
}

impl Shell {
    /// The notes the window shows, from the host: Help › What's new is
    /// offered while there are some, and notes that are `offered` (after
    /// an update) bring the card up on the start screen. `None` takes
    /// them away and closes what was open.
    pub fn set_whats_new(&mut self, notes: Option<WhatsNew>) {
        self.news.unfolded.clear();
        match notes {
            Some(notes) => {
                if notes.offered {
                    self.state.whats_new.card = true;
                }
                self.news.notes = Some(Arc::new(notes));
            }
            None => {
                self.news.notes = None;
                self.state.whats_new = crate::state::WhatsNewView::default();
            }
        }
    }

    /// The notes the host gave.
    pub fn whats_new(&self) -> Option<&WhatsNew> {
        self.news.notes.as_deref()
    }

    /// True once after the person closed the card or the window: the
    /// host then remembers this version as seen, where its preferences
    /// live. Closing them through `ui.set` is an agent's doing, not the
    /// person's, and is not told.
    pub fn take_whats_new_closed(&mut self) -> bool {
        std::mem::take(&mut self.news.closed)
    }

    /// Where the last frame drew the notes (`card` or `window`, `close`,
    /// `list`, `results`, `new`, `rest`, `entry:<section>:<entry>`), in
    /// points. Empty when it drew none.
    pub fn whats_new_hits(&self) -> &[(String, egui::Rect)] {
        &self.news.hits
    }

    /// Which of the two the last frame drew: `card`, `window`, or
    /// neither.
    pub fn whats_new_shown(&self) -> Option<&'static str> {
        self.news.shown
    }

    /// True when the last frame drew the card low: its title and the one
    /// line that unfolds it.
    pub fn whats_new_low(&self) -> bool {
        self.news.shown == Some("card") && !self.news.whole
    }

    /// `ui.get`'s `whatsNew`: null without notes.
    pub(crate) fn whats_new_json(&self) -> serde_json::Value {
        self.news
            .notes
            .as_ref()
            .map_or(serde_json::Value::Null, |notes| {
                notes.json(
                    self.news.shown,
                    self.news.shown == Some("card") && !self.news.whole,
                )
            })
    }

    /// Before the frame draws: nothing drawn yet, and without notes
    /// nothing can be open.
    pub(super) fn whats_new_frame(&mut self) {
        self.news.hits.clear();
        self.news.shown = None;
        self.news.whole_last = std::mem::take(&mut self.news.whole);
        if self.news.notes.is_none() && !self.state.whats_new.is_closed() {
            self.state.whats_new = crate::state::WhatsNewView::default();
        }
    }

    /// Help › What's new: the card on the start screen while that shows,
    /// else the window.
    pub(super) fn open_whats_new(&mut self) {
        if self.news.notes.is_none() {
            return;
        }
        if self.start_shown() && self.state.presentation.is_full() {
            self.state.whats_new.card = true;
        } else {
            self.state.whats_new.window = true;
        }
    }

    /// The person closed the notes: both places, and the host is told.
    fn close_whats_new(&mut self) {
        self.state.whats_new.card = false;
        self.state.whats_new.window = false;
        self.state.whats_new.expanded = false;
        self.news.closed = true;
    }

    /// The card on the start screen, `width` wide, when it is open: the
    /// title with its ×, then the notes, which scroll in themselves past
    /// a share of the start screen's height. Returns whether it was drawn.
    ///
    /// **Low in a low window** (the coordinator's ruling, 2026-11-15).
    /// Where the window cannot show the card with its notes and the first
    /// row of examples whole (1024 × 700 with a release's notes), the
    /// card starts low: its title, one line that says how many results
    /// may differ, and a click on that line unfolds it
    /// (`state.whatsNew.expanded`). So the examples a person came for
    /// stay in sight. In a window with the room it is the whole card at
    /// once. Once shown whole it stays whole while it is shown, whatever
    /// is unfolded in it.
    pub(super) fn whats_new_card(
        &mut self,
        ui: &mut egui::Ui,
        kit: &Kit,
        width: f32,
        place: CardPlace,
    ) -> bool {
        if !self.state.whats_new.card || !self.state.presentation.is_full() {
            return false;
        }
        let Some(notes) = self.news.notes.clone() else {
            return false;
        };
        // The window would say the same twice.
        self.state.whats_new.window = false;
        let t = self.tokens;
        let s = kit.skin;
        let pad = t.inset + 2.0;
        let most = if place.room.is_finite() {
            (place.room * t.whats_new_share).max(t.whats_new_least)
        } else {
            t.whats_new_least
        };
        let bottom = pad * 0.6;
        let mut close = false;
        let mut unfold = false;
        let mut whole = false;
        let mut hits: Vec<(String, egui::Rect)> = Vec::new();
        let shown = egui::Frame::new()
            .fill(color(s.info_fill))
            .corner_radius(egui::CornerRadius::same(t.notice_radius))
            .inner_margin(egui::Margin {
                left: (pad + t.info_bar_width).clamp(0.0, 64.0) as i8,
                right: (pad * 0.5).clamp(0.0, 64.0) as i8,
                top: (pad * 0.5).clamp(0.0, 64.0) as i8,
                bottom: bottom.clamp(0.0, 64.0) as i8,
            })
            .show(ui, |ui| {
                let inner = (width - 1.5 * pad - t.info_bar_width).max(0.0);
                ui.set_width(inner);
                ui.spacing_mut().item_spacing.y = t.nav_spacing;
                ui.horizontal(|ui| {
                    let title = kit.galley(ui, notes.title(), kit.title_font(ui), s.text);
                    let (place, _) = ui.allocate_exact_size(
                        sane(egui::vec2(title.size().x, t.control_height)),
                        egui::Sense::hover(),
                    );
                    ui.painter().galley(
                        egui::pos2(place.left(), place.center().y - title.size().y * 0.5),
                        title,
                        color(s.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = kit
                            .icon_button(ui, Glyph::Close, false, "Close")
                            .on_hover_text("Close; Help › What's new shows this again");
                        hits.push(("close".to_string(), button.rect));
                        close = button.clicked();
                    });
                });
                let subtitle = notes.subtitle();
                let list_width = (ui.available_width() - pad * 0.5).max(0.0);
                // Whole, or low: whether the notes and the first row of
                // examples under them are wholly in sight.
                whole = self.state.whats_new.expanded || self.news.whole_last || {
                    let said = if subtitle.is_empty() {
                        0.0
                    } else {
                        let job = egui::text::LayoutJob::simple(
                            subtitle.clone(),
                            kit.font(),
                            color(s.text_dim),
                            ui.available_width(),
                        );
                        ui.painter().layout_job(job).size().y + ui.spacing().item_spacing.y
                    };
                    match place.below {
                        // Not measured yet (the first frame): low.
                        None => false,
                        Some(below) => {
                            let from = ui.cursor().top() - place.top;
                            let free = place.sight - from - bottom - below - said;
                            // The notes' own height only where the share
                            // of the height does not fit: they may be
                            // short. Laid out unseen, in a child that is
                            // neither painted nor clicked.
                            free >= most || {
                                let mut probe = ui.new_child(
                                    egui::UiBuilder::new()
                                        .id_salt("craft3d-whats-new-probe")
                                        .max_rect(ui.available_rect_before_wrap())
                                        .invisible(),
                                );
                                probe.spacing_mut().item_spacing.y = t.nav_spacing;
                                probe.set_width(list_width);
                                let mut unseen = Vec::new();
                                self.whats_new_notes(&mut probe, kit, &notes, &mut unseen);
                                free >= probe.min_rect().height().min(most)
                            }
                        }
                    }
                };
                if !whole {
                    let line = self.whats_new_low_line(ui, kit, &notes);
                    hits.push(("summary".to_string(), line.rect));
                    unfold = line.clicked();
                    return;
                }
                if !subtitle.is_empty() {
                    kit.paragraph(ui, &subtitle, true);
                }
                let list = egui::ScrollArea::vertical()
                    .id_salt("craft3d-whats-new-card")
                    .max_height(most)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_width(list_width);
                        self.whats_new_notes(ui, kit, &notes, &mut hits);
                    });
                hits.push(("list".to_string(), list.inner_rect));
            });
        self.news.whole = whole;
        if unfold {
            self.state.whats_new.expanded = true;
            ui.ctx().request_repaint();
        }
        let rect = shown.response.rect;
        let r = t.notice_radius;
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                rect.min,
                egui::pos2(rect.left() + t.info_bar_width, rect.bottom()),
            ),
            egui::CornerRadius {
                nw: r,
                sw: r,
                ne: 0,
                se: 0,
            },
            color(s.info_bar),
        );
        hits.push(("card".to_string(), rect));
        self.news.hits = hits;
        self.news.shown = Some("card");
        if close {
            self.close_whats_new();
        }
        true
    }

    /// The notes in a window over the document (Help › What's new while
    /// one is open), when it is open: not modal, closed by its ×, its
    /// Close or Escape. Only in the whole window: where a page shows the
    /// view or a form alone, the window's own notes are not its business.
    pub(super) fn whats_new_window(&mut self, ctx: &egui::Context) {
        if !self.state.whats_new.window
            || self.news.shown == Some("card")
            || !self.state.presentation.is_full()
        {
            return;
        }
        let Some(notes) = self.news.notes.clone() else {
            return;
        };
        let t = self.tokens;
        let kit = Kit::panel(&t);
        let title = notes.title();
        let most = (ctx.content_rect().height() * t.whats_new_window_share).max(t.whats_new_least);
        let mut close = false;
        let mut hits: Vec<(String, egui::Rect)> = Vec::new();
        let dialog = Dialog::new(egui::Id::new("craft3d-whats-new"), &title)
            .width(t.whats_new_width)
            .modal(false);
        let shown = kit.dialog(ctx, dialog, |kit, ui| {
            let subtitle = notes.subtitle();
            if !subtitle.is_empty() {
                kit.paragraph(ui, &subtitle, true);
            }
            // The scroll bar shows at rest, as on the start screen: it is
            // seen that there is more.
            let scroll = &mut ui.spacing_mut().scroll;
            scroll.dormant_handle_opacity = scroll.active_handle_opacity;
            let list = egui::ScrollArea::vertical()
                .id_salt("craft3d-whats-new-window")
                .max_height(most)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = t.nav_spacing;
                    ui.set_width((ui.available_width() - t.inset).max(0.0));
                    self.whats_new_notes(ui, kit, &notes, &mut hits);
                });
            hits.push(("list".to_string(), list.inner_rect));
            kit.dialog_footer(ui, |ui| {
                kit.footer_right(ui, |ui| {
                    let button = kit.button(ui, Button::secondary("Close"));
                    hits.push(("close".to_string(), button.rect));
                    close = button.clicked();
                });
            });
        });
        hits.push(("window".to_string(), shown.rect));
        self.news.hits = hits;
        self.news.shown = Some("window");
        if close || shown.close {
            self.close_whats_new();
        }
    }

    /// The notes themselves, in the width there is: what changes an
    /// answer, what is new, and the rest under its fold.
    fn whats_new_notes(
        &mut self,
        ui: &mut egui::Ui,
        kit: &Kit,
        notes: &WhatsNew,
        hits: &mut Vec<(String, egui::Rect)>,
    ) {
        let t = self.tokens;
        let s = kit.skin;
        // Several versions: each line says which one it is of.
        let tagged = notes.sections.len() > 1;
        // What a group says of itself in the changelog stands under its
        // heading here: nothing under a group is dropped.
        let said = |ui: &mut egui::Ui, texts: &[&str]| {
            for text in texts {
                kit.paragraph(ui, text, true);
            }
        };
        let results = notes.results();
        let results_said = notes.said(|kind| kind == Kind::Results);
        if !results.is_empty() || !results_said.is_empty() {
            let top = ui.cursor().top();
            heading(
                ui,
                kit,
                Some((Glyph::Warning, s.attention)),
                RESULTS_TITLE,
                results.len(),
            );
            // The changelog's own words for what such an entry means, or
            // ours where it has none.
            if results_said.is_empty() {
                kit.paragraph(ui, RESULTS_HINT, true);
            } else {
                let texts: Vec<&str> = results_said.iter().map(|(_, text)| *text).collect();
                said(ui, &texts);
            }
            for line in &results {
                self.whats_new_line(ui, kit, line, tagged, hits);
            }
            hits.push(("results".to_string(), block(ui, top)));
            ui.add_space(t.nav_spacing * 2.0);
        }
        let added = notes.added();
        let added_said = notes.said(|kind| kind == Kind::Added);
        if !added.is_empty() || !added_said.is_empty() {
            let top = ui.cursor().top();
            heading(ui, kit, None, NEW_TITLE, added.len());
            let texts: Vec<&str> = added_said.iter().map(|(_, text)| *text).collect();
            said(ui, &texts);
            for line in &added {
                self.whats_new_line(ui, kit, line, tagged, hits);
            }
            hits.push(("new".to_string(), block(ui, top)));
            ui.add_space(t.nav_spacing * 2.0);
        }
        let rest = notes.rest();
        let rest_said = notes.said(|kind| !matches!(kind, Kind::Results | Kind::Added));
        if !rest.is_empty() || !rest_said.is_empty() {
            let top = ui.cursor().top();
            let mut open = self.state.whats_new.rest;
            kit.fold_header(ui, &mut open, REST_TITLE, Some(rest.len()), None);
            self.state.whats_new.rest = open;
            if open {
                let of = |heading: &str| -> Vec<&str> {
                    rest_said
                        .iter()
                        .filter(|(group, _)| *group == heading)
                        .map(|(_, text)| *text)
                        .collect()
                };
                let mut shown: Vec<&str> = Vec::new();
                for line in &rest {
                    let (_, _, entry) = line;
                    if shown.last() != Some(&entry.heading.as_str()) {
                        shown.push(entry.heading.as_str());
                        kit.caption(ui, &entry.heading);
                        said(ui, &of(&entry.heading));
                    }
                    self.whats_new_line(ui, kit, line, tagged, hits);
                }
                // A group that only says something, without an entry.
                let mut alone: Vec<&str> = Vec::new();
                for (group, text) in &rest_said {
                    if !shown.contains(group) {
                        if alone.last() != Some(group) {
                            alone.push(group);
                            kit.caption(ui, group);
                        }
                        said(ui, &[*text]);
                    }
                }
            }
            hits.push(("rest".to_string(), block(ui, top)));
        }
    }

    /// The card's one line while it is low: how many results may differ,
    /// and how much else there is; a click on it unfolds the card.
    fn whats_new_low_line(&self, ui: &mut egui::Ui, kit: &Kit, notes: &WhatsNew) -> egui::Response {
        let t = self.tokens;
        let s = kit.skin;
        let (lead, tail) = notes.low_line();
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(
            sane(egui::vec2(width, kit.hit(t.row_height))),
            egui::Sense::click(),
        );
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if response.hovered() {
                painter.rect_filled(rect, kit.radius(), color(s.hover));
            }
            let side = t.nav_icon_size * 0.85;
            let mut x = rect.left() + 2.0;
            if !lead.is_empty() {
                glyphs::paint(
                    painter,
                    Glyph::Warning,
                    centred_left(
                        egui::Rect::from_min_max(egui::pos2(x, rect.top()), rect.max),
                        egui::vec2(side, side),
                    ),
                    color(s.attention),
                    &t,
                );
                x += side + t.nav_icon_gap;
            }
            // At the right: Show, with the chevron that unfolds.
            let show = kit.galley(ui, LOW_SHOW, kit.font(), s.info_ink);
            let chevron = t.nav_icon_size * 0.75;
            let right = rect.right() - t.nav_icon_gap;
            glyphs::paint(
                painter,
                Glyph::ChevronDown,
                egui::Rect::from_center_size(
                    egui::pos2(right - chevron * 0.5, rect.center().y),
                    egui::vec2(chevron, chevron),
                ),
                color(s.info_ink),
                &t,
            );
            let show_left = right - chevron - t.nav_icon_gap - show.size().x;
            painter.galley(
                egui::pos2(show_left, rect.center().y - show.size().y * 0.5),
                show,
                color(s.info_ink),
            );
            // The line itself, cut where Show begins.
            let mut job = egui::text::LayoutJob::default();
            job.append(
                &lead,
                0.0,
                egui::TextFormat::simple(kit.medium_font(ui), color(s.text)),
            );
            job.append(
                &tail,
                0.0,
                egui::TextFormat::simple(kit.font(), color(s.text_dim)),
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(
                (show_left - x - t.nav_icon_gap).max(1.0),
            );
            let line = painter.layout_job(job);
            painter.galley(
                egui::pos2(x, rect.center().y - line.size().y * 0.5),
                line,
                color(s.text),
            );
        }
        let label = format!("{lead}{tail}");
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, false, &label)
        });
        response.on_hover_text("Show what is new and which results may differ")
    }

    /// One entry: its title, with a chevron when there is more to read
    /// under it; a click unfolds what the changelog says.
    fn whats_new_line(
        &mut self,
        ui: &mut egui::Ui,
        kit: &Kit,
        line: &Line<'_>,
        tagged: bool,
        hits: &mut Vec<(String, egui::Rect)>,
    ) {
        let (key, section, entry) = *line;
        let t = self.tokens;
        let s = kit.skin;
        let side = t.nav_icon_size * 0.75;
        let indent = side + t.nav_icon_gap + 2.0;
        let width = ui.available_width();
        let room = (width - indent).max(1.0);
        let more = !entry.text.is_empty();
        let open = more && self.news.unfolded.contains(&key);
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = room;
        job.append(
            &entry.title,
            0.0,
            egui::TextFormat::simple(kit.font(), color(s.text)),
        );
        if tagged {
            let tag = match section.version {
                Some(version) => format!("  {version}"),
                None => "  not released yet".to_string(),
            };
            job.append(
                &tag,
                0.0,
                egui::TextFormat::simple(kit.small_font(), color(s.text_dim)),
            );
        }
        let galley = ui.painter().layout_job(job);
        let line_height = ui.fonts_mut(|f| f.row_height(&kit.font()));
        let height = galley.size().y.max(line_height) + t.nav_spacing;
        let sense = if more {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(sane(egui::vec2(width, height)), sense);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if more && response.hovered() {
                painter.rect_filled(rect, kit.radius(), color(s.hover));
            }
            // The mark stands on the title's first line.
            let first = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.top() + t.nav_spacing * 0.5),
                egui::vec2(side, line_height),
            );
            if more {
                let chevron = if open {
                    Glyph::ChevronDown
                } else {
                    Glyph::ChevronRight
                };
                glyphs::paint(
                    painter,
                    chevron,
                    centred_left(first, egui::vec2(side, side)),
                    color(s.text_dim),
                    &t,
                );
            } else {
                painter.circle_filled(first.center(), t.info_bar_width * 0.5, color(s.text_dim));
            }
            painter.galley(
                egui::pos2(rect.left() + indent, rect.top() + t.nav_spacing * 0.5),
                galley,
                color(s.text),
            );
        }
        let (title, state) = (entry.title.clone(), open);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, state, &title)
        });
        if more && response.clicked() {
            if open {
                self.news.unfolded.remove(&key);
            } else {
                self.news.unfolded.insert(key);
            }
        }
        hits.push((format!("entry:{}:{}", key.0, key.1), rect));
        if open {
            ui.horizontal(|ui| {
                ui.add_space(indent);
                ui.vertical(|ui| {
                    ui.set_width(room);
                    kit.paragraph(ui, &entry.text, true);
                });
            });
        }
    }
}

/// A group's heading in the notes: an icon when it has one, the title in
/// the medium weight and how many entries follow.
fn heading(
    ui: &mut egui::Ui,
    kit: &Kit,
    icon: Option<(Glyph, crate::theme::Rgb)>,
    title: &str,
    count: usize,
) {
    let t = &kit.t;
    let s = &kit.skin;
    let title = kit.galley(ui, title, kit.medium_font(ui), s.text);
    let count = kit.galley(ui, count.to_string(), kit.font(), s.text_dim);
    let side = t.nav_icon_size * 0.85;
    let lead = if icon.is_some() {
        side + t.nav_icon_gap
    } else {
        0.0
    };
    let width = lead + title.size().x + t.nav_icon_gap + count.size().x;
    let (rect, _) = ui.allocate_exact_size(
        sane(egui::vec2(width, t.label_row_height)),
        egui::Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    if let Some((glyph, ink)) = icon {
        glyphs::paint(
            painter,
            glyph,
            centred_left(rect, egui::vec2(side, side)),
            color(ink),
            t,
        );
    }
    let x = rect.left() + lead;
    let wide = title.size().x;
    painter.galley(
        egui::pos2(x, rect.center().y - title.size().y * 0.5),
        title,
        color(s.text),
    );
    painter.galley(
        egui::pos2(
            x + wide + t.nav_icon_gap,
            rect.center().y - count.size().y * 0.5,
        ),
        count,
        color(s.text_dim),
    );
}

/// What was drawn from `top` down to the cursor, across the width.
fn block(ui: &egui::Ui, top: f32) -> egui::Rect {
    let across = ui.min_rect();
    egui::Rect::from_min_max(
        egui::pos2(across.left(), top),
        egui::pos2(across.right(), ui.cursor().top().max(top)),
    )
}
