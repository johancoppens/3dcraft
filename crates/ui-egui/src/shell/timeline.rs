//! The history timeline of the feature panel, and the eye of each row.
//!
//! The timeline is a slider over the steps of the history: dragging it
//! shows the model as it was after that step (`view.scrub`), which is view
//! state only: the document, its rollback marker and its undo history stay
//! as they are. "Roll back here" moves the real marker there
//! (`history.rollback`), so new features go in after it; "To end" takes
//! the marker away and shows the document as it is. The rows after the
//! step shown are greyed in the list. Under the slider sits the player
//! (`shell/player.rs`), which plays the steps one after the other.
//!
//! The eye of a row hides the feature's body, sketch or datum in the view
//! (`feature.visible`), and shows it again.
//!
//! Like the selection, the shell only asks: the commands are queued and
//! run when the session is here ([`Shell::apply_timeline_requests`]). Only
//! the last scrub asked for is run, so a fast drag while the engine thread
//! builds a step shows the latest step once that is done.

use super::Shell;
use super::views::FlyTo;
use crate::theme::{Tokens, color};
use craft3d_engine::{MotionView, Scene, ScrubPosition, Session};
use egui::{Color32, Pos2, Sense, Shape, Stroke, vec2};
use serde_json::{Value, json};

/// The status of a row after the step the scrubbed view shows.
pub(crate) const AFTER_SCRUB: &str = "after the timeline";

/// Most commands waiting for the session.
const MAX_PENDING: usize = 64;

/// What the timeline knows of the active document, and what it asked for.
#[derive(Default)]
pub(crate) struct Timeline {
    /// Commands to run when the session is here, in order.
    pending: Vec<(&'static str, Value)>,
    /// Where the view is scrubbed to, as the session said last.
    pub(super) position: Option<ScrubPosition>,
    /// How many features the history has.
    pub(super) steps: usize,
    /// The place of the real rollback marker (1 for the first feature).
    pub(super) rollback: Option<usize>,
    /// Why the last command of the timeline failed.
    problem: Option<String>,
    /// A flight waits for what was asked to show its target: the scene's
    /// revision when it was asked ([`Shell::prepare_fly`]).
    revealing: Option<u64>,
    /// The moment of the motion the view shows (`view.motion`), as the
    /// session said last.
    pub(super) motion: Option<MotionView>,
    /// Whether the document has a timeline (a variable `t` that is a
    /// plain number), so the motion can be played.
    pub(super) has_timeline: bool,
    /// The paths of points over the motion (`view.trace`).
    pub(super) traces: Vec<craft3d_engine::TraceView>,
    /// Counts the changes of `traces`: the picture's key holds this, not
    /// the points.
    pub(super) traces_serial: u64,
}

impl Timeline {
    /// The step the view shows: where it is scrubbed to, else the real
    /// marker, else the last feature.
    pub(crate) fn shown_step(&self) -> usize {
        self.position
            .as_ref()
            .map(|p| p.step)
            .or(self.rollback)
            .unwrap_or(self.steps)
    }

    /// Whether a scrub waits to be run.
    pub(super) fn scrub_waiting(&self) -> bool {
        self.pending.iter().any(|(id, _)| *id == "view.scrub")
    }

    /// Whether a moment of the motion waits to be shown.
    pub(super) fn motion_waiting(&self) -> bool {
        self.pending.iter().any(|(id, _)| *id == "view.motion")
    }

    /// The `t` the view shows, or will once what is queued has run.
    pub(super) fn motion_t(&self) -> Option<f64> {
        let queued = self
            .pending
            .iter()
            .rev()
            .filter(|(id, _)| *id == "view.motion")
            .find_map(|(_, params)| params.get("t"));
        match queued {
            Some(t) => t.as_f64(),
            None => self.motion.as_ref().map(|m| m.t),
        }
    }

