//! The side panel (docs/design/rendering.md › *The side panel*; plan of
//! 2026-11-02, `plan/mockups/sidepanel`): tabs chosen by the document's
//! kind, each drawn with the component kit.
//!
//! - a part: Features ([`features`], with the checks under the list,
//!   [`checks`], and what is selected at the foot, [`props`]), Variables
//!   ([`variables`]) and Bodies ([`bodies`]);
//! - an assembly: Assembly (the instances and mates, as before) and
//!   Variables;
//! - a document with public parameters (a Jig) and an assembly with
//!   joints to set: Parameters first.
//!
//! The Agent tab comes with the agent's log (r1-mcp-connect); its place is
//! after Bodies ([`tabs_for`]).
//!
//! Like the rest of the shell the panel only asks: what it changes it
//! queues as commands ([`Shell::side_queue`]), run when the session is
//! here ([`Shell::apply_side_requests`]): `feature.edit`, `variable.set`,
//! `feature.move`, `history.rollback`, ... each one undo step. What it
//! shows it reads from the session after each change of the document
//! ([`Shell::sync_side`]): the history from the document alone
//! (`Session::history_view`, nothing regenerated), the bodies' measures
//! (`document.inspect`) only while the Bodies tab shows, and what is
//! selected (`select.get`, `measure.continuity`, `document.measure`)
//! only while something is.

pub mod agent;
pub mod bodies;
pub mod checks;
pub mod features;
pub mod props;
pub mod variables;

use super::Shell;
use crate::kit::{Glyph, Ground, Kit, MenuItem, Tab};
use crate::state::SideTab;
use craft3d_engine::{HistoryView, Session};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

pub use bodies::{BodyLine, BodyTotals};
pub use features::{Dependencies, SideLine, Summary};
pub use props::{PropRow, Props};

/// The document and revision a read of the panel was for.
pub(crate) type ViewKey = Option<(u64, u64)>;

/// Most commands the panel keeps waiting for the session.
const MAX_PENDING: usize = 64;
/// Most errors kept at fields.
const MAX_ERRORS: usize = 64;

/// A command the panel asked for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SideRequest {
    pub(crate) command: &'static str,
    pub(crate) params: Value,
    /// The field its error is shown at (`var:width`, `value:F3`); `None`
    /// for one whose error goes to the panel's line.
    pub(crate) key: Option<String>,
}

/// What is being typed into a field of the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Editing {
    /// A feature's key value (`feature.edit` of its parameter).
    Value { id: String, text: String },
    /// A feature's name (`feature.rename`).
    Name { id: String, text: String },
    /// A variable's expression (`variable.set`).
    Variable { name: String, text: String },
    /// A variable not made yet: Enter makes it with its expression, one
    /// undo step (`variable.set`).
    NewVariable { name: String, text: String },
}

/// A drag in the feature list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Drag {
    /// A row by its grip, to another place in the history (`feature.move`).
    Row { id: String },
    /// The roll-back bar by its grip (`history.rollback`).
    Rollback,
}

/// Where the panel drew what can be hit, in the last frame (for tests and
/// for the pictures that check it fits).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SideHits {
    /// The panel's own rectangle.
    pub panel: Option<egui::Rect>,
    /// The tabs, and the ⋯ that holds those that do not fit.
    pub tabs: Vec<(SideTab, egui::Rect)>,
    pub more: Option<egui::Rect>,
    /// The rows of the list shown (features, variables, bodies), by id or
    /// name.
    pub rows: Vec<(String, egui::Rect)>,
    /// The key values that a click edits, by feature id.
    pub values: Vec<(String, egui::Rect)>,
    /// The eyes, by feature id.
    pub eyes: Vec<(String, egui::Rect)>,
    /// The fold chevrons, by row key.
    pub folds: Vec<(String, egui::Rect)>,
    /// Buttons and other targets, by what they do (`show where:F18`,
    /// `checks`, `measure`, `copy name`, `rollback`, `filter`, `menu:Delete`).
    pub targets: Vec<(String, egui::Rect)>,
    /// The checks' rows, by name.
    pub checks: Vec<(String, egui::Rect)>,
    /// The Agent tab's entries, newest first.
    pub agent_rows: Vec<egui::Rect>,
    /// The statuses after the mates' and the checks' names (the
    /// assembly's tree), each kept in room of its own.
    pub statuses: Vec<egui::Rect>,
    /// Everything drawn that must lie inside the panel: the rows' texts'
    /// places included.
    pub drawn: Vec<egui::Rect>,
}

