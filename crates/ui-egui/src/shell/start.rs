//! The start screen, in the viewport while no document is open: New,
//! Open, the example models of the user guide, the recent files, and how
//! to connect an AI agent to this window over MCP.
//!
//! The shell only asks, as the File menu does: New and Open are file
//! actions ([`super::FileAction`]), a recent file is opened by the host
//! ([`Shell::take_open_recent`]), and an example is built by the host
//! with [`run_example`] ([`Shell::take_example`]), which runs the guide's
//! command script in a new document like any other commands.
//!
//! While it shows, the feature list is put away, so the welcome sits in
//! the middle of the window. The logo and the examples' thumbnails are
//! PNGs built into the program: the thumbnails are made by
//! `cargo xtask example-thumbs` (docs/development.md, Example
//! thumbnails), which a test keeps in step with the scripts.

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
    /// Its picture, 160 × 120 (`docs/examples/thumbs`).
    thumb: &'static [u8],
}

macro_rules! example {
    ($id:literal, $title:literal, $summary:literal) => {
        Example {
            id: $id,
            title: $title,
            summary: $summary,
            script: include_str!(concat!("../../../../docs/examples/", $id, ".json")),
            thumb: include_bytes!(concat!("../../../../docs/examples/thumbs/", $id, ".png")),
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

/// The logo: the application's icon (`packaging/icons`).
const LOGO: &[u8] = include_bytes!("../../../../packaging/icons/craft3d-128.png");

/// Largest side of a picture built into the program that is decoded.
const MAX_PICTURE_SIDE: u32 = 1024;

/// A PNG built into the program, as an image for a texture; `None` when
/// it does not read (the start screen then shows a plain tile).
pub fn decode_png(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0 || height == 0 || width > MAX_PICTURE_SIDE || height > MAX_PICTURE_SIDE {
        return None;
    }
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    buffer.truncate(info.buffer_size());
    let size = [usize::try_from(width).ok()?, usize::try_from(height).ok()?];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[r, g, b]| [*r, *g, *b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[v, a]| [*v, *v, *v, *a])
            .collect(),
        png::ColorType::Grayscale => buffer.iter().flat_map(|v| [*v, *v, *v, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    if rgba.len() != size[0].checked_mul(size[1])?.checked_mul(4)? {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
}

/// The logo and thumbnails as textures, made once.
pub(crate) struct Pictures {
    logo: Option<egui::TextureHandle>,
    thumbs: Vec<Option<egui::TextureHandle>>,
}

impl Pictures {
    fn load(ctx: &egui::Context) -> Self {
        let texture = |name: String, bytes: &[u8]| {
            decode_png(bytes)
                .map(|image| ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
        };
        Self {
            logo: texture("craft3d-logo".to_string(), LOGO),
            thumbs: EXAMPLES
                .iter()
                .map(|example| texture(format!("craft3d-example-{}", example.id), example.thumb))
                .collect(),
        }
    }
}

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
    /// The token file of the window, absolute, while it listens.
    pub token_file: Option<String>,
    /// Where a token file would go when the window does not listen yet:
    /// an absolute path the host picks (the user's configuration folder).
    /// Never `~`: an MCP client starts the server without a shell, so
    /// nothing would expand it.
    pub suggested_token: Option<String>,
    /// Quote command lines for Windows (`"…"`) rather than a POSIX shell.
    pub windows: bool,
}

impl Default for AgentSetup {
    fn default() -> Self {
        Self {
            cli: "craft3d-cli".to_string(),
            port: None,
            token_file: None,
            suggested_token: None,
            windows: cfg!(windows),
        }
    }
}

/// The port the guide uses when the window does not listen yet, and the
/// token file named when the host suggests none: a placeholder to fill
/// in, absolute in form.
const GUIDE_PORT: u16 = 7878;
const PLACEHOLDER_TOKEN: &str = "/path/to/craft3d-control.token";

/// `text` as one word of a command line: as it is when it needs no
/// quotes; else in single quotes for a POSIX shell (a `'` in it written
/// `'\''`), or in double quotes for Windows (where a path cannot hold a
/// `"`).
pub fn shell_word(text: &str, windows: bool) -> String {
    let plain = !text.is_empty()
        && text.chars().all(|c| {
            c.is_ascii_alphanumeric() || "_-./:=@+,".contains(c) || (windows && c == '\\')
        });
    if plain {
        text.to_string()
    } else if windows {
        format!("\"{}\"", text.replace('"', ""))
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

impl AgentSetup {
    /// True when this window listens for an agent now.
    pub fn listening(&self) -> bool {
        self.port.is_some() && self.token_file.is_some()
    }

    fn token(&self) -> &str {
        self.token_file
            .as_deref()
            .or(self.suggested_token.as_deref())
            .unwrap_or(PLACEHOLDER_TOKEN)
    }

    /// The MCP client configuration that bridges an agent to this window
    /// (docs/guide/01-install.md).
    pub fn mcp_config(&self) -> String {
        let port = self.port.unwrap_or(GUIDE_PORT).to_string();
        // Written by hand, so the command comes before its arguments and
        // the arguments stay on one line; the strings by serde, escaped.
        let text = |s: &str| Value::String(s.to_string()).to_string();
        format!(
            "{{\n  \"mcpServers\": {{\n    \"craft3d\": {{\n      \"command\": {},\n      \"args\": [\"mcp\", \"--bridge\", {}, \"--control-token-file\",\n               {}]\n    }}\n  }}\n}}",
            text(&self.cli),
            text(&port),
            text(self.token())
        )
    }

    /// The same as a command line for a client that adds servers that
    /// way (Claude Code), quoted for the platform.
    pub fn claude_command(&self) -> String {
        format!(
            "claude mcp add craft3d -- {} mcp --bridge {} --control-token-file {}",
            shell_word(&self.cli, self.windows),
            self.port.unwrap_or(GUIDE_PORT),
            shell_word(self.token(), self.windows)
        )
    }

    /// How to start the window so that it listens.
    pub fn window_command(&self) -> String {
        format!(
            "craft3d --control {GUIDE_PORT} --control-token-file {}",
            shell_word(self.token(), self.windows)
        )
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
    /// The logo and the thumbnails, once loaded.
    pictures: Option<Pictures>,
}

/// Widest the card is, in points.
const CARD_WIDTH: f32 = 760.0;
/// A thumbnail as the card shows it, in points, and the examples per row.
const THUMB: egui::Vec2 = egui::vec2(128.0, 96.0);
const PER_ROW: usize = 5;
/// The logo's side, in points.
const LOGO_SIDE: f32 = 64.0;

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

    /// True when the window shows the welcome: no document, no work
    /// running and no problem to show. The feature list is put away
    /// meanwhile, so the welcome sits in the middle of the window.
    pub(super) fn welcome(&self, has_document: bool) -> bool {
        !has_document && self.work.is_none() && self.problem.is_none()
    }

    pub(super) fn no_start_screen(&mut self) {
        self.start.shown = false;
        self.start.hits.clear();
    }

    /// The start screen over the viewport `rect`.
    pub(super) fn start_screen(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        if !self.start.shown {
            // The recent files are looked at again as the screen comes up.
            self.refresh_recent();
        }
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
        if self.start.pictures.is_none() {
            self.start.pictures = Some(Pictures::load(ui.ctx()));
        }
        card.show(ui, |ui| {
            ui.set_width(width - 48.0);
            ui.vertical_centered(|ui| {
                let logo = self.start.pictures.as_ref().and_then(|p| p.logo.as_ref());
                if let Some(logo) = logo {
                    ui.add(egui::Image::new((logo.id(), egui::vec2(LOGO_SIDE, LOGO_SIDE))));
                }
                ui.label(egui::RichText::new(super::PRODUCT).size(26.0).strong());
                ui.colored_label(dim, format!("Version {}", super::help::VERSION));
                ui.add_space(tokens.inset * 0.4);
                ui.colored_label(
                    dim,
                    "Parametric CAD built for AI. Start a document, open one, or let an agent model while you watch.",
                );
                ui.add_space(tokens.inset);
                // Centred: the two buttons laid out in a row of their own width.
                let size = egui::vec2(0.0, 30.0);
                let gold = egui::Button::new(
                    egui::RichText::new("New document").color(color(tokens.plate_fill)),
                )
                .fill(color(tokens.accent))
                .min_size(size);
                let open = egui::Button::new("Open…").min_size(size);
                let row = egui::vec2(240.0, 30.0);
                ui.allocate_ui_with_layout(
                    row,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let new = ui.add(gold);
                        self.start.hits.push(("new".to_string(), new.rect));
                        if new.clicked() {
                            self.request_file_action(FileAction::New);
                        }
                        let open = ui.add(open);
                        self.start.hits.push(("open".to_string(), open.rect));
                        if open.clicked() {
                            self.request_file_action(FileAction::Open);
                        }
                    },
                );
            });
            ui.add_space(tokens.inset * 1.4);
            ui.label(egui::RichText::new("Examples").strong());
            ui.add_space(tokens.inset * 0.4);
            self.example_grid(ui);
            ui.add_space(tokens.inset * 1.4);
            ui.separator();
            ui.add_space(tokens.inset * 0.6);
            let left = 200.0_f32.min(ui.available_width() * 0.3);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(left, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(left);
                        ui.label(egui::RichText::new("Recent").strong());
                        ui.add_space(tokens.inset * 0.3);
                        if cfg!(target_arch = "wasm32") {
                            ui.colored_label(dim, "The browser keeps no recent files.");
                        } else {
                            self.recent_links(ui);
                        }
                    },
                );
                ui.add_space(tokens.inset * 1.5);
                ui.vertical(|ui| self.agent_block(ui));
            });
        });
    }

    /// The examples as cards: the thumbnail and the name; a click builds
    /// it in a new document.
    fn example_grid(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let gap = tokens.inset;
        let label_height = 22.0;
        let card = egui::vec2(THUMB.x, THUMB.y + label_height);
        let per_row = {
            let fit = ((ui.available_width() + gap) / (card.x + gap)).floor();
            if fit.is_finite() && fit >= 1.0 {
                (fit as usize).min(PER_ROW)
            } else {
                1
            }
        };
        let thumbs: Vec<Option<egui::TextureId>> = self
            .start
            .pictures
            .as_ref()
            .map(|p| {
                p.thumbs
                    .iter()
                    .map(|t| t.as_ref().map(egui::TextureHandle::id))
                    .collect()
            })
            .unwrap_or_default();
        // The rows centred as a block: a short last row lines up with the
        // others on the left.
        let block = per_row as f32 * card.x + (per_row as f32 - 1.0) * gap;
        let pad = ((ui.available_width() - block) * 0.5).max(0.0);
        for (row, examples) in EXAMPLES.chunks(per_row).enumerate() {
            if row > 0 {
                ui.add_space(gap);
            }
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.spacing_mut().item_spacing.x = gap;
                for example in examples {
                    let index = EXAMPLES.iter().position(|e| e.id == example.id);
                    let (rect, response) = ui.allocate_exact_size(card, egui::Sense::click());
                    let response = response
                        .on_hover_text(format!("{}: {}", example.title, example.summary))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    let painter = ui.painter_at(rect.expand(2.0));
                    let image = egui::Rect::from_min_size(rect.min, THUMB);
                    let radius = egui::CornerRadius::same(tokens.radius);
                    painter.rect_filled(image, radius, color(tokens.plate_fill));
                    if let Some(Some(id)) = index.and_then(|i| thumbs.get(i)) {
                        painter.image(
                            *id,
                            image,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    if response.hovered() {
                        painter.rect_stroke(
                            image,
                            radius,
                            egui::Stroke::new(2.0, color(tokens.accent)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    painter.text(
                        egui::pos2(rect.min.x + 2.0, image.max.y + 4.0),
                        egui::Align2::LEFT_TOP,
                        example.title,
                        egui::FontId::proportional(13.0),
                        color(if response.hovered() {
                            tokens.text
                        } else {
                            tokens.text_dim
                        }),
                    );
                    self.start
                        .hits
                        .push((format!("example:{}", example.id), rect));
                    if response.clicked() {
                        self.choose_example(example);
                    }
                }
            });
        }
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
                    .font(egui::FontId::monospace(11.5))
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
    fn a_step_that_fails_closes_the_new_document_again() {
        let broken = Example {
            id: "broken",
            title: "Broken",
            summary: "a step that fails",
            thumb: &[],
            script: r#"[["feature.block", {"size": [1, 2, 3]}], ["feature.fillet", {"edges": "nope"}]]"#,
        };
        let mut session = Session::new(craft3d_engine::Workspace::none());
        session.execute("file.new", &json!({})).unwrap();
        let error = run_example(&mut session, &broken).unwrap_err();
        assert!(error.contains("step 1 (feature.fillet)"), "{error}");
        // The document that was open before stays, alone and active.
        assert_eq!(session.document_statuses().len(), 1);
        // A script that does not read opens nothing.
        let unreadable = Example {
            script: "{",
            ..broken
        };
        assert!(run_example(&mut session, &unreadable).is_err());
        let not_steps = Example {
            script: r#"{"steps": 1}"#,
            ..broken
        };
        assert!(run_example(&mut session, &not_steps).is_err());
        assert_eq!(session.document_statuses().len(), 1);
    }

    #[test]
    fn the_logo_and_every_thumbnail_decode() {
        let logo = decode_png(LOGO).unwrap();
        assert_eq!(logo.size, [128, 128]);
        for example in &EXAMPLES {
            let thumb = decode_png(example.thumb).unwrap();
            assert_eq!(thumb.size, [160, 120], "{}", example.id);
        }
        assert!(decode_png(b"").is_none());
        assert!(decode_png(b"not a png").is_none());
        let mut cut = LOGO.to_vec();
        cut.truncate(LOGO.len() / 2);
        assert!(decode_png(&cut).is_none());
    }

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
            suggested_token: None,
            windows: false,
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
            suggested_token: None,
            windows: true,
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
        // Never `~`: nothing expands it in an MCP client's configuration.
        assert!(!not.mcp_config().contains('~'));
        let suggested = AgentSetup {
            suggested_token: Some("/home/me/.config/craft3d-control.token".to_string()),
            windows: false,
            ..AgentSetup::default()
        };
        let config: Value = serde_json::from_str(&suggested.mcp_config()).unwrap();
        assert_eq!(
            config["mcpServers"]["craft3d"]["args"][4],
            "/home/me/.config/craft3d-control.token"
        );
    }

    #[test]
    fn command_lines_quote_paths_with_spaces_per_platform() {
        assert_eq!(
            shell_word("/opt/x/craft3d-cli", false),
            "/opt/x/craft3d-cli"
        );
        assert_eq!(shell_word("/home/a b/t", false), "'/home/a b/t'");
        assert_eq!(shell_word("it's", false), r"'it'\''s'");
        assert_eq!(shell_word("", false), "''");
        assert_eq!(
            shell_word(r"C:\Program Files\3DCraft\craft3d-cli.exe", true),
            r#""C:\Program Files\3DCraft\craft3d-cli.exe""#
        );
        assert_eq!(shell_word(r"C:\x\t.token", true), r"C:\x\t.token");
        let windows = AgentSetup {
            cli: r"C:\Program Files\3DCraft\craft3d-cli.exe".to_string(),
            port: Some(7878),
            token_file: Some(r"C:\Users\An Na\t.token".to_string()),
            suggested_token: None,
            windows: true,
        };
        assert_eq!(
            windows.claude_command(),
            r#"claude mcp add craft3d -- "C:\Program Files\3DCraft\craft3d-cli.exe" mcp --bridge 7878 --control-token-file "C:\Users\An Na\t.token""#
        );
        let posix = AgentSetup {
            cli: "/opt/3D Craft/craft3d-cli".to_string(),
            windows: false,
            suggested_token: Some("/home/an na/.config/craft3d-control.token".to_string()),
            ..AgentSetup::default()
        };
        assert!(
            posix
                .claude_command()
                .contains("-- '/opt/3D Craft/craft3d-cli' mcp")
        );
        assert!(
            posix
                .window_command()
                .ends_with("'/home/an na/.config/craft3d-control.token'")
        );
    }
}