    pub(super) fn queue(&mut self, id: &'static str, mut params: Value) {
        // Only the last scrub counts: an earlier one gives its place.
        if id == "view.scrub" {
            self.pending.retain(|(queued, _)| *queued != "view.scrub");
        }
        // Likewise the last value of a joint dragged; it merges into the
        // last undo step only when the value it replaces did too (the
        // first value of a drag starts a step of its own).
        if id == "joint.set" {
            let mut merge = params.get("merge").and_then(Value::as_bool);
            self.pending.retain(|(queued, earlier)| {
                let same = *queued == "joint.set" && earlier.get("mate") == params.get("mate");
                if same {
                    let before = earlier
                        .get("merge")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    merge = merge.map(|m| m && before);
                }
                !same
            });
            if let (Some(merge), Some(map)) = (merge, params.as_object_mut()) {
                map.insert("merge".into(), merge.into());
            }
        }
        // One moment of the motion waits at most: a later one takes the
        // place of an earlier one, keeping what the earlier one set and
        // the later one does not (its collisions).
        if id == "view.motion"
            && let Some(at) = self
                .pending
                .iter()
                .position(|(queued, _)| *queued == "view.motion")
        {
            let (_, earlier) = self.pending.remove(at);
            if let (Value::Object(mut merged), Value::Object(later)) = (earlier, params.clone()) {
                merged.extend(later);
                params = Value::Object(merged);
            }
        }
        if self.pending.len() < MAX_PENDING {
            self.pending.push((id, params));
        } else {
            self.problem = Some(format!(
                "{id} not run: {MAX_PENDING} requests are waiting for the document"
            ));
        }
    }

    /// Whether `feature` is hidden once what is queued has run: the last
    /// `feature.visible` queued for it, else `shown` (the row as drawn).
    fn hidden_after_queue(&self, feature: &str, shown: bool) -> bool {
        self.pending
            .iter()
            .rev()
            .filter(|(id, _)| *id == "feature.visible")
            .find(|(_, params)| params["feature"].as_str() == Some(feature))
            .and_then(|(_, params)| params["visible"].as_bool())
            .map_or(shown, |visible| !visible)
    }
}

impl Shell {
    /// Another tab is shown: what was asked for the other document is
    /// not run on this one. Where this one's view is in its history is the
    /// session's (the scrub is per document), read by the next sync.
    pub(crate) fn timeline_switched(&mut self) {
        self.timeline.pending.clear();
        self.timeline.revealing = None;
        self.timeline.problem = None;
        self.player_switched();
    }

    /// Takes in where the active document's view is in its history.
    pub(crate) fn sync_timeline(&mut self, session: &mut Session) {
        self.timeline.position = session.scrub_position();
        let (steps, rollback) = session.history_marker().unwrap_or((0, None));
        self.timeline.steps = steps;
        self.timeline.rollback = rollback;
        self.timeline.motion = session.motion();
        self.timeline.has_timeline = session.timeline().is_ok();
        let traces = session.traces();
        if !same_traces(&traces, &self.timeline.traces) {
            self.timeline.traces = traces;
            self.timeline.traces_serial = self.timeline.traces_serial.wrapping_add(1);
        }
    }

    /// What of the traces the picture depends on: which traces there are
    /// (by [`Timeline::traces_serial`]) and, while one grows, the moment
    /// shown (its bits, so a `t` that is not a number still equals
    /// itself).
    pub(crate) fn traces_key(&self) -> (u64, Option<u64>) {
        let grows = self.timeline.traces.iter().any(|t| t.grow);
        let shown = self.timeline.motion.as_ref().map(|m| m.t.to_bits());
        (self.timeline.traces_serial, shown.filter(|_| grows))
    }

    /// The traces as the viewport draws them: a growing one
    /// (`view.trace {grow: true}`) only as far as the moment of the motion
    /// the view shows, whole while it shows none.
    pub(crate) fn drawn_traces(&self) -> Vec<Vec<craft3d_math::Vec3>> {
        let shown = self.timeline.motion.as_ref().map(|m| m.t);
        self.timeline
            .traces
            .iter()
            .map(|trace| match shown {
                Some(t) if trace.grow => trace.up_to(t),
                _ => trace.points.clone(),
            })
            .collect()
    }

