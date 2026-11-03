//! The page with its regenerating worker, natively: the page's own
//! [`WebHost`] and [`Scenes`] over a [`Transport`] whose worker answers
//! with what `worker.js` runs (`scene_begin`/`scene_next` for scenes,
//! [`crate::jobs::run`] for commands and the page's own jobs), a frame at
//! a time as `page.rs` drives them. What it checks: every export from the
//! menu, the dialog's choices included, and every edit that needs the
//! bodies are made in the worker, with the bytes and the document a page
//! that regenerates itself makes, while the page's session regenerates
//! and meshes nothing (`Session::work_here`); a worker that dies or hangs
//! mid-job is reported and replaced, never waited for.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use crate::checks::{Reply, Transport};
use crate::host::{Download, Outcome, Pick, WebHost};
use crate::scenes::{COMMAND_TOO_LONG, DEADLINE_MS, Scenes};
use craft3d_engine::{
    SceneQuality, Session, Workspace, measure_checks, scene_begin, scene_bytes, scene_next,
};
use craft3d_ui_egui::FileAction;
use craft3d_ui_egui::files::{ExportFormat, ExportWhat};
use craft3d_ui_egui::shell::Shell;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

enum Job {
    Scene(u64, Vec<u8>, String),
    Command(u64, Vec<u8>, String),
}

/// The worker: keeps what it is posted and answers it when the frame
/// asks, or dies (`Reply::Failed`), or never answers.
#[derive(Clone, Default)]
struct Beside {
    jobs: Rc<RefCell<Vec<Job>>>,
    stops: Rc<RefCell<usize>>,
    /// The next answer is its death.
    dies: Rc<RefCell<bool>>,
    /// It answers nothing (it hangs).
    hangs: Rc<RefCell<bool>>,
}

impl Transport for Beside {
    fn post(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
        self.jobs
            .borrow_mut()
            .push(Job::Scene(token, document.to_vec(), work.into()));
        Ok(())
    }
    fn post_command(&mut self, token: u64, document: &[u8], work: &str) -> Result<(), String> {
        self.jobs
            .borrow_mut()
            .push(Job::Command(token, document.to_vec(), work.into()));
        Ok(())
    }
    fn stop(&mut self) {
        *self.stops.borrow_mut() += 1;
        self.jobs.borrow_mut().clear();
    }
    fn cancel(&mut self) {}
}

impl Beside {
    fn answers(&self) -> Vec<Reply> {
        if *self.hangs.borrow() {
            return Vec::new();
        }
        if *self.dies.borrow() && !self.jobs.borrow().is_empty() {
            *self.dies.borrow_mut() = false;
            self.jobs.borrow_mut().clear();
            return vec![Reply::Failed("the worker stopped (out of memory)".into())];
        }
        let mut replies = Vec::new();
        for job in self.jobs.borrow_mut().drain(..) {
            match job {
                Job::Scene(token, document, work) => {
                    scene_begin(&document, &work);
                    while let Some(bytes) = scene_next() {
                        replies.push(Reply::Part { token, bytes });
                    }
                    replies.push(Reply::Done { token });
                }
                Job::Command(token, document, work) => {
                    replies.push(Reply::Command {
                        token,
                        answer: Ok(crate::jobs::run(&document, &work)),
                    });
                }
            }
        }
        replies
    }
}

/// The page: its host, and its regenerating worker; the checks measured
/// by their own worker's code, answered in the same frame.
struct Page {
    host: WebHost,
    scenes: Scenes<Beside>,
    worker: Beside,
    clock: f64,
}

impl Page {
    fn new(pickers: bool) -> Self {
        let mut host = WebHost::new(Shell::new());
        host.pickers = pickers;
        host.set_stepwise(true);
        host.session.set_checks_elsewhere(true);
        let worker = Beside::default();
        let scenes = Scenes::new(worker.clone(), &mut host.session);
        Self {
            host,
            scenes,
            worker,
            clock: 0.0,
        }
    }

