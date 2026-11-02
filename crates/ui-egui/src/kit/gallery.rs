//! Every component and icon of the kit on boards laid out as the plan's
//! (c1: the components on a panel and on a plate; c2: menus, popovers,
//! tooltips and the pill; the icons), so the pictures can be set beside
//! the mock-ups, and so the tests can draw every component at a window's
//! width and a phone's (`tests/kit.rs`; `examples/kit_gallery.rs` paints
//! them to PNG).

use super::glyphs::{self, Glyph};
use super::{
    AppIcon, BannerKind, Button, ChipKind, Ground, Kit, ListRow, MenuItem, PillState, Segment,
    Slider, Tab, ValueStyle, brand,
};
use crate::theme::{Tokens, color};
use egui::{CornerRadius, Frame, Margin, Rect, Response, Sense, Ui, pos2, vec2};

/// What the gallery's controls hold between frames, and every target the
/// last pass laid out (for the tests: inside the window, large enough).
#[derive(Clone, Debug)]
pub struct GalleryState {
    pub look: usize,
    pub axis: usize,
    pub kind: usize,
    pub shadows: bool,
    pub ao: bool,
    pub lens: bool,
    pub ground: bool,
    pub exposure: f64,
    pub aperture: f64,
    pub bloom: f64,
    pub length: String,
    pub typing: String,
    pub wrong: String,
    pub filter: String,
    pub format: usize,
    pub checks_open: bool,
    pub tab: usize,
    /// Every interactive target of the last pass, with what it is.
    pub targets: Vec<(&'static str, Rect)>,
}

impl Default for GalleryState {
    fn default() -> Self {
        Self {
            look: 0,
            axis: 1,
            kind: 0,
            shadows: true,
            ao: false,
            lens: true,
            ground: false,
            exposure: 0.0,
            aperture: 0.3,
            bloom: 0.04,
            length: "20.00".to_string(),
            typing: "26".to_string(),
            wrong: "-3".to_string(),
            filter: String::new(),
            format: 0,
            checks_open: true,
            tab: 0,
            targets: Vec::new(),
        }
    }
}

impl GalleryState {
    fn hit(&mut self, what: &'static str, response: &Response) {
        self.targets.push((what, response.rect));
    }
}

/// A board: the ground's fill, rounded, with a title.
pub fn board<R>(
    ui: &mut Ui,
    tokens: &Tokens,
    ground: Ground,
    title: &str,
    width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let kit = Kit::new(tokens, ground);
    let pad = tokens.inset * 2.4;
    Frame::new()
        .fill(color(kit.skin.fill))
        .corner_radius(CornerRadius::same(tokens.nav_radius + 2))
        .inner_margin(Margin::same(pad as i8))
        .show(ui, |ui| {
            ui.set_width((width - 2.0 * pad).max(0.0));
            ui.vertical(|ui| board_body(ui, &kit, tokens, title, add))
                .inner
        })
        .inner
}

fn board_body<R>(
    ui: &mut Ui,
    kit: &Kit,
    tokens: &Tokens,
    title: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let galley = kit.galley(ui, title, egui::FontId::proportional(15.0), kit.skin.text);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x, 28.0), Sense::hover());
    ui.painter().galley(rect.min, galley, color(kit.skin.text));
    ui.add_space(tokens.inset);
    ui.push_id(title, |ui| ui.vertical(add).inner).inner
}

fn section(ui: &mut Ui, kit: &Kit, caption: &str) {
    ui.add_space(kit.t.inset * 1.2);
    kit.caption(ui, caption);
}