/// What the panel remembers between frames (its view state is in
/// `UiState::side_panel` and `panels.sideTab`).
#[derive(Default)]
pub(crate) struct SideMemory {
    /// The active document's history, and for which document and
    /// revision.
    pub(crate) view: Option<Arc<HistoryView>>,
    pub(crate) view_for: Option<(u64, u64)>,
    /// What the features are made from, indexed with the view.
    pub(crate) graph: Arc<features::Graph>,
    /// The hovered feature's dependencies, for the view they were read in.
    pub(crate) deps_cache: Option<(ViewKey, String, Dependencies)>,
    /// The lines of the Features tab, and what they were made from.
    pub(crate) outline_cache: Option<(features::OutlineKey, Arc<Vec<SideLine>>)>,
    /// `agent.log` of the active document (`Session::agent_log_json`),
    /// and for which document, revision and latest agent command.
    pub(crate) agent_log: Option<Arc<Value>>,
    agent_log_for: Option<(u64, u64, u64)>,
    /// `document.inspect` with measures, for the Bodies tab.
    pub(crate) bodies: Option<Arc<Value>>,
    bodies_for: Option<(u64, u64)>,
    /// The material presets (`material.presets`).
    pub(crate) presets: Vec<bodies::Preset>,
    /// `assert.list`, for where a check flies to.
    pub(crate) checks_list: Option<Arc<Value>>,
    checks_for: Option<(u64, u64)>,
    /// What is selected, described, and for which selection.
    pub(crate) props: Option<Props>,
    props_for: Option<(u64, u64, u64)>,
    /// Commands waiting for the session.
    pending: Vec<SideRequest>,
    /// Errors at fields, by their key.
    pub(crate) errors: BTreeMap<String, String>,
    /// Why the last command the panel asked for failed, when no field
    /// holds it.
    pub(crate) problem: Option<String>,
    /// The last measure's answer, in a line.
    pub(crate) measured: Option<String>,
    pub(crate) editing: Option<Editing>,
    /// The field just opened takes the keyboard in its first frame.
    pub(crate) focus_pending: bool,
    /// The history step the list last scrolled to while the view is
    /// scrubbed: it scrolls again only when the step shown changes, so a
    /// list scrolled by hand stays.
    pub(crate) scrub_scrolled: Option<usize>,
    pub(crate) drag: Option<Drag>,
    /// The checks were opened by hand while they would fold for room:
    /// they stay open until the selection changes.
    pub(crate) checks_asked: bool,
    /// The hover was set by the pointer (and goes when it leaves), not by
    /// `ui.set` or the menu's Show dependencies (which stay until Esc).
    pub(crate) hover_by_pointer: bool,
    /// Where the context menu opens.
    pub(crate) menu_at: Option<egui::Pos2>,
    /// The ⋯ menu of the tabs is open.
    pub(crate) more_open: bool,
    /// The tab as of the last frame, to tell a tab set from outside from
    /// the Parameters tab's own memory (`state.parameters.open`).
    last_tab: Option<SideTab>,
    pub(crate) hits: SideHits,
}

/// The tabs a document offers, in order: Parameters first for a Jig (or
/// an assembly whose joints can be set), then the history (Features, or
/// Assembly for an assembly), Variables, and Bodies for a part.
pub fn tabs_for(assembly: bool, parameters: bool, agent: bool) -> Vec<SideTab> {
    let mut tabs = Vec::new();
    if parameters {
        tabs.push(SideTab::Parameters);
    }
    if assembly {
        tabs.push(SideTab::Assembly);
        tabs.push(SideTab::Variables);
    } else {
        tabs.extend([SideTab::Features, SideTab::Variables, SideTab::Bodies]);
    }
    // What agents did, while agents are allowed or the document has a
    // log (`sidepanel/agent.rs`; not in the browser).
    if agent {
        tabs.push(SideTab::Agent);
    }
    tabs
}

/// The tab shown of `offered` for `wanted`: itself, else Features for
/// Assembly and the other way round, else the first.
pub fn tab_shown(wanted: SideTab, offered: &[SideTab]) -> SideTab {
    if offered.contains(&wanted) {
        return wanted;
    }
    let twin = match wanted {
        SideTab::Features => SideTab::Assembly,
        SideTab::Assembly => SideTab::Features,
        other => other,
    };
    if offered.contains(&twin) {
        return twin;
    }
    offered.first().copied().unwrap_or(SideTab::Features)
}

