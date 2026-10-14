//! What the tabs of the open documents ask for ([`DocAction`]), done with
//! the engine's commands: `file.activate`, `file.close` and `file.save`.
//! On the engine thread when the window has one ([`crate::app::Desk`]),
//! here otherwise ([`Host::doc_action`]).
//!
//! `file.close` closes the active document, so closing another one from
//! its tab activates it, closes it and activates again the document that
//! was active (or, when the closed one was the active one, its neighbour
//! in the row: the next one, or the one before when it was the last).

use crate::files::{self, Picker, suggested_name};
use crate::host::Host;
use craft3d_engine::Session;
use craft3d_ui_egui::DocAction;
use serde_json::{Value, json};
use std::path::Path;

/// The command a document action is reported as in the status bar.
pub fn label(action: &DocAction) -> &'static str {
    match action {
        DocAction::Activate(_) => "file.activate",
        DocAction::Close { .. } => "file.close",
        DocAction::SaveAndClose(_) => "file.save",
    }
}

/// True when the action needs the save dialog first: Save and close for
/// a document that has no file of its own.
pub fn needs_a_name(session: &Session, action: &DocAction) -> bool {
    let DocAction::SaveAndClose(document) = action else {
        return false;
    };
    session
        .document_statuses()
        .get(*document)
        .is_some_and(|status| status.path.is_none())
}

fn execute(session: &mut Session, command: &str, params: &Value) -> Result<Value, String> {
    session
        .execute(command, params)
        .map_err(|e| e.message().to_string())
}

fn activate(session: &mut Session, document: usize) -> Result<Value, String> {
    execute(session, "file.activate", &json!({ "document": document }))
}

/// Does `action` on `session` with its commands. Guarded by whoever calls
/// it (the engine thread, or [`Host::doc_action`]).
pub fn perform(session: &mut Session, action: &DocAction) -> Result<Value, String> {
    match *action {
        DocAction::Activate(document) => activate(session, document),
        DocAction::Close { document, discard } => close(session, document, discard),
        DocAction::SaveAndClose(document) => {
            let before = session.document_status().map(|status| status.index);
            activate(session, document)?;
            let saved = save_active(session);
            // The one that was shown is shown again; when the save failed
            // the document stays open.
            if let Some(before) = before.filter(|before| *before != document) {
                let _ = activate(session, before);
            }
            let saved = saved?;
            close(session, document, false)
                .map(|closed| json!({ "saved": saved, "closed": closed }))
        }
    }
}

/// Saves the active document to its own file, as Save in the File menu
/// does: a file the person chose (an absolute path) with the person's
/// access to it, a path a request gave beneath the automation roots the
/// way a request would.
fn save_active(session: &mut Session) -> Result<Value, String> {
    let path = session
        .document_status()
        .and_then(|status| status.path)
        .ok_or_else(|| "file.save: the document has no file yet".to_string())?;
    if Path::new(&path).is_absolute() {
        files::run_as_user(session, "file.save", Path::new(&path), true, None)
    } else {
        execute(session, "file.save", &json!({}))
    }
}

/// Closes document `document`, and activates the one that should be shown
/// after it ([module documentation](self)).
pub fn close(session: &mut Session, document: usize, discard: bool) -> Result<Value, String> {
    let open = session.document_statuses().len();
    if document >= open {
        return Err(format!(
            "file.close: there is no document {document}; {open} are open (file.list)"
        ));
    }
    let before = session.document_status().map(|status| status.index);
    if before != Some(document) {
        activate(session, document)?;
    }
    let closed = execute(session, "file.close", &json!({ "discard": discard }));
    let left = session.document_statuses().len();
    let shown = match (&closed, before) {
        // Not closed: the one shown before stays shown.
        (Err(_), before) => before,
        (Ok(_), Some(before)) if before != document => Some(if before > document {
            before - 1
        } else {
            before
        }),
        // The active one was closed: its neighbour.
        (Ok(_), _) => (left > 0).then(|| document.min(left - 1)),
    };
    if let Some(shown) = shown
        && session.document_status().map(|status| status.index) != Some(shown)
    {
        let _ = activate(session, shown);
    }
    closed
}