    /// Marks the rows after the step the scrubbed view shows: greyed (the
    /// scene has them rolled back) and said to be after the timeline, not
    /// after the rollback marker.
    pub(crate) fn mark_scrubbed_rows(&mut self) {
        let Some(step) = self.timeline.position.as_ref().map(|p| p.step) else {
            return;
        };
        for row in self.rows.iter_mut().skip(step) {
            if row.error.is_none() {
                row.status = AFTER_SCRUB.to_string();
            }
        }
    }

    /// What a flight to `to` needs before it can frame its target in
    /// `scene` (decided 2026-10-23): a feature it aims at that is hidden is
    /// shown again (`feature.visible`), and when the view is scrubbed to a
    /// step before one of them, the scrub is let go of (`view.scrub` with
    /// null): the view shows the document as it is, where the feature is
    /// built. A flight to the selection looks at the features of every
    /// item (a face of a hidden body shows the body).
    fn reveal_requests(&self, scene: &Scene, to: &FlyTo) -> Vec<(&'static str, Value)> {
        let wanted: Vec<&str> = match to {
            FlyTo::Feature(feature) => vec![feature.as_str()],
            FlyTo::Selection => self
                .selection()
                .items
                .iter()
                .map(|item| item.feature.as_str())
                .collect(),
        };
        let step = self.timeline.position.as_ref().map(|p| p.step);
        let mut hidden = Vec::new();
        let mut after_scrub = false;
        for (index, row) in scene.features.iter().enumerate() {
            if !wanted.iter().any(|w| *w == row.id || *w == row.name) {
                continue;
            }
            if row.hidden && !hidden.contains(&row.id) {
                hidden.push(row.id.clone());
            }
            after_scrub |= step.is_some_and(|step| index >= step);
        }
        let mut requests = Vec::new();
        if !hidden.is_empty() {
            requests.push((
                "feature.visible",
                json!({"feature": hidden, "visible": true}),
            ));
        }
        if after_scrub {
            requests.push(("view.scrub", json!({"feature": null})));
        }
        requests
    }

    /// Before a flight in the window: asks for what [`Self::reveal_requests`]
    /// says, and true while the flight must wait for the scene that shows
    /// it. Asked once: when the next scene still does not show it (the
    /// command failed), the flight goes ahead and says what it cannot
    /// frame.
    pub(crate) fn prepare_fly(&mut self, scene: &Scene, to: &FlyTo) -> bool {
        if self.timeline.revealing == Some(scene.revision) {
            return true;
        }
        if self.timeline.revealing.take().is_some() {
            return false;
        }
        let requests = self.reveal_requests(scene, to);
        if requests.is_empty() {
            return false;
        }
        for (id, params) in requests {
            self.timeline.queue(id, params);
        }
        self.timeline.revealing = Some(scene.revision);
        true
    }

    /// [`Self::prepare_fly`] with the session at hand (`ui.fit`): runs the
    /// requests at once. True when it ran any, so the scene is to be
    /// taken again.
    pub(crate) fn reveal_now(
        &mut self,
        session: &mut Session,
        scene: &Scene,
        to: &FlyTo,
    ) -> Result<bool, String> {
        let requests = self.reveal_requests(scene, to);
        for (id, params) in &requests {
            session
                .execute(id, params)
                .map_err(|e| e.message().to_string())?;
        }
        Ok(!requests.is_empty())
    }

    /// The timeline as `ui.get` reports it: the steps, where the view is
    /// scrubbed to (`scrub`, null when it shows the document as it is),
    /// the real rollback marker's step, the player (`playing`, `speed`,
    /// `loop`, `orbit`, as `state.timeline` has them) and the caption over
    /// the viewport (null when there is none).
    pub fn timeline_json(&self) -> Value {
        json!({
            "steps": self.timeline.steps,
            "shown": self.timeline.shown_step(),
            "scrub": self.timeline.position.as_ref().map(|p| json!({
                "feature": p.feature, "step": p.step,
            })),
            "rollback": self.timeline.rollback,
            "problem": self.timeline.problem,
            "playing": self.state.timeline.playing,
            "speed": self.state.timeline.speed,
            "loop": self.state.timeline.looping,
            "orbit": self.state.timeline.orbit,
            "motion": self.state.timeline.motion,
            "bounce": self.state.timeline.bounce,
            "hasTimeline": self.timeline.has_timeline,
            "t": self.timeline.motion.as_ref().map(|m| m.t),
            "collisions": self.timeline.motion.as_ref().map(|m| {
                m.collisions.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>()
            }),
            "traces": self.timeline.traces.len(),
            "caption": self.player_caption(),
        })
    }

