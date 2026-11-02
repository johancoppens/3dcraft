//! "Allow AI agents": the switch that lets agents drive this window over
//! MCP, and where the person finds it (docs/control-protocol.md,
//! "Connecting an agent"): one panel ([`Shell::agents_panel`]), shown on
//! the start screen and in a popover over the status bar's agents pill,
//! which is always at the bar's left on the desktop. The pill says how
//! things stand in three looks (off: an outlined indigo pill; on: filled;
//! connected: filled with a green dot and the client's name), and what an
//! agent does while it does it, so that no agent acts unnoticed.
//!
//! The shell only asks: the host owns the listener, turns it on and off
//! when the shell asks it to ([`Shell::take_agents_switch`],
//! [`Shell::take_agents_disconnect`]) and tells the shell how things stand
//! ([`Shell::set_agents_status`]), which `ui.get` reports under `agents`.

use super::Shell;
use crate::state::AgentsStatus;
use crate::theme::{Tokens, color, color_alpha};

/// What the switch remembers between frames.
#[derive(Default)]
pub(crate) struct AgentsMemory {
    pub(super) status: AgentsStatus,
    /// The switch was turned, until the host takes it.
    switch: Option<bool>,
    /// "Disconnect all" was clicked, until the host takes it.
    disconnect: bool,
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
}

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
        return plain(PillKind::Off, "Connect AI agent");
    }
    if status.clients.is_empty() {
        return plain(PillKind::Waiting, "Agents on — waiting");
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

/// The colours of a pill, from the tokens: (fill, outline, text, dot).
pub fn pill_colours(
    kind: PillKind,
    tokens: &Tokens,
) -> (
    Option<crate::theme::Rgb>,
    Option<crate::theme::Rgb>,
    crate::theme::Rgb,
    Option<crate::theme::Rgb>,
) {
    match kind {
        PillKind::Web => (
            None,
            Some(tokens.chrome_text_dim),
            tokens.chrome_text_dim,
            None,
        ),
        PillKind::Off => (None, Some(tokens.info_code), tokens.info_code, None),
        PillKind::Waiting => (Some(tokens.info_accent), None, tokens.plate_text, None),
        PillKind::Connected | PillKind::Active => (
            Some(tokens.info_accent),
            None,
            tokens.plate_text,
            Some(tokens.agent_dot),
        ),
    }
}

/// Milliseconds since the epoch, 0 for a clock before it.
fn system_ms() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    use std::time::{SystemTime, UNIX_EPOCH};
    #[cfg(target_arch = "wasm32")]
    use web_time::{SystemTime, UNIX_EPOCH};
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

/// A switch, drawn as a small track with a knob; true when it was clicked.
pub(crate) fn toggle(ui: &mut egui::Ui, tokens: &Tokens, on: bool) -> egui::Response {
    let height = ui.spacing().interact_size.y * 0.8;
    let size = egui::vec2(height * 1.8, height);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), on, "")
    });
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, on);
        let track = if on { tokens.accent } else { tokens.inactive };
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, radius, color(track));
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter().circle_filled(
            egui::pos2(x, rect.center().y),
            radius * 0.78,
            color(tokens.knob),
        );
    }
    response
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
            || self.agents.change_folder
            || self.agents.document_folder.is_some()
    }

    /// The clock the pill reads (milliseconds since the epoch); `None`
    /// for the system's. For tests.
    pub fn set_agents_clock(&mut self, now_ms: Option<u64>) {
        self.agents.clock = now_ms;
    }

    fn agents_now(&self) -> u64 {
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

    /// The switch with its label; where it was drawn.
    pub(super) fn agents_switch_row(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        let tokens = self.tokens;
        let on = self.agents.switch.unwrap_or(self.agents.status.allowed);
        let mut turned = false;
        let row = ui.horizontal(|ui| {
            let response = toggle(ui, &tokens, on).on_hover_text(if on {
                "Agents may drive this window over MCP. Click to turn it off and close their connections."
            } else {
                "Let AI agents drive this window over MCP while you watch."
            });
            let label = ui.add(egui::Label::new("Allow AI agents").sense(egui::Sense::click()));
            if response.clicked() || label.clicked() {
                self.request_agents(!on);
                turned = true;
            }
        });
        if turned {
            ui.ctx().request_repaint();
        }
        row.response.rect
    }

    /// The folder agents may read and write files in, with Change… and
    /// Open folder, and the check box for the open document's folder;
    /// their places, by name.
    pub(super) fn agents_folder_rows(&mut self, ui: &mut egui::Ui) -> Vec<(String, egui::Rect)> {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        let status = self.agents.status.clone();
        let home = status.home.as_deref();
        let mut hits = Vec::new();
        let primary: Vec<&crate::state::AgentRoot> = status
            .roots
            .iter()
            .filter(|root| root.from != "document")
            .collect();
        let (text, folder) = if status.command_line {
            let paths: Vec<String> = primary
                .iter()
                .map(|root| shown_path(&root.path, home))
                .collect();
            (
                format!(
                    "Agents can read and write files in: {} (set by the command line)",
                    paths.join(", ")
                ),
                primary.first().map(|root| root.path.clone()),
            )
        } else {
            match &status.work_folder {
                Some(folder) => (
                    format!(
                        "Agents can read and write files in: {}",
                        shown_path(folder, home)
                    ),
                    Some(folder.clone()),
                ),
                None => (
                    "Agents can read and write no files: there is no folder for them".to_string(),
                    None,
                ),
            }
        };
        #[cfg(target_arch = "wasm32")]
        let _ = folder;
        ui.colored_label(dim, text);
        ui.horizontal_wrapped(|ui| {
            let change = ui
                .add_enabled(!status.command_line, egui::Button::new("Change…").small())
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
                let open = ui.small_button("Open folder");
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
        let label = match status.roots.iter().find(|root| root.from == "document") {
            Some(root) => format!(
                "Also the open document's folder ({})",
                shown_path(&root.path, home)
            ),
            None => "Also the open document's folder".to_string(),
        };
        let check = ui.checkbox(&mut document, label).on_hover_text(
            "While a document saved on disk is open, agents may read and write files in its folder too",
        );
        hits.push(("document-folder".to_string(), check.rect));
        if check.changed() {
            self.request_agents_document_folder(document);
        }
        hits
    }

    /// The panel that connects agents, the same on the start screen and in
    /// the popover over the pill: the switch; while it is on the folders,
    /// what to copy into the MCP client, who is connected and Disconnect
    /// all. Returns where its controls were drawn, by name.
    pub(super) fn agents_panel(&mut self, ui: &mut egui::Ui) -> Vec<(String, egui::Rect)> {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        let mut hits = Vec::new();
        if cfg!(target_arch = "wasm32") {
            ui.colored_label(
                dim,
                "An agent cannot drive 3DCraft in the browser yet. Use the desktop app, whose window an agent drives over MCP while you watch.",
            );
            return hits;
        }
        let setup = self.agent_setup().clone();
        let status = self.agents.status.clone();
        let font = egui::FontId::monospace(11.5);
        // A line of code on a dark plate, cut at the plate's edge (the
        // whole line on hover); the program it starts in the info accent's
        // light tint.
        let plate = |ui: &mut egui::Ui, program: &str, text: &str| {
            let (head, rest) = match text.strip_prefix(program) {
                Some(rest) => (program, rest),
                None => ("", text),
            };
            let mut job = egui::text::LayoutJob::default();
            for (part, ink) in [(head, tokens.info_code), (rest, tokens.plate_text)] {
                job.append(
                    part,
                    0.0,
                    egui::TextFormat::simple(font.clone(), color(ink)),
                );
            }
            egui::Frame::new()
                .fill(color(tokens.plate_fill))
                .corner_radius(egui::CornerRadius::same(tokens.radius))
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(job).truncate()).on_hover_text(text);
                });
        };
        let switch = self.agents_switch_row(ui);
        hits.push(("agents-switch".to_string(), switch));
        if let Some(error) = &status.error {
            ui.colored_label(color(tokens.failed), error);
        }
        let how = |shell: &mut Self, ui: &mut egui::Ui| {
            if ui.link("How it works").clicked() {
                shell.open_external(
                    ui.ctx(),
                    super::help::External::Url(super::help::AGENT_SETUP_URL.to_string()),
                );
            }
        };
        if !status.allowed {
            ui.colored_label(
                dim,
                "Off: nothing can connect to this window. Turn it on to let an agent such as Claude model here over MCP while you watch.",
            );
            how(self, ui);
            return hits;
        }
        hits.extend(self.agents_folder_rows(ui));
        ui.add_space(tokens.inset * 0.3);
        ui.colored_label(dim, "Add 3DCraft to Claude Code with:");
        plate(ui, "claude", &setup.claude_command());
        ui.add_space(tokens.inset * 0.3);
        // Wrapped: on a phone the three do not fit on one line.
        ui.horizontal_wrapped(|ui| {
            let copy = ui
                .button("Copy command")
                .on_hover_text(setup.claude_command());
            hits.push(("copy-command".to_string(), copy.rect));
            if copy.clicked() {
                ui.ctx().copy_text(setup.claude_command());
            }
            let config = ui
                .button("Copy MCP configuration")
                .on_hover_text("For clients configured with JSON, such as Claude Desktop");
            hits.push(("copy-config".to_string(), config.rect));
            if config.clicked() {
                ui.ctx().copy_text(setup.mcp_config());
            }
            how(self, ui);
        });
        ui.add_space(tokens.inset * 0.3);
        let now = self.agents_now();
        if status.clients.is_empty() {
            ui.colored_label(dim, "No agent connected.");
        } else {
            for client in &status.clients {
                let last = match &client.last_command {
                    Some(command) => format!(" · last: {command}"),
                    None => String::new(),
                };
                ui.horizontal_wrapped(|ui| {
                    // A green dot, painted (the font has no "●").
                    let side = ui.text_style_height(&egui::TextStyle::Body);
                    let (dot, _) =
                        ui.allocate_exact_size(egui::vec2(side * 0.6, side), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(dot.center(), side * 0.22, color(tokens.ok));
                    ui.label(egui::RichText::new(&client.name).strong());
                    ui.colored_label(
                        dim,
                        format!("connected {}{last}", for_how_long(client.since_ms, now)),
                    );
                });
            }
            let all = ui.small_button("Disconnect all");
            hits.push(("disconnect-all".to_string(), all.rect));
            if all.clicked() {
                self.request_agents_disconnect();
            }
        }
        hits
    }

    /// The pill's size for `view`, and its text laid out.
    fn pill_layout(
        &self,
        ui: &egui::Ui,
        view: &PillView,
        most: f32,
    ) -> (egui::Vec2, std::sync::Arc<egui::Galley>) {
        let tokens = self.tokens;
        let (_, _, ink, dot) = pill_colours(view.kind, &tokens);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let side = font.size;
        let height = ui.spacing().interact_size.y - 2.0;
        let pad = height * 0.5;
        let dot_room = if dot.is_some() { side * 0.5 + 6.0 } else { 0.0 };
        let fixed = pad * 2.0 + side + 6.0 + dot_room;
        let mut job =
            egui::text::LayoutJob::simple_singleline(view.label.clone(), font, color(ink));
        job.wrap = egui::text::TextWrapping {
            max_width: (most - fixed).max(24.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = ui.painter().layout_job(job);
        (egui::vec2(fixed + galley.size().x, height), galley)
    }

    /// The pill's width for the room `most`, for the status bar to keep.
    pub(super) fn agents_pill_width(&self, ui: &egui::Ui, most: f32) -> f32 {
        let view = self.agents_pill_view();
        self.pill_layout(ui, &view, most).0.x
    }

    /// The pill at the status bar's left: how agents stand, and what one
    /// does now; a click opens the popover with the panel.
    pub(super) fn agents_pill(&mut self, ui: &mut egui::Ui, most: f32) {
        let tokens = self.tokens;
        let view = self.agents_pill_view();
        let (size, galley) = self.pill_layout(ui, &view, most);
        let (fill, outline, ink, dot) = pill_colours(view.kind, &tokens);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let response = response.on_hover_text(match view.kind {
            PillKind::Web => "AI agents drive 3DCraft through the desktop app; the browser cannot listen for them yet",
            PillKind::Off => "Let an AI agent model in this window while you watch",
            _ => "AI agents: who is connected, the folder they may use, how to connect one",
        });
        self.agents.pill = Some(rect);
        if response.clicked() && view.kind != PillKind::Web {
            self.state.dialogs.agents = !self.state.dialogs.agents;
        }
        let painter = ui.painter();
        let radius = egui::CornerRadius::same(tokens.pill_radius);
        let hovered = response.hovered() && view.kind != PillKind::Web;
        if let Some(fill) = fill {
            let fill = if hovered {
                color(fill).gamma_multiply(1.15)
            } else {
                color(fill)
            };
            painter.rect_filled(rect, radius, fill);
        }
        if let Some(outline) = outline {
            let width = if hovered { 1.6 } else { 1.0 };
            painter.rect_stroke(
                rect.shrink(0.5),
                radius,
                egui::Stroke::new(width, color(outline)),
                egui::StrokeKind::Inside,
            );
        }
        let font_size = egui::TextStyle::Body.resolve(ui.style()).size;
        let pad = rect.height() * 0.5;
        let mut x = rect.left() + pad;
        let icon = egui::Rect::from_center_size(
            egui::pos2(x + font_size * 0.5, rect.center().y),
            egui::vec2(font_size, font_size),
        );
        super::icons::paint(
            painter,
            super::icons::Icon::Agent,
            icon,
            color(ink),
            &tokens,
        );
        x += font_size + 6.0;
        if let Some(dot) = dot {
            let r = font_size * 0.25;
            let centre = egui::pos2(x + r, rect.center().y);
            // While an agent acts the dot pulses: a ring that grows and
            // fades (none where animations are off: reduced motion).
            let still = ui.style().animation_time <= 0.0;
            if view.kind == PillKind::Active && !still {
                let phase = (view.age_ms % 750) as f32 / 750.0;
                let ring = r * (1.0 + 1.6 * phase);
                let alpha = ((1.0 - phase) * 200.0) as u8;
                painter.circle_stroke(
                    centre,
                    ring,
                    egui::Stroke::new(1.5, color_alpha(dot, alpha)),
                );
            }
            painter.circle_filled(centre, r, color(dot));
            x += r * 2.0 + 6.0;
        }
        painter.galley(
            egui::pos2(x, rect.center().y - galley.size().y * 0.5),
            galley,
            color(ink),
        );
        // Drawn again while the activity shows (the pulse), then once when
        // it fades back; nothing while idle.
        if view.kind == PillKind::Active {
            if ui.style().animation_time > 0.0 {
                ui.ctx().request_repaint();
            } else {
                let left = ACTIVITY_MS.saturating_sub(view.age_ms);
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(left));
            }
        }
    }

    /// The popover over the pill, while `state.dialogs.agents` is on: the
    /// agents panel. Escape or a click elsewhere closes it.
    pub(super) fn agents_popover(&mut self, ctx: &egui::Context) {
        if !self.state.dialogs.agents || cfg!(target_arch = "wasm32") {
            self.agents.popover_hits.clear();
            return;
        }
        let tokens = self.tokens;
        let screen = ctx.content_rect();
        let anchor = self.agents.pill.unwrap_or(egui::Rect::from_min_size(
            egui::pos2(screen.left() + 8.0, screen.bottom() - 24.0),
            egui::vec2(120.0, 20.0),
        ));
        let width = tokens
            .agent_popover_width
            .min((screen.width() - 32.0).max(160.0));
        let left = anchor
            .left()
            .min(screen.right() - width - 24.0)
            .max(screen.left() + 8.0);
        let mut hits = Vec::new();
        let shown = egui::Area::new(egui::Id::new("craft3d-agents-popover"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(egui::pos2(left, anchor.top() - 6.0))
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(width);
                    ui.label(egui::RichText::new("Connect an AI agent").strong());
                    ui.add_space(tokens.inset * 0.3);
                    hits = self.agents_panel(ui);
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
    pub(super) fn agents_menu_item(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("Connect an AI agent…")
            .on_hover_text(
                "The agents panel over the status bar: the switch, the folder, who is connected",
            )
            .clicked()
        {
            self.state.dialogs.agents = true;
            ui.close();
        }
    }
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
        }
    }

    #[test]
    fn the_pill_says_each_state_in_its_own_look() {
        let tokens = Tokens::default();
        let mut status = AgentsStatus::default();
        let now = 1_000_000;
        assert_eq!(pill_view(&status, now, true).kind, PillKind::Web);
        assert_eq!(pill_view(&status, now, true).label, "Agents: desktop only");
        let off = pill_view(&status, now, false);
        assert_eq!(
            (off.kind, off.label.as_str()),
            (PillKind::Off, "Connect AI agent")
        );
        status.allowed = true;
        let waiting = pill_view(&status, now, false);
        assert_eq!(
            (waiting.kind, waiting.label.as_str()),
            (PillKind::Waiting, "Agents on — waiting")
        );
        status.clients = vec![client("Claude Code", Some("feature.edit"), now - 5_000)];
        let connected = pill_view(&status, now, false);
        assert_eq!(
            (connected.kind, connected.label.as_str()),
            (PillKind::Connected, "Claude Code connected")
        );
        status.clients.push(client("Cursor", None, 0));
        assert_eq!(pill_view(&status, now, false).label, "2 agents connected");
        // The colours: outlined indigo off, filled indigo on, a green dot
        // when connected; all from the tokens.
        let (fill, outline, text, dot) = pill_colours(PillKind::Off, &tokens);
        assert_eq!(
            (fill, outline, text, dot),
            (None, Some(tokens.info_code), tokens.info_code, None)
        );
        let (fill, outline, text, dot) = pill_colours(PillKind::Waiting, &tokens);
        assert_eq!(
            (fill, outline, text, dot),
            (Some(tokens.info_accent), None, tokens.plate_text, None)
        );
        let (fill, _, _, dot) = pill_colours(PillKind::Connected, &tokens);
        assert_eq!(
            (fill, dot),
            (Some(tokens.info_accent), Some(tokens.agent_dot))
        );
        assert_eq!(
            pill_colours(PillKind::Active, &tokens),
            pill_colours(PillKind::Connected, &tokens)
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
