//! "Allow AI agents": the switch that lets agents drive this window over
//! MCP, and where the person finds it (docs/control-protocol.md,
//! "Connecting an agent"): one panel ([`Shell::agents_panel`]), shown on
//! the start screen and in a popover over the status bar's agents pill,
//! which is at the bar's right end on the desktop (decision of
//! 2026-11-02) and not shown in the browser, where agents cannot connect
//! yet. The pill is the kit's ([`crate::kit::PillState`]): its colour is
//! its state (off, waiting, connected, working, asks), and it says what
//! an agent does while it does it, so that no agent acts unnoticed.
//!
//! The shell only asks: the host owns the listener, turns it on and off
//! when the shell asks it to ([`Shell::take_agents_switch`],
//! [`Shell::take_agents_disconnect`]) and tells the shell how things stand
//! ([`Shell::set_agents_status`]), which `ui.get` reports under `agents`.

use super::Shell;
use super::start::McpClient;
use crate::kit::{Button, Dialog, Glyph, Kit, PillState};
use crate::state::AgentsStatus;
use crate::theme::color;

/// What the switch remembers between frames.
#[derive(Default)]
pub(crate) struct AgentsMemory {
    pub(super) status: AgentsStatus,
    /// The switch was turned, until the host takes it.
    switch: Option<bool>,
    /// "Disconnect all" was clicked, until the host takes it.
    disconnect: bool,
    /// The ✕ of one agent was clicked (its key), until the host takes it.
    disconnect_one: Option<String>,
    /// "Change…" was clicked: the host shows a folder picker.
    change_folder: bool,
    /// "Also the open document's folder" was turned.
    document_folder: Option<bool>,
    /// Where the pill was drawn in the last frame (the popover's anchor).
    pub(super) pill: Option<egui::Rect>,
    /// The clock the pill reads, in milliseconds since the epoch, when a
    /// test sets it; the system's otherwise.
    clock: Option<u64>,
    /// Where the popover's controls were drawn in the last frame.
    popover_hits: Vec<(String, egui::Rect)>,
    /// The person's answer to a prompt, until the host takes it.
    answer: Option<(u64, crate::state::PromptAnswer)>,
    /// A folder the person took back (Remove), until the host takes it.
    revoke: Option<String>,
    /// Where the prompt's buttons were drawn in the last frame.
    prompt_hits: Vec<(String, egui::Rect)>,
    /// The question shown, and since when (egui's clock).
    prompt_since: Option<(u64, f64)>,
    /// The agent that asks `ui.get` now: it sees its own entry only.
    viewer: Option<String>,
    /// "Remember for this folder" of the question at connect.
    remember: bool,
}

/// How long a question takes no click after it appears or changes, in
/// seconds: a click or a double click meant for what was under the cursor
/// never answers it.
pub const PROMPT_ARMING: f64 = 0.7;

/// How long the pill shows what an agent did, in milliseconds.
pub const ACTIVITY_MS: u64 = 1500;

/// How the pill looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillKind {
    /// The browser: agents cannot drive it yet.
    Web,
    /// Agents not allowed: an outlined pill, an invitation.
    Off,
    /// Allowed, nobody connected: a filled pill.
    Waiting,
    /// Connected: filled, with a green dot.
    Connected,
    /// Connected and doing something now: the dot pulses.
    Active,
    /// A question waits for the person's answer (a folder, a file).
    Asks,
}

/// The pill's words and look.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PillView {
    pub kind: PillKind,
    pub label: String,
    /// For `Active`: how long ago the command came, in milliseconds.
    pub age_ms: u64,
}

/// The pill for `status` at `now_ms` (milliseconds since the epoch);
/// `web` for the browser.
pub fn pill_view(status: &AgentsStatus, now_ms: u64, web: bool) -> PillView {
    let plain = |kind, label: &str| PillView {
        kind,
        label: label.to_string(),
        age_ms: 0,
    };
    if web {
        return plain(PillKind::Web, "Agents: desktop only");
    }
    if !status.allowed {
        return plain(PillKind::Off, "Agents off");
    }
    if let Some(prompt) = status.prompts.first() {
        let what = match prompt.kind.as_str() {
            "connect" => "asks for a folder",
            "read" => "asks to open a file",
            _ => "asks to write a file",
        };
        return plain(PillKind::Asks, &format!("{} {what}", prompt.client));
    }
    if status.clients.is_empty() {
        return plain(PillKind::Waiting, "Waiting for an agent");
    }
    // The latest command of any client, while it is recent.
    let latest = status
        .clients
        .iter()
        .filter_map(|client| {
            client
                .last_command
                .as_ref()
                .map(|command| (client.last_ms, client.name.as_str(), command.as_str()))
        })
        .max_by_key(|(at, _, _)| *at);
    if let Some((at, name, command)) = latest {
        let age = now_ms.saturating_sub(at);
        if at <= now_ms.saturating_add(1000) && age < ACTIVITY_MS {
            return PillView {
                kind: PillKind::Active,
                label: format!("{name} · {command}"),
                age_ms: age,
            };
        }
    }
    let label = match status.clients.as_slice() {
        [only] => format!("{} connected", only.name),
        many => format!("{} agents connected", many.len()),
    };
    plain(PillKind::Connected, &label)
}

