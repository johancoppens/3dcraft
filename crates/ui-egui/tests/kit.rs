//! The component kit (`craft3d_ui_egui::kit`): every component drawn
//! without a panic at a window's width and a phone's, in both appearances
//! and on both grounds; inside the window at 390 points; every target a
//! finger's size with touch sizes and the planned size without; the
//! controls answer clicks; the line icons stay in their place; the app
//! icons read; and the appearance through `ui.set` and `ui.get`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::kit::gallery::{self, GalleryState};
use craft3d_ui_egui::kit::{
    Button, Ground, Kit, ListRow, MenuItem, PillState, Segment, Slider, brand, glyphs,
};
use craft3d_ui_egui::theme::{Palette, Tokens, color};
use egui::{Event, PointerButton, Pos2, Rect, pos2, vec2};
use serde_json::{Value, json};

/// A pass of `add`, its textures taken as handled (nothing paints them).
fn run(
    ctx: &egui::Context,
    input: egui::RawInput,
    add: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut add = add;
    let mut output = ctx.run_ui(input, |ui| add(ui));
    output.textures_delta.clear();
    output
}

fn input(size: [f32; 2], events: Vec<Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(size[0], size[1]))),
        time: Some(0.0),
        events,
        ..Default::default()
    }
}

/// Every board of the gallery in one pass of `size`; the targets laid out
/// and how many shapes were painted.
fn gallery_pass(tokens: &Tokens, size: [f32; 2]) -> (Vec<(&'static str, Rect)>, usize) {
    let ctx = egui::Context::default();
    tokens.apply(&ctx);
    let mut state = GalleryState::default();
    let mut shapes = 0;
    for _ in 0..2 {
        state.targets.clear();
        let output = run(&ctx, input(size, Vec::new()), |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for ground in [Ground::Panel, Ground::Plate] {
                    let kit = Kit::new(tokens, ground);
                    gallery::components(ui, &kit, &mut state);
                    gallery::icons(ui, &kit);
                }
                gallery::overlays(ui, tokens, &mut state);
            });
        });
        shapes = output.shapes.len();
    }
    (state.targets, shapes)
}

fn sets() -> Vec<(&'static str, Tokens)> {
    vec![
        ("light", Tokens::light()),
        ("dark", Tokens::dark()),
        ("light touch", Tokens::light().touch()),
        ("dark touch", Tokens::dark().touch()),
    ]
}

/// Every component at 1024 × 700 and at a phone's 390 × 844, in both
/// appearances, with and without touch sizes: drawn, nothing outside the
/// window's width.
#[test]
fn every_component_draws_at_a_window_and_a_phone_width() {
    for (set, tokens) in sets() {
        for size in [[1024.0, 700.0], [390.0, 844.0]] {
            let (targets, shapes) = gallery_pass(&tokens, size);
            assert!(shapes > 200, "{set} {size:?}: only {shapes} shapes");
            for what in [
                "button",
                "icon button",
                "chip",
                "segment",
                "checkbox",
                "toggle",
                "number field",
                "text field",
                "dropdown",
                "slider",
                "fold header",
                "list row",
                "tab",
                "menu item",
                "pill",
            ] {
                assert!(
                    targets.iter().any(|(w, _)| *w == what),
                    "{set} {size:?}: no {what}"
                );
            }
            // A menu's items stand one under the other (laid out in a row
            // they once ran over each other).
            let mut tops: Vec<i64> = targets
                .iter()
                .filter(|(w, _)| *w == "menu item")
                .map(|(_, r)| r.top().round() as i64)
                .collect();
            tops.sort_unstable();
            tops.dedup();
            assert!(
                tops.len() >= 7,
                "{set} {size:?}: menu items in {} rows",
                tops.len()
            );
            for (what, rect) in &targets {
                assert!(
                    rect.left() >= -0.5 && rect.right() <= size[0] + 0.5,
                    "{set} {size:?}: a {what} lies outside the window: {rect:?}"
                );
            }
        }
    }
}

