//! "Allow AI agents": the switch that lets agents drive this window over
//! MCP, where the person finds it (the start screen, the Help menu) and
//! what the status bar says while it is on (docs/control-protocol.md,
//! "Connecting an agent").
//!
//! The shell only asks: the host owns the listener, turns it on and off
//! when the shell asks it to ([`Shell::take_agents_switch`],
//! [`Shell::take_agents_disconnect`]) and tells the shell how things stand
//! ([`Shell::set_agents_status`]), which `ui.get` reports under `agents`.

use super::Shell;
use crate::state::AgentsStatus;
use crate::theme::{Tokens, color};

/// What the switch remembers between frames.
#[derive(Default)]
pub(crate) struct AgentsMemory {
    pub(super) status: AgentsStatus,
    /// The switch was turned, until the host takes it.
    switch: Option<bool>,
    /// "Disconnect all" was clicked, until the host takes it.
    disconnect: bool,
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

    /// Asks the host to disconnect every agent, as the status bar's
    /// "Disconnect all" does.
    pub fn request_agents_disconnect(&mut self) {
        self.agents.disconnect = true;
    }

    pub fn take_agents_disconnect(&mut self) -> bool {
        std::mem::take(&mut self.agents.disconnect)
    }

    /// True while something the switch asked for waits for the host.
    pub(super) fn agents_pending(&self) -> bool {
        self.agents.switch.is_some() || self.agents.disconnect
    }

    /// The switch where it shows, with its label; where it was drawn.
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

    /// The Help menu's item: the same switch as a check box.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn agents_menu_item(&mut self, ui: &mut egui::Ui) {
        if !self.agents.status.available {
            return;
        }
        let mut on = self.agents.switch.unwrap_or(self.agents.status.allowed);
        let item = ui
            .checkbox(&mut on, "Allow AI agents")
            .on_hover_text("Let AI agents drive this window over MCP while you watch (the start screen says how to connect one)");
        if item.changed() {
            self.request_agents(on);
            ui.close();
        }
    }

    /// What the status bar says while agents are allowed: "Agents on", or
    /// how many are connected; `None` while they are not.
    pub(super) fn agents_status_text(&self) -> Option<String> {
        let status = &self.agents.status;
        if !status.available || !status.allowed {
            return None;
        }
        Some(match status.connected {
            0 => "Agents on".to_string(),
            1 => "● 1 agent connected".to_string(),
            n => format!("● {n} agents connected"),
        })
    }

    /// The status bar's agents item, a menu: who is connected, Disconnect
    /// all, and the switch.
    pub(super) fn agents_status_menu(&mut self, ui: &mut egui::Ui, text: &str) -> egui::Rect {
        let tokens = self.tokens;
        let status = self.agents.status.clone();
        let tint = if status.connected > 0 {
            tokens.chrome_ok
        } else {
            tokens.chrome_text_dim
        };
        let button = egui::Button::new(egui::RichText::new(text).color(color(tint))).frame(false);
        let response = ui.add(button).on_hover_text(match status.port {
            Some(port) => format!("AI agents may drive this window over MCP (port {port})"),
            None => "AI agents may drive this window over MCP".to_string(),
        });
        let rect = response.rect;
        egui::Popup::menu(&response).show(|ui| {
            if status.clients.is_empty() {
                ui.label("No agent is connected");
            } else {
                for client in &status.clients {
                    ui.label(client);
                }
            }
            ui.separator();
            if ui
                .add_enabled(status.connected > 0, egui::Button::new("Disconnect all"))
                .clicked()
            {
                self.request_agents_disconnect();
                ui.close();
            }
            if ui.button("Turn off AI agents").clicked() {
                self.request_agents(false);
                ui.close();
            }
        });
        if status.connected > 0 {
            // A client that goes quiet stops counting after a while: look
            // again now and then.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(10));
        }
        rect
    }
}