/// The kit's pill for a kind; `None` in the browser, where the pill is
/// not shown.
pub fn pill_state(kind: PillKind) -> Option<PillState> {
    match kind {
        PillKind::Web => None,
        PillKind::Off => Some(PillState::Off),
        PillKind::Waiting => Some(PillState::Waiting),
        PillKind::Connected => Some(PillState::Connected),
        PillKind::Active => Some(PillState::Working),
        PillKind::Asks => Some(PillState::Asks),
    }
}

/// Milliseconds since the epoch, 0 for a clock before it.
fn system_ms() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    use std::time::{SystemTime, UNIX_EPOCH};
    #[cfg(target_arch = "wasm32")]
    use web_time::{SystemTime, UNIX_EPOCH};
    // native clock: std's only off the browser (cfg above); there it is web_time's.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// "for 3 min", "for 2 h": how long since `since_ms`.
fn for_how_long(since_ms: u64, now_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(since_ms) / 1000;
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("for {} min", seconds / 60),
        _ => format!("for {} h", seconds / 3600),
    }
}

/// `path` with the home folder written `~`, for showing.
pub(crate) fn shown_path(path: &str, home: Option<&str>) -> String {
    match home.filter(|home| !home.is_empty()) {
        Some(home) => match path.strip_prefix(home) {
            Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => format!("~{rest}"),
            _ => path.to_string(),
        },
        None => path.to_string(),
    }
}

impl Shell {
    /// How things stand with agents, as the host sees them; true when it
    /// changed (the window draws it then).
    pub fn set_agents_status(&mut self, status: AgentsStatus) -> bool {
        if self.agents.status == status {
            return false;
        }
        self.agents.status = status;
        true
    }

    pub fn agents_status(&self) -> &AgentsStatus {
        &self.agents.status
    }

    /// Which agent reads the window's state now (`ui.get` over the control
    /// channel), `None` for the person and the host: an agent sees its own
    /// client, its own questions and the roots, not other agents' folders
    /// and questions.
    pub fn set_agents_viewer(&mut self, agent: Option<&str>) {
        self.agents.viewer = agent.map(str::to_string);
    }

    /// The status as the current reader may see it ([`Self::set_agents_viewer`]).
    pub(crate) fn agents_status_for_viewer(&self) -> AgentsStatus {
        let mut status = self.agents.status.clone();
        if let Some(viewer) = &self.agents.viewer {
            status.clients.retain(|client| &client.name == viewer);
            status.prompts.retain(|prompt| &prompt.client == viewer);
        }
        status
    }

    /// Asks the host to allow agents (`true`) or not, as the switch does.
    pub fn request_agents(&mut self, on: bool) {
        self.agents.switch = Some(on);
    }

    /// Whether the switch was turned since the host last asked.
    pub fn take_agents_switch(&mut self) -> Option<bool> {
        self.agents.switch.take()
    }

    /// Asks the host to disconnect every agent, as "Disconnect all" does.
    pub fn request_agents_disconnect(&mut self) {
        self.agents.disconnect = true;
    }

    pub fn take_agents_disconnect(&mut self) -> bool {
        std::mem::take(&mut self.agents.disconnect)
    }

    /// Asks the host to disconnect the agent that authenticated as `key`,
    /// as its ✕ in the agent panel does: refused for the rest of the run.
    pub fn request_agent_disconnect(&mut self, key: &str) {
        self.agents.disconnect_one = Some(key.to_string());
    }

    pub fn take_agent_disconnect(&mut self) -> Option<String> {
        self.agents.disconnect_one.take()
    }

    /// Asks the host for a folder picker for the agents' folder, as
    /// Change… does.
    pub fn request_agents_folder(&mut self) {
        self.agents.change_folder = true;
    }

    pub fn take_agents_folder(&mut self) -> bool {
        std::mem::take(&mut self.agents.change_folder)
    }

    /// Asks the host to give agents the open document's folder too, or not.
    pub fn request_agents_document_folder(&mut self, on: bool) {
        self.agents.document_folder = Some(on);
    }

    pub fn take_agents_document_folder(&mut self) -> Option<bool> {
        self.agents.document_folder.take()
    }

    /// True while something the panel asked for waits for the host.
    pub(super) fn agents_pending(&self) -> bool {
        self.agents.switch.is_some()
            || self.agents.disconnect
            || self.agents.disconnect_one.is_some()
            || self.agents.change_folder
            || self.agents.document_folder.is_some()
            || self.agents.answer.is_some()
            || self.agents.revoke.is_some()
    }

    /// Answers prompt `id` as the person's click does. Only the window's
    /// own buttons (and the host's tests) call this: no `ui.*` method
    /// reaches it, so an agent cannot approve its own access.
    pub fn answer_agent_prompt(&mut self, id: u64, answer: crate::state::PromptAnswer) {
        self.agents.answer = Some((id, answer));
    }

    pub fn take_agent_prompt_answer(&mut self) -> Option<(u64, crate::state::PromptAnswer)> {
        self.agents.answer.take()
    }

