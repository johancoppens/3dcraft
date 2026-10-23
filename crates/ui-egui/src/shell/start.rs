//! The start screen, in the viewport while no document is open: New,
//! Open, the example models of the user guide, the recent files, and how
//! to connect an AI agent to this window over MCP.
//!
//! The shell only asks, as the File menu does: New and Open are file
//! actions ([`super::FileAction`]), a recent file is opened by the host
//! ([`Shell::take_open_recent`]), and an example is built by the host
//! with [`run_example`] ([`Shell::take_example`]), which runs the guide's
//! command script in a new document like any other commands.

use super::{FileAction, Shell};
use crate::theme::{color, color_alpha};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// One of the example models of the user guide (`docs/examples`): a
/// command script that builds it in a new document, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Example {
    /// The script's name, without `.json`.
    pub id: &'static str,
    pub title: &'static str,
    /// What it shows, in a few words.
    pub summary: &'static str,
    script: &'static str,
}

macro_rules! example {
    ($id:literal, $title:literal, $summary:literal) => {
        Example {
            id: $id,
            title: $title,
            summary: $summary,
            script: include_str!(concat!("../../../../docs/examples/", $id, ".json")),
        }
    };
}

/// The guide's examples, in the order its README lists them.
pub const EXAMPLES: [Example; 9] = [
    example!("bracket", "L-bracket", "extrude, fillet, holes"),
    example!("flange", "Bolt flange", "pattern, boolean, chamfer"),
    example!("case", "Shelled case", "fillet, shell"),
    example!("bushing", "Flanged bushing", "revolve, thread"),
    example!("pipe", "Bent pipe", "sweep along a path"),
    example!("spring", "Coil spring", "sweep along a helix"),
    example!("hopper", "Hopper", "loft"),
    example!("panel", "Perforated panel", "sketch pattern"),
    example!("wedge", "Wedge", "datums, fill, intersect"),
];

/// An example by its id.
pub fn example(id: &str) -> Option<&'static Example> {
    EXAMPLES.iter().find(|example| example.id == id)
}

/// Builds `example` in a new document (`file.new` in millimetres, then
/// its script's commands). A step that fails closes the new document
/// again and is the error.
pub fn run_example(session: &mut Session, example: &Example) -> Result<Value, String> {
    let script: Value =
        serde_json::from_str(example.script).map_err(|e| format!("example {}: {e}", example.id))?;
    let steps = craft3d_engine::task_score::parse_script(&script)
        .map_err(|e| format!("example {}: {}", example.id, e.message()))?;
    session
        .execute("file.new", &json!({ "units": "mm" }))
        .map_err(|e| format!("file.new: {}", e.message()))?;
    for (index, step) in steps.iter().enumerate() {
        let params = if step.params.is_null() {
            json!({})
        } else {
            step.params.clone()
        };
        if let Err(e) = session.execute(&step.command, &params) {
            let _ = session.execute("file.close", &json!({ "discard": true }));
            return Err(format!(
                "example {}: step {index} ({}): {}",
                example.id,
                step.command,
                e.message()
            ));
        }
    }
    Ok(json!({ "example": example.id, "steps": steps.len() }))
}

/// How an agent reaches this window: the `craft3d-cli` to start as the
/// MCP server, and the window's control port and token file when it
/// listens (`--control`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSetup {
    pub cli: String,
    pub port: Option<u16>,
    pub token_file: Option<String>,
}

impl Default for AgentSetup {
    fn default() -> Self {
        Self {
            cli: "craft3d-cli".to_string(),
            port: None,
            token_file: None,
        }
    }
}

/// The port and token file the guide uses when the window does not
/// listen yet.
const GUIDE_PORT: u16 = 7878;
const GUIDE_TOKEN: &str = "~/.config/craft3d-control.token";

impl AgentSetup {
    /// True when this window listens for an agent now.
    pub fn listening(&self) -> bool {
        self.port.is_some() && self.token_file.is_some()
    }

    /// The MCP client configuration that bridges an agent to this window
    /// (docs/guide/01-install.md).
    pub fn mcp_config(&self) -> String {
        let port = self.port.unwrap_or(GUIDE_PORT).to_string();
        let token = self.token_file.as_deref().unwrap_or(GUIDE_TOKEN);
        // Written by hand, so the command comes before its arguments and
        // the arguments stay on one line; the strings by serde, escaped.
        let text = |s: &str| Value::String(s.to_string()).to_string();
        format!(
            "{{\n  \"mcpServers\": {{\n    \"craft3d\": {{\n      \"command\": {},\n      \"args\": [\"mcp\", \"--bridge\", {}, \"--control-token-file\",\n               {}]\n    }}\n  }}\n}}",
            text(&self.cli),
            text(&port),
            text(token)
        )
    }

