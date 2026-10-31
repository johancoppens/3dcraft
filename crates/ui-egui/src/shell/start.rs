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
use crate::theme::{Tokens, color, color_alpha};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// A section of the start screen's examples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExampleSection {
    /// The guide's example parts, built by their scripts.
    #[default]
    Parts,
    /// Published Jigs, opened on their parameters.
    Jigs,
    /// Packed assemblies, opened ready to play.
    Assemblies,
    /// Showcase parts built by their scripts: class-A surfaces with their
    /// proofs (the fuel tank, the mouse) and sheet metal with its press
    /// brake (the mini-PC chassis), 2026-10-31.
    Showcase,
}

impl ExampleSection {
    /// Every section, in the start screen's order.
    pub const ALL: [ExampleSection; 4] = [
        ExampleSection::Parts,
        ExampleSection::Showcase,
        ExampleSection::Jigs,
        ExampleSection::Assemblies,
    ];

    pub fn title(self) -> &'static str {
        match self {
            ExampleSection::Parts => "Parts",
            ExampleSection::Jigs => "Jigs",
            ExampleSection::Assemblies => "Assemblies",
            ExampleSection::Showcase => "Showcase",
        }
    }

    /// The word for the section in a hit (`section:<word>`).
    pub fn word(self) -> &'static str {
        match self {
            ExampleSection::Parts => "parts",
            ExampleSection::Jigs => "jigs",
            ExampleSection::Assemblies => "assemblies",
            ExampleSection::Showcase => "showcase",
        }
    }
}

/// How an example is made in a new document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A command script of the guide (`docs/examples/<id>.json`), run in a
    /// new document in millimetres.
    Script(&'static str),
    /// A published file built into the program
    /// (`docs/examples/published`), opened as it is, without a file of its
    /// own (docs/decisions.md, 2026-10-30).
    File {
        name: &'static str,
        bytes: &'static [u8],
    },
}

impl Example {
    /// A published file that is not built into this program (the web
    /// page's): its file name, for the host to fetch from `examples/` beside
    /// the page and open there. `None` for a script or a built-in file.
    pub fn to_fetch(&self) -> Option<&'static str> {
        match self.source {
            Source::File { name, bytes: [] } => Some(name),
            _ => None,
        }
    }

    /// The fonts a script example's texts are set in, each once, in order:
    /// the `font` of every `sketch.text…` step, `sans` (the commands'
    /// default) for a `sketch.text` or `sketch.textOnCurve` without one. A
    /// host whose fonts come later (the web page fetches them) wants these
    /// before it runs the script. Empty for a published file, and for a
    /// script that cannot be read (running it says why).
    pub fn fonts(&self) -> Vec<String> {
        let Source::Script(text) = self.source else {
            return Vec::new();
        };
        let Ok(Value::Array(steps)) = serde_json::from_str::<Value>(text) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for step in &steps {
            let Some(command) = step.get(0).and_then(Value::as_str) else {
                continue;
            };
            if !command.starts_with("sketch.text") {
                continue;
            }
            let font = match step
                .get(1)
                .and_then(|p| p.get("font"))
                .and_then(Value::as_str)
            {
                Some(font) => font,
                None if matches!(command, "sketch.text" | "sketch.textOnCurve") => "sans",
                None => continue,
            };
            if !out.iter().any(|f| f == font) {
                out.push(font.to_string());
            }
        }
        out
    }
}

/// One of the start screen's examples: a part of the user guide
/// (`docs/examples`), or a published Jig or assembly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Example {
    /// The script's name, without `.json`, or the published file's id.
    pub id: &'static str,
    pub title: &'static str,
    /// What it shows, in a few words.
    pub summary: &'static str,
    pub section: ExampleSection,
    source: Source,
    /// Its picture, 160 × 120 (`docs/examples/thumbs`).
    thumb: &'static [u8],
}

macro_rules! example {
    ($id:literal, $title:literal, $summary:literal) => {
        example!(ExampleSection::Parts, $id, $title, $summary)
    };
    ($section:expr, $id:literal, $title:literal, $summary:literal) => {
        Example {
            id: $id,
            title: $title,
            summary: $summary,
            section: $section,
            source: Source::Script(include_str!(concat!(
                "../../../../docs/examples/",
                $id,
                ".json"
            ))),
            thumb: include_bytes!(concat!("../../../../docs/examples/thumbs/", $id, ".png")),
        }
    };
}