/// With touch sizes every target is at least `touch_target` (40) each way;
/// without them buttons are `control_height` (24) high and list rows
/// `row_height` (22).
#[test]
fn targets_are_a_finger_large_with_touch_sizes() {
    for tokens in [Tokens::light().touch(), Tokens::dark().touch()] {
        let least = tokens.touch_target;
        assert_eq!(least, 40.0);
        let (targets, _) = gallery_pass(&tokens, [390.0, 844.0]);
        for (what, rect) in targets {
            assert!(
                rect.height() >= least - 0.01 && rect.width() >= least - 0.01,
                "a {what} of {:?} is smaller than {least}",
                rect.size()
            );
        }
    }
    let tokens = Tokens::light();
    let (targets, _) = gallery_pass(&tokens, [1024.0, 700.0]);
    let heights = |name: &str| -> Vec<f32> {
        targets
            .iter()
            .filter(|(w, _)| *w == name)
            .map(|(_, r)| r.height())
            .collect()
    };
    for h in heights("button").into_iter().chain(heights("icon button")) {
        assert_eq!(h, tokens.control_height);
    }
    for h in heights("list row") {
        assert_eq!(h, tokens.row_height);
    }
    for h in heights("menu item") {
        assert_eq!(h, tokens.menu_item_height);
    }
    assert_eq!((tokens.row_height, tokens.control_height), (22.0, 24.0));
}

/// The kit takes its colours from the appearance: a dark panel, a popover
/// a step lighter in the dark set; the plate's are the same in both but
/// for that step.
#[test]
fn the_kit_follows_the_appearance() {
    let (light, dark) = (Tokens::light(), Tokens::dark());
    assert_eq!(Kit::panel(&light).skin.fill, light.panel_fill);
    assert_eq!(Kit::panel(&dark).skin.fill, Palette::dark().panel_fill);
    assert_eq!(Kit::panel(&dark).skin.text, Palette::dark().text);
    assert_eq!(Kit::plate(&light).skin.fill, light.plate_fill);
    // One step above the dark panel (#161514): the developer's darkest
    // dark set of 2026-11-04.
    assert_eq!(Kit::plate(&dark).skin.fill, [0x20, 0x1F, 0x1D]);
    assert_eq!(Kit::plate(&dark).skin.text, Kit::plate(&light).skin.text);
    assert!(!Kit::plate(&light).touch() && Kit::plate(&light.touch()).touch());
}

/// A frame of `add` with a click at `at` (pressed and released).
fn click_frames(
    ctx: &egui::Context,
    at: Option<Pos2>,
    button: PointerButton,
    mut add: impl FnMut(&mut egui::Ui),
) {
    let mut frames = vec![vec![Event::PointerMoved(pos2(1.0, 1.0))]];
    if let Some(at) = at {
        frames.push(vec![Event::PointerMoved(at)]);
        frames.push(vec![Event::PointerButton {
            pos: at,
            button,
            pressed: true,
            modifiers: Default::default(),
        }]);
        frames.push(vec![Event::PointerButton {
            pos: at,
            button,
            pressed: false,
            modifiers: Default::default(),
        }]);
    }
    frames.push(Vec::new());
    for events in frames {
        let _ = run(ctx, input([600.0, 400.0], events), &mut add);
    }
}

