//! The window over a session that regenerates elsewhere (the page's Web
//! Worker, `Session::set_regen_elsewhere`): a frame reads what the state
//! shown holds (the selection, a section on a datum plane) without
//! regenerating that state on the page's thread, whatever the window held
//! for the tab it showed before. And what is selected is measured where
//! the bodies are (0.2.1: in 0.2.0 the browser's card of one thing showed
//! only what the names say and two things were not measured at all): the
//! card and the view show what the window shows, an answer asked for an
//! earlier selection is never shown, and *Wall thickness* and *Keep as a
//! check* are made there too.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{
    CommandAnswer, CommandRequest, SceneQuality, Session, Workspace, run_on_state, scene_begin,
    scene_next,
};
use craft3d_ui_egui::shell::sidepanel::props::{CARD_JOB, card_job};
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, PointerButton, Pos2, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// A host that regenerates elsewhere.
fn elsewhere() -> Session {
    let mut session = Session::new(Workspace::none());
    session.set_stepwise(true);
    session.set_checks_elsewhere(true);
    session.set_regen_elsewhere(true);
    assert!(session.regen_elsewhere());
    session
}

/// Answers the scenes the window waits for, as the worker does.
fn answer(session: &mut Session, quality: SceneQuality) {
    for _ in 0..10 {
        let Some(token) = session.scene_job(quality) else {
            assert!(session.scene_ready(quality));
            return;
        };
        let request = session.scene_request(token).unwrap();
        scene_begin(&request.document, &request.work);
        while let Some(part) = scene_next() {
            assert!(session.scene_answered(token, Ok(&part)));
        }
        assert!(session.scene_done(token));
    }
    panic!("jobs without end");
}

/// A frame of the window, the scene of the state shown there.
fn frame(ctx: &egui::Context, shell: &mut Shell, session: &mut Session, time: f64) {
    answer(session, shell.scene_quality());
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
        time: Some(time),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
    output.textures_delta.clear();
}

/// The review's case (2026-10-28): a face selected in one document, a
/// second opened beside it; the frame that shows the second read its
/// selection, which regenerated the whole document on the page's thread
/// (a heavy Jig: an unstoppable wait). And a section on a datum plane of
/// the second, which regenerated it to find the plane.
#[test]
fn a_frame_never_regenerates_the_state_it_shows() {
    let ctx = egui::Context::default();
    let mut shell = Shell::new();
    let mut session = elsewhere();
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [10, 20, 5], "name": "A"}),
    );
    frame(&ctx, &mut shell, &mut session, 0.0);
    // A click selects: a command, which regenerates here.
    run(&mut session, "select.set", json!({"names": ["A"]}));
    assert!(session.regenerated_here());
    frame(&ctx, &mut shell, &mut session, 0.1);
    assert_eq!(shell.selection().items.len(), 1);

    // A second document, opened beside it.
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [30, 30, 30], "name": "C"}),
    );
    run(
        &mut session,
        "variable.set",
        json!({"name": "d", "expr": "11 mm"}),
    );
    run(
        &mut session,
        "variable.publish",
        json!({"name": "d", "min": "1 mm", "max": "100 mm"}),
    );
    run(
        &mut session,
        "datum.plane",
        json!({"from": "xy", "distance": "d", "name": "Top"}),
    );
    // Its last edit left to the host, as the Parameters form leaves it
    // (a command that adds a feature may regenerate to answer).
    run(&mut session, "param.set", json!({"values": {"d": "12 mm"}}));
    assert!(!session.regenerated_here());
    for k in 0..3 {
        frame(&ctx, &mut shell, &mut session, 1.0 + f64::from(k));
        assert!(
            !session.regenerated_here(),
            "frame {k} regenerated the document it shows"
        );
    }
    assert!(shell.selection().items.is_empty());

    // A section on its datum plane.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": true, "plane": "Top", "offset": 0}}),
    );
    frame(&ctx, &mut shell, &mut session, 5.0);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["point"], json!([0.0, 0.0, 12.0]), "{got}");
    assert_eq!(got["section"]["normal"], json!([0.0, 0.0, 1.0]));
    // An edit, as the Parameters form makes it: a new state, the
    // section found on it again.
    let done = run(&mut session, "param.set", json!({"values": {"d": "14 mm"}}));
    assert_eq!(done["deferred"], true, "{done}");
    frame(&ctx, &mut shell, &mut session, 6.0);
    frame(&ctx, &mut shell, &mut session, 7.0);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["point"], json!([0.0, 0.0, 14.0]), "{got}");
    assert!(
        !session.regenerated_here(),
        "the section regenerated the document it cuts"
    );
}