macro_rules! published {
    ($section:expr, $id:literal, $file:literal, $title:literal, $summary:literal) => {
        Example {
            id: $id,
            title: $title,
            summary: $summary,
            section: $section,
            source: Source::File {
                name: $file,
                // Built into the desktop programs; the web page fetches
                // the file beside it when its card is clicked
                // (`examples/<file>`), so the module stays small.
                #[cfg(not(target_arch = "wasm32"))]
                bytes: include_bytes!(concat!("../../../../docs/examples/published/", $file)),
                #[cfg(target_arch = "wasm32")]
                bytes: &[],
            },
            thumb: include_bytes!(concat!("../../../../docs/examples/thumbs/", $id, ".png")),
        }
    };
}

/// The start screen's examples, section by section: ten of the guide's
/// parts, then the published Jigs and assemblies.
pub const EXAMPLES: [Example; 19] = [
    example!("bracket", "L-bracket", "extrude, fillet, holes"),
    example!("nameplate", "Name plate", "text, fillet, materials"),
    example!("clamp", "Clamp block", "fillet, bore, chamfer"),
    example!("channel", "Sheet-metal bracket", "sheet metal, bends"),
    example!("fin", "G2 fin", "surface network, curvature seam"),
    example!("shade", "Lamp shade", "developable, plate, wall lamp"),
    example!("flange", "Bolt flange", "pattern, boolean, chamfer"),
    example!("case", "Shelled case", "fillet, shell"),
    example!("spring", "Coil spring", "sweep along a helix"),
    example!("gear", "Involute gear", "curves from formulas, pattern"),
    example!(
        ExampleSection::Showcase,
        "tank",
        "Fuel tank",
        "class-A surfaces, G2 seams, litres"
    ),
    example!(
        ExampleSection::Showcase,
        "mouse",
        "Class-A mouse",
        "G2 top on flat flanks, sewn"
    ),
    example!(
        ExampleSection::Showcase,
        "chassis",
        "Mini-PC chassis",
        "sheet metal: louvres, hem, jog"
    ),
    published!(
        ExampleSection::Jigs,
        "jig-planetary",
        "planetary-gearbox.3dcraftjig",
        "Planetary gearbox",
        "a Jig: module and teeth"
    ),
    published!(
        ExampleSection::Jigs,
        "jig-reducer",
        "gear-reducer.3dcraftjig",
        "Gear reducer",
        "a Jig: a ratio and its gears"
    ),
    published!(
        ExampleSection::Jigs,
        "jig-strandbeest",
        "strandbeest-leg.3dcraftjig",
        "Jansen linkage leg",
        "a Jig after the mechanism of Theo Jansen's Strandbeests"
    ),
    published!(
        ExampleSection::Assemblies,
        "asm-planetary",
        "planetary-gearbox.3dcraftasm",
        "Planetary gearbox",
        "an assembly: gears that turn"
    ),
    published!(
        ExampleSection::Assemblies,
        "asm-reducer",
        "gear-reducer.3dcraftasm",
        "Gear reducer",
        "an assembly: two stages"
    ),
    published!(
        ExampleSection::Assemblies,
        "asm-walker",
        "strandbeest-walker.3dcraftasm",
        "Jansen linkage walker",
        "an assembly of four legs, after the mechanism of Theo Jansen's Strandbeests"
    ),
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
    let text = match example.source {
        Source::Script(text) => text,
        Source::File { name, bytes } => {
            let opened = session
                .open_bytes(name, bytes)
                .map_err(|e| format!("example {}: {}", example.id, e.message()))?;
            return Ok(json!({ "example": example.id, "opened": opened }));
        }
    };
    let script: Value =
        serde_json::from_str(text).map_err(|e| format!("example {}: {e}", example.id))?;
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
    /// The program the MCP client starts, absolute where it is known.
    pub cli: String,
    /// Words before `mcp` on its command line: `["cli"]` when `cli` is
    /// the AppImage, which runs the command line that way.
    pub cli_prefix: Vec<String>,
    /// The window's own program, absolute where it is known (on Linux
    /// the AppImage itself, never its temporary mount).
    pub window: String,
    /// The folder the agent may read and write files in, absolute (the
    /// user's `Documents/3DCraft`; the window makes it when it starts).
    pub work_folder: Option<String>,
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
            cli_prefix: Vec::new(),
            window: "craft3d".to_string(),
            work_folder: None,
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
const PLACEHOLDER_WORK: &str = "/path/to/3DCraft";

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
        let prefix: String = self
            .cli_prefix
            .iter()
            .map(|word| format!("{}, ", text(word)))
            .collect();
        format!(
            "{{\n  \"mcpServers\": {{\n    \"craft3d\": {{\n      \"command\": {},\n      \"args\": [{prefix}\"mcp\", \"--bridge\", {}, \"--control-token-file\",\n               {}]\n    }}\n  }}\n}}",
            text(&self.cli),
            text(&port),
            text(self.token())
        )
    }

    /// The same as a command line for a client that adds servers that
    /// way (Claude Code), quoted for the platform.
    pub fn claude_command(&self) -> String {
        format!("claude mcp add craft3d -- {}", self.server_command())
    }

    /// The MCP server's command line, quoted for the platform: what the
    /// configuration starts.
    pub fn server_command(&self) -> String {
        let prefix: String = self
            .cli_prefix
            .iter()
            .map(|word| format!(" {}", shell_word(word, self.windows)))
            .collect();
        format!(
            "{}{prefix} mcp --bridge {} --control-token-file {}",
            shell_word(&self.cli, self.windows),
            self.port.unwrap_or(GUIDE_PORT),
            shell_word(self.token(), self.windows)
        )
    }

    /// How to start the window so that it listens, with the folder the
    /// agent may read and write files in.
    pub fn window_command(&self) -> String {
        let work = shell_word(
            self.work_folder.as_deref().unwrap_or(PLACEHOLDER_WORK),
            self.windows,
        );
        format!(
            "{} --control {} --control-token-file {} --automation-read-root {work} --automation-write-root {work}",
            shell_word(&self.window, self.windows),
            self.port.unwrap_or(GUIDE_PORT),
            shell_word(self.token(), self.windows)
        )
    }
}

