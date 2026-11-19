//! The side panel (docs/design/rendering.md › *The side panel*; mock-ups
//! approved 2026-11-02): tabs chosen by the document's
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
//! only while something is. Beside a host that builds the bodies
//! elsewhere (the browser's worker) what is selected is measured there
//! (`props::CARD_JOB`, its answers taken by [`Shell::card_answered`]),
//! never on the host's one thread.

pub mod agent;
pub mod bodies;
pub mod checks;
pub mod features;
pub mod notes;
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

/// The card of what is selected a read was for: the document, its
/// revision and the selection's serial.
type CardKey = (u64, u64, u64);

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
    /// The rows cut with "…" to the panel, by id (feature id, variable or
    /// body), with what their hover says: the whole text first.
    pub cut: Vec<(String, String)>,
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
    pub(crate) props_for: Option<(u64, u64, u64)>,
    /// The selection whose relation was kept as a check last, by its
    /// document and selection serial ([`SideMemory::kept_key`]), and the
    /// check's name.
    pub(crate) kept: Option<((u64, u64), String)>,
    /// The relation of two things selected being measured on a thread of
    /// its own, for the card of `props_for` it was asked for; and the one
    /// waiting for it to finish (the newest asked, replacing any before):
    /// one runs, one waits, an answer for an older card is dropped (the
    /// review of 2026-11-17: a quick run of selections ran them side by
    /// side).
    pub(crate) between: Option<((u64, u64, u64), craft3d_engine::BetweenJob)>,
    /// Where the Features tab's foot (the checks and the card) began in
    /// the last frame: the list's room ends there.
    pub(crate) foot_top: Option<f32>,
    waiting: Option<((u64, u64, u64), Value)>,
    /// Beside a host that builds the bodies elsewhere (the browser's
    /// worker), what the card asked there (`props::CARD_JOB`), each for
    /// the card of `props_for` it was asked for, by its job's number: the
    /// facts of the first thing, the relation of two (one at a time, as
    /// [`SideMemory::between`]), the wall; and what came of the first two,
    /// taken at the next sync (an answer for another card is dropped).
    facts_asked: Option<(CardKey, u64)>,
    facts_came: Option<(CardKey, Result<props::Facts, String>)>,
    /// The facts' job, while the host takes no more jobs for now.
    facts_later: Option<(CardKey, Value)>,
    between_asked: Option<(CardKey, u64)>,
    between_came: Option<(CardKey, props::Away)>,
    /// The wall's card and job, whether it is a body's, and the tolerance
    /// its answer is written to.
    wall_asked: Option<(CardKey, u64, bool, f64)>,
    /// Every job the card handed over whose answer has not come: an
    /// answer for an earlier card is the card's to drop, never the host's
    /// to report.
    card_jobs: std::collections::BTreeSet<u64>,
    /// A check kept is being made where the bodies are (an edit handed
    /// over waits): *Keep as a check* says so until it is back.
    pub(crate) keeping: bool,
    /// Commands waiting for the session.
    pending: Vec<SideRequest>,
    /// Errors at fields, by their key.
    pub(crate) errors: BTreeMap<String, String>,
    /// Why the last command the panel asked for failed, when no field
    /// holds it.
    pub(crate) problem: Option<String>,
    /// The wall measured for the card (*Wall thickness*): it heads the
    /// card until the selection or the model changes.
    pub(crate) wall: Option<props::Wall>,
    /// A number of the card was copied: its hover says so for a moment;
    /// and *Copy name* says *Copied*.
    pub(crate) copied: Option<crate::shell::hints::Timed>,
    pub(crate) name_copied: Option<crate::shell::hints::Timed>,
    /// The check just kept: its row in the Checks is tinted for a moment
    /// and scrolled into sight once (its name, the time, scrolled).
    pub(crate) kept_tint: Option<(String, crate::shell::hints::Timed, bool)>,
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
    /// How high the selection's card under a tab drew its content in the
    /// last frame: its height next.
    card_content: Option<f32>,
}

