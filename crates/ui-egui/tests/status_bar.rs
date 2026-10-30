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
        item(StatusSlot::Selection, 180.0, 48.0),
        item(StatusSlot::Command, 60.0, 60.0),
        item(StatusSlot::Answer, 500.0, 48.0),
        item(StatusSlot::Work, 250.0, 48.0),
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
/// autosave, the path, the work in progress, the renderer's numbers, and
/// the units last. An item
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

/// Work in progress has its own item at the right of the bar, before the
/// renderer's numbers, always as wide (`Tokens::status_work_width`)
/// whatever it says, so nothing moves while the steps count up; a long
/// name is elided within it. Without work (and once the indicator's
/// hold is over), it is not there.
#[test]
fn work_in_progress_has_a_place_of_its_own_that_never_changes_size() {
    use craft3d_ui_egui::{Tokens, Work};
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(&mut session, "feature.block", json!({"size": [10, 10, 10]}));
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let ctx = egui::Context::default();
    // Frames 0.2 s apart: work shows once it has run
    // `Tokens::work_after_seconds` (0.35 s) and stays
    // `Tokens::work_hold_seconds` (0.3 s) after it is done.
    let mut clock = 0_u32;
    let mut frame = |shell: &mut Shell, _: u32| {
        clock += 1;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(f64::from(clock) * 0.2),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| shell.ui(ui, &mut session));
        output.textures_delta.clear();
    };
    for k in 0..3 {
        frame(&mut shell, k);
    }
    let slots = |shell: &Shell| {
        shell
            .status_hits()
            .iter()
            .map(|(s, _)| *s)
            .collect::<Vec<_>>()
    };
    assert!(!slots(&shell).contains(&StatusSlot::Work));
    let width = Tokens::default().status_work_width;
    let long = "a-very-long-name-of-a-welding-table-with-ribs-and-slots-and-more";
    let mut rects = Vec::new();
    let mut renders = Vec::new();
    for (subject, step) in [("welding-table", 1), ("welding-table", 41), (long, 412)] {
        shell.set_work(Some(Work {
            label: "file.open".to_string(),
            subject: Some(subject.to_string()),
            phase: "regenerating".to_string(),
            step,
            steps: 412,
            ..Work::default()
        }));
        for k in 3..6 {
            frame(&mut shell, k);
        }
        let hits = shell.status_hits().to_vec();
        let (_, rect) = hits
            .iter()
            .find(|(s, _)| *s == StatusSlot::Work)
            .copied()
            .unwrap_or_else(|| panic!("no work item in {hits:?}"));
        // The words within the item's room (the hit is the words'); the
        // room itself shows in where the item starts, which does not move
        // below.
        assert!(rect.width() <= width + 0.5, "{rect:?}");
        // Before the renderer's numbers, which stay at the right end.
        let (_, render) = hits
            .iter()
            .find(|(s, _)| *s == StatusSlot::Render)
            .copied()
            .unwrap_or_else(|| panic!("no renderer's numbers in {hits:?}"));
        assert!(rect.right() <= render.left() + 0.5, "{rect:?} {render:?}");
        // The item is as wide as its room: the next starts that far on.
        assert!(
            render.left() - rect.left() >= width - 0.5,
            "{rect:?} {render:?}"
        );
        rects.push(rect);
        renders.push(render);
    }
    assert!(
        rects
            .windows(2)
            .all(|w| (w[0].left() - w[1].left()).abs() < 0.5),
        "{rects:?}"
    );
    assert!(
        renders
            .windows(2)
            .all(|w| (w[0].left() - w[1].left()).abs() < 0.5),
        "{renders:?}"
    );
    assert_eq!(
        shell.busy().map(|(text, _)| text),
        Some(format!("Opening {long}\u{2026} 412/412"))
    );
    shell.set_work(None);
    for k in 6..9 {
        frame(&mut shell, k);
    }
    assert!(!slots(&shell).contains(&StatusSlot::Work));
}

/// What the status bar says for the kinds of work.
#[test]
fn the_words_for_work_in_progress() {
    use craft3d_ui_egui::Work;
    let work = |label: &str, subject: Option<&str>, phase: &str, step, steps| Work {
        label: label.to_string(),
        subject: subject.map(str::to_string),
        phase: phase.to_string(),
        step,
        steps,
        ..Work::default()
    };
    let table = Some("welding-table");
    for (w, said) in [
        (
            work("file.open", table, "regenerating", 120, 412),
            "Opening welding-table\u{2026} 120/412",
        ),
        (
            work("file.open", table, "meshing", 3, 82),
            "Opening welding-table\u{2026} meshing",
        ),
        (work("file.open", None, "running", 0, 0), "Opening\u{2026}"),
        (
            work("param.set", None, "regenerating", 3, 12),
            "Regenerating\u{2026} 3/12",
        ),
        (work("meshing", None, "meshing", 3, 12), "Meshing\u{2026}"),
        (
            work("feature.extrude", None, "running", 0, 0),
            "feature.extrude\u{2026}",
        ),
    ] {
        assert_eq!(w.status_text(), said);
    }
}