/// A click chooses a segment, ticks a box, moves a slider, picks from a
/// dropdown and opens a context menu.
#[test]
fn the_controls_answer_clicks() {
    let tokens = Tokens::light();
    let kit = Kit::panel(&tokens);
    let ctx = egui::Context::default();
    tokens.apply(&ctx);

    // A segment: where the third one was drawn.
    let mut chosen = 0;
    let mut items = Vec::new();
    click_frames(&ctx, None, PointerButton::Primary, |ui| {
        items = kit
            .segmented(
                ui,
                &mut chosen,
                &[Segment::Text("A"), Segment::Text("B"), Segment::Text("C")],
            )
            .items;
    });
    let third = items[2].center();
    click_frames(&ctx, Some(third), PointerButton::Primary, |ui| {
        kit.segmented(
            ui,
            &mut chosen,
            &[Segment::Text("A"), Segment::Text("B"), Segment::Text("C")],
        );
    });
    assert_eq!(chosen, 2);

    // A check box.
    let mut on = false;
    let mut at = Rect::NOTHING;
    click_frames(&ctx, None, PointerButton::Primary, |ui| {
        at = kit.checkbox(ui, &mut on, "Shadows").rect;
    });
    click_frames(&ctx, Some(at.center()), PointerButton::Primary, |ui| {
        kit.checkbox(ui, &mut on, "Shadows");
    });
    assert!(on);

    // A slider clicked at the end of its rail.
    let mut value = 0.0;
    let mut rail = Rect::NOTHING;
    click_frames(&ctx, None, PointerButton::Primary, |ui| {
        rail = kit
            .slider(ui, Slider::new("Exposure", &mut value, 0.0..=10.0))
            .rect;
    });
    let end = pos2(rail.right() - 1.0, rail.center().y);
    click_frames(&ctx, Some(end), PointerButton::Primary, |ui| {
        kit.slider(ui, Slider::new("Exposure", &mut value, 0.0..=10.0));
    });
    assert!(value > 9.0, "{value}");

    // A button reports its click; a disabled one does not.
    let mut clicked = (false, false);
    let mut places = (Rect::NOTHING, Rect::NOTHING);
    click_frames(&ctx, None, PointerButton::Primary, |ui| {
        ui.horizontal(|ui| {
            places.0 = kit.button(ui, Button::primary("Export")).rect;
            places.1 = kit
                .button(ui, Button::secondary("Open…").enabled(false))
                .rect;
        });
    });
    for (place, slot) in [(places.0, 0), (places.1, 1)] {
        click_frames(&ctx, Some(place.center()), PointerButton::Primary, |ui| {
            ui.horizontal(|ui| {
                let a = kit.button(ui, Button::primary("Export")).clicked();
                let b = kit
                    .button(ui, Button::secondary("Open…").enabled(false))
                    .clicked();
                if slot == 0 {
                    clicked.0 |= a;
                } else {
                    clicked.1 |= b;
                }
            });
        });
    }
    assert_eq!(clicked, (true, false));

    // A dropdown: a click opens it, a click on an option picks it.
    let mut pick = 0;
    let mut field = Rect::NOTHING;
    let options = ["STEP", "STL", "3MF"];
    click_frames(&ctx, None, PointerButton::Primary, |ui| {
        field = kit.dropdown(ui, "format", &mut pick, &options, 150.0).rect;
    });
    click_frames(&ctx, Some(field.center()), PointerButton::Primary, |ui| {
        kit.dropdown(ui, "format", &mut pick, &options, 150.0);
    });
    // The menu hangs under the field: its third item, a menu item's
    // height apart.
    let third = pos2(
        field.left() + 30.0,
        field.bottom() + 2.0 + tokens.nav_padding + tokens.menu_item_height * 2.5,
    );
    click_frames(&ctx, Some(third), PointerButton::Primary, |ui| {
        kit.dropdown(ui, "format", &mut pick, &options, 150.0);
    });
    assert_eq!(pick, 2);

    // A context menu opens on a secondary click and closes on its item.
    let mut row = Rect::NOTHING;
    let mut open = false;
    click_frames(&ctx, None, PointerButton::Primary, |ui| {
        row = kit.list_row(ui, ListRow::new("Bore")).rect;
    });
    click_frames(&ctx, Some(row.center()), PointerButton::Secondary, |ui| {
        let response = kit.list_row(ui, ListRow::new("Bore"));
        open = kit.context_menu(ui, &response, |plate, ui| {
            plate.menu_item(ui, MenuItem::new("Rename")).clicked()
        });
    });
    assert!(open, "the context menu did not open");
    let item = pos2(
        row.center().x + 30.0,
        row.center().y + tokens.nav_padding + tokens.menu_item_height * 0.5,
    );
    click_frames(&ctx, Some(item), PointerButton::Primary, |ui| {
        let response = kit.list_row(ui, ListRow::new("Bore"));
        open = kit.context_menu(ui, &response, |plate, ui| {
            plate.menu_item(ui, MenuItem::new("Rename")).clicked()
        });
    });
    assert!(!open, "choosing an item closes the context menu");
}