    /// Takes back a folder the person allowed (Remove).
    pub fn request_agents_revoke(&mut self, folder: &str) {
        self.agents.revoke = Some(folder.to_string());
    }

    pub fn take_agents_revoke(&mut self) -> Option<String> {
        self.agents.revoke.take()
    }

    /// Where the prompt's buttons were drawn in the last frame, by name.
    pub fn agent_prompt_hits(&self) -> &[(String, egui::Rect)] {
        &self.agents.prompt_hits
    }

    /// The oldest question about an agent's access, as a dialog in the
    /// middle of the window (board s3): what it asks, the folder or the
    /// file on a code line of its own (escaped), then the answers, "no"
    /// first at the left and the amber one last at the right. The buttons
    /// take no click for [`PROMPT_ARMING`] seconds after a question
    /// appears or changes, so a click meant for something else (or a
    /// double click) never answers it.
    ///
    /// At connect (decisions of 2026-11-02 and 2026-11-04): Refuse
    /// disconnects that agent and refuses it for the rest of the run;
    /// Only the work folder refuses the folder only (the connection
    /// stays, with the work folder); Allow this folder allows it for this
    /// run, or for good with "Remember for this folder".
    pub(super) fn agent_prompt(&mut self, ctx: &egui::Context) {
        use crate::state::PromptAnswer;
        self.agents.prompt_hits.clear();
        let Some(prompt) = self.agents.status.prompts.first().cloned() else {
            self.agents.prompt_since = None;
            return;
        };
        let now = ctx.input(|i| i.time);
        let since = match self.agents.prompt_since {
            Some((id, at)) if id == prompt.id => at,
            _ => {
                self.agents.prompt_since = Some((prompt.id, now));
                self.agents.remember = false;
                now
            }
        };
        let armed = now - since >= PROMPT_ARMING;
        let tokens = self.tokens;
        let home = self.agents.status.home.clone();
        let escaped = |text: &str| {
            text.chars()
                .flat_map(char::escape_debug)
                .collect::<String>()
        };
        let client = escaped(&prompt.client);
        let connect = prompt.kind == "connect";
        let (title, icon, question, path) = if connect {
            (
                format!("{client} wants to connect"),
                (Glyph::Plug, tokens.info_ink),
                "It is working in this folder. 3DCraft can let it read and write files there, besides the work folder:".to_string(),
                escaped(&shown_path(&prompt.folder, home.as_deref())),
            )
        } else {
            let (verb, what) = if prompt.kind == "read" {
                ("open", "read")
            } else {
                ("write", "write")
            };
            let command = prompt
                .command
                .as_deref()
                .map_or_else(|| "It".to_string(), escaped);
            (
                format!("{client} asks to {verb} a file"),
                (Glyph::Shield, tokens.attention),
                format!("{command} wants to {what} outside the folders it may use:"),
                escaped(&shown_path(&prompt.path, home.as_deref())),
            )
        };
        let mut hits = Vec::new();
        let mut chosen = None;
        let mut remember = self.agents.remember;
        let kit = Kit::panel(&tokens);
        // On a phone a bottom sheet with the answers one under the other,
        // the amber one first (board m7).
        // The narrow layout's rule, as for the other sheets.
        let phone = self.narrow();
        let dialog = Dialog::new(egui::Id::new("craft3d-agent-prompt"), &title)
            .icon(icon.0, icon.1)
            .width(tokens.dialog_wide_width)
            .closable(false)
            .sheet(phone);
        let mut left_hits = Vec::new();
        let t_gap = tokens.nav_spacing;
        kit.dialog(ctx, dialog, |kit, ui| {
            kit.paragraph(ui, &question, false);
            kit.code_line(ui, "", &path);
            if connect {
                let check = kit.checkbox(ui, &mut remember, "Remember for this folder");
                left_hits.push(("remember".to_string(), check.rect));
            }
            let mut answer = |ui: &mut egui::Ui, label: &str, button: Button<'_>, to| {
                let response = kit.button(ui, button.enabled(armed));
                hits.push((label.to_string(), response.rect));
                if response.clicked() {
                    chosen = Some(to);
                }
                response
            };
            if phone {
                // Stacked, full width: the amber answer first, "no" last.
                ui.add_space(t_gap);
                let folder = if remember {
                    PromptAnswer::Folder
                } else {
                    PromptAnswer::Session
                };
                let stack: Vec<(&str, Button<'_>, PromptAnswer)> = if connect {
                    vec![
                        ("Allow this folder", Button::primary("Allow this folder"), folder),
                        ("Only the work folder", Button::secondary("Only the work folder"), PromptAnswer::Deny),
                        ("Refuse", Button::danger("Refuse"), PromptAnswer::Refuse),
                    ]
                } else {
                    vec![
                        ("Allow once", Button::primary("Allow once"), PromptAnswer::Once),
                        ("Allow this folder", Button::secondary("Allow this folder"), PromptAnswer::Folder),
                        ("Deny", Button::danger("Deny"), PromptAnswer::Deny),
                    ]
                };
                for (label, button, to) in stack {
                    answer(ui, label, button.wide(true), to);
                }
                return;
            }
            kit.dialog_footer(ui, |ui| {
                if connect {
                    answer(ui, "Refuse", Button::danger("Refuse"), PromptAnswer::Refuse)
                        .on_hover_text("Disconnect this agent; it is refused until 3DCraft restarts");
                } else {
                    answer(ui, "Deny", Button::danger("Deny"), PromptAnswer::Deny);
                }
                kit.footer_right(ui, |ui| {
                    // From the right: the amber one first.
                    if connect {
                        let folder = if remember {
                            PromptAnswer::Folder
                        } else {
                            PromptAnswer::Session
                        };
                        answer(
                            ui,
                            "Allow this folder",
                            Button::primary("Allow this folder"),
                            folder,
                        );
                        answer(
                            ui,
                            "Only the work folder",
                            Button::secondary("Only the work folder"),
                            PromptAnswer::Deny,
                        );
                    } else {
                        answer(
                            ui,
                            "Allow once",
                            Button::primary("Allow once"),
                            PromptAnswer::Once,
                        );
                        answer(
                            ui,
                            "Allow this folder",
                            Button::secondary("Allow this folder"),
                            PromptAnswer::Folder,
                        )
                        .on_hover_text(
                            "Every agent may write in this folder from now on (all agents, remembered; the agent panel's Remove takes it back)",
                        );
                    }
                });
            });
        });
        // In reading order: the left one, then the right ones from the
        // left (they were laid out from the right).
        let mut ordered: Vec<(String, egui::Rect)> = if phone {
            hits
        } else {
            let (first, rest) = hits.split_at(hits.len().min(1));
            let mut ordered = first.to_vec();
            ordered.extend(rest.iter().rev().cloned());
            ordered
        };
        ordered.extend(left_hits);
        self.agents.remember = remember;
        self.agents.prompt_hits = ordered;
        if let Some(answer) = chosen {
            self.answer_agent_prompt(prompt.id, answer);
        }
        if armed {
            // The host times a request's question out: looked at again
            // now and then.
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                (PROMPT_ARMING - (now - since)).max(0.05),
            ));
        }
    }

    /// The clock the pill reads (milliseconds since the epoch); `None`
    /// for the system's. For tests.
    pub fn set_agents_clock(&mut self, now_ms: Option<u64>) {
        self.agents.clock = now_ms;
    }

    pub(crate) fn agents_now(&self) -> u64 {
        self.agents.clock.unwrap_or_else(system_ms)
    }

    /// What the pill shows now.
    pub fn agents_pill_view(&self) -> PillView {
        pill_view(
            &self.agents.status,
            self.agents_now(),
            cfg!(target_arch = "wasm32"),
        )
    }

    /// Where the pill was drawn in the last frame.
    pub fn agents_pill_rect(&self) -> Option<egui::Rect> {
        self.agents.pill
    }

    /// Opens or closes the agents popover (`state.dialogs.agents`).
    pub fn show_agents_popover(&mut self, open: bool) {
        self.state.dialogs.agents = open && !cfg!(target_arch = "wasm32");
    }

    /// The panel's heading: the info accent's bar, "Connect an AI agent",
    /// and the switch at the right; where the switch was drawn.
    pub(super) fn agents_heading(&mut self, ui: &mut egui::Ui, kit: &Kit) -> egui::Rect {
        let mut on = self.agents.switch.unwrap_or(self.agents.status.allowed);
        let was = on;
        let mut switch = egui::Rect::NOTHING;
        ui.horizontal(|ui| {
            kit.section_header(ui, "Connect an AI agent");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let response = kit.toggle(ui, &mut on, "").on_hover_text(if was {
                    "Agents may drive this window over MCP. Click to turn it off and close their connections."
                } else {
                    "Let AI agents drive this window over MCP while you watch."
                });
                switch = response.rect;
            });
        });
        if on != was {
            self.request_agents(on);
            ui.ctx().request_repaint();
        }
        switch
    }

    /// The folder agents may read and write files in, with Change… and
    /// Open folder, and the check box for the open document's folder;
    /// their places, by name.
    pub(super) fn agents_folder_rows(
        &mut self,
        ui: &mut egui::Ui,
        kit: &Kit,
    ) -> Vec<(String, egui::Rect)> {
        let s = kit.skin;
        let status = self.agents.status.clone();
        let home = status.home.as_deref();
        let mut hits = Vec::new();
        let primary: Vec<&crate::state::AgentRoot> = status
            .roots
            .iter()
            .filter(|root| root.from != "document")
            .collect();
        let (shown, folder) = if status.command_line {
            let paths: Vec<String> = primary
                .iter()
                .map(|root| shown_path(&root.path, home))
                .collect();
            (
                format!("{} (set by the command line)", paths.join(", ")),
                primary.first().map(|root| root.path.clone()),
            )
        } else {
            match &status.work_folder {
                Some(folder) => (shown_path(folder, home), Some(folder.clone())),
                None => (
                    "No folder: agents can read and write no files".to_string(),
                    None,
                ),
            }
        };
        #[cfg(target_arch = "wasm32")]
        let _ = folder;
        label(ui, kit, "Work folder");
        folder_field(ui, kit, &shown);
        ui.horizontal_wrapped(|ui| {
            let change = kit
                .button(
                    ui,
                    Button::secondary("Change…").enabled(!status.command_line),
                )
                .on_hover_text("Choose the folder agents may read and write files in")
                .on_disabled_hover_text(
                    "Set by --automation-read-root and --automation-write-root on the command line",
                );
            hits.push(("change-folder".to_string(), change.rect));
            if change.clicked() {
                self.request_agents_folder();
            }
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(folder) = folder {
                let open = kit.button(ui, Button::quiet("Open folder").icon(Glyph::External));
                hits.push(("open-folder".to_string(), open.rect));
                if open.clicked() {
                    self.open_external(
                        ui.ctx(),
                        super::help::External::Folder(std::path::PathBuf::from(folder)),
                    );
                }
            }
        });
        let mut document = self
            .agents
            .document_folder
            .unwrap_or(status.document_folder);
        let text = match status.roots.iter().find(|root| root.from == "document") {
            Some(root) => format!(
                "Also the open document's folder ({})",
                shown_path(&root.path, home)
            ),
            None => "Also the open document's folder".to_string(),
        };
        let before = document;
        let check = kit.checkbox(ui, &mut document, &text).on_hover_text(
            "While a document saved on disk is open, agents may read and write files in its folder too",
        );
        hits.push(("document-folder".to_string(), check.rect));
        if document != before {
            self.request_agents_document_folder(document);
        }
        // The folders the person allowed when an agent asked: each can be
        // taken back.
        for root in status
            .roots
            .iter()
            .filter(|root| root.from == "allowed" || root.from == "session")
        {
            ui.horizontal_wrapped(|ui| {
                let text = if root.from == "session" {
                    format!("This session: {}", shown_path(&root.path, home))
                } else {
                    format!("Always: {}", shown_path(&root.path, home))
                };
                ui.label(
                    egui::RichText::new(text)
                        .font(kit.font())
                        .color(color(s.text_dim)),
                );
                let remove = kit
                    .button(ui, Button::quiet("Remove"))
                    .on_hover_text("Agents may no longer read and write files there");
                hits.push((format!("remove:{}", root.path), remove.rect));
                if remove.clicked() {
                    self.request_agents_revoke(&root.path);
                }
            });
        }
        hits
    }

    /// The panel that connects agents (board s3), the same on the start
    /// screen (on the panel) and in the popover over the pill (on the
    /// plate): the heading with the switch; while it is on the work
    /// folder, what to give the MCP client (`claude mcp add …`, the JSON
    /// configuration, ready snippets for other clients), who is connected
    /// and Disconnect all. Returns where its controls were drawn, by name.
    pub(super) fn agents_panel(
        &mut self,
        ui: &mut egui::Ui,
        kit: &Kit,
    ) -> Vec<(String, egui::Rect)> {
        let s = kit.skin;
        let t = kit.t;
        let mut hits = Vec::new();
        if cfg!(target_arch = "wasm32") {
            kit.section_header(ui, "Connect an AI agent");
            kit.paragraph(
                ui,
                "In the browser an agent cannot drive 3DCraft yet. Use the desktop app, whose window an agent drives over MCP while you watch.",
                true,
            );
            return hits;
        }
        ui.spacing_mut().item_spacing.y = t.nav_spacing * 1.5;
        let setup = self.agent_setup().clone();
        let status = self.agents.status.clone();
        let switch = self.agents_heading(ui, kit);
        hits.push(("agents-switch".to_string(), switch));
        if let Some(error) = &status.error {
            ui.label(
                egui::RichText::new(error)
                    .font(kit.font())
                    .color(color(s.failed)),
            );
        }
        let how = |shell: &mut Self, ui: &mut egui::Ui| {
            if kit.button(ui, Button::link("How it works")).clicked() {
                shell.open_external(
                    ui.ctx(),
                    super::help::External::Url(super::help::AGENT_SETUP_URL.to_string()),
                );
            }
        };
        if !status.allowed {
            kit.paragraph(
                ui,
                "Off: nothing can connect to this window. Turn it on to let an agent such as Claude model here over MCP while you watch.",
                true,
            );
            how(self, ui);
            return hits;
        }
        kit.paragraph(
            ui,
            "Agents may connect to this window over MCP and work on the open documents while you watch.",
            true,
        );
        hits.extend(self.agents_folder_rows(ui, kit));
        label(ui, kit, "Connect");
        kit.code_line(ui, "claude", &setup.claude_command());
        // Wrapped: on a phone they do not fit on one line.
        ui.horizontal_wrapped(|ui| {
            let copy = kit
                .button(ui, Button::primary("Copy command").icon(Glyph::Copy))
                .on_hover_text(setup.claude_command());
            hits.push(("copy-command".to_string(), copy.rect));
            if copy.clicked() {
                ui.ctx().copy_text(setup.claude_command());
            }
            let config = kit
                .button(ui, Button::secondary("Copy MCP configuration").icon(Glyph::Copy))
                .on_hover_text("The JSON for clients configured with mcpServers, such as Claude Desktop and Cursor");
            hits.push(("copy-config".to_string(), config.rect));
            if config.clicked() {
                ui.ctx().copy_text(setup.mcp_config());
            }
            // Other clients: a menu of ready snippets, each copied as it
            // is chosen.
            let other = kit
                .button(ui, Button::quiet("Other clients").icon(Glyph::ChevronDown))
                .on_hover_text("Copy the configuration for Claude Desktop, Cursor, VS Code or any stdio client");
            hits.push(("other-clients".to_string(), other.rect));
            let plate = Kit::plate(&t);
            let frame = egui::Frame::new()
                .fill(color(t.popover_fill))
                .stroke(egui::Stroke::new(
                    1.0,
                    crate::theme::color_alpha(t.plate_rim, t.popover_rim_opacity),
                ))
                .corner_radius(egui::CornerRadius::same(t.menu_radius))
                .inner_margin(egui::Margin::same(t.nav_padding.clamp(0.0, 16.0) as i8));
            egui::Popup::menu(&other)
                .frame(frame)
                .width(t.menu_width + 60.0)
                .show(|ui| {
                    ui.style_mut().visuals = t.plate_visuals();
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for client in McpClient::ALL {
                        let label = format!("Copy for {}", client.label());
                        let item = plate
                            .menu_item(ui, crate::kit::MenuItem::new(&label).icon(Glyph::Copy))
                            .on_hover_text(client.hint());
                        if item.clicked() {
                            ui.ctx().copy_text(setup.client_config(client));
                            ui.close();
                        }
                    }
                });
        });
        let now = self.agents_now();
        if status.clients.is_empty() {
            ui.horizontal(|ui| {
                label(ui, kit, "Connected");
                ui.label(
                    egui::RichText::new("none yet")
                        .font(kit.font())
                        .color(color(s.text_dim)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    how(self, ui);
                });
            });
        } else {
            label(ui, kit, &format!("Connected  {}", status.clients.len()));
            for (i, client) in status.clients.iter().enumerate() {
                let last = match &client.last_command {
                    Some(command) => format!(" · last: {command}"),
                    None => " · idle".to_string(),
                };
                let close = ui
                    .push_id(("craft3d-agent-client", i), |ui| {
                        client_card(
                            ui,
                            kit,
                            &client.name,
                            &format!("{}{last}", for_how_long(client.since_ms, now)),
                            client.key.is_some(),
                        )
                    })
                    .inner;
                if let (Some(close), Some(key)) = (close, client.key.as_deref()) {
                    hits.push((format!("disconnect:{key}"), close.rect));
                    if close.clicked() {
                        self.request_agent_disconnect(key);
                    }
                }
            }
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let all = kit.button(ui, Button::danger("Disconnect all"));
                    hits.push(("disconnect-all".to_string(), all.rect));
                    if all.clicked() {
                        self.request_agents_disconnect();
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        how(self, ui);
                    });
                });
            });
        }
        hits
    }

    /// The pill's width for the room `most`, for the status bar to keep:
    /// none in the browser, where it is not shown.
    pub(super) fn agents_pill_width(&self, ui: &egui::Ui, most: f32) -> f32 {
        let view = self.agents_pill_view();
        match pill_state(view.kind) {
            Some(state) => Kit::plate(&self.tokens).pill_width(ui, state, &view.label, most),
            None => 0.0,
        }
    }

    /// The pill at the status bar's right end: how agents stand, and what
    /// one does now; a click opens the popover with the panel.
    pub(super) fn agents_pill(&mut self, ui: &mut egui::Ui, most: f32) {
        let view = self.agents_pill_view();
        let Some(state) = pill_state(view.kind) else {
            self.agents.pill = None;
            return;
        };
        let kit = Kit::plate(&self.tokens);
        let response = kit
            .pill_within(ui, state, &view.label, most)
            .on_hover_text(match view.kind {
                PillKind::Off => "Let an AI agent model in this window while you watch",
                _ => "AI agents: who is connected, the folder they may use, how to connect one",
            })
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        self.agents.pill = Some(response.rect);
        if response.clicked() {
            self.state.dialogs.agents = !self.state.dialogs.agents;
        }
        // Drawn again once the activity fades back to "connected" (the
        // kit's working pill asks for its own frames while it turns).
        if view.kind == PillKind::Active {
            let left = ACTIVITY_MS.saturating_sub(view.age_ms);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(left));
        }
    }

    /// The popover over the pill (dark, with its pointer down to the
    /// pill), while `state.dialogs.agents` is on: the agents panel.
    /// Escape or a click elsewhere closes it.
    pub(super) fn agents_popover(&mut self, ctx: &egui::Context) {
        if !self.state.dialogs.agents || cfg!(target_arch = "wasm32") {
            self.agents.popover_hits.clear();
            return;
        }
        let tokens = self.tokens;
        let screen = ctx.content_rect();
        let anchor = self.agents.pill.unwrap_or(egui::Rect::from_min_size(
            egui::pos2(screen.right() - 160.0, screen.bottom() - 24.0),
            egui::vec2(150.0, 20.0),
        ));
        let margin = tokens.inset * 0.8;
        let width = tokens
            .agent_popover_width
            .min((screen.width() - 2.0 * margin).max(160.0));
        // Its right edge in line with the pill's, within the window.
        let left = (anchor.right() - width)
            .min(screen.right() - width - margin)
            .max(screen.left() + margin);
        let pointer = anchor.center().x - left;
        let kit = Kit::plate(&tokens);
        let mut hits = Vec::new();
        let shown = egui::Area::new(egui::Id::new("craft3d-agents-popover"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(egui::pos2(left, anchor.top() - 2.0))
            .constrain(true)
            .show(ctx, |ui| {
                ui.style_mut().visuals = tokens.plate_visuals();
                kit.popover_frame_pointing(ui, width, Some(pointer), true, |ui| {
                    // Tall on a short window: it scrolls.
                    let room = (anchor.top() - screen.top() - 3.0 * tokens.inset).max(120.0);
                    egui::ScrollArea::vertical()
                        .max_height(room)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            hits = self.agents_panel(ui, &kit);
                        });
                });
            });
        self.agents.popover_hits = hits;
        let area = shown.response.rect;
        let clicked_outside = ctx.input(|i| {
            i.pointer.any_click()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|at| !area.contains(at) && !anchor.contains(at))
        });
        if clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.state.dialogs.agents = false;
        }
    }

    /// Where the popover's controls were drawn in the last frame, by name
    /// (empty while it is closed).
    pub fn agents_popover_hits(&self) -> &[(String, egui::Rect)] {
        &self.agents.popover_hits
    }

    /// The Help menu's item: opens the popover.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn agents_menu_item(&mut self, ui: &mut egui::Ui, kit: &Kit) {
        let item = kit
            .menu_item(ui, crate::kit::MenuItem::new("Connect an AI agent…"))
            .on_hover_text(
                "The agents panel over the status bar: the switch, the folder, who is connected",
            );
        if item.clicked() {
            self.state.dialogs.agents = true;
            ui.close();
        }
    }
}