/// The components of board c1, on `kit`'s ground, as wide as `ui` is.
pub fn components(ui: &mut Ui, kit: &Kit, state: &mut GalleryState) {
    let t = kit.t;
    ui.spacing_mut().item_spacing = vec2(t.nav_spacing * 4.0, t.nav_spacing * 3.0);
    section(ui, kit, "Buttons");
    ui.horizontal_wrapped(|ui| {
        let r = kit.button(ui, Button::primary("Export"));
        state.hit("button", &r);
        let r = kit.button(ui, Button::secondary("Cancel"));
        state.hit("button", &r);
        let r = kit.button(ui, Button::quiet("Copy details").icon(Glyph::Copy));
        state.hit("button", &r);
        let r = kit.button(ui, Button::link("How it works"));
        state.hit("button", &r);
    });
    ui.horizontal_wrapped(|ui| {
        let r = kit.button(ui, Button::primary("Recover").icon(Glyph::Undo));
        state.hit("button", &r);
        let r = kit.button(ui, Button::danger("Discard"));
        state.hit("button", &r);
        kit.button(ui, Button::secondary("Open…").enabled(false));
        let r = kit.button(ui, Button::quiet("Later").hovered(true));
        state.hit("button", &r);
    });

    section(ui, kit, "Icon buttons · chips");
    ui.horizontal_wrapped(|ui| {
        for (glyph, on) in [
            (Glyph::Nav(crate::shell::icons::Icon::Fit), false),
            (Glyph::Nav(crate::shell::icons::Icon::Studio), true),
            (Glyph::Nav(crate::shell::icons::Icon::Loop), false),
            (Glyph::Nav(crate::shell::icons::Icon::Orbit), false),
            (Glyph::Copy, false),
            (Glyph::Close, false),
        ] {
            let r = kit.icon_button(ui, glyph, on);
            state.hit("icon button", &r);
        }
        ui.add_space(t.inset);
        for (text, kind) in [
            ("1×", ChipKind::Plain),
            ("Studio", ChipKind::On),
            ("F25", ChipKind::Info),
            ("failed", ChipKind::Failed),
        ] {
            let r = kit.chip(ui, text, kind);
            state.hit("chip", &r);
        }
    });

    section(ui, kit, "Segmented control · toggles");
    let choice = kit.segmented(
        ui,
        &mut state.look,
        &[
            Segment::Text("Studio"),
            Segment::Text("Outdoor"),
            Segment::Text("Workshop"),
        ],
    );
    for item in choice.items {
        state.targets.push(("segment", item));
    }
    ui.horizontal_wrapped(|ui| {
        let choice = kit.segmented(
            ui,
            &mut state.axis,
            &[Segment::Text("X"), Segment::Text("Y"), Segment::Text("Z")],
        );
        for item in choice.items {
            state.targets.push(("segment", item));
        }
        let choice = kit.segmented(
            ui,
            &mut state.kind,
            &[
                Segment::Icon(Glyph::DatumPlane),
                Segment::Icon(Glyph::Square),
                Segment::Icon(Glyph::Block),
            ],
        );
        for item in choice.items {
            state.targets.push(("segment", item));
        }
    });
    ui.horizontal_wrapped(|ui| {
        let r = kit.checkbox(ui, &mut state.shadows, "Shadows");
        state.hit("checkbox", &r);
        let r = kit.checkbox(ui, &mut state.ao, "AO");
        state.hit("checkbox", &r);
        let r = kit.toggle(ui, &mut state.lens, "Lens");
        state.hit("toggle", &r);
        let r = kit.toggle(ui, &mut state.ground, "Ground");
        state.hit("toggle", &r);
    });

    section(ui, kit, "Fields");
    ui.horizontal_wrapped(|ui| {
        let r = kit.number_field(ui, "g-length", &mut state.length, "mm", 104.0, false);
        state.hit("number field", &r);
        let r = kit.number_field(ui, "g-typing", &mut state.typing, "mm", 104.0, false);
        state.hit("number field", &r);
        let r = kit.number_field(ui, "g-wrong", &mut state.wrong, "mm", 104.0, true);
        state.hit("number field", &r);
    });
    ui.horizontal_wrapped(|ui| {
        let r = kit.text_field(
            ui,
            "g-filter",
            &mut state.filter,
            "Filter features",
            Some(Glyph::Search),
            190.0,
        );
        state.hit("text field", &r);
        let r = kit.dropdown(
            ui,
            "g-format",
            &mut state.format,
            &["STEP (.step)", "STL (.stl)", "3MF (.3mf)"],
            150.0,
        );
        state.hit("dropdown", &r);
    });

    section(ui, kit, "Sliders");
    let r = kit.slider(
        ui,
        Slider::new("Exposure", &mut state.exposure, -3.0..=3.0).text("0.00 EV"),
    );
    state.hit("slider", &r);
    let r = kit.slider(
        ui,
        Slider::new("Aperture", &mut state.aperture, 0.0..=1.0)
            .text("f/2.8")
            .hint("smaller f-number: more blur"),
    );
    state.hit("slider", &r);
    kit.slider(
        ui,
        Slider::new("Bloom", &mut state.bloom, 0.0..=1.0)
            .text("0.04")
            .enabled(false),
    );

    section(ui, kit, "Section header · rows");
    kit.section_header(ui, "Examples");
    let ok = kit.skin.text_dim;
    let r = kit.fold_header(
        ui,
        &mut state.checks_open,
        "Checks",
        Some(7),
        Some(("all pass", ok)),
    );
    state.hit("fold header", &r);
    kit.prop_row(ui, "Volume", "112.12 cm³", ValueStyle::Plain, 96.0);
    kit.prop_row(ui, "Made by", "Sleeve (F8)", ValueStyle::Link, 96.0);
    kit.prop_row(ui, "Name", "F8:face(2)", ValueStyle::Name, 96.0);
    let s = kit.skin;
    for row in [
        ListRow::new("Pedestal")
            .icon(Glyph::Extrude)
            .value("Up to Sleeve"),
        ListRow::new("Sleeve")
            .icon(Glyph::Cylinder)
            .value("R18 × 60 mm")
            .hovered(true)
            .eye(true),
        ListRow::new("Bored")
            .icon(Glyph::Subtract)
            .value("− Bore")
            .selected(true),
        ListRow::new("Foot")
            .icon(Glyph::Sketch)
            .icon_ink(s.icon_sketch)
            .value("2 splines · 12 DOF")
            .indent(1),
        ListRow::new("Mid")
            .icon(Glyph::DatumPlane)
            .icon_ink(s.icon_datum)
            .value("YZ · 0 mm"),
        ListRow::new("Oiled")
            .icon(Glyph::Fillet)
            .value("R3")
            .failed(true),
        ListRow::new("Hole")
            .icon(Glyph::Cylinder)
            .value("Ø7 × 20 mm")
            .dimmed(true),
        ListRow::new("bracket.3dcraft")
            .icon(Glyph::Part)
            .value("2 h ago"),
    ] {
        let r = kit.list_row(ui, row);
        state.hit("list row", &r);
    }

    section(ui, kit, "Tab bar");
    let choice = kit.tab_bar(
        ui,
        &mut state.tab,
        &[
            Tab::new("Features"),
            Tab::new("Variables"),
            Tab::new("Bodies"),
            Tab::new("Agent").dot(true),
        ],
    );
    for item in choice.items {
        state.targets.push(("tab", item));
    }

    section(ui, kit, "Banners");
    let mut buttons = Vec::new();
    kit.banner(
        ui,
        BannerKind::Error,
        Some("Could not save"),
        "'/media/usb/bracket.3dcraft' could not be written: the disk is full.",
        |kit, ui| {
            buttons.push(kit.button(ui, Button::primary("OK")));
            buttons.push(kit.button(ui, Button::secondary("Copy details")));
        },
    );
    kit.banner(
        ui,
        BannerKind::Info,
        None,
        "Lens focuses on the centre: nothing is selected.",
        |_, _| {},
    );
    kit.banner(
        ui,
        BannerKind::Attention,
        Some("3DCraft closed unexpectedly"),
        "Two documents can be recovered.",
        |kit, ui| {
            buttons.push(kit.button(ui, Button::primary("Recover")));
            buttons.push(kit.button(ui, Button::quiet("Later")));
        },
    );
    for r in &buttons {
        state.hit("button", r);
    }

    section(ui, kit, "Progress");
    ui.horizontal_wrapped(|ui| {
        kit.progress(ui, "Opening welding-table… 41/82", 0.5, 230.0);
        ui.add_space(t.inset);
        kit.progress(ui, "Checking assertions…", 0.6, 200.0);
    });
    kit.spinner(ui, "Rebuilding 4 features…");
}

