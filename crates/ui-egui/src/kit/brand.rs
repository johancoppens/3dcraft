//! The app icon and its document variants, where 3DCraft shows itself
//! (the plan's *middle way*, 2026-11-02): the start screen's and the About
//! dialog's header, a badge on an example card where kinds mix. Inside the
//! interface (tabs, menus, lists) the documents have the simplified line
//! icons instead ([`super::glyphs`]).
//!
//! The pictures are the installers' own PNGs (`packaging/icons`, MIT,
//! ATTRIBUTION.md), built into the program: at 16 pixels the drawing made
//! for that size (`craft3d*-small.svg`), larger the full one. On a dark
//! ground the icon gets a faint white rim, without which its dark tile's
//! edge disappears.

use crate::theme::{Tokens, color, color_alpha};
use egui::{CornerRadius, Rect, Stroke, StrokeKind, TextureHandle, Ui};

/// Which variant of the app icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppIcon {
    Part,
    Assembly,
    Jig,
}

impl AppIcon {
    pub const ALL: [AppIcon; 3] = [AppIcon::Part, AppIcon::Assembly, AppIcon::Jig];

    /// The PNG for a picture `pixels` across: the 16-pixel drawing up to
    /// 20 pixels, then the 32, the 64 and the 128.
    fn png(self, pixels: f32) -> (&'static str, &'static [u8]) {
        macro_rules! png {
            ($name:literal) => {
                (
                    $name,
                    include_bytes!(concat!("../../../../packaging/icons/", $name, ".png")),
                )
            };
        }
        let size = if pixels <= 20.0 {
            0
        } else if pixels <= 40.0 {
            1
        } else if pixels <= 80.0 {
            2
        } else {
            3
        };
        match (self, size) {
            (AppIcon::Part, 0) => png!("craft3d-16"),
            (AppIcon::Part, 1) => png!("craft3d-32"),
            (AppIcon::Part, 2) => png!("craft3d-64"),
            (AppIcon::Part, 3) => png!("craft3d-128"),
            (AppIcon::Assembly, 0) => png!("craft3d-asm-16"),
            (AppIcon::Assembly, 1) => png!("craft3d-asm-32"),
            (AppIcon::Assembly, 2) => png!("craft3d-asm-64"),
            (AppIcon::Assembly, 3) => png!("craft3d-asm-128"),
            (AppIcon::Jig, 0) => png!("craft3d-jig-16"),
            (AppIcon::Jig, 1) => png!("craft3d-jig-32"),
            (AppIcon::Jig, 2) => png!("craft3d-jig-64"),
            (AppIcon::Jig, 3) => png!("craft3d-jig-128"),
            (AppIcon::Part, _) => png!("craft3d-128"),
            (AppIcon::Assembly, _) => png!("craft3d-asm-128"),
            (AppIcon::Jig, _) => png!("craft3d-jig-128"),
        }
    }

    /// The line icon of the same kind of document.
    pub fn glyph(self) -> super::Glyph {
        match self {
            AppIcon::Part => super::Glyph::Part,
            AppIcon::Assembly => super::Glyph::Assembly,
            AppIcon::Jig => super::Glyph::Jig,
        }
    }
}

/// The texture of `name`, decoded once per context. `None` when the PNG
/// does not read (it is built in, so only a broken build gets there).
fn texture(ui: &Ui, name: &'static str, bytes: &'static [u8]) -> Option<TextureHandle> {
    let id = egui::Id::new(("kit-app-icon", name));
    if let Some(handle) = ui.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(handle);
    }
    let image = crate::shell::start::decode_png(bytes)?;
    let handle = ui
        .ctx()
        .load_texture(name, image, egui::TextureOptions::LINEAR);
    ui.data_mut(|d| d.insert_temp(id, handle.clone()));
    Some(handle)
}

/// Paints the app icon `icon` in `rect` (square), with the faint rim when
/// `rim` (on a dark ground). Should the picture not read, the line icon of
/// the same kind stands in, so a place never stays empty.
pub fn paint(ui: &Ui, tokens: &Tokens, icon: AppIcon, rect: Rect, rim: bool) {
    let side = rect.width().min(rect.height());
    if !(side.is_finite() && side > 0.0) {
        return;
    }
    let rect = Rect::from_center_size(rect.center(), egui::vec2(side, side));
    let (name, bytes) = icon.png(side * ui.ctx().pixels_per_point());
    match texture(ui, name, bytes) {
        Some(handle) => {
            ui.painter().image(
                handle.id(),
                rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            if rim {
                // The tile's corner is 3.5 of 16.
                let radius = (side * 3.5 / 16.0).clamp(0.0, 255.0).round() as u8;
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(radius),
                    Stroke::new(
                        1.0,
                        color_alpha(tokens.plate_rim, tokens.app_icon_rim_opacity),
                    ),
                    StrokeKind::Inside,
                );
            }
        }
        None => {
            static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!("the app icon {name} does not read; its line icon stands in");
            }
            super::glyphs::paint(
                ui.painter(),
                icon.glyph(),
                rect,
                color(tokens.text_dim),
                tokens,
            );
        }
    }
}

/// Lays out the app icon `side` points large at the layout's place.
pub fn show(ui: &mut Ui, tokens: &Tokens, icon: AppIcon, side: f32, rim: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(super::sane(egui::vec2(side, side)), egui::Sense::hover());
    paint(ui, tokens, icon, rect, rim);
    response
}

/// Whether every variant's pictures read, for the tests.
pub fn all_read() -> bool {
    AppIcon::ALL.iter().all(|icon| {
        [16.0, 32.0, 64.0, 128.0].iter().all(|px| {
            let (_, bytes) = icon.png(*px);
            crate::shell::start::decode_png(bytes).is_some()
        })
    })
}