/// The block of 60 × 40 × 20 with two holes of radius 3 and a chamfered
/// edge of `measure_view.rs`, as the bytes of its file.
fn part_file() -> Vec<u8> {
    let mut s = Session::new(Workspace::memory());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 40, 20], "name": "Block"}),
    );
    for (x, name) in [(15, "Left"), (45, "Right")] {
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": 3, "height": 30, "position": [x, 20, -5], "name": name}),
        );
    }
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Left", "name": "Once"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Once", "tool": "Right", "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:face(1)", "F1:face(5)"]], "distance": 8, "name": "Bevel"}),
    );
    run(&mut s, "file.save", json!({"path": "part.3dcraft"}));
    s.take_file("part.3dcraft").unwrap()
}

/// `bytes` opened in `session`.
fn open(session: &mut Session, bytes: &[u8]) {
    session.put_file("part.3dcraft", bytes.to_vec()).unwrap();
    run(session, "file.open", json!({"path": "part.3dcraft"}));
}

/// What the page's worker runs for a command it is posted (`jobs.rs` of
/// `craft3d-web`): the side panel's card's own job, else the command.
fn worker_runs(request: &CommandRequest) -> CommandAnswer {
    run_on_state(
        &request.document,
        &request.work,
        |session, command, params| {
            if command == CARD_JOB {
                card_job(session, params)
            } else {
                session.execute(command, params)
            }
        },
    )
}

/// The commands waiting for the worker, written out as the page posts
/// them, not answered yet.
fn posted(session: &mut Session) -> Vec<CommandRequest> {
    session
        .command_jobs()
        .into_iter()
        .map(|token| session.command_request(token).unwrap())
        .collect()
}

/// Whether `request` is a job of the card's (not a command).
fn the_cards(request: &CommandRequest) -> bool {
    serde_json::from_str::<Value>(&request.work).unwrap()["command"] == CARD_JOB
}

/// The window, or the page beside its worker: a `Shell` drawn frame by
/// frame, at the same size for both.
struct Drawn {
    ctx: egui::Context,
    time: f64,
    shell: Shell,
    session: Session,
}

impl Drawn {
    /// The window: the bodies here.
    fn window(bytes: &[u8]) -> Self {
        let mut session = Session::new(Workspace::memory());
        open(&mut session, bytes);
        Self::with(session)
    }

    /// The page: the regeneration, and what needs the bodies, elsewhere.
    fn page(bytes: &[u8]) -> Self {
        let mut session = Session::new(Workspace::memory());
        session.set_stepwise(true);
        session.set_checks_elsewhere(true);
        session.set_regen_elsewhere(true);
        session.set_commands_elsewhere(true);
        open(&mut session, bytes);
        assert!(!session.regenerated_here());
        Self::with(session)
    }

    fn with(session: Session) -> Self {
        let mut drawn = Self {
            ctx: egui::Context::default(),
            time: 0.0,
            shell: Shell::with_renderer(RendererChoice::Cpu),
            session,
        };
        drawn.settle();
        drawn
    }

    /// One frame; on the page the scenes it waits for answered first, as
    /// the worker does.
    fn draw(&mut self, events: Vec<Event>) {
        if self.session.regen_elsewhere() {
            answer(&mut self.session, self.shell.scene_quality());
        }
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        let mut output = ctx.run_ui(input, |ui| self.shell.ui(ui, &mut self.session));
        output.textures_delta.clear();
    }

    /// `requests` answered by the worker: the answers to the session, the
    /// card's to the shell, as `WebHost::answered` hands them.
    fn reply(&mut self, requests: Vec<CommandRequest>) {
        let mut answered = Vec::new();
        for request in requests {
            let answer = worker_runs(&request);
            answered.extend(self.session.command_answered(request.token, Ok(answer)));
        }
        let others = self.shell.card_answered(answered);
        // The card takes its own answers only.
        assert!(others.iter().all(|a| a.command != CARD_JOB), "{others:?}");
    }