impl Host {
    /// Does what a tab asked for, here: for a window without an engine
    /// thread. Save and close for a document without a file activates it
    /// and opens the save dialog; it is closed once it is saved there
    /// ([`Self::close_after_save`]).
    pub fn doc_action(&mut self, action: DocAction, picker: &mut Picker) {
        let label = label(&action);
        if needs_a_name(&self.session, &action) {
            let DocAction::SaveAndClose(document) = action else {
                return;
            };
            let outcome = self.guarded(|session| activate(session, document));
            self.shell.sync(&mut self.session);
            if outcome.is_err() {
                self.report(label, &outcome);
                return;
            }
            self.ask_name_then_close(picker);
            return;
        }
        let outcome = self.guarded(|session| perform(session, &action));
        self.report(label, &outcome);
        self.shell.sync(&mut self.session);
    }

    /// Opens the save dialog for the active document, which is closed once
    /// it is saved there.
    pub(crate) fn ask_name_then_close(&mut self, picker: &mut Picker) {
        match picker.ask_save(suggested_name(None)) {
            Ok(()) => self.close_after_save = true,
            Err(error) => self.report("file.save", &Err(error)),
        }
    }

    /// Runs `run` on the session, a panic reported as an error after an
    /// autosave.
    fn guarded(
        &mut self,
        run: impl FnOnce(&mut Session) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let session = &mut self.session;
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(session)));
        ran.unwrap_or_else(|_| {
            self.autosave_after_a_panic();
            Err(
                "internal error (a bug in 3DCraft, please report it); the document was kept"
                    .to_string(),
            )
        })
    }
}

