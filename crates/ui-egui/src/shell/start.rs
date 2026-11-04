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
use crate::kit::status::BannerKind;
use crate::kit::{AppIcon, Button, Glyph, Kit, Segment, brand};
use crate::theme::color;
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
        ExampleSection::Assemblies,
        ExampleSection::Jigs,
    ];

    /// The line icon of its tab (the plan's middle way); none for the
    /// Showcase, whose cards mix kinds and carry the app icon's badge.
    pub fn glyph(self) -> Option<crate::kit::Glyph> {
        match self {
            ExampleSection::Parts => Some(crate::kit::Glyph::Part),
            ExampleSection::Assemblies => Some(crate::kit::Glyph::Assembly),
            ExampleSection::Jigs => Some(crate::kit::Glyph::Jig),
            ExampleSection::Showcase => None,
        }
    }

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
#[cfg(test)]
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
    thumbs: Vec<Option<egui::TextureHandle>>,
}

impl Pictures {
    fn load(ctx: &egui::Context) -> Self {
        let texture = |name: String, bytes: &[u8]| {
            decode_png(bytes)
                .map(|image| ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
        };
        Self {
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

/// How an MCP client starts the bridge to this window: the `craft3d-cli`
/// it runs, as `craft3d-cli mcp --bridge`. The bridge finds the window
/// itself (docs/control-protocol.md, "Discovery"), so what is copied names
/// no port and no token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSetup {
    /// The program the MCP client starts, absolute where it is known.
    pub cli: String,
    /// Words before `mcp` on its command line: `["cli"]` when `cli` is
    /// the AppImage, which runs the command line that way.
    pub cli_prefix: Vec<String>,
    /// Quote command lines for Windows (`"…"`) rather than a POSIX shell.
    pub windows: bool,
}

impl Default for AgentSetup {
    fn default() -> Self {
        Self {
            cli: "craft3d-cli".to_string(),
            cli_prefix: Vec::new(),
            windows: cfg!(windows),
        }
    }
}

/// The name the MCP server goes by in a client's configuration.
pub const MCP_SERVER_NAME: &str = "3dcraft";

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

/// An MCP client other than Claude Code, for which the agent panel copies
/// a ready configuration (decision of 2026-11-02: Claude Code by its
/// command, the others by their JSON).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum McpClient {
    #[default]
    ClaudeDesktop,
    Cursor,
    VsCode,
    /// Any client that starts a server over stdio from a command and its
    /// arguments.
    Stdio,
}

impl McpClient {
    pub const ALL: [McpClient; 4] = [
        McpClient::ClaudeDesktop,
        McpClient::Cursor,
        McpClient::VsCode,
        McpClient::Stdio,
    ];

    pub fn label(self) -> &'static str {
        match self {
            McpClient::ClaudeDesktop => "Claude Desktop",
            McpClient::Cursor => "Cursor",
            McpClient::VsCode => "VS Code",
            McpClient::Stdio => "Other (stdio)",
        }
    }

    /// Where the person pastes what is copied.
    pub fn hint(self) -> &'static str {
        match self {
            McpClient::ClaudeDesktop => {
                "Into claude_desktop_config.json (Settings › Developer › Edit Config), then restart Claude Desktop"
            }
            McpClient::Cursor => "Into ~/.cursor/mcp.json, or .cursor/mcp.json in a project",
            McpClient::VsCode => "Into .vscode/mcp.json in a workspace, or the user's mcp.json",
            McpClient::Stdio => {
                "The server's command and arguments, for any client that starts an MCP server over stdio"
            }
        }
    }
}