    /// One frame, as `page.rs` runs it: the File menu's work, then what
    /// the worker answered, then the checks.
    fn frame(&mut self) -> Outcome {
        let mut outcome = self.host.pump();
        self.clock += 16.0;
        let quality = self.host.shell.scene_quality();
        let replies = self.worker.answers();
        let done = self
            .scenes
            .frame_at(&mut self.host.session, quality, replies, self.clock);
        let mut commands = done.commands;
        if self.host.take_worker_restart() {
            commands.extend(self.scenes.restart(&mut self.host.session));
        }
        let answered = self.host.answered(commands);
        outcome.downloads.extend(answered.downloads);
        outcome.pick = outcome.pick.or(answered.pick);
        self.host.prepare(|| true);
        while let Some(token) = self.host.session.checks_job() {
            let request = self.host.session.checks_request(token).unwrap();
            let answer = measure_checks(&request.document, &request.work);
            self.host.session.checks_answered(token, Ok(&answer));
        }
        outcome
    }

    /// Frames until nothing waits for the worker.
    fn settle(&mut self) -> Outcome {
        let mut outcome = Outcome::default();
        for _ in 0..50 {
            let more = self.frame();
            outcome.downloads.extend(more.downloads);
            outcome.pick = outcome.pick.or(more.pick);
            let quality = self.host.shell.scene_quality();
            if self.scenes.busy().is_none()
                && !self.host.session.commands_waiting()
                && self.host.session.scene_job(quality).is_none()
                && !self.host.export_choices_waiting()
            {
                return outcome;
            }
        }
        panic!("the page never settled");
    }

    /// File › Export answered as `what` in `format`: what the page is
    /// left to offer once the worker answered.
    fn export(&mut self, what: &ExportWhat, format: ExportFormat) -> Outcome {
        assert_eq!(
            self.host.file_action(FileAction::Export),
            Outcome::default()
        );
        // What the dialog offers is read in the worker: open once it
        // answered.
        assert!(!self.host.shell.export_dialog_open());
        self.settle();
        assert!(self.host.shell.export_dialog_open());
        self.host.shell.set_export_what(what.clone()).unwrap();
        self.host.shell.set_export_format(format).unwrap();
        self.host.shell.answer_export(true);
        let request = self.host.shell.take_export_answer().unwrap();
        let now = self.host.export_request(request);
        assert_eq!(now, Outcome::default(), "made in the worker, not now");
        self.settle()
    }
}

/// A plate with a colour, a sheet-metal part and a named set of flat
/// faces: every kind of export has something to write.
fn model() -> Vec<u8> {
    let mut s = Session::new(Workspace::memory());
    for (id, params) in [
        ("file.new", json!({"units": "mm"})),
        (
            "feature.block",
            json!({"size": [40, 20, 10], "name": "Plate"}),
        ),
        (
            "body.appearance",
            json!({"body": "Plate", "color": "#2060c0"}),
        ),
        ("sketch.create", json!({"plane": "xy", "name": "S"})),
        (
            "sketch.rect",
            json!({"sketch": "S", "from": [60, 0], "to": [110, 40]}),
        ),
        (
            "feature.sheetMetal",
            json!({"sketch": "S", "name": "Part", "thickness": "1 mm", "radius": "1 mm",
                "kFactor": 0.4, "flanges": [{"edge": "e6", "length": "15 mm", "angle": "90 deg"}]}),
        ),
        (
            "set.define",
            json!({"name": "Tops", "kind": "face", "where": {"normal": {"along": "z"}, "in": "Plate"}}),
        ),
    ] {
        s.execute(id, &params)
            .unwrap_or_else(|e| panic!("{id}: {}", e.message()));
    }
    s.execute("file.save", &json!({"path": "model.3dcraft"}))
        .unwrap();
    s.take_file("model.3dcraft").unwrap()
}

/// A host that regenerates itself (no worker): the bytes to compare.
///
/// It does what the page does without its workers (`?pageregen`): the
/// long work a piece at a time, all of it done before the click. The
/// drawing of the model as seen is drawn from the window's camera, which
/// follows the model once a scene is there (`Shell::sync`), so each case
/// starts from a fresh session on both sides: the same camera.
fn in_process(bytes: &[u8]) -> WebHost {
    let mut host = WebHost::new(Shell::new());
    host.set_stepwise(true);
    host.receive("model.3dcraft", bytes.to_vec(), None).unwrap();
    while host.prepare(|| false) {}
    host
}

fn opened(bytes: &[u8], pickers: bool) -> Page {
    let mut page = Page::new(pickers);
    page.host
        .receive("model.3dcraft", bytes.to_vec(), None)
        .unwrap();
    page.settle();
    page
}