/// After the save dialog's file was saved: closes the active document
/// when that was asked for (Save in the question before closing), and
/// says so with the save's outcome.
pub fn close_when_saved(
    session: &mut Session,
    saved: Result<Value, String>,
    close_after: bool,
) -> Result<Value, String> {
    let Ok(saved) = saved else {
        return saved;
    };
    if !close_after {
        return Ok(saved);
    }
    let Some(document) = session.document_status().map(|status| status.index) else {
        return Ok(saved);
    };
    close(session, document, false).map(|closed| json!({ "saved": saved, "closed": closed }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Desk;
    use crate::files::Dialogs;
    use craft3d_engine::Workspace;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    /// The save dialog's answers, in turn.
    struct Answers(Mutex<Vec<Option<PathBuf>>>);

    impl Dialogs for Answers {
        fn open(&self) -> Option<PathBuf> {
            None
        }
        fn save(&self, _: &str) -> Option<PathBuf> {
            self.0.lock().unwrap().pop().flatten()
        }
    }

    fn workdir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/tmp/craft3d-tabs-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(dir).unwrap()
    }

    fn run(session: &mut Session, command: &str, params: Value) -> Value {
        session
            .execute(command, &params)
            .unwrap_or_else(|e| panic!("{command}: {e}"))
    }

    /// A window's desk with an engine thread that is never waited for, so
    /// every task leaves the session away for at least one turn.
    fn desk(session: Session, answers: Vec<Option<PathBuf>>) -> Desk {
        let (_requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(Answers(Mutex::new(answers))), Arc::new(|| {}));
        let mut host = Host::new(session);
        host.shell.sync(&mut host.session);
        Desk::new(host, incoming, picker)
            .with_engine_thread(Arc::new(|| {}))
            .with_patience(Duration::ZERO)
    }

    /// Turns until the session is home and nothing is left to do.
    fn settle(desk: &mut Desk) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let turn = desk.turn();
            if !turn.changed && !desk.away() {
                return;
            }
            assert!(Instant::now() < deadline, "the desk did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn active(desk: &Desk) -> Option<usize> {
        desk.host
            .session
            .document_status()
            .map(|status| status.index)
    }

    fn last(desk: &Desk) -> (String, bool, String) {
        let report = desk.host.shell.last_command.clone().unwrap();
        (report.label, report.ok, report.text)
    }

    /// A ball (meshing it takes the engine thread a moment) and a block.
    fn two() -> Session {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({}));
        run(&mut s, "feature.sphere", json!({"radius": 10}));
        run(&mut s, "file.new", json!({}));
        run(&mut s, "feature.block", json!({"size": [1, 2, 3]}));
        s
    }

    #[test]
    fn a_click_on_a_tab_runs_file_activate_on_the_engine_thread() {
        let mut desk = desk(two(), Vec::new());
        settle(&mut desk);
        assert_eq!(active(&desk), Some(1));
        desk.host.shell.request_doc_action(DocAction::Activate(0));
        let _ = desk.turn();
        // The session went to the engine thread with the command; the
        // window draws what it showed before meanwhile.
        assert!(desk.away(), "file.activate ran on this thread");
        assert!(desk.host.shell.tabs()[1].active);
        settle(&mut desk);
        assert_eq!(active(&desk), Some(0));
        let (label, ok, _) = last(&desk);
        assert_eq!((label.as_str(), ok), ("file.activate", true));
        let tabs = desk.host.shell.tabs();
        assert!(tabs[0].active && !tabs[1].active);
        // The scene shown is the ball's, built there.
        assert_eq!(desk.host.shell.tree_rows().len(), 1);
        assert_eq!(desk.host.shell.tree_rows()[0].kind, "sphere");
    }

    #[test]
    fn closing_from_a_tab_keeps_the_active_document_or_shows_its_neighbour() {
        let mut s = two();
        run(&mut s, "file.new", json!({}));
        run(&mut s, "file.activate", json!({"document": 1}));
        let mut desk = desk(s, Vec::new());
        settle(&mut desk);
        // Another one than the active one: the active one stays shown.
        desk.host.shell.request_doc_action(DocAction::Close {
            document: 0,
            discard: true,
        });
        settle(&mut desk);
        assert_eq!(desk.host.session.document_statuses().len(), 2);
        assert_eq!(active(&desk), Some(0), "the block, now first");
        assert_eq!(desk.host.shell.tree_rows()[0].kind, "block");
        // Unsaved changes without discard: refused, nothing changes.
        desk.host.shell.request_doc_action(DocAction::Close {
            document: 0,
            discard: false,
        });
        settle(&mut desk);
        let (label, ok, text) = last(&desk);
        assert_eq!((label.as_str(), ok), ("file.close", false));
        assert!(text.contains("unsaved changes"), "{text}");
        assert_eq!(desk.host.session.document_statuses().len(), 2);
        // The empty new one behind it, from its tab: the block stays.
        desk.host.shell.request_doc_action(DocAction::Close {
            document: 1,
            discard: false,
        });
        settle(&mut desk);
        assert_eq!(desk.host.session.document_statuses().len(), 1);
        assert_eq!(active(&desk), Some(0));
        // The active one, of three: its neighbour to the right is shown.
        for _ in 0..2 {
            run(&mut desk.host.session, "file.new", json!({}));
        }
        run(
            &mut desk.host.session,
            "feature.cylinder",
            json!({"radius": 1, "height": 2}),
        );
        run(
            &mut desk.host.session,
            "file.activate",
            json!({"document": 1}),
        );
        desk.host.shell.request_doc_action(DocAction::Close {
            document: 1,
            discard: false,
        });
        settle(&mut desk);
        assert_eq!(desk.host.session.document_statuses().len(), 2);
        assert_eq!(active(&desk), Some(1));
        assert_eq!(desk.host.shell.tree_rows()[0].kind, "cylinder");
        // A document that is not there: an error, nothing else.
        desk.host.shell.request_doc_action(DocAction::Close {
            document: 7,
            discard: true,
        });
        settle(&mut desk);
        assert!(!last(&desk).1);
        assert_eq!(desk.host.session.document_statuses().len(), 2);
    }

    #[test]
    fn save_in_the_question_saves_to_the_own_file_or_asks_for_one_then_closes() {
        let dir = workdir("save");
        let own = dir.join("ball.3dcraft");
        let mut s = two();
        run(&mut s, "file.activate", json!({"document": 0}));
        files::run_as_user(&mut s, "file.save", &own, true, None).unwrap();
        run(&mut s, "feature.block", json!({"size": [4, 4, 4]}));
        run(&mut s, "file.activate", json!({"document": 1}));
        let named = dir.join("block.3dcraft");
        let mut desk = desk(s, vec![None, Some(named.clone())]);
        settle(&mut desk);
        assert!(desk.host.shell.tabs()[0].dirty);

        // Its own file: saved there with the change, closed, and the
        // block stays shown.
        desk.host
            .shell
            .request_doc_action(DocAction::SaveAndClose(0));
        settle(&mut desk);
        let (label, ok, text) = last(&desk);
        assert_eq!((label.as_str(), ok), ("file.save", true), "{text}");
        assert_eq!(desk.host.session.document_statuses().len(), 1);
        assert_eq!(desk.host.shell.tree_rows()[0].kind, "block");
        let mut check = Session::new(Workspace::none());
        files::run_as_user(&mut check, "file.open", &own, false, None).unwrap();
        assert_eq!(
            run(&mut check, "file.list", json!({}))["documents"][0]["features"],
            2
        );

        // No file: the save dialog, and closed once saved where it said.
        desk.host
            .shell
            .request_doc_action(DocAction::SaveAndClose(0));
        let deadline = Instant::now() + Duration::from_secs(20);
        while desk.host.session.document_statuses().len() == 1 || desk.away() {
            assert!(Instant::now() < deadline, "not closed");
            let _ = desk.turn();
            std::thread::sleep(Duration::from_millis(2));
        }
        settle(&mut desk);
        assert!(named.exists());
        assert!(desk.host.session.document_statuses().is_empty());
        assert!(desk.host.shell.tabs().is_empty());
        assert!(!desk.host.close_after_save);

        // A dialog that is cancelled closes nothing: the document stays,
        // shown, with its changes.
        run(&mut desk.host.session, "file.new", json!({}));
        run(
            &mut desk.host.session,
            "feature.sphere",
            json!({"radius": 1}),
        );
        run(&mut desk.host.session, "file.new", json!({}));
        desk.host
            .shell
            .request_doc_action(DocAction::SaveAndClose(0));
        let deadline = Instant::now() + Duration::from_secs(20);
        while last(&desk).0 != "file.save" || desk.away() {
            assert!(Instant::now() < deadline, "the dialog did not answer");
            let _ = desk.turn();
            std::thread::sleep(Duration::from_millis(2));
        }
        settle(&mut desk);
        let (_, ok, text) = last(&desk);
        assert!(!ok && text.contains("cancelled"), "{text}");
        assert_eq!(desk.host.session.document_statuses().len(), 2);
        assert_eq!(active(&desk), Some(0));
        assert!(!desk.host.close_after_save);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Without an engine thread the host does the same, where it is.
    #[test]
    fn without_an_engine_thread_the_host_does_it_here() {
        let mut host = Host::new(two());
        let mut picker = Picker::new(Arc::new(Answers(Mutex::new(Vec::new()))), Arc::new(|| {}));
        host.shell.sync(&mut host.session);
        host.shell.request_doc_action(DocAction::Activate(0));
        assert!(host.pump_files(&mut picker));
        assert_eq!(host.session.document_status().unwrap().index, 0);
        assert!(host.shell.tabs()[0].active);
        host.shell.request_doc_action(DocAction::Close {
            document: 1,
            discard: true,
        });
        assert!(host.pump_files(&mut picker));
        assert_eq!(host.session.document_statuses().len(), 1);
        assert_eq!(
            host.shell.last_command.as_ref().unwrap().label,
            "file.close"
        );
        // No file: the dialog, cancelled here.
        host.shell.request_doc_action(DocAction::SaveAndClose(0));
        assert!(host.pump_files(&mut picker));
        assert!(host.close_after_save);
        let choice = picker.wait().unwrap();
        host.file_choice(choice);
        assert!(!host.close_after_save);
        assert_eq!(host.session.document_statuses().len(), 1);
    }
}