/// What the start screen remembers between frames.
#[derive(Default)]
pub(crate) struct StartMemory {
    /// The example chosen, until the host builds it.
    pub(super) chosen: Option<&'static Example>,
    /// The section of examples shown.
    section: ExampleSection,
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
    /// The layout of the last frame.
    layout: Option<StartLayout>,
}

/// How the card is laid out for the room the window gives it: roomy
/// (a large logo, large thumbnails five to a row) or, below
/// [`ROOMY_HEIGHT`], compact (a smaller logo, small thumbnails, also five
/// to a row: two rows for the ten examples), so it fits without scrolling
/// down to 1024 × 700 and 1366 × 768.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartLayout {
    /// The card's width, in points.
    pub width: f32,
    pub logo: f32,
    pub thumb: egui::Vec2,
    pub per_row: usize,
    /// The space between the card's parts, in units of `Tokens::inset`
    /// (less in the compact layout).
    pub spacing: f32,
}

/// The least height (of the room for the card, in points) for the roomy
/// layout.
pub const ROOMY_HEIGHT: f32 = 760.0;

impl StartLayout {
    /// The layout for room of `size` points.
    pub fn for_room(size: egui::Vec2) -> Self {
        if size.y >= ROOMY_HEIGHT {
            Self {
                width: size.x.min(880.0),
                logo: 64.0,
                thumb: egui::vec2(128.0, 96.0),
                per_row: 5,
                spacing: 1.0,
            }
        } else {
            Self {
                width: size.x.min(980.0),
                logo: 40.0,
                thumb: egui::vec2(96.0, 72.0),
                per_row: 5,
                spacing: 0.6,
            }
        }
    }
}