fn files(downloads: &[Download]) -> Vec<(String, Vec<u8>)> {
    downloads
        .iter()
        .map(|d| (d.name.clone(), d.bytes.clone()))
        .collect()
}

/// The same export on a page that regenerates itself, through the same
/// dialog.
fn wanted(host: &mut WebHost, what: &ExportWhat, format: ExportFormat) -> Vec<Download> {
    assert_eq!(host.file_action(FileAction::Export), Outcome::default());
    host.shell.set_export_what(what.clone()).unwrap();
    host.shell.set_export_format(format).unwrap();
    host.shell.answer_export(true);
    let request = host.shell.take_export_answer().unwrap();
    host.export_request(request).downloads
}

/// Every export from the menu, in every format and of every kind of
/// *What* (the model, the selection, a set's sheet, a flat pattern), is
/// made in the worker: the dialog's choices read there, the file written
/// there, the same bytes as a page that regenerates itself, the OBJ with
/// its library; and the page's session never regenerates or meshes.
#[test]
fn every_export_from_the_menu_is_made_where_the_bodies_are() {
    let bytes = model();
    let cases = [
        (ExportWhat::Model, ExportFormat::Step),
        (ExportWhat::Model, ExportFormat::Stl),
        (ExportWhat::Model, ExportFormat::ThreeMf),
        (ExportWhat::Model, ExportFormat::Obj),
        (ExportWhat::Model, ExportFormat::Dxf),
        (ExportWhat::Model, ExportFormat::Svg),
        (ExportWhat::Set("Tops".into()), ExportFormat::Svg),
        (ExportWhat::SheetMetal("Part".into()), ExportFormat::Dxf),
    ];
    for (what, format) in &cases {
        let want = wanted(&mut in_process(&bytes), what, *format);
        assert!(!want.is_empty(), "{what:?} {format:?}");
        let mut page = opened(&bytes, false);
        let work = page.host.session.work_here();
        let got = page.export(what, *format);
        assert_eq!(files(&got.downloads), files(&want), "{what:?} {format:?}");
        assert!(page.host.shell.error_dialog().is_none(), "{what:?}");
        assert!(page.host.session.file_names().is_empty());
        assert_eq!(page.host.session.work_here(), work, "{what:?} {format:?}");
    }
    // The selection, chosen by a click (answered by the worker): one body
    // to a mesh holds that body only; a flat face to DXF its outline.
    let face = {
        let mut s = in_process(&bytes);
        let top = s
            .session
            .execute(
                "select.query",
                &json!({"kind": "face", "where": {"in": "Plate", "normal": {"along": "z"}}}),
            )
            .unwrap();
        top["items"][0]["name"].as_str().unwrap().to_string()
    };
    for (click, format) in [
        (json!({"names": ["Plate"]}), ExportFormat::Stl),
        (
            json!({"items": [{"kind": "face", "name": face, "feature": "F1"}]}),
            ExportFormat::Dxf,
        ),
    ] {
        let mut want = in_process(&bytes);
        want.session.execute("select.set", &click).unwrap();
        let want = wanted(&mut want, &ExportWhat::Selection, format);
        let mut page = opened(&bytes, false);
        let work = page.host.session.work_here();
        page.host
            .session
            .execute_deferrable("select.set", &click)
            .unwrap();
        page.settle();
        let got = page.export(&ExportWhat::Selection, format);
        assert_eq!(files(&got.downloads), files(&want), "{click}");
        assert_eq!(page.host.session.work_here(), work, "{click}");
    }
}