/// Board c2: a menu, a context menu, a popover with its pointer,
/// tooltips, and the six states of the agent's pill on the status bar.
pub fn overlays(ui: &mut Ui, tokens: &Tokens, state: &mut GalleryState) {
    let plate = Kit::plate(tokens);
    let t = *tokens;
    let wide = ui.available_width() > 2.0 * t.menu_width + t.popover_width;
    let menus = |ui: &mut Ui, state: &mut GalleryState| {
        plate.menu_frame(ui, t.menu_width + 40.0, |ui| {
            for item in [
                MenuItem::new("New Part")
                    .icon(Glyph::Part)
                    .shortcut("Ctrl+N"),
                MenuItem::new("New Assembly")
                    .icon(Glyph::Assembly)
                    .shortcut("Ctrl+Shift+N"),
                MenuItem::new("Open…").shortcut("Ctrl+O").hovered(true),
                MenuItem::new("Open recent").submenu(),
            ] {
                let r = plate.menu_item(ui, item);
                state.hit("menu item", &r);
            }
            plate.menu_separator(ui);
            for item in [
                MenuItem::new("Save").shortcut("Ctrl+S"),
                MenuItem::new("Save As…").shortcut("Ctrl+Shift+S"),
            ] {
                let r = plate.menu_item(ui, item);
                state.hit("menu item", &r);
            }
            plate.menu_separator(ui);
            let r = plate.menu_item(ui, MenuItem::new("Close").shortcut("Ctrl+W"));
            state.hit("menu item", &r);
        });
        plate.menu_frame(ui, t.menu_width + 30.0, |ui| {
            for item in [
                MenuItem::new("Edit…").shortcut("Enter"),
                MenuItem::new("Rename").shortcut("F2").hovered(true),
                MenuItem::new("Show dependencies"),
            ] {
                let r = plate.menu_item(ui, item);
                state.hit("menu item", &r);
            }
            plate.menu_separator(ui);
            for item in [
                MenuItem::new("Roll back to here"),
                MenuItem::new("Suppress"),
                MenuItem::new("Hide body").checked(true).shortcut("H"),
            ] {
                let r = plate.menu_item(ui, item);
                state.hit("menu item", &r);
            }
            plate.menu_separator(ui);
            let r = plate.menu_item(ui, MenuItem::new("Delete").danger().shortcut("Del"));
            state.hit("menu item", &r);
        });
    };
    let popover = |ui: &mut Ui, state: &mut GalleryState| {
        let width = t.popover_width.min(ui.available_width());
        plate.popover_frame(ui, width, Some(width - 40.0), |ui| {
            plate.section_header(ui, "Popover");
            let job = egui::text::LayoutJob::simple(
                "Hangs under the button that opened it, right edge in line with the bar; Escape or the caret closes it.".to_string(),
                plate.font(),
                color(plate.skin.text_dim),
                ui.available_width(),
            );
            let galley = ui.painter().layout_job(job);
            let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
            ui.painter().galley(rect.min, galley, color(plate.skin.text_dim));
            let r = plate.slider(
                ui,
                Slider::new("Exposure", &mut state.exposure, -3.0..=3.0)
                    .text("0.00 EV")
                    .widths(80.0, 60.0),
            );
            state.hit("slider", &r);
        });
        ui.add_space(t.inset);
        plate.tooltip_box(ui, "Show the datum planes, axes and points", 300.0);
        plate.tooltip_box(
            ui,
            "The version, commit, platform and graphics, to paste into a bug report",
            260.0,
        );
    };
    if wide {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = t.inset * 3.0;
            menus(ui, state);
            ui.vertical(|ui| popover(ui, state));
        });
    } else {
        ui.vertical(|ui| {
            menus(ui, state);
            popover(ui, state);
        });
    }
    ui.add_space(t.inset * 2.0);
    // The pill stands on the status bar.
    Frame::new()
        .fill(color(t.status_fill))
        .corner_radius(CornerRadius::same(t.nav_radius))
        .inner_margin(Margin::same((t.inset * 1.6) as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let chrome = Kit::plate(tokens);
            let galley = chrome.galley(
                ui,
                "Status pill (the agent), on status_fill",
                chrome.medium_font(ui),
                t.chrome_text,
            );
            let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
            ui.painter().galley(rect.min, galley, color(t.chrome_text));
            ui.horizontal_wrapped(|ui| {
                for (pill, text) in [
                    (PillState::Off, "Agents off"),
                    (PillState::Waiting, "Waiting for an agent"),
                    (PillState::Connected, "Claude Code connected"),
                    (PillState::Working, "Claude Code · fillet R3…"),
                    (PillState::Asks, "Claude Code asks for a folder"),
                    (PillState::Lost, "Agent disconnected"),
                ] {
                    let r = chrome.pill(ui, pill, text);
                    state.hit("pill", &r);
                }
            });
        });
}