/// A label of the panel in the medium weight (Work folder, Connect),
/// with a little room above it.
fn label(ui: &mut egui::Ui, kit: &Kit, text: &str) {
    ui.add_space(kit.t.nav_spacing);
    ui.label(
        egui::RichText::new(text)
            .font(kit.medium_font(ui))
            .color(color(kit.skin.text)),
    );
}

/// The work folder as a field that is only read: a folder icon and the
/// path, cut at the field's edge (the whole path on hover).
fn folder_field(ui: &mut egui::Ui, kit: &Kit, path: &str) {
    let t = kit.t;
    let s = kit.skin;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(0.0), t.control_height),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect(
        rect,
        kit.field_radius(),
        color(s.field),
        egui::Stroke::new(1.0, color(s.field_stroke)),
        egui::StrokeKind::Inside,
    );
    let side = t.nav_icon_size;
    let icon = egui::Rect::from_center_size(
        egui::pos2(rect.left() + t.field_text_pad + side * 0.5, rect.center().y),
        egui::vec2(side, side),
    );
    crate::kit::glyphs::paint(painter, Glyph::Folder, icon, color(s.text_dim), &t);
    let x = icon.right() + t.nav_icon_gap;
    let mut job =
        egui::text::LayoutJob::simple_singleline(path.to_string(), kit.font(), color(s.text));
    job.wrap =
        egui::text::TextWrapping::truncate_at_width((rect.right() - t.field_text_pad - x).max(1.0));
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(x, rect.center().y - galley.size().y * 0.5),
        galley,
        color(s.text),
    );
    response.on_hover_text(path);
}