    /// Frames until nothing waits: the worker answering whatever was
    /// posted after each, the window's thread measuring.
    fn settle(&mut self) {
        for _ in 0..5000 {
            self.draw(Vec::new());
            let requests = posted(&mut self.session);
            let busy = !requests.is_empty() || self.shell.measuring();
            self.reply(requests);
            if !busy && !self.session.commands_waiting() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        self.draw(Vec::new());
        self.draw(Vec::new());
    }

    /// Selects `items` as a click does (on the page handed to the worker,
    /// which answers it), and draws one frame.
    fn select(&mut self, items: &Value) {
        let done = self
            .session
            .execute_deferrable("select.set", &json!({ "items": items }))
            .unwrap();
        if self.session.regen_elsewhere() {
            assert_eq!(done["deferred"], true, "{done}");
            let clicks = posted(&mut self.session);
            assert!(clicks.iter().all(|r| !the_cards(r)));
            self.reply(clicks);
        }
        self.draw(Vec::new());
    }

    /// A click on the side panel's target `name`.
    fn click(&mut self, name: &str) {
        let at = self
            .shell
            .side_hits()
            .targets
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no {name}: {:?}", self.shell.side_hits().targets))
            .1
            .center();
        self.draw(vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            self.draw(vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
    }

    /// What the card and the view show: the card as it was read, the
    /// wall, the dimension's number, and `ui.get` › `measurement`.
    fn shown(&mut self) -> (String, Option<String>, Value) {
        let card = format!("{:?} {:?}", self.shell.side_props(), self.shell.side_wall());
        let dimension = self.shell.dimension_place().map(|p| p.text.clone());
        let got = call(&mut self.shell, &mut self.session, "ui.get", json!({}));
        (card, dimension, got["measurement"].clone())
    }
}

/// The items `filter` finds of `kind` in the window, as `select.set`
/// takes them: `count` of them, when given; at least one.
fn items(window: &mut Drawn, kind: &str, filter: Value, count: Option<u64>) -> Vec<Value> {
    let found = run(
        &mut window.session,
        "select.query",
        json!({"kind": kind, "where": filter}),
    );
    if let Some(count) = count {
        assert_eq!(found["count"], count, "{found}");
    }
    assert!(found["count"].as_u64().unwrap() > 0, "{found}");
    found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| json!({"kind": i["kind"], "name": i["name"], "feature": i["feature"]}))
        .collect()
}

/// The first `n` of `items`, as one selection.
fn first(items: &[Value], n: usize) -> Value {
    Value::Array(items.iter().take(n).cloned().collect())
}

/// The developer's case (0.2.0 in the browser, 2026-11-19): one edge
/// selected showed only Between and Name, nothing in the view, and two
/// things no relation. Beside the worker the card and the view show what
/// the window shows, item for item: one edge its length, one hole its
/// diameter, one body its volume, one vertex its position, two holes
/// their centre distance; meanwhile the card says it is measuring; and
/// the page regenerates nothing.
#[test]
fn what_is_selected_is_measured_where_the_bodies_are_as_the_window_measures_it() {
    let bytes = part_file();
    let mut window = Drawn::window(&bytes);
    let mut page = Drawn::page(&bytes);
    let work = page.session.work_here();
    let edges = items(
        &mut window,
        "edge",
        json!({"in": "Bevel", "length": 60}),
        None,
    );
    let hole = items(
        &mut window,
        "face",
        json!({"in": "Bevel", "hole": true, "inside": [[0, 0, -10], [30, 40, 30]]}),
        Some(1),
    );
    // The block's corner at the origin.
    let vertices = items(
        &mut window,
        "vertex",
        json!({"in": "Bevel", "inside": [[-1, -1, -1], [1, 1, 1]]}),
        Some(1),
    );
    let holes = items(
        &mut window,
        "face",
        json!({"in": "Bevel", "hole": true}),
        Some(2),
    );
    let cases = [
        (first(&edges, 1), Some("60 mm")),
        (first(&hole, 1), Some("Ø 6 mm")),
        (
            json!([{"kind": "body", "name": "Bevel", "feature": "F6"}]),
            None,
        ),
        (first(&vertices, 1), None),
        (first(&holes, 2), Some("30 mm")),
    ];
    for (case, dimension) in cases {
        window.select(&case);
        window.settle();
        let want = window.shown();
        assert_eq!(want.1.as_deref(), dimension, "{case}: {want:?}");
        assert!(want.2["text"].is_string(), "{case}: {want:?}");
        page.select(&case);
        // Asked where the bodies are: being measured until it comes.
        let (_, drawn, measuring) = page.shown();
        if case.as_array().unwrap().len() == 1 {
            assert_eq!(measuring["measure"], "measuring", "{case}: {measuring}");
            assert_eq!(measuring["words"], "Measuring…");
        } else {
            assert_eq!(measuring["measuring"], true, "{case}: {measuring}");
        }
        assert_eq!(drawn, None, "{case}");
        page.settle();
        assert_eq!(page.shown(), want, "{case}");
    }
    assert!(!page.session.regenerated_here());
    assert_eq!(page.session.work_here(), work);
}

/// An answer asked for one selection is never shown for the next: the
/// facts of an edge come after a hole was selected, and the relation of
/// two holes after two faces were. The card shows the hole being
/// measured, then its diameter; the faces' relation being measured, then
/// their distance, as the window shows them.
#[test]
fn an_answer_asked_for_an_earlier_selection_is_never_shown() {
    let bytes = part_file();
    let mut window = Drawn::window(&bytes);
    let mut page = Drawn::page(&bytes);
    let edge = first(
        &items(
            &mut window,
            "edge",
            json!({"in": "Bevel", "length": 60}),
            None,
        ),
        1,
    );
    let hole = first(
        &items(
            &mut window,
            "face",
            json!({"in": "Bevel", "hole": true, "inside": [[0, 0, -10], [30, 40, 30]]}),
            Some(1),
        ),
        1,
    );
    let holes = first(
        &items(
            &mut window,
            "face",
            json!({"in": "Bevel", "hole": true}),
            Some(2),
        ),
        2,
    );
    let faces = first(
        &items(
            &mut window,
            "face",
            json!({"in": "Bevel", "any": [{"normal": {"along": "z"}}, {"normal": {"along": "-z"}}]}),
            Some(2),
        ),
        2,
    );
    // The edge's facts asked, held by the worker; the hole selected.
    page.select(&edge);
    let held = posted(&mut page.session);
    assert_eq!(held.len(), 1);
    assert!(the_cards(&held[0]));
    page.select(&hole);
    // The edge's answer comes now: dropped.
    page.reply(held);
    page.draw(Vec::new());
    let props = page.shell.side_props().cloned().unwrap();
    assert_eq!(props.kind, "Face", "{props:?}");
    assert_eq!(
        props.head.as_ref().unwrap().measure,
        "measuring",
        "{props:?}"
    );
    assert!(props.rows.iter().all(|r| r.label != "Length"), "{props:?}");
    assert!(page.shell.single_measure().is_none());
    page.settle();
    window.select(&hole);
    window.settle();
    assert_eq!(page.shown(), window.shown());
    assert_eq!(
        page.shell.single_measure().map(|s| s.text.as_str()),
        Some("Ø 6 mm")
    );
    // Two holes, their facts and relation held; then two faces, whose
    // relation waits for the holes' to come back, which is dropped.
    page.select(&holes);
    let held = posted(&mut page.session);
    assert_eq!(held.len(), 2, "the facts and the relation");
    page.select(&faces);
    page.reply(held);
    page.draw(Vec::new());
    let m = page.shell.measurement().cloned().unwrap();
    assert!(m.measuring, "{m:?}");
    assert_eq!(Value::Array(m.items.to_vec()), faces);
    page.settle();
    window.select(&faces);
    window.settle();
    assert_eq!(page.shown(), window.shown());
    assert_eq!(page.shell.measurement().unwrap().text, "20 mm");
    assert!(!page.session.regenerated_here());
}

/// *Wall thickness* and *Keep as a check* beside the worker: the wall is
/// measured there and heads the card as the window's does; the check is
/// made there as an edit (it measures every check of the document), the
/// button saying so until it is back, and the document has it, one step
/// to undo, as the window's has.
#[test]
fn wall_thickness_and_keep_as_a_check_are_made_where_the_bodies_are() {
    let bytes = part_file();
    let mut window = Drawn::window(&bytes);
    let mut page = Drawn::page(&bytes);
    let top = first(
        &items(
            &mut window,
            "face",
            json!({"in": "Bevel", "normal": {"along": "z"}}),
            None,
        ),
        1,
    );
    for drawn in [&mut window, &mut page] {
        drawn.select(&top);
        drawn.settle();
        drawn.click("measure");
    }
    assert!(page.shell.side_wall().unwrap().measuring);
    window.settle();
    page.settle();
    let wall = window.shell.side_wall().cloned().unwrap();
    assert!(wall.value.is_some(), "{wall:?}");
    assert_eq!(page.shell.side_wall(), Some(&wall));
    assert_eq!(page.shown(), window.shown());

    let holes = first(
        &items(
            &mut window,
            "face",
            json!({"in": "Bevel", "hole": true}),
            Some(2),
        ),
        2,
    );
    for drawn in [&mut window, &mut page] {
        drawn.select(&holes);
        drawn.settle();
        drawn.click("keep");
    }
    // Made where the bodies are: an edit that waits; Keep cannot be
    // clicked twice meanwhile.
    assert!(page.session.edits_waiting());
    page.click("keep");
    window.settle();
    page.settle();
    assert!(!page.session.edits_waiting());
    let names = |drawn: &mut Drawn| -> Vec<String> {
        drawn
            .session
            .assertion_states()
            .into_iter()
            .map(|a| a.name)
            .collect()
    };
    let kept = names(&mut window);
    assert_eq!(kept, ["Centre distance Left face – Right face"]);
    assert_eq!(names(&mut page), kept);
    let undo = |drawn: &Drawn| drawn.session.document_status().unwrap().undo;
    assert_eq!(undo(&page), undo(&window));
    assert!(!page.session.regenerated_here());
}