/// The calm status bar (docs/design/engine.md › *Saying what is being
/// done*): a motion played as short jobs 10 times a second (each 50 ms of
/// work and a new answer) never shows the work indicator, and its texts
/// change at most 4 times a second; one long job shows it only after
/// `Tokens::work_after_seconds` and keeps it `Tokens::work_hold_seconds`
/// after it is done; an error shows in the frame after it came.
#[test]
fn short_jobs_never_show_the_indicator_and_the_texts_change_calmly() {
    use craft3d_ui_egui::{Tokens, Work};
    let tokens = Tokens::default();
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(&mut session, "feature.block", json!({"size": [10, 10, 10]}));
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let ctx = egui::Context::default();
    let step = 1.0 / 60.0;
    let mut now = 0.0_f64;
    let mut frame = |shell: &mut Shell, now: &mut f64| {
        *now += step;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(*now),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| shell.ui(ui, &mut session));
        output.textures_delta.clear();
    };
    for _ in 0..3 {
        frame(&mut shell, &mut now);
    }
    let motion = |k: usize| Work {
        label: "view.motion".to_string(),
        phase: "running".to_string(),
        feature: Some(format!("moment {k}")),
        seconds: 0.05,
        ..Work::default()
    };
    // 4 s of a motion: 40 jobs of 3 frames each, 3 frames apart.
    let changes_before = shell.status_changes();
    let start = now;
    for k in 0..40 {
        shell.set_work(Some(motion(k)));
        shell.last_command = Some(CommandReport {
            label: "view.motion".to_string(),
            ok: true,
            text: format!("{{\"t\":{}}}", k as f64 / 40.0),
        });
        for _ in 0..3 {
            frame(&mut shell, &mut now);
            assert_eq!(shell.work_shown(), None, "job {k}");
        }
        shell.set_work(None);
        for _ in 0..3 {
            frame(&mut shell, &mut now);
            assert_eq!(shell.work_shown(), None, "after job {k}");
        }
    }
    let seconds = now - start;
    let changes = shell.status_changes() - changes_before;
    assert!(
        (changes as f64) <= seconds * 4.0 + 2.0,
        "{changes} changes in {seconds:.2} s"
    );
    // One long job: not shown before the wait, shown after it.
    let after = f64::from(tokens.work_after_seconds);
    let hold = f64::from(tokens.work_hold_seconds);
    shell.set_work(Some(Work {
        label: "param.set".to_string(),
        phase: "regenerating".to_string(),
        step: 1,
        steps: 9,
        ..Work::default()
    }));
    let begun = now + step;
    let mut first_shown = None;
    while now < begun + 1.0 {
        frame(&mut shell, &mut now);
        if shell.work_shown().is_some() && first_shown.is_none() {
            first_shown = Some(now);
        }
        if first_shown.is_none() {
            assert!(now - begun < after + 1e-9, "not shown at {}", now - begun);
        }
    }
    let first_shown = first_shown.expect("the long job shows the indicator");
    assert!(
        first_shown - begun >= after - 1e-9,
        "{}",
        first_shown - begun
    );
    assert_eq!(shell.work_shown(), Some("Regenerating\u{2026} 1/9"));
    assert_eq!(
        shell.status_shown(StatusSlot::Work),
        Some("Regenerating\u{2026} 1/9")
    );
    // Done: it stays for the hold, then goes.
    shell.set_work(None);
    let done = now;
    while now < done + hold - step {
        frame(&mut shell, &mut now);
        assert!(shell.work_shown().is_some(), "gone {} s after", now - done);
    }
    while now < done + hold + 2.0 * step {
        frame(&mut shell, &mut now);
    }
    assert_eq!(shell.work_shown(), None);
    // An error shows at once, and stays.
    shell.last_command = Some(CommandReport {
        label: "feature.extrude".to_string(),
        ok: false,
        text: "the profile is not closed".to_string(),
    });
    frame(&mut shell, &mut now);
    assert_eq!(
        shell.status_shown(StatusSlot::Command),
        Some("feature.extrude")
    );
    // Its answer in the same frame: the label and the answer are one.
    assert_eq!(
        shell.status_shown(StatusSlot::Answer),
        Some("the profile is not closed")
    );
    // A success soon after waits its turn, label and answer together.
    shell.last_command = Some(CommandReport {
        label: "view.fit".to_string(),
        ok: true,
        text: "{}".to_string(),
    });
    frame(&mut shell, &mut now);
    assert_eq!(
        shell.status_shown(StatusSlot::Command),
        Some("feature.extrude")
    );
    assert_eq!(
        shell.status_shown(StatusSlot::Answer),
        Some("the profile is not closed")
    );
    shell.last_command = Some(CommandReport {
        label: "feature.extrude".to_string(),
        ok: false,
        text: "the profile is not closed".to_string(),
    });
    for _ in 0..30 {
        frame(&mut shell, &mut now);
    }
    assert_eq!(
        shell.status_shown(StatusSlot::Answer),
        Some("the profile is not closed")
    );
    // `ui.get` says what is being done and whether the indicator shows.
    shell.set_work(Some(motion(0)));
    frame(&mut shell, &mut now);
    let got = shell
        .handle_method(&mut session, "ui.get", None)
        .unwrap()
        .unwrap();
    assert_eq!(got["work"]["label"], "view.motion");
    assert_eq!(got["workShown"], Value::Null);
}