/// The icons: every feature's and document's line icon at 16 points, dim
/// and active, the documents at 32 too, the controls' icons, and the app
/// icon's variants with and without the rim.
pub fn icons(ui: &mut Ui, kit: &Kit) {
    let t = kit.t;
    let s = kit.skin;
    let cell = vec2(96.0, 40.0);
    section(ui, kit, "Features");
    let grid = |ui: &mut Ui, list: &[Glyph], dim: bool| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for glyph in list {
                let (rect, _) = ui.allocate_exact_size(cell, Sense::hover());
                let ink = if dim {
                    glyph.feature_ink(s.icon_sketch, s.icon_datum, s.text_dim)
                } else {
                    s.text
                };
                glyphs::paint(
                    ui.painter(),
                    *glyph,
                    Rect::from_center_size(
                        pos2(rect.center().x, rect.top() + 10.0),
                        vec2(16.0, 16.0),
                    ),
                    color(ink),
                    &t,
                );
                let name = format!("{glyph:?}");
                let name = name.trim_start_matches("Nav(").trim_end_matches(')');
                let galley = kit.galley(ui, name, kit.small_font(), s.text_dim);
                ui.painter().galley(
                    pos2(rect.center().x - galley.size().x * 0.5, rect.top() + 22.0),
                    galley,
                    color(s.text_dim),
                );
            }
        });
    };
    grid(ui, &Glyph::FEATURES, true);
    section(ui, kit, "Documents · 16 dim, 16 active, 32");
    ui.horizontal_wrapped(|ui| {
        for glyph in Glyph::DOCUMENTS {
            for (side, ink) in [(16.0, s.text_dim), (16.0, s.text), (32.0, s.text)] {
                let (rect, _) = ui.allocate_exact_size(vec2(44.0, 40.0), Sense::hover());
                glyphs::paint(
                    ui.painter(),
                    glyph,
                    Rect::from_center_size(rect.center(), vec2(side, side)),
                    color(ink),
                    &t,
                );
            }
            ui.add_space(t.inset * 2.0);
        }
    });
    section(ui, kit, "Controls");
    grid(ui, &Glyph::CONTROLS, false);
    section(ui, kit, "App icon · 16, 22, 32, 64; rim on dark grounds");
    let rim = kit.ground == Ground::Plate || t.is_dark();
    ui.horizontal_wrapped(|ui| {
        for icon in AppIcon::ALL {
            for side in [16.0, 22.0, 32.0, 64.0] {
                brand::show(ui, &t, icon, side, rim);
            }
            ui.add_space(t.inset * 2.0);
        }
    });
}