/// Hostile values draw without a panic: a slider at NaN or with an empty
/// range, a field narrower than nothing, an empty list, a choice beyond
/// the options.
#[test]
fn hostile_values_draw_without_a_panic() {
    let tokens = Tokens::dark().touch();
    let kit = Kit::plate(&tokens);
    let ctx = egui::Context::default();
    for size in [[0.0, 0.0], [10.0, 10.0], [390.0, 844.0]] {
        let _ = run(&ctx, input(size, Vec::new()), |ui| {
            let mut v = f64::NAN;
            kit.slider(ui, Slider::new("x", &mut v, 5.0..=1.0));
            let mut v = 1.0;
            kit.slider(
                ui,
                Slider::new("", &mut v, f64::NEG_INFINITY..=f64::INFINITY),
            );
            let mut text = String::new();
            kit.number_field(ui, "n", &mut text, "mm", -5.0, true);
            kit.text_field(ui, "t", &mut text, "", None, f32::NAN);
            let mut chosen = 99;
            kit.dropdown(ui, "d", &mut chosen, &[], f32::INFINITY);
            kit.segmented(ui, &mut chosen, &[]);
            kit.tab_bar(ui, &mut chosen, &[]);
            kit.progress(ui, "", f32::NAN, -1.0);
            kit.pill(ui, PillState::Working, "");
            kit.list_row(ui, ListRow::new("").indent(255));
            // Popovers too narrow for their pointer, or of no size at all
            // (crossed bounds once panicked in `f32::clamp`).
            for (width, pointer) in [
                (0.0, Some(0.0)),
                (10.0, Some(5.0)),
                (31.0, Some(30.0)),
                (f32::NAN, Some(1.0)),
                (100.0, Some(f32::NAN)),
                (100.0, Some(f32::INFINITY)),
                (-50.0, None),
            ] {
                kit.popover_frame(ui, width, pointer, |ui| {
                    ui.label("x");
                });
            }
            for anchor in [
                Rect::NOTHING,
                Rect::from_min_size(Pos2::ZERO, vec2(0.0, 0.0)),
                Rect::from_min_max(pos2(f32::NAN, 0.0), pos2(f32::NAN, 1.0)),
            ] {
                for width in [0.0, 20.0, f32::NAN, 5000.0] {
                    kit.popover(
                        ui.ctx(),
                        ui.id().with(("p", width.to_bits())),
                        anchor,
                        width,
                        |ui| {
                            ui.label("x");
                        },
                    );
                }
            }
            kit.menu_frame(ui, f32::NAN, |ui| {
                kit.menu_item(ui, MenuItem::new(""));
            });
            kit.tooltip_box(ui, "a tip", f32::NAN);
            kit.tooltip_box(ui, "", 0.0);
        });
    }
}

/// Every line icon stays within the place it is given (a stroke's width
/// more), at 16 and at 32 points.
#[test]
fn every_icon_stays_in_its_place() {
    let tokens = Tokens::light();
    let ctx = egui::Context::default();
    let all: Vec<_> = glyphs::Glyph::FEATURES
        .iter()
        .chain(glyphs::Glyph::DOCUMENTS.iter())
        .chain(glyphs::Glyph::CONTROLS.iter())
        .copied()
        .collect();
    for side in [16.0, 32.0] {
        for glyph in &all {
            let place = Rect::from_min_size(pos2(40.0, 40.0), vec2(side, side));
            let output = run(&ctx, input([200.0, 200.0], Vec::new()), |ui| {
                glyphs::paint(ui.painter(), *glyph, place, color(tokens.text), &tokens);
            });
            let shapes: Vec<_> = output.shapes.into_iter().map(|c| c.shape).collect();
            assert!(!shapes.is_empty(), "{glyph:?} draws nothing");
            assert!(
                glyphs::bounds_ok(&shapes, place),
                "{glyph:?} at {side} leaves its place"
            );
        }
    }
    // Every feature kind has an icon of its own, read from the document
    // model's own list (`FeatureKind::type_name`), so a kind added later
    // fails here until it has one; an unknown one gets the generic icon.
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../doc/src/lib.rs"),
    )
    .unwrap();
    let named = |prefix: &str| -> Vec<String> {
        source
            .lines()
            .map(str::trim_start)
            .filter(|l| l.starts_with(prefix) && l.contains("=> \"") && l.ends_with("\","))
            .filter_map(|l| l.split('"').nth(1).map(str::to_string))
            .collect()
    };
    let kinds = named("FeatureKind::");
    assert!(kinds.len() >= 37, "{kinds:?}");
    for kind in &kinds {
        assert_ne!(
            glyphs::Glyph::for_feature(kind, None),
            glyphs::Glyph::Feature,
            "the feature kind `{kind}` has no icon"
        );
    }
    // Each boolean operation (`BooleanOp::name`) its own icon, sheets
    // joined (sew) too.
    let ops = named("BooleanOp::");
    assert!(ops.contains(&"sew".to_string()), "{ops:?}");
    let mut seen = Vec::new();
    for op in &ops {
        let glyph = glyphs::Glyph::for_feature("boolean", Some(op));
        assert!(!seen.contains(&glyph), "`{op}` shares its icon");
        seen.push(glyph);
    }
    assert_eq!(
        glyphs::Glyph::for_feature("nonsense", None),
        glyphs::Glyph::Feature
    );
}