impl Shell {
    /// The example chosen on the start screen, for the host to build
    /// with [`run_example`].
    pub fn take_example(&mut self) -> Option<&'static Example> {
        self.start.chosen.take()
    }

    /// The example chosen, without taking it.
    pub fn peek_example(&self) -> Option<&'static Example> {
        self.start.chosen
    }

    /// Chooses an example as a click on the start screen does.
    pub fn choose_example(&mut self, example: &'static Example) {
        // An assembly opens ready to play: the player on its motion.
        if example.section == ExampleSection::Assemblies {
            self.state.timeline.motion = true;
        }
        self.start.chosen = Some(example);
    }

    /// The section of examples the start screen shows.
    pub fn start_section(&self) -> ExampleSection {
        self.start.section
    }

    /// Shows a section of examples, as a click on its name does.
    pub fn show_section(&mut self, section: ExampleSection) {
        self.start.section = section;
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

    /// True when the window shows the welcome: no document (as of the
    /// last sync), no work running and no problem to show (the scene's or
    /// the picture's). One test for both: the feature list is put away
    /// meanwhile, so the welcome sits in the middle of the window.
    pub(super) fn welcome(&self) -> bool {
        self.shown_status.is_none() && self.work.is_none() && self.problem().is_none()
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
        let layout = StartLayout::for_room(inner.size());
        self.start.layout = Some(layout);
        let width = layout.width;
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
        // Less space between the parts in the compact layout.
        let space = tokens.inset * self.start.layout.map_or(1.0, |l| l.spacing);
        let shown = card.show(ui, |ui| {
            ui.set_width(width - 48.0);
            ui.vertical_centered(|ui| {
                let side = self.start.layout.map_or(64.0, |l| l.logo);
                let logo = self.start.pictures.as_ref().and_then(|p| p.logo.as_ref());
                if let Some(logo) = logo {
                    ui.add(egui::Image::new((logo.id(), egui::vec2(side, side))));
                }
                ui.label(egui::RichText::new(super::PRODUCT).size(26.0).strong());
                ui.colored_label(dim, format!("Version {}", super::help::VERSION));
                ui.add_space(space * 0.4);
                ui.colored_label(
                    dim,
                    "Parametric CAD built for AI. Start a document, open one, or let an agent model while you watch.",
                );
                ui.add_space(space);
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
            ui.add_space(space * 1.4);
            self.section_row(ui);
            ui.add_space(space * 0.4);
            self.example_grid(ui);
            ui.add_space(space * 1.4);
            ui.separator();
            ui.add_space(space * 0.6);
            // Side by side; on a narrow card (a phone) one under the other,
            // so nothing runs past the card's edge.
            let stacked = width < tokens.start_stacked_width;
            let left = if stacked {
                ui.available_width()
            } else {
                200.0_f32.min(ui.available_width() * 0.3)
            };
            let row = if stacked {
                egui::Layout::top_down(egui::Align::Min)
            } else {
                egui::Layout::left_to_right(egui::Align::Min)
            };
            ui.with_layout(row, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(left, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(left);
                        heading(ui, &tokens, "Recent");
                        ui.add_space(tokens.inset * 0.3);
                        if cfg!(target_arch = "wasm32") {
                            ui.colored_label(dim, "The browser keeps no recent files.");
                        } else {
                            self.recent_links(ui);
                        }
                    },
                );
                ui.add_space(tokens.inset * if stacked { 0.8 } else { 1.5 });
                // The agent block with the info accent's bar beside it.
                let pad = (tokens.info_bar_width + tokens.inset * 0.9).round();
                let block = egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: pad.clamp(0.0, 64.0) as i8,
                        ..egui::Margin::ZERO
                    })
                    .show(ui, |ui| {
                        ui.vertical(|ui| self.agent_block(ui));
                    });
                let rect = block.response.rect;
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        rect.min,
                        egui::vec2(tokens.info_bar_width, rect.height()),
                    ),
                    egui::CornerRadius::same(tokens.info_bar_radius),
                    color(tokens.info_accent),
                );
            });
        });
        // The whole card, for the test that it stays inside the window.
        self.start
            .hits
            .push(("card".to_string(), shown.response.rect));
    }

    /// "Examples" and the names of the sections that have examples, one
    /// of them chosen; a click shows that section's cards. A single
    /// section shows only the heading.
    fn section_row(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let sections: Vec<ExampleSection> = ExampleSection::ALL
            .into_iter()
            .filter(|section| EXAMPLES.iter().any(|e| e.section == *section))
            .collect();
        if !sections.contains(&self.start.section) {
            self.start.section = sections.first().copied().unwrap_or_default();
        }
        ui.horizontal_wrapped(|ui| {
            heading(ui, &tokens, "Examples");
            if sections.len() < 2 {
                return;
            }
            ui.add_space(tokens.inset);
            for section in sections {
                let chosen = section == self.start.section;
                let text = egui::RichText::new(section.title()).color(color(if chosen {
                    tokens.text
                } else {
                    tokens.text_dim
                }));
                let response = ui.selectable_label(chosen, text);
                self.start
                    .hits
                    .push((format!("section:{}", section.word()), response.rect));
                if response.clicked() {
                    self.start.section = section;
                }
            }
        });
    }

    /// The examples of the section shown as cards: the thumbnail and the
    /// name; a click builds or opens it in a new document.
    fn example_grid(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let gap = tokens.inset;
        let layout = self
            .start
            .layout
            .unwrap_or_else(|| StartLayout::for_room(egui::vec2(760.0, ROOMY_HEIGHT)));
        let thumb = layout.thumb;
        let gap = if layout.thumb.x < 110.0 {
            gap * 0.8
        } else {
            gap
        };
        let label_height = 20.0;
        let card = egui::vec2(thumb.x, thumb.y + label_height);
        let per_row = {
            let fit = ((ui.available_width() + gap) / (card.x + gap)).floor();
            if fit.is_finite() && fit >= 1.0 {
                (fit as usize).min(layout.per_row)
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
        let section = self.start.section;
        let shown: Vec<&'static Example> =
            EXAMPLES.iter().filter(|e| e.section == section).collect();
        for (row, examples) in shown.chunks(per_row).enumerate() {
            if row > 0 {
                ui.add_space(gap);
            }
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.spacing_mut().item_spacing.x = gap;
                for &example in examples {
                    let index = EXAMPLES.iter().position(|e| e.id == example.id);
                    let (rect, response) = ui.allocate_exact_size(card, egui::Sense::click());
                    let response = response
                        .on_hover_text(format!("{}: {}", example.title, example.summary))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    let painter = ui.painter_at(rect.expand(2.0));
                    let image = egui::Rect::from_min_size(rect.min, thumb);
                    let radius = egui::CornerRadius::same(tokens.thumb_radius);
                    if let Some(Some(id)) = index.and_then(|i| thumbs.get(i)) {
                        egui::Image::new((*id, thumb))
                            .corner_radius(radius)
                            .paint_at(ui, image);
                    } else {
                        painter.rect_filled(image, radius, color(tokens.viewport_background));
                    }
                    // The ring of the info accent: a click here opens, it
                    // does not act on the document.
                    if response.hovered() || response.has_focus() {
                        painter.rect_stroke(
                            image,
                            radius,
                            egui::Stroke::new(2.0, color(tokens.info_accent)),
                            egui::StrokeKind::Outside,
                        );
                    }
                    // One row, cut with an ellipsis at the card's width
                    // (the whole title is in the tooltip).
                    let mut job = egui::text::LayoutJob::single_section(
                        example.title.to_string(),
                        egui::TextFormat::simple(
                            egui::FontId::proportional(if thumb.x < 110.0 { 12.0 } else { 13.0 }),
                            color(if response.hovered() {
                                tokens.text
                            } else {
                                tokens.text_dim
                            }),
                        ),
                    );
                    job.wrap =
                        egui::text::TextWrapping::truncate_at_width((rect.width() - 2.0).max(1.0));
                    painter.galley(
                        egui::pos2(rect.min.x + 2.0, image.max.y + 3.0),
                        painter.layout_job(job),
                        egui::Color32::PLACEHOLDER,
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
        heading(ui, &tokens, "Connect an AI agent");
        if cfg!(target_arch = "wasm32") {
            ui.colored_label(
                dim,
                "An agent cannot drive 3DCraft in the browser yet. Use the desktop app, whose window an agent drives over MCP while you watch.",
            );
            return;
        }
        let setup = self.start.agent.clone();
        let font = egui::FontId::monospace(11.5);
        let mono = |text: &str| {
            egui::RichText::new(text)
                .font(font.clone())
                .color(color(tokens.plate_text))
        };
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
        if !setup.listening() {
            ui.colored_label(dim, "Start the window so that it listens:");
            plate(ui, &setup.window, &setup.window_command());
            ui.add_space(tokens.inset * 0.3);
        }
        ui.colored_label(
            dim,
            if setup.listening() {
                "This window listens. Give your MCP client this server, and the agent models here while you watch:"
            } else {
                "Then give your MCP client this server:"
            },
        );
        plate(
            ui,
            &shell_word(&setup.cli, setup.windows),
            &setup.server_command(),
        );
        let config = setup.mcp_config();
        egui::CollapsingHeader::new("The configuration as JSON")
            .id_salt("craft3d-start-config")
            .default_open(false)
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(color(tokens.plate_fill))
                    .corner_radius(egui::CornerRadius::same(tokens.radius))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(mono(&config));
                    });
            });
        ui.add_space(tokens.inset * 0.4);
        // Wrapped: on a phone the three do not fit on one line.
        ui.horizontal_wrapped(|ui| {
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

/// A heading of the start screen, with the info accent's bar before it.
fn heading(ui: &mut egui::Ui, tokens: &Tokens, text: &str) {
    ui.horizontal(|ui| {
        let height = egui::TextStyle::Body.resolve(ui.style()).size;
        let (bar, _) = ui.allocate_exact_size(
            egui::vec2(tokens.info_bar_width, height),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(
            bar,
            egui::CornerRadius::same(tokens.info_bar_radius),
            color(tokens.info_accent),
        );
        ui.label(egui::RichText::new(text).strong());
    });
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
            section: ExampleSection::Parts,
            thumb: &[],
            source: Source::Script(
                r#"[["feature.block", {"size": [1, 2, 3]}], ["feature.fillet", {"edges": "nope"}]]"#,
            ),
        };
        let mut session = Session::new(craft3d_engine::Workspace::none());
        session.execute("file.new", &json!({})).unwrap();
        let error = run_example(&mut session, &broken).unwrap_err();
        assert!(error.contains("step 1 (feature.fillet)"), "{error}");
        // The document that was open before stays, alone and active.
        assert_eq!(session.document_statuses().len(), 1);
        // A script that does not read opens nothing.
        let unreadable = Example {
            source: Source::Script("{"),
            ..broken
        };
        assert!(run_example(&mut session, &unreadable).is_err());
        let not_steps = Example {
            source: Source::Script(r#"{"steps": 1}"#),
            ..broken
        };
        assert!(run_example(&mut session, &not_steps).is_err());
        // A published file that is no document, or not what its name
        // says, opens nothing either.
        for (name, bytes) in [
            ("bad.3dcraftjig", &b"not a zip"[..]),
            ("bad.3dcraftasm", &b""[..]),
        ] {
            let bad = Example {
                source: Source::File { name, bytes },
                ..broken
            };
            assert!(run_example(&mut session, &bad).is_err());
        }
        assert_eq!(session.document_statuses().len(), 1);
    }

    #[test]
    fn a_short_window_gets_the_compact_layout() {
        let roomy = StartLayout::for_room(egui::vec2(1500.0, 900.0));
        assert_eq!((roomy.per_row, roomy.logo), (5, 64.0));
        assert_eq!(roomy.width, 880.0);
        for room in [
            egui::vec2(1240.0, 716.0),
            egui::vec2(1326.0, 684.0),
            egui::vec2(984.0, 616.0),
        ] {
            let compact = StartLayout::for_room(room);
            // At most two rows of five per section.
            assert_eq!(compact.per_row, 5);
            for section in ExampleSection::ALL {
                let count = EXAMPLES.iter().filter(|e| e.section == section).count();
                assert!(count.div_ceil(compact.per_row) <= 2, "{section:?}");
            }
            assert!(compact.width <= room.x);
            assert!(compact.thumb.y < roomy.thumb.y && compact.logo < roomy.logo);
            assert!(compact.spacing < roomy.spacing);
        }
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
    fn every_example_reads_as_a_script_or_opens() {
        for example in &EXAMPLES {
            match example.source {
                Source::Script(text) => {
                    let script: Value = serde_json::from_str(text).unwrap();
                    let steps = craft3d_engine::task_score::parse_script(&script).unwrap();
                    assert!(!steps.is_empty(), "{}", example.id);
                    // Scripts are parts: plain ones under Parts, the
                    // class-A and sheet-metal showcases under Showcase.
                    assert!(
                        matches!(
                            example.section,
                            ExampleSection::Parts | ExampleSection::Showcase
                        ),
                        "{}",
                        example.id
                    );
                }
                Source::File { name, .. } => {
                    let mut session = Session::new(craft3d_engine::Workspace::none());
                    let opened = run_example(&mut session, example)
                        .unwrap_or_else(|e| panic!("{}: {e}", example.id));
                    let jig = name.ends_with(".3dcraftjig");
                    assert_eq!(opened["opened"]["jigInUse"] == true, jig, "{opened}");
                    assert_eq!(opened["opened"]["assembly"] == true, !jig, "{opened}");
                    assert_eq!(
                        example.section,
                        if jig {
                            ExampleSection::Jigs
                        } else {
                            ExampleSection::Assemblies
                        }
                    );
                }
            }
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
            ..AgentSetup::default()
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
            ..AgentSetup::default()
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
            ..AgentSetup::default()
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
                .contains("--control-token-file '/home/an na/.config/craft3d-control.token' ")
        );
    }

    #[test]
    fn the_window_command_names_the_programs_and_the_agents_folder() {
        // The AppImage: its own path for the window, and `cli` before
        // `mcp` for the server, in the configuration and on the line.
        let appimage = AgentSetup {
            cli: "/home/me/Apps/3DCraft-0.1.0-x86_64.AppImage".to_string(),
            cli_prefix: vec!["cli".to_string()],
            window: "/home/me/Apps/3DCraft-0.1.0-x86_64.AppImage".to_string(),
            work_folder: Some("/home/me/Documents/3DCraft".to_string()),
            suggested_token: Some("/home/me/.config/craft3d-control.token".to_string()),
            windows: false,
            ..AgentSetup::default()
        };
        assert_eq!(
            appimage.window_command(),
            "/home/me/Apps/3DCraft-0.1.0-x86_64.AppImage --control 7878 --control-token-file /home/me/.config/craft3d-control.token --automation-read-root /home/me/Documents/3DCraft --automation-write-root /home/me/Documents/3DCraft"
        );
        let config: Value = serde_json::from_str(&appimage.mcp_config()).unwrap();
        assert_eq!(config["mcpServers"]["craft3d"]["args"][0], "cli");
        assert_eq!(config["mcpServers"]["craft3d"]["args"][1], "mcp");
        assert!(
            appimage
                .claude_command()
                .contains("x86_64.AppImage cli mcp --bridge 7878")
        );
        // Without a known folder, a placeholder to fill in; both roots
        // always named, so the agent can read and write there.
        let plain = AgentSetup::default();
        let line = plain.window_command();
        assert!(line.starts_with("craft3d --control 7878"), "{line}");
        assert!(
            line.contains("--automation-read-root /path/to/3DCraft"),
            "{line}"
        );
        assert!(
            line.contains("--automation-write-root /path/to/3DCraft"),
            "{line}"
        );
        // Windows quoting of a folder with a space.
        let windows = AgentSetup {
            window: r"C:\Program Files\3DCraft\craft3d.exe".to_string(),
            work_folder: Some(r"C:\Users\An Na\Documents\3DCraft".to_string()),
            windows: true,
            ..AgentSetup::default()
        };
        let line = windows.window_command();
        assert!(
            line.starts_with(r#""C:\Program Files\3DCraft\craft3d.exe" --control"#),
            "{line}"
        );
        assert!(
            line.ends_with(r#"--automation-write-root "C:\Users\An Na\Documents\3DCraft""#),
            "{line}"
        );
    }
}