impl Shell {
    /// What the panel drew where, in the last frame.
    pub fn side_hits(&self) -> &SideHits {
        &self.side.hits
    }

    /// The active document's history as the panel last read it.
    pub fn side_view(&self) -> Option<&HistoryView> {
        self.side.view.as_deref()
    }

    /// The errors shown at the panel's fields, by field
    /// (`var:NAME`, `value:F3`, `name:F3`), and the panel's own line.
    pub fn side_errors(&self) -> (&BTreeMap<String, String>, Option<&str>) {
        (&self.side.errors, self.side.problem.as_deref())
    }

    /// Queues `command` for the session; its error goes to the field
    /// `key`, or to the panel's line.
    pub(crate) fn side_queue(&mut self, command: &'static str, params: Value, key: Option<String>) {
        if self.side.pending.len() >= MAX_PENDING {
            self.side.problem = Some(format!(
                "{command} not run: {MAX_PENDING} requests are waiting for the document"
            ));
            return;
        }
        if let Some(key) = &key {
            self.side.errors.remove(key);
        }
        self.side.pending.push(SideRequest {
            command,
            params,
            key,
        });
    }

    /// Whether the tab shown wants what was not read for the document as
    /// it is (the Bodies tab chosen since the last sync).
    pub(crate) fn side_stale(&self) -> bool {
        self.state.panels.side_tab == SideTab::Bodies
            && self.side.view_for.is_some()
            && self.side.bodies_for != self.side.view_for
    }

    /// Whether the panel has commands waiting.
    pub fn side_waiting(&self) -> bool {
        !self.side.pending.is_empty()
    }

    /// Runs what the panel asked for, in order. True when there was
    /// anything; the caller syncs.
    pub(crate) fn apply_side_requests(&mut self, session: &mut Session) -> bool {
        let pending = std::mem::take(&mut self.side.pending);
        for request in &pending {
            match session.execute(request.command, &request.params) {
                Ok(result) => {
                    self.side.problem = None;
                    if request.command.starts_with("measure.") {
                        self.side.measured = Some(props::measured_line(request.command, &result));
                    }
                }
                Err(e) => {
                    let why = e.message().to_string();
                    match &request.key {
                        Some(key) if self.side.errors.len() < MAX_ERRORS => {
                            self.side.errors.insert(key.clone(), why);
                        }
                        _ => self.side.problem = Some(why),
                    }
                }
            }
        }
        !pending.is_empty()
    }