    /// The eye of the row of feature `id` was clicked; `hidden` is the
    /// row as drawn. Switches from what the clicks queued before it, so
    /// two clicks before the document answers (a double click) come back
    /// to where they started.
    pub(crate) fn toggle_visible(&mut self, id: String, hidden: bool) {
        let visible = self.timeline.hidden_after_queue(&id, hidden);
        self.timeline.queue(
            "feature.visible",
            json!({"feature": id, "visible": visible}),
        );
    }

    /// The step the view is scrubbed to, if it is.
    pub(crate) fn scrubbed_step(&self) -> Option<usize> {
        self.timeline.position.as_ref().map(|p| p.step)
    }

    /// Runs what the timeline and the eyes asked for. True when there was
    /// anything. The scene of a new step is built by whoever builds the
    /// window's scenes (the engine thread in the desktop app), not here.
    pub(crate) fn apply_timeline_requests(&mut self, session: &mut Session) -> bool {
        let pending = std::mem::take(&mut self.timeline.pending);
        for (id, params) in &pending {
            match session.execute(id, params) {
                Ok(_) => self.timeline.problem = None,
                Err(e) => {
                    self.timeline.problem = Some(e.message().to_string());
                    // A step that cannot be shown is never waited for.
                    if *id == "view.scrub" || *id == "view.motion" {
                        self.stop_playing();
                    }
                }
            }
        }
        if !pending.is_empty() {
            self.sync_timeline(session);
        }
        !pending.is_empty()
    }

    /// The timeline at the foot of the feature panel: a slider over the
    /// steps, the player's buttons, the step shown, and the button to roll
    /// back there.
    pub(crate) fn timeline_ui(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let steps = self.timeline.steps;
        if steps == 0 {
            // An assembly has no history of its own, but its motion
            // plays (docs/design/motion.md › *Assemblies*).
            if self.timeline.has_timeline {
                self.motion_ui(ui);
                if self.state.timeline.motion {
                    self.player_controls(ui);
                }
            }
            return;
        }
        let shown = self.timeline.shown_step().clamp(1, steps);
        let mut step = shown;
        let name = self
            .rows
            .get(step.saturating_sub(1))
            .map(|row| row.name.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(40.0);
            let slider = egui::Slider::new(&mut step, 1..=steps)
                .show_value(false)
                .trailing_fill(true);
            ui.add(slider).on_hover_text(
                "Drag through the history to see the model after each step. \
                 The document does not change.",
            );
        });
        if step != shown {
            self.go_to_step(step);
            ui.ctx().request_repaint();
        }
        let scrubbed = self.timeline.position.clone();
        self.motion_ui(ui);
        self.player_controls(ui);
        ui.horizontal(|ui| {
            let text = format!("Step {shown}/{steps}  {name}");
            let label = if scrubbed.is_some() || self.state.timeline.playing {
                egui::RichText::new(text).color(color(tokens.attention))
            } else {
                egui::RichText::new(text).color(color(tokens.text_dim))
            };
            ui.label(label);
        });
        ui.horizontal(|ui| {
            let here = ui
                // A Jig in use is looked at and played, not changed.
                .add_enabled(
                    scrubbed.is_some() && !self.jig_in_use(),
                    egui::Button::new("Roll back here"),
                )
                .on_hover_text(
                    "Move the rollback marker to this step: the model is built up to here, \
                     and new features go in after it.",
                );
            if here.clicked()
                && let Some(position) = &scrubbed
            {
                self.stop_playing();
                self.timeline.queue(
                    "history.rollback",
                    json!({"feature": position.feature.clone()}),
                );
                self.timeline.queue("view.scrub", json!({"feature": null}));
            }
        });
        if let Some(problem) = &self.timeline.problem {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(problem)
                        .color(color(tokens.failed))
                        .small(),
                )
                .wrap(),
            );
        }
    }
}