impl AgentSetup {
    /// What to paste into `client`'s configuration: the `mcpServers`
    /// entry for Claude Desktop and Cursor, the `servers` entry of VS
    /// Code (with its `type`), or the bare command and arguments.
    pub fn client_config(&self, client: McpClient) -> String {
        let args: Vec<Value> = self.args().into_iter().map(Value::String).collect();
        let config = match client {
            McpClient::ClaudeDesktop | McpClient::Cursor => return self.mcp_config(),
            McpClient::VsCode => serde_json::json!({
                "servers": { MCP_SERVER_NAME: { "type": "stdio", "command": self.cli, "args": args } }
            }),
            McpClient::Stdio => serde_json::json!({ "command": self.cli, "args": args }),
        };
        serde_json::to_string_pretty(&config).unwrap_or_default()
    }

    /// The arguments of the MCP server: `mcp --bridge`, after the prefix.
    fn args(&self) -> Vec<String> {
        let mut args = self.cli_prefix.clone();
        args.extend(["mcp".to_string(), "--bridge".to_string()]);
        args
    }

    /// The MCP client configuration that bridges an agent to 3DCraft, for
    /// clients configured with JSON (Claude Desktop and the like).
    pub fn mcp_config(&self) -> String {
        let args: Vec<Value> = self.args().into_iter().map(Value::String).collect();
        let config = serde_json::json!({
            "mcpServers": { MCP_SERVER_NAME: { "command": self.cli, "args": args } }
        });
        serde_json::to_string_pretty(&config).unwrap_or_default()
    }

    /// The same as a command line for a client that adds servers that
    /// way (Claude Code), quoted for the platform.
    pub fn claude_command(&self) -> String {
        format!(
            "claude mcp add {MCP_SERVER_NAME} -- {}",
            self.server_command()
        )
    }

    /// The MCP server's command line, quoted for the platform: what the
    /// configuration starts.
    pub fn server_command(&self) -> String {
        let mut words = vec![shell_word(&self.cli, self.windows)];
        words.extend(self.args().iter().map(|w| shell_word(w, self.windows)));
        words.join(" ")
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

/// How the start screen is laid out for the room the window gives it
/// (board f1): roomy, with large thumbnails, or, below [`ROOMY_HEIGHT`],
/// compact (a smaller header icon, smaller thumbnails, less space), so
/// two rows of five fit without scrolling down to 1024 × 700 and
/// 1366 × 768. On a phone the cards go two to a row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartLayout {
    /// The content's width, in points.
    pub width: f32,
    /// The app icon in the header.
    pub logo: f32,
    /// The largest a thumbnail is (4:3).
    pub thumb: egui::Vec2,
    pub per_row: usize,
    /// The space between the screen's parts, in units of `Tokens::inset`
    /// (less in the compact layout).
    pub spacing: f32,
}

/// The least height (of the room for the start screen, in points) for the
/// roomy layout.
pub const ROOMY_HEIGHT: f32 = 760.0;

/// The widest the start screen's content gets, in points (board f1).
pub const START_WIDTH: f32 = 1000.0;

impl StartLayout {
    /// The layout for room of `size` points.
    pub fn for_room(size: egui::Vec2) -> Self {
        let width = if size.x.is_finite() {
            size.x.clamp(0.0, START_WIDTH)
        } else {
            START_WIDTH
        };
        if size.y >= ROOMY_HEIGHT {
            Self {
                width,
                logo: 46.0,
                // Five fill the 1000 points (184 × 138 each, board f1).
                thumb: egui::vec2(200.0, 150.0),
                per_row: 5,
                spacing: 1.0,
            }
        } else {
            Self {
                width,
                logo: 36.0,
                thumb: egui::vec2(132.0, 99.0),
                per_row: 5,
                spacing: 0.6,
            }
        }
    }