#[test]
fn the_app_icons_read() {
    assert!(brand::all_read());
}

fn call(
    shell: &mut Shell,
    session: &mut Session,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    shell.handle_method(session, method, Some(&params)).unwrap()
}

/// `ui.set {"appearance": …}` switches the panels' colours after the next
/// frame and `ui.get` says which are in force; the viewport's tokens stay
/// as they are; a wrong name changes nothing.
#[test]
fn the_appearance_is_set_and_read_over_the_control_channel() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let ctx = egui::Context::default();
    let frame = |shell: &mut Shell, session: &mut Session| {
        let _ = run(&ctx, input([1024.0, 700.0], Vec::new()), |ui| {
            shell.ui(ui, session)
        });
    };
    frame(&mut shell, &mut session);
    let got = call(&mut shell, &mut session, "ui.get", json!({})).unwrap();
    assert_eq!(got["state"]["appearance"], json!("system"));
    // No system preference in a test: light.
    assert_eq!(got["appearance"], json!("light"));
    let viewport = shell.tokens.viewport_background;

    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"appearance": "dark"}),
    )
    .unwrap();
    // Said at once, before a frame has drawn it.
    let got = call(&mut shell, &mut session, "ui.get", json!({})).unwrap();
    assert_eq!(got["appearance"], json!("dark"));
    frame(&mut shell, &mut session);
    let got = call(&mut shell, &mut session, "ui.get", json!({})).unwrap();
    assert_eq!(got["state"]["appearance"], json!("dark"));
    assert_eq!(got["appearance"], json!("dark"));
    assert_eq!(shell.tokens.panel_fill, Palette::dark().panel_fill);
    assert_eq!(shell.tokens.viewport_background, viewport);
    assert!(shell.dark());

    let why = call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"appearance": "dusk"}),
    )
    .unwrap_err();
    assert!(why.starts_with("ui.set"), "{why}");
    assert_eq!(
        shell.state.appearance,
        craft3d_ui_egui::state::Appearance::Dark
    );

    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"appearance": "light"}),
    )
    .unwrap();
    frame(&mut shell, &mut session);
    assert_eq!(shell.tokens.panel_fill, Tokens::light().panel_fill);
    assert!(!shell.dark());
}

/// What the window paints over the viewport, as text: every shape that
/// lies wholly inside the viewport's place.
fn viewport_shapes(shell: &mut Shell, session: &mut Session, ctx: &egui::Context) -> Vec<String> {
    let mut last = None;
    for _ in 0..3 {
        last = Some(run(ctx, input([1024.0, 700.0], Vec::new()), |ui| {
            shell.ui(ui, session);
        }));
    }
    let output = last.unwrap();
    let place = shell.viewport_rect().expect("a viewport");
    let mut shapes: Vec<String> = output
        .shapes
        .iter()
        .filter(|c| {
            let b = c.shape.visual_bounding_rect();
            b.is_positive() && place.contains_rect(b)
        })
        .map(|c| format!("{:?}", c.shape))
        .collect();
    shapes.sort();
    shapes
}

/// The viewport is not themed: what is painted over it (a failed
/// feature's problem, the axes, the plates of the controls, the view
/// cube) is the same shape for shape in both appearances.
#[test]
fn the_viewport_is_painted_the_same_in_both_appearances() {
    let mut session = Session::new(Workspace::none());
    for (id, params) in [
        ("file.new", json!({})),
        (
            "feature.block",
            json!({"name": "Block", "size": [10, 10, 10]}),
        ),
        (
            "feature.fillet",
            json!({"edges": [["F1:face(0)", "F1:face(2)"]], "radius": 100}),
        ),
    ] {
        let _ = session.execute(id, &params);
    }
    let mut shell = Shell::new();
    let ctx = egui::Context::default();
    let light = viewport_shapes(&mut shell, &mut session, &ctx);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"appearance": "dark"}),
    )
    .unwrap();
    let dark = viewport_shapes(&mut shell, &mut session, &ctx);
    assert!(shell.dark());
    assert!(light.len() > 5, "{}", light.len());
    let differ: Vec<_> = light.iter().filter(|s| !dark.contains(s)).take(3).collect();
    assert!(
        differ.is_empty() && light.len() == dark.len(),
        "{differ:#?}"
    );
}