    /// Reads what the panel shows from the session, when what it was read
    /// from changed: the history at every revision of the document, the
    /// bodies' measures while the Bodies tab shows, the selection's
    /// properties while something is selected.
    pub(crate) fn sync_side(
        &mut self,
        session: &mut Session,
        scene: Option<&craft3d_engine::Scene>,
    ) {
        let Some(doc) = session.document_status().map(|s| s.id) else {
            self.side.view = None;
            self.side.view_for = None;
            self.side.bodies = None;
            self.side.props = None;
            self.side.checks_list = None;
            self.side.agent_log = None;
            self.side.agent_log_for = None;
            return;
        };
        let revision = session.document_revision().unwrap_or(0);
        let key = (doc, revision);
        if self.side.view_for != Some(key) {
            self.side.view = session.history_view().map(Arc::new);
            self.side.graph = Arc::new(
                self.side
                    .view
                    .as_ref()
                    .map(|v| features::Graph::of(&v.features))
                    .unwrap_or_default(),
            );
            self.side.view_for = Some(key);
            self.side.deps_cache = None;
            // Errors were about the document as it was.
            self.side.errors.clear();
            // An edit in a field of another document's is let go of.
            if self.side.editing.as_ref().is_some_and(|e| match e {
                Editing::Value { id, .. } | Editing::Name { id, .. } => !self
                    .side
                    .view
                    .as_ref()
                    .is_some_and(|v| v.features.iter().any(|f| f.id == *id)),
                Editing::Variable { name, .. } => !self
                    .side
                    .view
                    .as_ref()
                    .is_some_and(|v| v.variables.iter().any(|x| x.name == *name)),
                Editing::NewVariable { .. } => false,
            }) {
                self.side.editing = None;
            }
        }
        // Measures need the bodies: not read here while they are built
        // elsewhere and not here (the browser's worker), where reading
        // them would build them all again on the page's one thread.
        let bodies_here = !session.regen_elsewhere() || session.regenerated_here();
        let tab = self.state.panels.side_tab;
        if tab == SideTab::Bodies && !bodies_here && self.side.bodies_for != Some(key) {
            // Not read here (the worker builds the bodies): read for this
            // revision all the same, so the tab does not ask again every
            // frame; the tab says where they are measured.
            self.side.bodies = None;
            self.side.bodies_for = Some(key);
        }
        if tab == SideTab::Bodies && bodies_here && self.side.bodies_for != Some(key) {
            // Without the inertia, which nothing here shows and which takes
            // seconds on a large spline body after every edit or undo.
            self.side.bodies = match session.execute(
                "document.inspect",
                &json!({"measures": true, "inertia": false}),
            ) {
                Ok(v) => Some(Arc::new(v)),
                Err(e) => {
                    // Said on the tab's line, not taken for "elsewhere".
                    self.side.problem = Some(e.message().to_string());
                    None
                }
            };
            self.side.bodies_for = Some(key);
            if self.side.presets.is_empty() {
                self.side.presets = session
                    .execute("material.presets", &json!({}))
                    .ok()
                    .map(|v| bodies::presets_of(&v))
                    .unwrap_or_default();
            }
        }
        // What agents did: read again when the document changes or an
        // agent ran a command (its client's last time moves).
        if !cfg!(target_arch = "wasm32") {
            let latest = self
                .agents_status()
                .clients
                .iter()
                .map(|c| c.last_ms)
                .max()
                .unwrap_or(0);
            let agent_key = (doc, revision, latest);
            if self.side.agent_log_for != Some(agent_key) {
                self.side.agent_log = Some(Arc::new(session.agent_log_json(agent::LOG_ROWS)));
                self.side.agent_log_for = Some(agent_key);
            }
        }
        if !self.assertion_rows.is_empty() && self.side.checks_for != Some(key) {
            self.side.checks_list = session
                .execute("assert.list", &json!({"wait": false}))
                .ok()
                .map(Arc::new);
            self.side.checks_for = Some(key);
        }
        let serial = session.selection_serial();
        let props_key = (doc, revision, serial);
        if self.side.props_for != Some(props_key) {
            self.side.props_for = Some(props_key);
            self.side.measured = None;
            self.side.checks_asked = false;
            self.side.props = if self.selection().items.is_empty() {
                None
            } else {
                props::read(session, scene, bodies_here)
            };
        }
    }

    /// The side panel: the tab bar, then the tab shown. `jig`: the
    /// document has public parameters; `joints`: an assembly with joints
    /// to set.
    pub(super) fn side_tabs(
        &mut self,
        ui: &mut egui::Ui,
        has_document: bool,
        jig: bool,
        joints: bool,
    ) {
        let tokens = self.tokens;
        self.side.hits = SideHits {
            panel: Some(ui.max_rect()),
            ..SideHits::default()
        };
        if !has_document {
            self.parameters_hidden();
            ui.add_space(tokens.inset * 0.5);
            self.tree(ui, false);
            return;
        }
        let assembly = self.is_assembly_shown();
        let offered = tabs_for(assembly, jig || joints, self.agent_tab_offered());
        let tab = self.reconcile_tab(&offered, jig || joints);
        let chosen = self.tab_bar(ui, &offered, tab);
        if chosen != tab {
            self.choose_tab(chosen, jig || joints);
        }
        let tab = self.state.panels.side_tab;
        ui.add_space(tokens.nav_gap);
        if tab != SideTab::Parameters {
            self.parameters_hidden();
        }
        match tab {
            SideTab::Parameters => {
                if joints {
                    self.joints_ui(ui);
                }
                if jig {
                    self.parameters_tab(ui);
                }
            }
            SideTab::Assembly => self.tree(ui, true),
            SideTab::Features => self.features_tab(ui),
            SideTab::Variables => self.variables_tab(ui),
            SideTab::Bodies => self.bodies_tab(ui),
            SideTab::Agent => self.agent_tab(ui),
        }
    }