    /// The same as a command line for a client that adds servers that
    /// way (Claude Code).
    pub fn claude_command(&self) -> String {
        format!(
            "claude mcp add craft3d -- {} mcp --bridge {} --control-token-file {}",
            self.cli,
            self.port.unwrap_or(GUIDE_PORT),
            self.token_file.as_deref().unwrap_or(GUIDE_TOKEN)
        )
    }

    /// How to start the window so that it listens.
    pub fn window_command(&self) -> String {
        format!("craft3d --control {GUIDE_PORT} --control-token-file {GUIDE_TOKEN}")
    }
}

/// What the start screen remembers between frames.
#[derive(Default)]
pub(crate) struct StartMemory {
    /// The example chosen, until the host builds it.
    pub(super) chosen: Option<&'static Example>,
    agent: AgentSetup,
    /// The card's height in the last frame, to centre it in this one.
    height: f32,
    /// True when the last frame drew the start screen.
    shown: bool,
    /// Where the last frame drew its buttons and links: `new`, `open`,
    /// and `example:<id>` for each example.
    hits: Vec<(String, egui::Rect)>,
}

/// Widest the card is, in points.
const CARD_WIDTH: f32 = 640.0;

impl Shell {
    /// The example chosen on the start screen, for the host to build
    /// with [`run_example`].
    pub fn take_example(&mut self) -> Option<&'static Example> {
        self.start.chosen.take()
    }

    /// Chooses an example as a click on the start screen does.
    pub fn choose_example(&mut self, example: &'static Example) {
        self.start.chosen = Some(example);
    }

    /// How an agent reaches this window, for the start screen.
    pub fn set_agent_setup(&mut self, setup: AgentSetup) {
        self.start.agent = setup;
    }

    pub fn agent_setup(&self) -> &AgentSetup {
        &self.start.agent
    }

    /// True when the last frame showed the start screen.
    pub fn start_shown(&self) -> bool {
        self.start.shown
    }

    /// Where the start screen's buttons and links were drawn in the last
    /// frame, in points: `new`, `open`, `example:<id>`. Empty when it
    /// was not shown.
    pub fn start_hits(&self) -> &[(String, egui::Rect)] {
        &self.start.hits
    }

    pub(super) fn no_start_screen(&mut self) {
        self.start.shown = false;
        self.start.hits.clear();
    }

    /// The start screen over the viewport `rect`.
    pub(super) fn start_screen(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        self.start.shown = true;
        self.start.hits.clear();
        let tokens = self.tokens;
        let inner = rect.shrink(tokens.inset * 2.0);
        if inner.width() < 120.0 || inner.height() < 80.0 {
            return;
        }
        let width = CARD_WIDTH.min(inner.width());
        let top = (inner.center().y - self.start.height * 0.5).max(inner.top());
        let area = egui::Rect::from_min_max(
            egui::pos2(inner.center().x - width * 0.5, top),
            egui::pos2(inner.center().x + width * 0.5, inner.bottom()),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("craft3d-start")
                .max_rect(area)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.style_mut().visuals = tokens.panel_visuals();
        let height = egui::ScrollArea::vertical()
            .max_height(inner.height())
            .auto_shrink([false, true])
            .show(&mut child, |ui| self.start_card(ui, width))
            .content_size
            .y;
        if (height - self.start.height).abs() > 0.5 {
            self.start.height = height;
            ui.ctx().request_repaint();
        }
    }

    fn start_card(&mut self, ui: &mut egui::Ui, width: f32) {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        let card = egui::Frame::new()
            .fill(color(tokens.panel_fill))
            .stroke(egui::Stroke::new(
                1.0,
                color_alpha(tokens.text_dim, tokens.plate_rim_opacity.saturating_mul(2)),
            ))
            .corner_radius(egui::CornerRadius::same(tokens.nav_radius))
            .inner_margin(egui::Margin::same(24));
        card.show(ui, |ui| {
            ui.set_width(width - 48.0);
            ui.label(egui::RichText::new(super::PRODUCT).size(26.0).strong());
            ui.colored_label(
                dim,
                "Parametric CAD built for AI. Start a document, open one, or let an agent model while you watch.",
            );
            ui.add_space(tokens.inset * 1.4);
            ui.horizontal(|ui| {
                let size = egui::vec2(0.0, 30.0);
                let gold = egui::Button::new(
                    egui::RichText::new("New document").color(color(tokens.plate_fill)),
                )
                .fill(color(tokens.accent))
                .min_size(size);
                let new = ui.add(gold);
                self.start.hits.push(("new".to_string(), new.rect));
                if new.clicked() {
                    self.request_file_action(FileAction::New);
                }
                let open = ui.add(egui::Button::new("Open…").min_size(size));
                self.start.hits.push(("open".to_string(), open.rect));
                if open.clicked() {
                    self.request_file_action(FileAction::Open);
                }
            });
            ui.add_space(tokens.inset * 1.4);
            let half = (ui.available_width() - tokens.inset * 2.0) * 0.5;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(half, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(half);
                        ui.label(egui::RichText::new("Examples").strong());
                        ui.add_space(tokens.inset * 0.3);
                        for example in &EXAMPLES {
                            ui.horizontal(|ui| {
                                let link = ui.link(example.title);
                                self.start
                                    .hits
                                    .push((format!("example:{}", example.id), link.rect));
                                if link.clicked() {
                                    self.choose_example(example);
                                }
                                ui.colored_label(dim, example.summary);
                            });
                        }
                    },
                );
                ui.add_space(tokens.inset * 2.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(half, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(half);
                        ui.label(egui::RichText::new("Recent").strong());
                        ui.add_space(tokens.inset * 0.3);
                        if cfg!(target_arch = "wasm32") {
                            ui.colored_label(dim, "The browser keeps no recent files.");
                        } else {
                            self.recent_links(ui);
                        }
                    },
                );
            });
            ui.add_space(tokens.inset * 1.4);
            ui.separator();
            ui.add_space(tokens.inset * 0.6);
            self.agent_block(ui);
        });
    }

    fn agent_block(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        ui.label(egui::RichText::new("Connect an AI agent").strong());
        if cfg!(target_arch = "wasm32") {
            ui.colored_label(
                dim,
                "An agent cannot drive 3DCraft in the browser yet. Use the desktop app, whose window an agent drives over MCP while you watch.",
            );
            return;
        }
        let setup = self.start.agent.clone();
        if setup.listening() {
            ui.colored_label(
                dim,
                "This window listens for an agent. Give your MCP client this server, and the agent models here while you watch:",
            );
        } else {
            ui.colored_label(
                dim,
                format!(
                    "Start the window so that it listens ({}), then give your MCP client this server:",
                    setup.window_command()
                ),
            );
        }
        ui.add_space(tokens.inset * 0.4);
        let config = setup.mcp_config();
        let code = egui::Frame::new()
            .fill(color(tokens.plate_fill))
            .corner_radius(egui::CornerRadius::same(tokens.radius))
            .inner_margin(egui::Margin::same(10));
        code.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(&config)
                    .monospace()
                    .color(color(tokens.plate_text)),
            );
        });
        ui.add_space(tokens.inset * 0.4);
        ui.horizontal(|ui| {
            if ui.button("Copy configuration").clicked() {
                ui.ctx().copy_text(config.clone());
            }
            if ui
                .button("Copy for Claude Code")
                .on_hover_text(setup.claude_command())
                .clicked()
            {
                ui.ctx().copy_text(setup.claude_command());
            }
            if ui.link("How it works").clicked() {
                self.open_external(
                    ui.ctx(),
                    super::help::External::Url(super::help::AGENT_SETUP_URL.to_string()),
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_example_reads_as_a_script() {
        for example in &EXAMPLES {
            let script: Value = serde_json::from_str(example.script).unwrap();
            let steps = craft3d_engine::task_score::parse_script(&script).unwrap();
            assert!(!steps.is_empty(), "{}", example.id);
            assert_eq!(super::example(example.id), Some(example));
        }
        assert_eq!(super::example("nope"), None);
    }

    #[test]
    fn the_mcp_configuration_names_this_windows_port_and_token() {
        let listening = AgentSetup {
            cli: "/opt/3dcraft/craft3d-cli".to_string(),
            port: Some(7901),
            token_file: Some("/home/me/t.token".to_string()),
        };
        assert!(listening.listening());
        let config: Value = serde_json::from_str(&listening.mcp_config()).unwrap();
        assert_eq!(
            config["mcpServers"]["craft3d"]["args"],
            json!([
                "mcp",
                "--bridge",
                "7901",
                "--control-token-file",
                "/home/me/t.token"
            ])
        );
        assert_eq!(
            config["mcpServers"]["craft3d"]["command"],
            "/opt/3dcraft/craft3d-cli"
        );
        assert!(listening.claude_command().contains("--bridge 7901"));
        // Paths with quotes and backslashes stay valid JSON.
        let odd = AgentSetup {
            cli: r"C:\3D Craft\craft3d-cli.exe".to_string(),
            port: Some(1),
            token_file: Some(r#"C:\a "b".token"#.to_string()),
        };
        let config: Value = serde_json::from_str(&odd.mcp_config()).unwrap();
        assert_eq!(
            config["mcpServers"]["craft3d"]["args"][4],
            r#"C:\a "b".token"#
        );
        let not = AgentSetup::default();
        assert!(!not.listening());
        assert!(not.mcp_config().contains("7878"));
        assert!(not.window_command().contains("--control 7878"));
    }
}