/// With the system's file dialogs: the export is made in the worker
/// before its dialog, and the dialog is asked for with the file made
/// (an OBJ's library waits for its own click); a failing export shows
/// the error as on the desktop and no dialog at all.
#[test]
fn with_file_dialogs_the_file_is_made_in_the_worker_before_the_dialog() {
    let bytes = model();
    let mut page = opened(&bytes, true);
    let mut want = in_process(&bytes);
    want.pickers = true;
    let got = page.export(&ExportWhat::Model, ExportFormat::Obj);
    assert!(got.downloads.is_empty());
    let Some(Pick::Export { file }) = got.pick else {
        panic!("{:?}", got.pick)
    };
    let wanted = {
        assert_eq!(want.file_action(FileAction::Export), Outcome::default());
        want.shell.set_export_what(ExportWhat::Model).unwrap();
        want.shell.set_export_format(ExportFormat::Obj).unwrap();
        want.shell.answer_export(true);
        let request = want.shell.take_export_answer().unwrap();
        want.export_request(request)
    };
    let Some(Pick::Export { file: want_file }) = wanted.pick else {
        panic!()
    };
    assert_eq!(
        (&file.name, &file.bytes),
        (&want_file.name, &want_file.bytes)
    );
    let library = page.host.companion.clone().unwrap();
    assert_eq!(library.bytes, want.companion.clone().unwrap().bytes);
    // A selection a mesh cannot hold (an edge): the dialog does not offer
    // it any more, so the edge is selected after the dialog was answered
    // as the selection; the worker refuses it as the page would.
    let work = page.host.session.work_here();
    page.host.companion = None;
    page.host
        .session
        .execute_deferrable(
            "select.query",
            &json!({"kind": "face", "limit": 1, "then": "replace"}),
        )
        .unwrap();
    page.settle();
    assert_eq!(
        page.host.file_action(FileAction::Export),
        Outcome::default()
    );
    page.settle();
    page.host
        .shell
        .set_export_what(ExportWhat::Selection)
        .unwrap();
    page.host
        .shell
        .set_export_format(ExportFormat::Dxf)
        .unwrap();
    page.host.shell.answer_export(true);
    let request = page.host.shell.take_export_answer().unwrap();
    page.host
        .session
        .execute_deferrable(
            "select.query",
            &json!({"kind": "edge", "limit": 1, "then": "replace"}),
        )
        .unwrap();
    page.settle();
    page.host.export_request(request);
    let failed = page.settle();
    assert!(failed.pick.is_none() && failed.downloads.is_empty());
    assert_eq!(
        page.host.shell.error_dialog().map(|e| e.title.as_str()),
        Some("Could not export")
    );
    assert_eq!(page.host.session.work_here(), work);
}

/// A worker that dies while it makes an export: the export is an error
/// the person sees (never waited for), the worker is replaced, and the
/// next export is made by the new one. One that hangs is given up at the
/// deadline the same way.
#[test]
fn a_worker_that_dies_or_hangs_mid_export_is_reported_and_replaced() {
    let bytes = model();
    let mut page = opened(&bytes, false);
    let work = page.host.session.work_here();
    assert_eq!(
        page.host.file_action(FileAction::Export),
        Outcome::default()
    );
    page.settle();
    page.host.shell.set_export_what(ExportWhat::Model).unwrap();
    page.host
        .shell
        .set_export_format(ExportFormat::Step)
        .unwrap();
    page.host.shell.answer_export(true);
    let request = page.host.shell.take_export_answer().unwrap();
    page.host.export_request(request);
    *page.worker.dies.borrow_mut() = true;
    let died = page.settle();
    assert!(died.downloads.is_empty());
    let error = page.host.shell.error_dialog().unwrap();
    assert!(error.message.contains("stopped"), "{error:?}");
    assert_eq!(*page.worker.stops.borrow(), 1);
    page.host.shell.close_error_dialog();
    let again = page.export(&ExportWhat::Model, ExportFormat::Step);
    assert_eq!(again.downloads.len(), 1);
    // Hangs: nothing comes; at the deadline it is given up.
    assert_eq!(
        page.host.file_action(FileAction::Export),
        Outcome::default()
    );
    page.settle();
    page.host.shell.answer_export(true);
    let request = page.host.shell.take_export_answer().unwrap();
    page.host.export_request(request);
    *page.worker.hangs.borrow_mut() = true;
    page.frame();
    assert!(page.host.session.commands_waiting());
    page.clock += DEADLINE_MS + 1.0;
    page.frame();
    assert!(!page.host.session.commands_waiting());
    let error = page.host.shell.error_dialog().unwrap();
    assert!(
        error.message.contains(&COMMAND_TOO_LONG[1..40]),
        "{error:?}"
    );
    assert_eq!(*page.worker.stops.borrow(), 2);
    assert_eq!(page.host.session.work_here(), work);
}

