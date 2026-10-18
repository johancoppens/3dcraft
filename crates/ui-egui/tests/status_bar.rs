//! The status bar at the bottom of the window: its items never overlap.
//! When the bar is too narrow they give way in a fixed order
//! (`STATUS_YIELD`): first elided, then left out.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::{
    CommandReport, RendererChoice, STATUS_YIELD, Shell, StatusFit, StatusSlot, fit_status,
};
use egui::{Pos2, Rect, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn items() -> Vec<StatusFit> {
    let item = |slot, natural: f32, least: f32| StatusFit {
        slot,
        natural,
        least,
    };
    vec![
        item(StatusSlot::Units, 50.0, 50.0),
        item(StatusSlot::Path, 400.0, 48.0),
        item(StatusSlot::Autosave, 120.0, 48.0),
        item(StatusSlot::Command, 60.0, 60.0),
        item(StatusSlot::Answer, 500.0, 48.0),
        item(StatusSlot::Render, 250.0, 250.0),
    ]
}

/// The widths in all, with the gaps between the items shown.
fn total(widths: &[Option<f32>], gap: f32) -> f32 {
    let shown: Vec<f32> = widths.iter().flatten().copied().collect();
    shown.iter().sum::<f32>() + gap * shown.len().saturating_sub(1) as f32
}

/// Wide enough, every item is as wide as it wants. Narrower, never more
/// than the room there is: the answer is elided first, then the autosave
/// and then the path; then the answer and the command go, then the
/// autosave, the path, the renderer's numbers, and the units last. An item
/// that has gone never comes back in a narrower bar, and none is narrower
/// than its least width.
#[test]
fn the_items_give_way_in_a_fixed_order() {
    let items = items();
    let gap = 13.0;
    let all = fit_status(10_000.0, gap, &items);
    for (item, width) in items.iter().zip(&all) {
        assert_eq!(*width, Some(item.natural));
    }
    let mut gone_before: Vec<StatusSlot> = Vec::new();
    let mut available = 1500.0;
    while available >= 0.0 {
        let widths = fit_status(available, gap, &items);
        assert!(
            total(&widths, gap) <= available + 1e-3,
            "{available}: {widths:?}"
        );
        for (item, width) in items.iter().zip(&widths) {
            if let Some(w) = width {
                assert!(*w >= item.least - 1e-3 && *w <= item.natural, "{available}");
            }
        }
        let gone: Vec<StatusSlot> = items
            .iter()
            .zip(&widths)
            .filter(|(_, w)| w.is_none())
            .map(|(i, _)| i.slot)
            .collect();
        // Left out in the order of STATUS_YIELD, and for good.
        let expected: Vec<StatusSlot> = STATUS_YIELD
            .iter()
            .copied()
            .filter(|s| gone.contains(s))
            .collect();
        assert_eq!(STATUS_YIELD[..gone.len()].to_vec(), expected, "{available}");
        assert!(gone_before.iter().all(|s| gone.contains(s)), "{available}");
        gone_before = gone;
        // Something is elided only once everything before it in the order
        // is as narrow as it may be.
        for (k, slot) in STATUS_YIELD.iter().enumerate() {
            let at = items.iter().position(|i| i.slot == *slot).unwrap();
            if widths[at].is_some_and(|w| w < items[at].natural) {
                for earlier in &STATUS_YIELD[..k] {
                    let e = items.iter().position(|i| i.slot == *earlier).unwrap();
                    assert!(
                        widths[e].is_none_or(|w| (w - items[e].least).abs() < 1e-3),
                        "{available}: {slot:?} elided before {earlier:?}"
                    );
                }
            }
        }
        available -= 7.0;
    }
    assert!(fit_status(0.0, gap, &items).iter().all(Option::is_none));
    // Numbers that are not numbers take no room.
    let odd = fit_status(f32::NAN, f32::INFINITY, &items);
    assert!(odd.iter().all(Option::is_none));
}

/// In the window, at 1280 and at 1920 wide, and in a narrow window, with
/// a long path and a long answer: the items are drawn in the bar, inside
/// the window, and no two overlap.
#[test]
fn the_items_never_overlap_in_the_window() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("status-bar");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(&mut session, "feature.block", json!({"size": [10, 10, 10]}));
    let long = "a-rather-long-name-of-a-weld-table-with-ribs-and-slots-for-testing";
    run(
        &mut session,
        "file.save",
        json!({"path": format!("{long}.3dcraft")}),
    );
    for (width, height) in [(1280.0, 800.0), (1920.0, 1080.0), (640.0, 480.0)] {
        let mut shell = Shell::with_renderer(RendererChoice::Cpu);
        shell.last_command = Some(CommandReport {
            label: "file.open".to_string(),
            ok: true,
            text: format!("{{\"document\":0,\"features\":17,\"path\":\"/home/someone/{long}\"}}"),
        });
        let ctx = egui::Context::default();
        for k in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, height))),
                time: Some(f64::from(k) / 60.0),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| shell.ui(ui, &mut session));
            output.textures_delta.clear();
        }
        let hits = shell.status_hits().to_vec();
        let slots: Vec<StatusSlot> = hits.iter().map(|(s, _)| *s).collect();
        assert!(slots.contains(&StatusSlot::Units), "{width}: {slots:?}");
        if width >= 1280.0 {
            for slot in [StatusSlot::Path, StatusSlot::Command, StatusSlot::Render] {
                assert!(slots.contains(&slot), "{width}: no {slot:?} in {slots:?}");
            }
        }
        for (k, (slot, rect)) in hits.iter().enumerate() {
            assert!(
                rect.left() >= -0.5 && rect.right() <= width + 0.5 && rect.bottom() <= height + 0.5,
                "{width}: {slot:?} {rect:?}"
            );
            for (other, second) in hits.iter().skip(k + 1) {
                assert!(
                    rect.right() <= second.left() + 0.5,
                    "{width}: {slot:?} {rect:?} runs into {other:?} {second:?}"
                );
            }
        }
    }
}