    /// How many cards go to a row in `width` with `gap` between them, and
    /// the thumbnail's size: two on a phone, three on a tablet (boards m1
    /// and t3), else `per_row`; each as wide as the row allows up to
    /// `thumb`.
    pub fn cards(&self, width: f32, gap: f32) -> (usize, egui::Vec2) {
        let per_row = if !width.is_finite() || width < 220.0 {
            1
        } else if width < 600.0 {
            2
        } else if width < 900.0 {
            3
        } else {
            self.per_row.max(1)
        };
        let n = per_row as f32;
        let each = ((width - (n - 1.0) * gap) / n).min(self.thumb.x);
        let each = if each.is_finite() {
            each.max(24.0)
        } else {
            24.0
        };
        (per_row, egui::vec2(each, each * 0.75))
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

    /// The start screen over the viewport `rect` (boards f1 and w3): the
    /// header with the app icon, New part, New assembly and Open…, the
    /// examples by section, the recent files and how to connect an AI
    /// agent, on the panel's ground.
    pub(super) fn start_screen(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        if !self.start.shown {
            // The recent files are looked at again as the screen comes up.
            self.refresh_recent();
        }
        self.start.shown = true;
        self.start.hits.clear();
        let tokens = self.tokens;
        ui.painter()
            .rect_filled(rect, 0.0, color(tokens.panel_fill));
        let margin = tokens.inset * if rect.width() < 600.0 { 1.6 } else { 2.0 };
        let inner = rect.shrink(margin);
        if inner.width() < 120.0 || inner.height() < 80.0 {
            return;
        }
        let layout = StartLayout::for_room(inner.size());
        self.start.layout = Some(layout);
        let width = layout.width;
        let top = inner.top() + tokens.inset * layout.spacing;
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
            .max_height((inner.bottom() - top).max(0.0))
            .auto_shrink([false, true])
            .show(&mut child, |ui| self.start_card(ui, width))
            .content_size
            .y;
        self.start.height = height;
    }

    fn start_card(&mut self, ui: &mut egui::Ui, width: f32) {
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        if self.start.pictures.is_none() {
            self.start.pictures = Some(Pictures::load(ui.ctx()));
        }
        let layout = self
            .start
            .layout
            .unwrap_or_else(|| StartLayout::for_room(egui::vec2(width, ROOMY_HEIGHT)));
        // Less space between the parts in the compact layout.
        let space = tokens.inset * layout.spacing;
        let top = ui.cursor().top();
        ui.set_width(width);
        // The header: the app icon, the name and the version.
        ui.horizontal(|ui| {
            let side = layout.logo;
            let (place, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
            brand::paint(ui, &tokens, AppIcon::Part, place, self.dark);
            ui.add_space(tokens.inset * 0.6);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(
                    egui::RichText::new(super::PRODUCT)
                        .font(egui::FontId::new(
                            if layout.spacing < 1.0 { 22.0 } else { 26.0 },
                            heading_family(ui),
                        ))
                        .color(color(tokens.text)),
                );
                ui.label(
                    egui::RichText::new(format!("Version {}", super::help::VERSION))
                        .font(kit.font())
                        .color(color(tokens.text_dim)),
                );
            });
        });
        ui.add_space(space * 0.4);
        kit.paragraph(
            ui,
            "Parametric CAD built for AI. Start a document, open one, or let an agent model while you watch.",
            true,
        );
        ui.add_space(space * 0.4);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = tokens.nav_gap;
            let new = kit.button(ui, Button::primary("New part").icon(Glyph::Part));
            self.start.hits.push(("new".to_string(), new.rect));
            if new.clicked() {
                self.request_file_action(FileAction::New);
            }
            let assembly = kit.button(ui, Button::secondary("New assembly").icon(Glyph::Assembly));
            self.start
                .hits
                .push(("new-assembly".to_string(), assembly.rect));
            if assembly.clicked() {
                self.request_file_action(FileAction::NewAssembly);
            }
            let open = kit.button(ui, Button::secondary("Open…").icon(Glyph::Folder));
            self.start.hits.push(("open".to_string(), open.rect));
            if open.clicked() {
                self.request_file_action(FileAction::Open);
            }
        });
        ui.add_space(space * 1.6);
        self.section_row(ui, &kit, width);
        ui.add_space(space * 0.6);
        self.example_grid(ui, &kit, width);
        ui.add_space(space * 1.2);
        kit.divider(ui);
        ui.add_space(space * 0.8);
        // Side by side; on a narrow screen (a phone) one under the other.
        let stacked = width < tokens.start_stacked_width;
        let gap = tokens.inset * 4.0;
        let left = if stacked {
            width
        } else {
            ((width - gap) * 0.38).max(0.0)
        };
        let right = if stacked {
            width
        } else {
            (width - gap - left).max(0.0)
        };
        let row = if stacked {
            egui::Layout::top_down(egui::Align::Min)
        } else {
            egui::Layout::left_to_right(egui::Align::Min)
        };
        ui.with_layout(row, |ui| {
            // The gap is all there is between the columns: the item
            // spacing beside it would push the right column past the
            // examples' right edge.
            let spacing = ui.spacing().item_spacing.x;
            if !stacked {
                ui.spacing_mut().item_spacing.x = 0.0;
            }
            ui.allocate_ui_with_layout(
                egui::vec2(left, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.x = spacing;
                    ui.set_width(left);
                    kit.section_header(ui, "Recent");
                    ui.add_space(tokens.nav_spacing);
                    if cfg!(target_arch = "wasm32") {
                        kit.paragraph(ui, "The browser keeps no recent files.", true);
                    } else {
                        self.recent_links(ui, &kit);
                    }
                },
            );
            ui.add_space(if stacked { space } else { gap });
            ui.allocate_ui_with_layout(
                egui::vec2(right, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.x = spacing;
                    ui.set_width(right);
                    self.agent_block(ui, &kit);
                },
            );
        });
        ui.add_space(space);
        // The whole screen's content, for the test that it stays inside
        // the window.
        let rect = egui::Rect::from_min_max(
            egui::pos2(ui.min_rect().left(), top),
            egui::pos2(ui.min_rect().left() + width, ui.min_rect().bottom()),
        );
        self.start.hits.push(("card".to_string(), rect));
    }

    /// "Examples" and, at the right (under it on a phone), the sections
    /// as a segmented control with their line icons; a click shows that
    /// section's cards. A single section shows only the heading.
    fn section_row(&mut self, ui: &mut egui::Ui, kit: &Kit, width: f32) {
        let sections: Vec<ExampleSection> = ExampleSection::ALL
            .into_iter()
            .filter(|section| EXAMPLES.iter().any(|e| e.section == *section))
            .collect();
        if !sections.contains(&self.start.section) {
            self.start.section = sections.first().copied().unwrap_or_default();
        }
        // With the line icons where they fit, else the words alone (a
        // narrow phone).
        let mut segments: Vec<Segment<'_>> = sections
            .iter()
            .map(|section| match section.glyph() {
                Some(glyph) => Segment::IconText(glyph, section.title()),
                None => Segment::Text(section.title()),
            })
            .collect();
        if kit.segmented_width(ui, &segments) > width {
            segments = sections
                .iter()
                .map(|section| Segment::Text(section.title()))
                .collect();
        }
        let mut chosen = sections
            .iter()
            .position(|s| *s == self.start.section)
            .unwrap_or(0);
        let mut show = |ui: &mut egui::Ui| {
            let choice = kit.segmented(ui, &mut chosen, &segments);
            for (section, rect) in sections.iter().zip(&choice.items) {
                self.start
                    .hits
                    .push((format!("section:{}", section.word()), *rect));
            }
        };
        let narrow = width < self.tokens.start_stacked_width;
        if sections.len() < 2 {
            kit.section_header(ui, "Examples");
        } else if narrow {
            kit.section_header(ui, "Examples");
            ui.add_space(self.tokens.nav_spacing);
            show(ui);
        } else {
            ui.horizontal(|ui| {
                kit.section_header(ui, "Examples");
                // At the right: the control is one allocation.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), show);
            });
        }
        if let Some(section) = sections.get(chosen) {
            self.start.section = *section;
        }
    }

    /// The examples of the section shown as cards: the thumbnail, the
    /// name and what it shows; in the Showcase (where kinds mix) the app
    /// icon's badge at the thumbnail's corner. A click builds or opens it
    /// in a new document.
    fn example_grid(&mut self, ui: &mut egui::Ui, kit: &Kit, width: f32) {
        let tokens = self.tokens;
        let layout = self
            .start
            .layout
            .unwrap_or_else(|| StartLayout::for_room(egui::vec2(width, ROOMY_HEIGHT)));
        let gap = tokens.inset * if layout.spacing < 1.0 { 1.4 } else { 2.0 };
        let (per_row, thumb) = layout.cards(width, gap);
        let compact = thumb.x < 150.0;
        let text_height = if compact { 32.0 } else { 38.0 };
        let card = egui::vec2(thumb.x, thumb.y + text_height);
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
        let section = self.start.section;
        let badge = section == ExampleSection::Showcase;
        let shown: Vec<&'static Example> =
            EXAMPLES.iter().filter(|e| e.section == section).collect();
        for (row, examples) in shown.chunks(per_row).enumerate() {
            if row > 0 {
                ui.add_space(gap * 0.6);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for &example in examples {
                    let index = EXAMPLES.iter().position(|e| e.id == example.id);
                    let (rect, response) = ui.allocate_exact_size(card, egui::Sense::click());
                    let response = response
                        .on_hover_text(format!("{}: {}", example.title, example.summary))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    let painter = ui.painter_at(rect.expand(3.0));
                    let image = egui::Rect::from_min_size(rect.min, thumb);
                    let radius = egui::CornerRadius::same(tokens.thumb_radius);
                    if let Some(Some(id)) = index.and_then(|i| thumbs.get(i)) {
                        egui::Image::new((*id, thumb))
                            .corner_radius(radius)
                            .paint_at(ui, image);
                    } else {
                        painter.rect_filled(image, radius, color(tokens.viewport_background));
                    }
                    if badge {
                        let side = tokens.card_badge;
                        let place = egui::Rect::from_min_size(
                            image.min
                                + egui::vec2(tokens.nav_spacing * 2.0, tokens.nav_spacing * 2.0),
                            egui::vec2(side, side),
                        );
                        brand::paint(ui, &tokens, example_icon(example), place, false);
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
                    // The name and what it shows, each on one row cut with
                    // an ellipsis at the card's width.
                    let mut y = image.max.y + 4.0;
                    for (text, font, ink) in [
                        (example.title, kit.font(), tokens.text),
                        (example.summary, kit.small_font(), tokens.text_dim),
                    ] {
                        let mut job = egui::text::LayoutJob::single_section(
                            text.to_string(),
                            egui::TextFormat::simple(font, color(ink)),
                        );
                        job.wrap = egui::text::TextWrapping::truncate_at_width(
                            (rect.width() - 2.0).max(1.0),
                        );
                        let galley = painter.layout_job(job);
                        let h = galley.size().y;
                        painter.galley(egui::pos2(rect.min.x + 2.0, y), galley, color(ink));
                        y += h;
                    }
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

    /// "Connect an AI agent": the same panel as the popover over the pill
    /// (switch, work folder, `claude mcp add …`, other clients); in the
    /// browser a banner that says agents need the desktop app.
    fn agent_block(&mut self, ui: &mut egui::Ui, kit: &Kit) {
        if cfg!(target_arch = "wasm32") {
            kit.banner(
                ui,
                BannerKind::Info,
                None,
                "In the browser an agent cannot drive 3DCraft yet. Use the desktop app to connect one.",
                |_, _| {},
            );
            return;
        }
        let hits = self.agents_panel(ui, kit);
        self.start.hits.extend(hits);
    }
}

/// The kind of document an example opens as, for its badge.
fn example_icon(example: &Example) -> AppIcon {
    match example.source {
        Source::File { name, .. } if name.ends_with(".3dcraftasm") => AppIcon::Assembly,
        Source::File { name, .. } if name.ends_with(".3dcraftjig") => AppIcon::Jig,
        _ => match example.section {
            ExampleSection::Assemblies => AppIcon::Assembly,
            ExampleSection::Jigs => AppIcon::Jig,
            _ => AppIcon::Part,
        },
    }
}

/// The headings' family where the fonts in force have one (Semi Bold).
fn heading_family(ui: &egui::Ui) -> egui::FontFamily {
    ui.style()
        .text_styles
        .get(&egui::TextStyle::Heading)
        .map_or(egui::FontFamily::Proportional, |font| font.family.clone())
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
        assert_eq!((roomy.per_row, roomy.logo), (5, 46.0));
        assert_eq!(roomy.width, START_WIDTH);
        // Five cards fill the content's width (board f1).
        let (per_row, thumb) = roomy.cards(START_WIDTH, 20.0);
        assert_eq!(per_row, 5);
        assert!(
            (thumb.x * 5.0 + 80.0 - START_WIDTH).abs() < 0.5,
            "{thumb:?}"
        );
        // On a phone, two to a row.
        let phone = StartLayout::for_room(egui::vec2(358.0, 780.0));
        assert_eq!(phone.cards(358.0, 20.0).0, 2);
        let tablet = StartLayout::for_room(egui::vec2(728.0, 900.0));
        assert_eq!(tablet.cards(728.0, 20.0).0, 3);
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

    /// What is copied names the bridge only: no port and no token
    /// (2026-11-02: the bridge finds the window and its token itself).
    #[test]
    fn the_mcp_configuration_starts_the_bridge_and_holds_no_secret() {
        let setup = AgentSetup {
            cli: "/opt/3dcraft/craft3d-cli".to_string(),
            cli_prefix: Vec::new(),
            windows: false,
        };
        let config: Value = serde_json::from_str(&setup.mcp_config()).unwrap();
        assert_eq!(
            config,
            json!({"mcpServers": {"3dcraft": {
                "command": "/opt/3dcraft/craft3d-cli",
                "args": ["mcp", "--bridge"]
            }}})
        );
        assert_eq!(
            setup.claude_command(),
            "claude mcp add 3dcraft -- /opt/3dcraft/craft3d-cli mcp --bridge"
        );
        for text in [setup.mcp_config(), setup.claude_command()] {
            assert!(!text.contains("token") && !text.contains("7878"), "{text}");
            assert!(!text.contains('~'), "nothing expands `~` there: {text}");
        }
        // Paths with quotes and backslashes stay valid JSON.
        let odd = AgentSetup {
            cli: r#"C:\3D "Craft"\craft3d-cli.exe"#.to_string(),
            cli_prefix: Vec::new(),
            windows: true,
        };
        let config: Value = serde_json::from_str(&odd.mcp_config()).unwrap();
        assert_eq!(
            config["mcpServers"]["3dcraft"]["command"],
            r#"C:\3D "Craft"\craft3d-cli.exe"#
        );
        // The AppImage runs the command line as `<AppImage> cli`.
        let appimage = AgentSetup {
            cli: "/home/me/Apps/3DCraft-0.1.0-x86_64.AppImage".to_string(),
            cli_prefix: vec!["cli".to_string()],
            windows: false,
        };
        let config: Value = serde_json::from_str(&appimage.mcp_config()).unwrap();
        assert_eq!(
            config["mcpServers"]["3dcraft"]["args"],
            json!(["cli", "mcp", "--bridge"])
        );
        assert!(
            appimage
                .claude_command()
                .ends_with("x86_64.AppImage cli mcp --bridge")
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
            cli_prefix: Vec::new(),
            windows: true,
        };
        assert_eq!(
            windows.claude_command(),
            r#"claude mcp add 3dcraft -- "C:\Program Files\3DCraft\craft3d-cli.exe" mcp --bridge"#
        );
        let posix = AgentSetup {
            cli: "/opt/3D Craft/craft3d-cli".to_string(),
            cli_prefix: Vec::new(),
            windows: false,
        };
        assert_eq!(
            posix.claude_command(),
            "claude mcp add 3dcraft -- '/opt/3D Craft/craft3d-cli' mcp --bridge"
        );
    }
}