/// The eye of a row: open while the feature is shown, struck through while
/// it is hidden. Clicked to switch.
pub(crate) fn eye(ui: &mut egui::Ui, hidden: bool, tokens: &Tokens) -> egui::Response {
    let side = ui.spacing().interact_size.y.max(12.0);
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    let ink = if response.hovered() {
        color(tokens.text)
    } else if hidden {
        color(tokens.inactive)
    } else {
        color(tokens.text_dim)
    };
    paint_eye(ui.painter(), rect, ink, hidden, tokens);
    response.on_hover_text(if hidden {
        "Hidden in the view: click to show it"
    } else {
        "Click to hide it in the view (the model does not change)"
    })
}

/// An eye in `rect`: two arcs and a pupil, and a stroke across it when
/// `hidden`.
fn paint_eye(
    painter: &egui::Painter,
    rect: egui::Rect,
    ink: Color32,
    hidden: bool,
    tokens: &Tokens,
) {
    let half = rect.width().min(rect.height()) * 0.42;
    if !(half.is_finite() && half > 0.0) {
        return;
    }
    let c = rect.center();
    let p = |x: f32, y: f32| -> Pos2 { c + vec2(x, y) * half };
    let stroke = Stroke::new(tokens.nav_icon_width.max(1.0), ink);
    // The lids: arcs through the corners at (±1, 0), as polylines.
    let lid = |sign: f32| -> Vec<Pos2> {
        (0..=12)
            .map(|k| {
                let x = -1.0 + 2.0 * (k as f32) / 12.0;
                p(x, sign * 0.55 * (1.0 - x * x))
            })
            .collect()
    };
    painter.add(Shape::line(lid(-1.0), stroke));
    painter.add(Shape::line(lid(1.0), stroke));
    if hidden {
        painter.add(Shape::line_segment(
            [p(-0.85, 0.75), p(0.85, -0.75)],
            stroke,
        ));
    } else {
        painter.add(Shape::circle_filled(c, half * 0.28, ink));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::super::Shell;
    use craft3d_engine::{Session, Workspace};
    use serde_json::json;

    fn frame(ctx: &egui::Context, shell: &mut Shell, session: &mut Session) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    #[test]
    fn the_eye_hides_and_the_timeline_scrubs_without_editing_the_history() {
        let mut session = Session::new(Workspace::none());
        for (id, params) in [
            ("file.new", json!({})),
            (
                "feature.block",
                json!({"size": [10, 10, 10], "name": "Box"}),
            ),
            ("feature.sphere", json!({"radius": 3, "name": "Ball"})),
            (
                "feature.cylinder",
                json!({"radius": 1, "height": 30, "name": "Rod"}),
            ),
        ] {
            session.execute(id, &params).unwrap();
        }
        let mut shell = Shell::new();
        let ctx = egui::Context::default();
        frame(&ctx, &mut shell, &mut session);
        assert_eq!(shell.timeline_json()["steps"], 3);
        assert_eq!(shell.timeline_json()["shown"], 3);

        // The eye of the second row, clicked twice before the document
        // answered, then once more.
        shell.toggle_visible("F2".to_string(), false);
        shell.toggle_visible("F2".to_string(), false);
        frame(&ctx, &mut shell, &mut session);
        assert!(!shell.tree_rows()[1].hidden);
        shell.toggle_visible("F2".to_string(), false);
        frame(&ctx, &mut shell, &mut session);
        frame(&ctx, &mut shell, &mut session);
        assert!(shell.tree_rows()[1].hidden && !shell.tree_rows()[0].hidden);
        assert_eq!(shell.shown_scene_arc().unwrap().bodies.len(), 2);

        // The timeline at step 1 (only the last scrub asked for runs): the
        // later rows are after it, and the history did not change.
        let revision = session.document_revision();
        shell.timeline.queue("view.scrub", json!({"step": 2}));
        shell.timeline.queue("view.scrub", json!({"step": 1}));
        frame(&ctx, &mut shell, &mut session);
        frame(&ctx, &mut shell, &mut session);
        assert_eq!(session.document_revision(), revision);
        let rows = shell.tree_rows();
        assert_eq!(rows[1].status, super::AFTER_SCRUB);
        assert_eq!(rows[2].status, super::AFTER_SCRUB);
        assert_eq!(shell.timeline_json()["scrub"]["step"], 1);
        assert_eq!(shell.shown_scene_arc().unwrap().bodies.len(), 1);

        // Roll back here, then the marker is the step shown.
        shell
            .timeline
            .queue("history.rollback", json!({"feature": "F1"}));
        shell.timeline.queue("view.scrub", json!({"feature": null}));
        frame(&ctx, &mut shell, &mut session);
        frame(&ctx, &mut shell, &mut session);
        assert_eq!(shell.timeline_json()["rollback"], 1);
        assert_eq!(shell.timeline_json()["scrub"], json!(null));
        assert_eq!(shell.timeline_json()["shown"], 1);
        // A double click on a hidden row after the scrub: the feature is
        // shown and the timeline goes back to the end, then the flight.
        shell
            .timeline
            .queue("history.rollback", json!({"feature": null}));
        shell.timeline.queue("view.scrub", json!({"step": 1}));
        frame(&ctx, &mut shell, &mut session);
        frame(&ctx, &mut shell, &mut session);
        assert_eq!(shell.timeline_json()["scrub"]["step"], 1);
        shell.request_fly(super::FlyTo::Feature("Ball".to_string()));
        for _ in 0..4 {
            frame(&ctx, &mut shell, &mut session);
        }
        assert_eq!(shell.timeline_json()["scrub"], json!(null));
        assert!(!shell.tree_rows()[1].hidden);
        assert!(!shell.state.camera.auto_fit, "the flight was made");
        // A command that fails is said, not a crash.
        shell.timeline.queue("view.scrub", json!({"step": 99}));
        frame(&ctx, &mut shell, &mut session);
        assert!(shell.timeline_json()["problem"].is_string());
        // While scrubbed, a click in the viewport selects nothing.
        shell.timeline.queue("view.scrub", json!({"step": 1}));
        frame(&ctx, &mut shell, &mut session);
        frame(&ctx, &mut shell, &mut session);
        let serial = session.selection_serial();
        let scene = shell.shown_scene_arc();
        shell.click_select(
            scene.as_deref(),
            1.0,
            1.0,
            &craft3d_engine::ScenePointed::default(),
            (false, false, false),
        );
        frame(&ctx, &mut shell, &mut session);
        assert_eq!(session.selection_serial(), serial);
        assert!(
            shell
                .selection_status()
                .is_some_and(|(text, _)| text.contains("To end"))
        );
    }
}

/// Whether two lists of traces are the same, bit for bit (a point that is
/// not a number equals itself here, so it does not count as a change at
/// every sync).
fn same_traces(a: &[craft3d_engine::TraceView], b: &[craft3d_engine::TraceView]) -> bool {
    let bits = |p: &craft3d_math::Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.name == y.name
                && x.grow == y.grow
                && x.t.map(f64::to_bits) == y.t.map(f64::to_bits)
                && x.points.len() == y.points.len()
                && x.points
                    .iter()
                    .zip(&y.points)
                    .all(|(p, q)| bits(p) == bits(q))
        })
}

#[cfg(test)]
mod trace_tests {
    use super::*;
    use craft3d_engine::TraceView;
    use craft3d_math::Vec3;

    #[test]
    fn traces_with_a_point_that_is_not_a_number_equal_themselves() {
        let trace = TraceView {
            name: "Trace 1".to_string(),
            points: vec![Vec3::new(f64::NAN, 0.0, 0.0), Vec3::new(1.0, 2.0, 3.0)],
            t: [0.0, 1.0],
            grow: false,
        };
        assert!(same_traces(
            std::slice::from_ref(&trace),
            std::slice::from_ref(&trace)
        ));
        let mut moved = trace.clone();
        moved.points[1].x = 1.5;
        assert!(!same_traces(std::slice::from_ref(&trace), &[moved]));
        assert!(!same_traces(&[trace], &[]));
    }
}