/// An edit that needs the bodies, asked for on the page, is made in the
/// worker and applied to the page's history: the same document as a page
/// that regenerates itself, undone as any edit. What was asked for after
/// it (an undo) waits for it and runs on the document it leaves.
#[test]
fn an_edit_that_needs_the_bodies_is_made_in_the_worker_and_undone_here() {
    let bytes = model();
    let mut want = in_process(&bytes);
    let mut page = opened(&bytes, false);
    let work = page.host.session.work_here();
    let add = json!({"name": "plate", "measure": "volume", "of": "Plate", "eq": "8000 * (1 mm)^3", "tol": "1e-6 * (1 mm)^3"});
    let wanted = want.session.execute("assert.add", &add).unwrap();
    let now = page
        .host
        .session
        .execute_deferrable("assert.add", &add)
        .unwrap();
    assert_eq!(now["deferred"], true);
    let before = page.host.session.document_bytes();
    // An undo asked for meanwhile waits for the edit.
    let id = page.host.session.document_status().unwrap().id;
    page.host
        .shell
        .request_edit_action(craft3d_ui_egui::shell::EditAction::Undo, id);
    assert!(page.host.session.edits_waiting());
    let report_before = page.host.shell.last_command.clone();
    page.host.pump();
    assert_eq!(page.host.session.document_bytes(), before, "held back");
    assert_eq!(page.host.shell.last_command, report_before);
    // The worker answers: the edit is applied, then the undo runs.
    for _ in 0..10 {
        if !page.host.session.edits_waiting() {
            break;
        }
        page.frame();
    }
    assert!(!page.host.session.edits_waiting());
    assert_eq!(
        page.host.session.document_bytes(),
        want.session.document_bytes()
    );
    let report = page.host.shell.last_command.clone().unwrap();
    assert!(report.ok, "{report:?} {wanted}");
    page.frame();
    assert_eq!(
        page.host.session.document_bytes(),
        before,
        "undone after it"
    );
    page.settle();
    assert_eq!(page.host.session.work_here(), work);
}

/// An export answered after the document was edited (a slider, an eye):
/// its file is still written, the export of the state it was asked on.
#[test]
fn an_export_survives_an_edit_before_its_answer() {
    let bytes = model();
    let want = wanted(
        &mut in_process(&bytes),
        &ExportWhat::Model,
        ExportFormat::Step,
    );
    let mut page = opened(&bytes, false);
    assert_eq!(
        page.host.file_action(FileAction::Export),
        Outcome::default()
    );
    page.settle();
    page.host.shell.set_export_what(ExportWhat::Model).unwrap();
    page.host
        .shell
        .set_export_format(ExportFormat::Step)
        .unwrap();
    page.host.shell.answer_export(true);
    let request = page.host.shell.take_export_answer().unwrap();
    assert_eq!(page.host.export_request(request), Outcome::default());
    // An edit before the worker answers.
    page.host
        .session
        .execute(
            "feature.block",
            &json!({"size": [1, 1, 1], "position": [0, 0, 50]}),
        )
        .unwrap();
    let got = page.settle();
    assert_eq!(files(&got.downloads), files(&want));
    assert!(page.host.shell.error_dialog().is_none());
}

/// An edit made in the worker that the person cancels: not made, no
/// error dialog, and what waited behind it (an undo) runs.
#[test]
fn an_edit_cancelled_is_not_made() {
    let bytes = model();
    let mut page = opened(&bytes, false);
    let before = page.host.session.document_bytes();
    let add = json!({"name": "plate", "measure": "volume", "of": "Plate", "eq": "8000 * (1 mm)^3", "tol": "1e-6 * (1 mm)^3"});
    let now = page
        .host
        .session
        .execute_deferrable("assert.add", &add)
        .unwrap();
    assert_eq!(now["deferred"], true);
    page.host.cancel_edits();
    assert!(!page.host.session.edits_waiting());
    page.settle();
    assert_eq!(page.host.session.document_bytes(), before);
    assert!(page.host.shell.error_dialog().is_none());
    let report = page.host.shell.last_command.clone().unwrap();
    assert!(
        !report.ok && report.text == crate::host::CANCELLED,
        "{report:?}"
    );
}