impl SideMemory {
    /// The document and the selection the card is of, without the
    /// document's revision: what a relation kept as a check is kept for.
    pub(crate) fn kept_key(&self) -> Option<(u64, u64)> {
        self.props_for.map(|(doc, _, serial)| (doc, serial))
    }
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
        let tolerance = session.linear_tolerance().unwrap_or(1e-6);
        let wall_of_body = self.side.props.as_ref().is_some_and(|p| p.kind == "Body");
        // The bodies are built elsewhere and are not here (the browser's
        // worker): what needs them is made there, not here on the host's
        // one thread.
        let away = session.regen_elsewhere() && !session.regenerated_here();
        for request in &pending {
            // Wall thickness, measured where the bodies are: its answer
            // heads the card when it comes, for the card it was asked for.
            if away
                && request.command == "measure.thickness"
                && let Some(key) = self.side.props_for
            {
                match session.hand_over(props::CARD_JOB, &json!({ "wall": request.params })) {
                    Some(token) => {
                        self.side.card_jobs.insert(token);
                        self.side.wall_asked = Some((key, token, wall_of_body, tolerance));
                        continue;
                    }
                    None if session.commands_full() => {
                        self.side.wall = Some(props::Wall::not_measured(
                            "as many measures wait beside the page as may; ask again in a moment"
                                .to_string(),
                        ));
                        continue;
                    }
                    None => {}
                }
            }
            // A check kept is made where the bodies are, as an edit the
            // person asked for (adding it measures every check of the
            // document): handed over, applied here from its answer.
            let deferrable = away && request.command == "assert.add";
            // The notes the window writes are the person's: typed here.
            let run = |session: &mut Session| {
                if deferrable {
                    session.execute_deferrable(request.command, &request.params)
                } else {
                    session.execute(request.command, &request.params)
                }
            };
            let done = if request.command.starts_with("note.") {
                session.by_person(run)
            } else {
                run(session)
            };
            match done {
                Ok(result) => {
                    self.side.problem = None;
                    if deferrable && result.get("deferred") == Some(&Value::Bool(true)) {
                        self.side.keeping = true;
                    }
                    // Wall thickness: its answer heads the card.
                    if request.command == "measure.thickness" {
                        self.side.wall =
                            Some(props::Wall::answered(Ok(&result), wall_of_body, tolerance));
                    }
                    // Clearing a body's face colours says on the status
                    // line how many it cleared (and what it kept).
                    if request.command == "face.appearance" {
                        self.last_command = Some(crate::shell::CommandReport {
                            label: request.command.to_string(),
                            ok: true,
                            text: crate::answer::summary(request.command, &result),
                        });
                    }
                }
                Err(e) => {
                    let why = e.message().to_string();
                    if request.command == "measure.thickness" {
                        self.side.wall =
                            Some(props::Wall::answered(Err(why), wall_of_body, tolerance));
                        continue;
                    }
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
        self.sync_notes(session, key, bodies_here);
        let serial = session.selection_serial();
        let props_key = (doc, revision, serial);
        if self.side.props_for != Some(props_key) {
            // Another selection, not only another revision (a check kept
            // changes the revision): the Checks opened by hand fold again,
            // the check just kept is no longer tinted.
            let selection_changed = self
                .side
                .props_for
                .is_none_or(|(d, _, s)| (d, s) != (doc, serial));
            self.side.props_for = Some(props_key);
            self.side.wall = None;
            if selection_changed {
                self.side.checks_asked = false;
                self.side.kept_tint = None;
                self.side.copied = None;
                self.side.name_copied = None;
            }
            // The relation of the same two things measured again (after a
            // check was kept, an edit): the last answer stays shown while
            // it is (V4: nothing blinks).
            let before = self
                .side
                .props
                .as_ref()
                .and_then(|p| p.measurement.clone())
                .filter(|m| !m.measuring);
            let mut wanted = None;
            // What the card needs of the bodies: read here where they are;
            // else asked where they are, and said to be measured until it
            // comes (an answer asked for another card is dropped; an ask
            // not posted yet is let go, so the worker never runs it).
            if let Some((_, token)) = self.side.facts_asked.take()
                && !session.command_posted(token)
                && session.command_dropped(token, "asked again").is_some()
            {
                self.side.card_jobs.remove(&token);
            }
            self.side.facts_came = None;
            self.side.facts_later = None;
            let source = if bodies_here || self.selection().items.is_empty() {
                None
            } else {
                props::facts_job(session, scene)
                    .and_then(|job| self.ask_facts(session, props_key, job))
            };
            let source = source.unwrap_or(props::Source::Here);
            self.side.props = if self.selection().items.is_empty() {
                None
            } else {
                props::read(session, scene, source, &mut wanted)
            };
            if let (Some(before), Some(m)) = (
                before,
                self.side
                    .props
                    .as_mut()
                    .and_then(|p| p.measurement.as_mut()),
            ) && m.measuring
                && m.items == before.items
            {
                *m = props::Measurement {
                    measuring: true,
                    ..before
                };
            }
            // One measure runs at a time: a newer one waits for it.
            self.side.waiting = None;
            if let Some(params) = wanted {
                let busy = self
                    .side
                    .between
                    .as_ref()
                    .is_some_and(|(_, job)| job.answer().is_none())
                    || self.side.between_asked.is_some();
                if busy {
                    self.side.waiting = Some((props_key, params));
                } else {
                    self.start_between(session, props_key, &params, bodies_here);
                }
            }
        }
        // The facts the host could not take when they were asked for (as
        // many jobs waited as may): asked again; read here where it takes
        // none any more (the page regenerates itself now).
        if let Some((key, job)) = self.side.facts_later.take()
            && self.side.props_for == Some(key)
            && self.ask_facts(session, key, job).is_none()
        {
            self.read_again(session, scene, props::Source::Here);
        }
        // The facts asked where the bodies are, come for this card: the
        // card read again with them, its relation as it stands. Where none
        // came and the bodies are here now (the page regenerates itself:
        // its worker could not be had), read here.
        if let Some((key, came)) = self.side.facts_came.take()
            && self.side.props_for == Some(key)
        {
            let source = match came {
                Err(_) if bodies_here => props::Source::Here,
                came => props::Source::Elsewhere(Some(came)),
            };
            self.read_again(session, scene, source);
        }
        // The relation being measured: its answer, once it is there, for
        // the card it was asked for only; then the one waiting. One asked
        // where the bodies are that brought nothing is measured here when
        // they are here now.
        let came = match self
            .side
            .between
            .as_ref()
            .and_then(|(key, job)| Some((*key, job.answer()?)))
        {
            Some((key, answer)) => {
                self.side.between = None;
                Some((key, props::Away::Answer(answer)))
            }
            None => self.side.between_came.take(),
        };
        if let Some((key, answer)) = came {
            if self.side.props_for == Some(key) {
                match answer {
                    props::Away::Answer(answer) => self.settle_measurement(answer),
                    props::Away::Missed(_) if bodies_here => {
                        if let Some(params) = self.between_params() {
                            self.side.waiting = Some((key, params));
                        }
                    }
                    props::Away::Missed(why) => self.measurement_missed(why),
                }
            }
            if let Some((key, params)) = self.side.waiting.take()
                && self.side.props_for == Some(key)
            {
                self.start_between(session, key, &params, bodies_here);
            }
        }
        // The relation the host could not take yet (as many jobs waited as
        // may), while none is measured: asked again.
        if self.side.between.is_none()
            && self.side.between_asked.is_none()
            && let Some((key, params)) = self.side.waiting.take()
            && self.side.props_for == Some(key)
        {
            self.start_between(session, key, &params, bodies_here);
        }
        // A check kept where the bodies are is back (or was cancelled).
        if self.side.keeping && !session.edits_waiting() {
            self.side.keeping = false;
        }
    }

    /// Hands the card's facts `job` for the card of `key` over to where the
    /// bodies are: the card says they are measured (`Some`), also while
    /// the host takes no more jobs for now (as many wait as may: asked
    /// again at the next sync); `None` where it takes none (read here).
    fn ask_facts(
        &mut self,
        session: &mut Session,
        key: (u64, u64, u64),
        job: Value,
    ) -> Option<props::Source> {
        match session.hand_over(props::CARD_JOB, &job) {
            Some(token) => {
                self.side.card_jobs.insert(token);
                self.side.facts_asked = Some((key, token));
            }
            None if session.commands_full() => self.side.facts_later = Some((key, job)),
            None => return None,
        }
        Some(props::Source::Elsewhere(None))
    }

    /// The card read again with its facts from `source`, its relation as
    /// it stands.
    fn read_again(
        &mut self,
        session: &mut Session,
        scene: Option<&craft3d_engine::Scene>,
        source: props::Source,
    ) {
        let measurement = self.side.props.as_ref().and_then(|p| p.measurement.clone());
        let mut wanted = None;
        self.side.props = props::read(session, scene, source, &mut wanted).map(|mut p| {
            p.measurement = measurement;
            p
        });
    }

    /// Starts measuring `params` (`measure.between`'s) for the card of
    /// `key`: on a thread of its own, or where the bodies are when they
    /// are not here (`bodies_here` false: the browser's worker; while it
    /// takes no more jobs for now, asked again at the next sync); refused
    /// at once, the card says why.
    fn start_between(
        &mut self,
        session: &mut Session,
        key: (u64, u64, u64),
        params: &Value,
        bodies_here: bool,
    ) {
        if !bodies_here {
            match session.hand_over(props::CARD_JOB, &json!({ "between": params })) {
                Some(token) => {
                    self.side.card_jobs.insert(token);
                    self.side.between_asked = Some((key, token));
                    return;
                }
                None if session.commands_full() => {
                    self.side.waiting = Some((key, params.clone()));
                    return;
                }
                None => {}
            }
        }
        match session.between_later(params) {
            Ok(job) => self.side.between = Some((key, job)),
            Err(e) => self.settle_measurement(Err(e.message().to_string())),
        }
    }

    /// The parameters the card's relation is measured with, while it is
    /// being measured.
    fn between_params(&self) -> Option<Value> {
        let m = self.side.props.as_ref()?.measurement.as_ref()?;
        let [from, to] = &m.items;
        Some(json!({"from": from, "to": to}))
    }

    /// The card's relation, asked where the bodies are, brought nothing
    /// from there: `why`.
    fn measurement_missed(&mut self, why: String) {
        if let Some(m) = self
            .side
            .props
            .as_mut()
            .and_then(|p| p.measurement.as_mut())
            && m.measuring
        {
            *m = m.not_measured(why);
        }
    }

    /// Takes the answers of the jobs the card handed over to be run where
    /// the bodies are ([`props::CARD_JOB`]: what is selected, measured in
    /// the browser's worker), each for the card it was asked for, and
    /// gives back the others for the host to report as before.
    pub fn card_answered(
        &mut self,
        answered: Vec<craft3d_engine::Answered>,
    ) -> Vec<craft3d_engine::Answered> {
        let mut others = Vec::new();
        for answer in answered {
            let token = answer.token;
            if !self.side.card_jobs.remove(&token) {
                others.push(answer);
                continue;
            }
            match (
                self.side.facts_asked,
                self.side.between_asked,
                self.side.wall_asked,
            ) {
                (Some((key, asked)), ..) if asked == token => {
                    self.side.facts_asked = None;
                    self.side.facts_came = Some((key, props::facts_answered(answer.result)));
                }
                (_, Some((key, asked)), _) if asked == token => {
                    self.side.between_asked = None;
                    self.side.between_came = Some((key, props::Away::of(answer.result)));
                }
                (.., Some((key, asked, body, tolerance))) if asked == token => {
                    self.side.wall_asked = None;
                    // For the card it was asked for, while it is measured.
                    if self.side.props_for == Some(key)
                        && self.side.wall.as_ref().is_some_and(|w| w.measuring)
                    {
                        self.side.wall = Some(match props::Away::of(answer.result) {
                            props::Away::Answer(Ok(result)) => {
                                props::Wall::answered(Ok(&result), body, tolerance)
                            }
                            props::Away::Answer(Err(why)) => {
                                props::Wall::answered(Err(why), body, tolerance)
                            }
                            props::Away::Missed(why) => props::Wall::not_measured(why),
                        });
                    }
                }
                // Asked for a card shown no longer: dropped.
                _ => {}
            }
        }
        others
    }

    /// The card's relation, measured.
    fn settle_measurement(&mut self, answer: Result<Value, String>) {
        if let Some(m) = self
            .side
            .props
            .as_mut()
            .and_then(|p| p.measurement.as_mut())
            && m.measuring
        {
            *m = m.settled(answer);
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
        let mut offered = tabs_for(assembly, jig || joints, self.agent_tab_offered());
        notes::offer(
            &mut offered,
            !assembly && self.notes_tab_offered(),
            self.notes_for_you() > 0,
        );
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
        // What is selected shows under every tab, as at the foot of the
        // Features tab, which has it there with the checks (the developer,
        // 2026-11-17: with another tab open, the length of the edge he had
        // selected was nowhere in sight).
        if tab != SideTab::Features && self.side.props.is_some() {
            let inset = f32::from(crate::shell::side_inset(&tokens));
            // As high as its content drew in the last frame (the estimate
            // before the first), so its buttons are never out of reach;
            // at most what leaves the tab three rows (review-pins 2b: on a
            // short window Measure and Copy name ended below the panel).
            let room = ui.available_height();
            let most = (room - 3.0 * tokens.row_height).max(room * 0.5);
            // Where the whole card has no room, its short form, as the
            // Features tab has it (the card opens with its number since
            // the UX round of 2026-11-18, V4, and is taller).
            let compact = self.props_height() > most;
            let wanted = if compact {
                self.props_compact_height()
            } else {
                self.side
                    .card_content
                    .filter(|h| h.is_finite() && *h > 0.0)
                    .unwrap_or_else(|| self.props_height())
            };
            let height = wanted.min(most).max(0.0);
            let mut drawn = None;
            egui::Panel::bottom("craft3d-side-selection")
                .resizable(false)
                .exact_size(height)
                .frame(egui::Frame::new())
                .show(ui, |ui| {
                    let shown = egui::ScrollArea::vertical()
                        .id_salt("craft3d-side-selection-scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width((ui.available_width() - inset).max(40.0));
                            ui.spacing_mut().item_spacing.y = 0.0;
                            if compact {
                                self.props_compact_ui(ui);
                            } else {
                                self.props_ui(ui);
                            }
                        });
                    drawn = Some(shown.content_size.y);
                });
            self.side.card_content = drawn.filter(|_| !compact);
        } else {
            self.side.card_content = None;
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
            SideTab::Notes => self.notes_tab(ui),
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
                    .layout_no_wrap(self.tab_words(*tab), font.clone(), egui::Color32::BLACK)
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
        // The Notes tab says how many are open.
        let words: Vec<String> = shown.iter().map(|tab| self.tab_words(*tab)).collect();
        let labels: Vec<Tab<'_>> = words.iter().map(|words| Tab::new(words)).collect();
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
