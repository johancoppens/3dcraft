//! Pictures of the component kit (`craft3d_ui_egui::kit`), laid out as the
//! UI overhaul's boards, to set beside its mock-ups (approved
//! 2026-11-02, not in the repository): the components on a panel and on a
//! plate in the light appearance (c1) and in the dark one (c3), the menus,
//! popover, tooltips and pill (c2) in both, the icons in both, and the
//! components at a phone's width (390 points) with touch sizes. Painted by
//! egui's own wgpu renderer into a texture and read back, at one pixel a
//! point (the icons at two), with the clock at 0, so the same build gives the same pictures.
//!
//! ```sh
//! CRAFT3D_FONTS=<dir with InterVariable.ttf> \
//!     cargo run -p craft3d-ui-egui --example kit_gallery -- [OUT_DIR]
//! ```
//!
//! `OUT_DIR` is `target/work-uikit` by default.

use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::fonts::UiFonts;
use craft3d_ui_egui::kit::gallery::{self, GalleryState};
use craft3d_ui_egui::kit::{Ground, Kit};
use craft3d_ui_egui::record::Offscreen;
use craft3d_ui_egui::theme::{Tokens, color};
use egui::{Pos2, Rect, vec2};
use std::path::PathBuf;

fn main() -> Result<(), String> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/work-uikit"));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let state = headless_render_state()?;
    let mut offscreen = Offscreen::new(state);
    let (light, dark) = (Tokens::light(), Tokens::dark());
    for (name, size, scale, tokens, what) in [
        (
            "kit-c1-components-light",
            [1320, 1560],
            1.0,
            light,
            What::Components,
        ),
        (
            "kit-c3-components-dark",
            [1320, 1560],
            1.0,
            dark,
            What::Components,
        ),
        (
            "kit-c2-menus-popovers-pills-light",
            [1240, 700],
            1.0,
            light,
            What::Overlays,
        ),
        (
            "kit-c2-menus-popovers-pills-dark",
            [1240, 700],
            1.0,
            dark,
            What::Overlays,
        ),
        ("kit-icons-light", [1320, 1000], 2.0, light, What::Icons),
        ("kit-icons-dark", [1320, 1000], 2.0, dark, What::Icons),
        (
            "kit-phone-390-light",
            [390, 1720],
            1.0,
            light.touch(),
            What::Phone,
        ),
        (
            "kit-phone-390-dark",
            [390, 1720],
            1.0,
            dark.touch(),
            What::Phone,
        ),
        (
            "kit-phone-390-overlays-dark",
            [390, 1100],
            1.0,
            dark.touch(),
            What::PhoneOverlays,
        ),
    ] {
        let path = out.join(format!("{name}.png"));
        picture(&mut offscreen, &tokens, size, scale, what, &path)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum What {
    Components,
    Overlays,
    Icons,
    Phone,
    PhoneOverlays,
}

fn picture(
    offscreen: &mut Offscreen,
    tokens: &Tokens,
    size: [u32; 2],
    scale: f32,
    what: What,
    path: &PathBuf,
) -> Result<(), String> {
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(scale);
    let pixels = size.map(|n| (n as f32 * scale).round() as u32);
    tokens.apply(&ctx);
    let mut fonts = UiFonts::default();
    let mut gallery = GalleryState::default();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(
            Pos2::ZERO,
            vec2(size[0] as f32, size[1] as f32),
        )),
        time: Some(0.0),
        ..Default::default()
    };
    // The fonts arrive in the first pass and take effect in the next.
    let passes = 3;
    for pass in 0..passes {
        gallery.targets.clear();
        let output = ctx.run_ui(input(), |ui| {
            fonts.update(ui);
            draw(ui, tokens, what, &mut gallery);
        });
        let read = pass + 1 == passes;
        if let Some(frame) = offscreen.paint(&ctx, output, pixels, read)? {
            let png = craft3d_render::encode_png(frame.width, frame.height, &frame.rgb)
                .map_err(|e| e.to_string())?;
            std::fs::write(path, png).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn draw(ui: &mut egui::Ui, tokens: &Tokens, what: What, gallery: &mut GalleryState) {
    let screen = ui.max_rect();
    // The backdrop under the boards, as the mock-ups have one: a field's
    // outline, warm grey in the light set and dark in the dark one.
    ui.painter()
        .rect_filled(screen, 0.0, color(tokens.field_stroke));
    let margin = match what {
        What::Phone | What::PhoneOverlays => 0.0,
        _ => 40.0,
    };
    let inner = screen.shrink(margin);
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        ui.add_space(margin * 0.5);
        let half = (inner.width() - 40.0) * 0.5;
        let set = if tokens.is_dark() {
            "Dark mode"
        } else {
            "Light"
        };
        match what {
            What::Components => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 40.0;
                    gallery::board(
                        ui,
                        tokens,
                        Ground::Panel,
                        &format!("{set}: panel"),
                        half,
                        |ui| {
                            gallery::components(ui, &Kit::panel(tokens), gallery);
                        },
                    );
                    let plate = if tokens.is_dark() {
                        "Dark mode: popover, one step lighter"
                    } else {
                        "Light: plate (popovers, menus, bars)"
                    };
                    gallery::board(ui, tokens, Ground::Plate, plate, half, |ui| {
                        gallery::components(ui, &Kit::plate(tokens), gallery);
                    });
                });
            }
            What::Overlays => {
                gallery::board(
                    ui,
                    tokens,
                    Ground::Panel,
                    &format!("{set}: menus, popovers, tooltip, status pill"),
                    inner.width(),
                    |ui| gallery::overlays(ui, tokens, gallery),
                );
            }
            What::Icons => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 40.0;
                    gallery::board(
                        ui,
                        tokens,
                        Ground::Panel,
                        &format!("{set}: icons on a panel"),
                        half,
                        |ui| {
                            gallery::icons(ui, &Kit::panel(tokens));
                        },
                    );
                    gallery::board(ui, tokens, Ground::Plate, "Icons on a plate", half, |ui| {
                        gallery::icons(ui, &Kit::plate(tokens));
                    });
                });
            }
            What::Phone => {
                gallery::board(
                    ui,
                    tokens,
                    Ground::Panel,
                    &format!("{set}: 390 points, touch"),
                    inner.width(),
                    |ui| {
                        gallery::components(ui, &Kit::panel(tokens), gallery);
                    },
                );
            }
            What::PhoneOverlays => {
                gallery::board(
                    ui,
                    tokens,
                    Ground::Panel,
                    &format!("{set}: overlays at 390"),
                    inner.width(),
                    |ui| {
                        gallery::overlays(ui, tokens, gallery);
                    },
                );
            }
        }
    });
}