/// An edit the worker hangs on, cancelled: the worker is ended, the
/// document stays as it was, and the next job goes to a new worker.
#[test]
fn a_cancelled_edit_ends_a_worker_that_hangs_on_it() {
    let bytes = model();
    let mut page = opened(&bytes, false);
    let before = page.host.session.document_bytes();
    let add = json!({"name": "plate", "measure": "volume", "of": "Plate", "eq": "8000 * (1 mm)^3", "tol": "1e-6 * (1 mm)^3"});
    *page.worker.hangs.borrow_mut() = true;
    page.host
        .session
        .execute_deferrable("assert.add", &add)
        .unwrap();
    page.frame();
    assert!(page.host.session.edit_posted());
    let stops = *page.worker.stops.borrow();
    page.host.cancel_edits();
    page.frame();
    assert_eq!(*page.worker.stops.borrow(), stops + 1);
    assert!(!page.host.session.edits_waiting());
    assert_eq!(page.host.session.document_bytes(), before);
    // Nothing went to the bug report: no command failed.
    assert!(page.host.session.last_failure().is_none());
    // A new worker takes the next job.
    *page.worker.hangs.borrow_mut() = false;
    let got = page.export(&ExportWhat::Model, ExportFormat::Step);
    assert_eq!(got.downloads.len(), 1);
}

/// The export dialog answered while an edit waits: made after the edit,
/// of the document it leaves.
#[test]
fn an_export_answered_during_an_edit_follows_it() {
    let bytes = model();
    let mut page = opened(&bytes, false);
    assert_eq!(
        page.host.file_action(FileAction::Export),
        Outcome::default()
    );
    page.settle();
    page.host.shell.set_export_what(ExportWhat::Model).unwrap();
    page.host
        .shell
        .set_export_format(ExportFormat::Stl)
        .unwrap();
    let add = json!({"name": "plate", "measure": "volume", "of": "Plate", "eq": "8000 * (1 mm)^3", "tol": "1e-6 * (1 mm)^3"});
    page.host
        .session
        .execute_deferrable("assert.add", &add)
        .unwrap();
    assert!(page.host.session.edits_waiting());
    page.host.shell.answer_export(true);
    let ctx = egui::Context::default();
    let mut held = Outcome::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        held = page.host.dialog_ui(ui.ctx());
    });
    output.textures_delta.clear();
    assert_eq!(held, Outcome::default(), "kept while the edit waits");
    let revision = page.host.session.document_revision();
    let got = page.settle();
    assert_ne!(
        page.host.session.document_revision(),
        revision,
        "the edit came first"
    );
    assert_eq!(got.downloads.len(), 1);
    let mut want = in_process(&bytes);
    want.session.execute("assert.add", &add).unwrap();
    let want = wanted(&mut want, &ExportWhat::Model, ExportFormat::Stl);
    assert_eq!(files(&got.downloads), files(&want));
}

/// The published planetary gearbox (a packed assembly) on the page: placed
/// in the worker, and its motion's moments made there as the person plays
/// it, frame by frame; each is drawn as a page that places it itself
/// draws it, the tree lists its mates, and the page's own thread never
/// places, solves or regenerates.
#[test]
fn an_assembly_is_placed_and_played_in_the_worker() {
    let bytes = include_bytes!("../../../docs/examples/published/planetary-gearbox.3dcraftasm");
    let mut page = Page::new(false);
    page.host
        .session
        .open_bytes("planetary-gearbox.3dcraftasm", bytes)
        .unwrap();
    page.settle();
    let before = page.host.session.work_here();
    let mut here = Session::new(Workspace::memory());
    here.open_bytes("planetary-gearbox.3dcraftasm", bytes)
        .unwrap();
    let same = |page: &mut Page, here: &mut Session| {
        for quality in [SceneQuality::Preview, SceneQuality::display()] {
            assert_eq!(
                scene_bytes(&page.host.session.scene_with(quality).unwrap().unwrap()),
                scene_bytes(&here.scene_with(quality).unwrap().unwrap()),
            );
        }
    };
    same(&mut page, &mut here);
    page.host.shell.sync(&mut page.host.session);
    assert_eq!(
        page.host.shell.assembly_outline().map(|o| o.mates.len()),
        here.assembly_outline().map(|o| o.mates.len())
    );
    for t in [0.05, 0.1, 0.4] {
        page.host
            .session
            .execute("view.motion", &serde_json::json!({"t": t}))
            .unwrap();
        // Asked for, not made on the page: pending until the worker
        // answers.
        assert!(page.host.session.motion().unwrap().pending);
        page.settle();
        assert!(!page.host.session.motion().unwrap().pending);
        here.execute("view.motion", &serde_json::json!({"t": t}))
            .unwrap();
        same(&mut page, &mut here);
        assert_eq!(
            page.host.session.assembly_outline(),
            here.assembly_outline()
        );
    }
    assert_eq!(page.host.session.work_here(), before);
}