/// A connected agent: a dot, its name, and since when and what it did
/// last, on a card of the field's colour, with a ✕ at its right that
/// disconnects it (board s3) when `closable`: the ✕'s response.
fn client_card(
    ui: &mut egui::Ui,
    kit: &Kit,
    name: &str,
    detail: &str,
    closable: bool,
) -> Option<egui::Response> {
    let t = kit.t;
    let s = kit.skin;
    egui::Frame::new()
        .fill(color(s.field))
        .corner_radius(kit.radius())
        .inner_margin(egui::Margin::symmetric(
            t.field_text_pad.clamp(0.0, 32.0) as i8,
            4,
        ))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let side = t.secondary_font;
                let (dot, _) =
                    ui.allocate_exact_size(egui::vec2(side * 0.8, side), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot.center(), side * 0.22, color(s.info_bar));
                let labels = |ui: &mut egui::Ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(name)
                                    .font(kit.font())
                                    .color(color(s.text)),
                            )
                            .truncate(),
                        );
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(detail)
                                    .font(kit.small_font())
                                    .color(color(s.text_dim)),
                            )
                            .truncate(),
                        );
                    });
                };
                if !closable {
                    labels(ui);
                    return None;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close = kit
                        .icon_button(ui, Glyph::Close, false, "Disconnect")
                        .on_hover_text(format!(
                            "Disconnect {name}; it is refused until 3DCraft restarts"
                        ));
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), labels);
                    Some(close)
                })
                .inner
            })
            .inner
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AgentClient;

    fn client(name: &str, command: Option<&str>, at: u64) -> AgentClient {
        AgentClient {
            name: name.to_string(),
            since_ms: 1_000,
            last_command: command.map(str::to_string),
            last_ms: at,
            cwd: None,
            key: None,
        }
    }

    #[test]
    fn the_pill_says_each_state_in_its_own_look() {
        let mut status = AgentsStatus::default();
        let now = 1_000_000;
        // The browser: no pill at all (decision of 2026-11-02).
        assert_eq!(pill_view(&status, now, true).kind, PillKind::Web);
        assert_eq!(pill_state(PillKind::Web), None);
        let off = pill_view(&status, now, false);
        assert_eq!(
            (off.kind, off.label.as_str()),
            (PillKind::Off, "Agents off")
        );
        assert_eq!(pill_state(off.kind), Some(PillState::Off));
        status.allowed = true;
        let waiting = pill_view(&status, now, false);
        assert_eq!(
            (waiting.kind, waiting.label.as_str()),
            (PillKind::Waiting, "Waiting for an agent")
        );
        assert_eq!(pill_state(waiting.kind), Some(PillState::Waiting));
        status.clients = vec![client("Claude Code", Some("feature.edit"), now - 5_000)];
        let connected = pill_view(&status, now, false);
        assert_eq!(
            (connected.kind, connected.label.as_str()),
            (PillKind::Connected, "Claude Code connected")
        );
        assert_eq!(pill_state(connected.kind), Some(PillState::Connected));
        assert_eq!(pill_state(PillKind::Active), Some(PillState::Working));
        status.clients.push(client("Cursor", None, 0));
        assert_eq!(pill_view(&status, now, false).label, "2 agents connected");
        // A question waiting: the pill says what is asked, before anything
        // else, in the amber look.
        for (kind, words) in [
            ("connect", "Claude Code asks for a folder"),
            ("write", "Claude Code asks to write a file"),
            ("read", "Claude Code asks to open a file"),
        ] {
            status.prompts = vec![crate::state::AgentPrompt {
                client: "Claude Code".to_string(),
                kind: kind.to_string(),
                ..crate::state::AgentPrompt::default()
            }];
            let asks = pill_view(&status, now, false);
            assert_eq!((asks.kind, asks.label.as_str()), (PillKind::Asks, words));
            assert_eq!(pill_state(asks.kind), Some(PillState::Asks));
        }
        // The colours are the kit's chrome pill tokens.
        let kit = Kit::plate(&crate::theme::Tokens::default());
        let t = kit.t;
        assert_eq!(
            kit.pill_inks(PillState::Off),
            (None, Some(t.pill_off_rim), None, t.chrome_text_dim)
        );
        assert_eq!(kit.pill_inks(PillState::Waiting).1, Some(t.pill_connected));
        assert_eq!(
            kit.pill_inks(PillState::Connected).0,
            Some(t.pill_connected)
        );
        assert_eq!(
            kit.pill_inks(PillState::Asks).0,
            Some(t.plate_attention_fill)
        );
    }

    /// What an agent does shows at once and goes after the idle time, on
    /// an injected clock.
    #[test]
    fn activity_shows_while_recent_and_fades_after_idle() {
        let mut status = AgentsStatus {
            allowed: true,
            ..AgentsStatus::default()
        };
        let at = 5_000_000;
        status.clients = vec![client("Claude Code", Some("feature.edit"), at)];
        let active = pill_view(&status, at + 200, false);
        assert_eq!(active.kind, PillKind::Active);
        assert_eq!(active.label, "Claude Code · feature.edit");
        assert_eq!(active.age_ms, 200);
        assert_eq!(
            pill_view(&status, at + ACTIVITY_MS - 1, false).kind,
            PillKind::Active
        );
        assert_eq!(
            pill_view(&status, at + ACTIVITY_MS, false).kind,
            PillKind::Connected
        );
        // The latest of several clients.
        status
            .clients
            .push(client("Cursor", Some("ui.screenshot"), at + 100));
        assert_eq!(
            pill_view(&status, at + 300, false).label,
            "Cursor · ui.screenshot"
        );
        // A clock far behind the command's (another machine's?) shows it
        // no longer than a second ahead.
        assert_eq!(
            pill_view(&status, at - 5_000, false).kind,
            PillKind::Connected
        );
    }

    #[test]
    fn paths_beneath_home_are_written_with_a_tilde() {
        assert_eq!(
            shown_path("/home/me/Documents/3DCraft", Some("/home/me")),
            "~/Documents/3DCraft"
        );
        assert_eq!(shown_path("/home/me", Some("/home/me")), "~");
        assert_eq!(shown_path("/home/meg/x", Some("/home/me")), "/home/meg/x");
        assert_eq!(shown_path("/srv/x", None), "/srv/x");
        assert_eq!(for_how_long(0, 30_000), "just now");
        assert_eq!(for_how_long(0, 180_000), "for 3 min");
        assert_eq!(for_how_long(0, 7_200_000), "for 2 h");
    }
}