    /// The tab to show: one set from outside (`ui.set`) or by a click
    /// wins; else the Parameters tab follows `state.parameters.open`,
    /// which the window keeps per document (a Jig opens on it).
    fn reconcile_tab(&mut self, offered: &[SideTab], parameters: bool) -> SideTab {
        let mut tab = self.state.panels.side_tab;
        // The first frame: nothing was set from outside yet.
        if self.side.last_tab.is_none() {
            self.side.last_tab = Some(tab);
        }
        if self.side.last_tab != Some(tab) {
            // Set from outside: the Parameters tab's memory follows.
            if parameters {
                self.state.parameters.open = tab == SideTab::Parameters;
            }
        } else if parameters && self.state.parameters.open && tab != SideTab::Parameters {
            tab = SideTab::Parameters;
        } else if tab == SideTab::Parameters && !self.state.parameters.open {
            tab = SideTab::Features;
        }
        let tab = tab_shown(tab, offered);
        self.state.panels.side_tab = tab;
        self.side.last_tab = Some(tab);
        tab
    }

    fn choose_tab(&mut self, tab: SideTab, parameters: bool) {
        self.state.panels.side_tab = tab;
        self.side.last_tab = Some(tab);
        if parameters {
            self.state.parameters.open = tab == SideTab::Parameters;
        }
        self.side.more_open = false;
    }

    /// The tab bar: the tabs that fit, the others behind ⋯ (the one shown
    /// is always among those that show). Returns the tab chosen.
    fn tab_bar(&mut self, ui: &mut egui::Ui, offered: &[SideTab], tab: SideTab) -> SideTab {
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let width = ui.available_width();
        let font = kit.medium_font(ui);
        let widths: Vec<f32> = offered
            .iter()
            .map(|tab| {
                let w = ui
                    .painter()
                    .layout_no_wrap(tab.label().to_string(), font.clone(), egui::Color32::BLACK)
                    .size()
                    .x;
                (w + 2.0 * t.field_inset).max(t.hit_min)
            })
            .collect();
        let more_w = kit.hit(t.control_height) + t.nav_spacing * 2.0;
        let total: f32 = widths.iter().sum();
        let mut shown: Vec<SideTab> = offered.to_vec();
        let mut hidden: Vec<SideTab> = Vec::new();
        if total > width {
            shown.clear();
            let mut used = more_w;
            for (tab, w) in offered.iter().zip(&widths) {
                if used + w <= width {
                    shown.push(*tab);
                    used += w;
                } else {
                    hidden.push(*tab);
                }
            }
            // The one shown is always a tab of the bar.
            if let Some(at) = hidden.iter().position(|h| *h == tab) {
                hidden.remove(at);
                if let Some(last) = shown.pop() {
                    hidden.insert(0, last);
                }
                shown.push(tab);
            }
        }
        let mut active = shown.iter().position(|s| *s == tab).unwrap_or(0);
        let labels: Vec<Tab<'_>> = shown.iter().map(|tab| Tab::new(tab.label())).collect();
        let mut chosen = tab;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let bar_w = if hidden.is_empty() {
                width
            } else {
                (width - more_w).max(0.0)
            };
            let bar = ui
                .allocate_ui(egui::vec2(bar_w, kit.hit(t.panel_tab_height)), |ui| {
                    kit.tab_bar(ui, &mut active, &labels)
                })
                .inner;
            for (tab, rect) in shown.iter().zip(&bar.items) {
                self.side.hits.tabs.push((*tab, *rect));
            }
            if bar.response.changed() {
                chosen = shown.get(active).copied().unwrap_or(tab);
            }
            if !hidden.is_empty() {
                let more = kit.icon_button(ui, Glyph::More, self.side.more_open, "More tabs");
                self.side.hits.more = Some(more.rect);
                if more.clicked() {
                    self.side.more_open = !self.side.more_open;
                }
                if self.side.more_open {
                    let plate = Kit::new(&self.tokens, Ground::Plate);
                    let area = egui::Area::new(egui::Id::new("craft3d-side-more"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(more.rect.left_bottom())
                        .constrain(true)
                        .show(ui.ctx(), |ui| {
                            plate.menu_frame(ui, t.menu_width * 0.7, |ui| {
                                for tab in &hidden {
                                    let item = plate.menu_item(ui, MenuItem::new(tab.label()));
                                    self.side.hits.tabs.push((*tab, item.rect));
                                    if item.clicked() {
                                        chosen = *tab;
                                    }
                                }
                            })
                        });
                    let elsewhere = ui.input(|i| i.pointer.any_pressed())
                        && !area.response.contains_pointer()
                        && !more.contains_pointer();
                    if elsewhere || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.side.more_open = false;
                    }
                }
            }
        });
        chosen
    }

    /// A quiet line at the top of a tab, under its filter, when the last
    /// command the panel asked for failed without a field to show it at.
    pub(super) fn side_problem(&mut self, ui: &mut egui::Ui) {
        let Some(problem) = self.side.problem.clone() else {
            return;
        };
        let kit = Kit::panel(&self.tokens);
        let text = egui::RichText::new(problem)
            .color(kit.c(kit.skin.failed))
            .size(kit.t.secondary_font - 1.0);
        let label = ui.add(egui::Label::new(text).wrap().sense(egui::Sense::click()));
        self.side.hits.drawn.push(label.rect);
        if label.clicked() {
            self.side.problem = None;
        }
    }

    /// The panel's search field, `hint` while empty.
    pub(super) fn side_filter(&mut self, ui: &mut egui::Ui, which: Filter, hint: &str) {
        let kit = Kit::panel(&self.tokens);
        let mut text = match which {
            Filter::Features => self.state.side_panel.filter.clone(),
            Filter::Variables => self.state.side_panel.variable_filter.clone(),
            Filter::Bodies => self.state.side_panel.body_filter.clone(),
        };
        let width = ui.available_width();
        let field = kit.text_field(
            ui,
            ("craft3d-side-filter", which),
            &mut text,
            hint,
            Some(Glyph::Search),
            width,
        );
        self.side
            .hits
            .targets
            .push(("filter".to_string(), field.rect));
        if field.changed() {
            let text: String = text.chars().take(crate::state::MAX_FILTER_CHARS).collect();
            match which {
                Filter::Features => self.state.side_panel.filter = text,
                Filter::Variables => self.state.side_panel.variable_filter = text,
                Filter::Bodies => self.state.side_panel.body_filter = text,
            }
        }
    }
}

