//! The bar of the graphics notice: what the shell shows when the desktop
//! app passed over a graphics interface that ended an earlier start
//! (`apps/craft3d/src/graphics_start.rs`), without a window
//! (`egui::Context::run_ui` with synthetic clicks): it takes room under the
//! menu, opens its details, and hands its Try again and Dismiss to the host
//! once, gone from then on.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{GraphicsButton, GraphicsNotice, GraphicsNoticeAction, Shell};
use egui::{Event, PointerButton, Pos2, vec2};

struct Window {
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    fn idle(&mut self, shell: &mut Shell, session: &mut Session) {
        for _ in 0..3 {
            self.frame(shell, session, Vec::new());
        }
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, button: GraphicsButton) {
        let at = shell
            .graphics_hits()
            .iter()
            .find(|(b, _)| *b == button)
            .unwrap_or_else(|| panic!("no {button:?}: {:?}", shell.graphics_hits()))
            .1
            .center();
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            self.frame(
                shell,
                session,
                vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        self.idle(shell, session);
    }
}

fn notice(retry: &str) -> GraphicsNotice {
    GraphicsNotice {
        text: "3DCraft draws through OpenGL on this computer: starting with Vulkan failed here \
               (Intel(R) UHD Graphics 615, driver Intel Corporation Intel driver)."
            .to_string(),
        details:
            "Drawing through OpenGL on Intel(R) UHD Graphics 615. Journal: graphics-start.json."
                .to_string(),
        retry: retry.to_string(),
    }
}

#[test]
fn no_notice_no_bar() {
    let mut s = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.idle(&mut shell, &mut s);
    assert!(shell.graphics_hits().is_empty());
    assert!(shell.viewport_rect().unwrap().top() < 40.0);
    assert_eq!(shell.take_graphics_notice_action(), None);
}

#[test]
fn the_bar_shows_its_details_and_hands_dismiss_to_the_host_once() {
    let mut s = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::new();
    shell.set_graphics_notice(Some(notice("Try Vulkan next time")));
    window.idle(&mut shell, &mut s);
    let top = shell.viewport_rect().unwrap().top();
    assert!(top > 50.0, "the bar takes room: {top}");
    let buttons: Vec<GraphicsButton> = shell.graphics_hits().iter().map(|(b, _)| *b).collect();
    for wanted in [
        GraphicsButton::Details,
        GraphicsButton::Retry,
        GraphicsButton::Dismiss,
    ] {
        assert!(buttons.contains(&wanted), "{wanted:?} in {buttons:?}");
    }
    for (_, rect) in shell.graphics_hits() {
        assert!(rect.bottom() <= top && rect.top() > 12.0, "{rect:?}");
    }
    // Details opens under the line, and closes again; neither is the
    // host's.
    assert!(!shell.graphics_details_open());
    window.click(&mut shell, &mut s, GraphicsButton::Details);
    assert!(shell.graphics_details_open());
    assert!(
        shell.viewport_rect().unwrap().top() > top,
        "the details take room"
    );
    assert_eq!(shell.take_graphics_notice_action(), None);
    window.click(&mut shell, &mut s, GraphicsButton::Details);
    assert!(!shell.graphics_details_open());
    // Dismiss: handed to the host once, and the bar is gone.
    window.click(&mut shell, &mut s, GraphicsButton::Dismiss);
    assert_eq!(
        shell.take_graphics_notice_action(),
        Some(GraphicsNoticeAction::Dismiss)
    );
    assert_eq!(shell.take_graphics_notice_action(), None);
    assert!(shell.graphics_notice().is_none());
    assert!(shell.graphics_hits().is_empty());
    assert!(shell.viewport_rect().unwrap().top() < 40.0);
}

#[test]
fn try_again_is_handed_to_the_host_and_a_notice_without_it_has_none() {
    let mut s = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let mut window = Window::new();
    shell.set_graphics_notice(Some(notice("Try Vulkan next time")));
    window.idle(&mut shell, &mut s);
    window.click(&mut shell, &mut s, GraphicsButton::Retry);
    assert_eq!(
        shell.take_graphics_notice_action(),
        Some(GraphicsNoticeAction::Retry)
    );
    assert!(shell.graphics_notice().is_none());
    // Every interface had failed: nothing to try again, so no such button.
    shell.set_graphics_notice(Some(notice("")));
    window.idle(&mut shell, &mut s);
    let buttons: Vec<GraphicsButton> = shell.graphics_hits().iter().map(|(b, _)| *b).collect();
    assert_eq!(
        buttons,
        vec![GraphicsButton::Dismiss, GraphicsButton::Details]
    );
}