/// Which list a search field filters.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) enum Filter {
    Features,
    Variables,
    Bodies,
}

/// Whether `text` holds `filter`, ignoring case (an empty filter holds
/// everywhere).
pub fn matches(text: &str, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty() || text.to_lowercase().contains(&filter.to_lowercase())
}

/// A number with its thousands apart by a thin space, as the panel
/// writes measures (`31 858`), with `decimals` after the point.
pub fn grouped(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return "?".to_string();
    }
    let decimals = decimals.min(6);
    let text = format!("{:.*}", decimals, v.abs());
    let (whole, fraction) = match text.split_once('.') {
        Some((w, f)) => (w.to_string(), Some(f.to_string())),
        None => (text.clone(), None),
    };
    let mut out = String::new();
    let digits: Vec<char> = whole.chars().collect();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) && digits.len() > 4 {
            out.push('\u{2009}');
        }
        out.push(*c);
    }
    if let Some(f) = fraction {
        out.push('.');
        out.push_str(&f);
    }
    let negative = v < 0.0 && out.chars().any(|c| c.is_ascii_digit() && c != '0');
    if negative { format!("−{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_grouped_by_thousands() {
        assert_eq!(grouped(31858.4, 0), "31\u{2009}858");
        assert_eq!(grouped(6412.0, 0), "6412");
        assert_eq!(grouped(112.123, 2), "112.12");
        assert_eq!(grouped(-1234567.0, 0), "−1\u{2009}234\u{2009}567");
        assert_eq!(grouped(f64::NAN, 1), "?");
    }

    #[test]
    fn tabs_follow_the_document() {
        use SideTab::*;
        assert_eq!(
            tabs_for(false, false, false),
            vec![Features, Variables, Bodies]
        );
        assert_eq!(
            tabs_for(false, false, true),
            vec![Features, Variables, Bodies, SideTab::Agent]
        );
        assert_eq!(
            tabs_for(false, true, false),
            vec![Parameters, Features, Variables, Bodies]
        );
        assert_eq!(tabs_for(true, false, false), vec![Assembly, Variables]);
        assert_eq!(
            tab_shown(Assembly, &tabs_for(false, false, false)),
            Features
        );
        assert_eq!(tab_shown(Bodies, &tabs_for(true, false, false)), Assembly);
        assert_eq!(
            tab_shown(Parameters, &tabs_for(false, false, false)),
            Features
        );
    }
}
