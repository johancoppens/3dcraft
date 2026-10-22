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

/// Saves every document with unsaved changes worth keeping (those
/// `file.close` refuses without `discard`) to its own file, for
/// `ui.quit {save: "save"}` from the control channel: through the
/// automation write root, as `file.save` does for a request, never with
/// the person's own access (a document the person opened from outside the
/// roots is not the channel's to overwrite). The document that was shown
/// is shown again. Refused, with nothing saved, when one of them was
/// never saved (there is nobody to name its file) or its file lies
/// outside the write root. A save that fails stops there, with the
/// documents before it saved.
pub fn save_all(session: &mut Session) -> Result<Value, String> {
    let statuses = session.document_statuses();
    let unnamed: Vec<String> = craft3d_ui_egui::doc_tabs(&statuses, None)
        .into_iter()
        .filter(|tab| tab.asks_to_close && tab.path.is_none())
        .map(|tab| format!("\"{}\" (document {})", tab.name, tab.document))
        .collect();
    if !unnamed.is_empty() {
        return Err(format!(
            "ui.quit: {} never saved; save it with file.save and a path first, or quit with save: \"discard\" or \"ask\"; nothing was saved",
            unnamed.join(", ")
        ));
    }
    let outside: Vec<String> = statuses
        .iter()
        .filter(|status| status.close_asks)
        .filter_map(|status| status.path.as_deref())
        .filter(|path| session.workspace().resolve_write(path).is_err())
        .map(|path| format!("`{path}`"))
        .collect();
    if !outside.is_empty() {
        return Err(format!(
            "ui.quit: {} lies outside the automation write root, so the control channel may not save it; save it at the window, or quit with save: \"discard\" or \"ask\"; nothing was saved",
            outside.join(", ")
        ));
    }
    let before = session.document_status().map(|status| status.index);
    let mut saved = Vec::new();
    let mut outcome = Ok(());
    for status in statuses.iter().filter(|status| status.close_asks) {
        // Through the roots, as a request's `file.save`.
        let done =
            activate(session, status.index).and_then(|_| execute(session, "file.save", &json!({})));
        match done {
            Ok(_) => saved.push(status.path.clone().unwrap_or_default()),
            Err(error) => {
                outcome = Err(error);
                break;
            }
        }
    }
    if let Some(before) = before
        && session.document_status().map(|status| status.index) != Some(before)
    {
        let _ = activate(session, before);
    }
    outcome.map(|()| json!({ "quitting": true, "saved": saved }))
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
    use craft3d_ui_egui::CloseAnswer;
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
        fn import(&self) -> Option<PathBuf> {
            None
        }
        fn export(&self, _: &str) -> Option<PathBuf> {
            None
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
        // Wait for this save's own report, not the one before it, and read
        // it before settling: on a busy machine the shell's next command
        // (a `file.list` for the tabs) can report after it (gate 63).
        desk.host.shell.last_command = None;
        desk.host
            .shell
            .request_doc_action(DocAction::SaveAndClose(0));
        let deadline = Instant::now() + Duration::from_secs(20);
        while desk
            .host
            .shell
            .last_command
            .as_ref()
            .map(|r| r.label.as_str())
            != Some("file.save")
            || desk.away()
        {
            assert!(Instant::now() < deadline, "the dialog did not answer");
            let _ = desk.turn();
            std::thread::sleep(Duration::from_millis(2));
        }
        let (_, ok, text) = last(&desk);
        settle(&mut desk);
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

    /// Three documents: a ball saved to its own file and changed since,
    /// a block never saved, and a cylinder saved with nothing unsaved.
    fn three(dir: &Path) -> (Session, PathBuf) {
        let own = dir.join("ball.3dcraft");
        let mut s = two();
        run(&mut s, "file.activate", json!({"document": 0}));
        files::run_as_user(&mut s, "file.save", &own, true, None).unwrap();
        run(&mut s, "feature.block", json!({"size": [4, 4, 4]}));
        run(&mut s, "file.new", json!({}));
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": 1, "height": 2}),
        );
        let saved = dir.join("can.3dcraft");
        files::run_as_user(&mut s, "file.save", &saved, true, None).unwrap();
        (s, own)
    }

    /// Turns until the window asks a question, or the application is
    /// asked to stop.
    fn until_asked(desk: &mut Desk) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while desk.host.shell.closing().is_none() && !desk.host.shell.quit_requested() {
            assert!(Instant::now() < deadline, "no question, no quit");
            let _ = desk.turn();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn features_in(file: &Path) -> Value {
        let mut check = Session::new(Workspace::none());
        files::run_as_user(&mut check, "file.open", file, false, None).unwrap();
        run(&mut check, "file.list", json!({}))["documents"][0]["features"].clone()
    }

    /// Roadmap R1: the window's close button asks about each document
    /// with unsaved changes, one after the other (Save to its own file,
    /// Save of one never saved through the save dialog), leaves the saved
    /// one alone, and then stops the application with nothing unsaved,
    /// so the autosaves may go.
    #[test]
    fn closing_the_window_asks_about_each_unsaved_document_then_quits() {
        let dir = workdir("quit-save");
        let (s, own) = three(&dir);
        let named = dir.join("block.3dcraft");
        let mut desk = desk(s, vec![Some(named.clone())]);
        settle(&mut desk);
        assert!(!desk.window_close(), "two documents have unsaved changes");
        assert!(!desk.window_close(), "asked again: still asking");
        until_asked(&mut desk);
        let closing = desk.host.shell.closing().unwrap().clone();
        assert_eq!((closing.name.as_str(), closing.quitting), ("ball", true));
        assert!(desk.host.shell.quitting() && !desk.host.shell.quit_requested());
        desk.host.shell.answer_closing(CloseAnswer::Save);
        settle(&mut desk);
        until_asked(&mut desk);
        let closing = desk.host.shell.closing().unwrap().clone();
        assert_eq!((closing.name.as_str(), closing.path), ("Untitled 1", None));
        desk.host.shell.answer_closing(CloseAnswer::Save);
        settle(&mut desk);
        until_asked(&mut desk);
        assert!(desk.host.shell.quit_requested());
        assert!(!desk.host.shell.quitting() && desk.host.shell.closing().is_none());
        assert!(!desk.host.shell.quit_discards());
        assert!(desk.may_end_recovery());
        // Both were saved where they belong; the can stays open, as it was.
        let statuses = desk.host.session.document_statuses();
        assert_eq!(statuses.len(), 1);
        assert!(
            statuses[0]
                .path
                .as_deref()
                .unwrap()
                .ends_with("can.3dcraft")
        );
        assert_eq!(features_in(&own), 2);
        assert_eq!(features_in(&named), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Don't Save closes the document without saving it; Cancel keeps the
    /// window with what is still open; a window with nothing unsaved
    /// closes at once.
    #[test]
    fn dont_save_drops_and_cancel_keeps_the_window() {
        let dir = workdir("quit-cancel");
        let (s, own) = three(&dir);
        let mut desk = self::desk(s, Vec::new());
        settle(&mut desk);
        assert!(!desk.window_close());
        until_asked(&mut desk);
        desk.host.shell.answer_closing(CloseAnswer::Discard);
        settle(&mut desk);
        until_asked(&mut desk);
        assert_eq!(desk.host.shell.closing().unwrap().name, "Untitled 1");
        desk.host.shell.answer_closing(CloseAnswer::Cancel);
        settle(&mut desk);
        assert!(!desk.host.shell.quitting() && !desk.host.shell.quit_requested());
        assert!(desk.host.shell.closing().is_none());
        assert_eq!(desk.host.session.document_statuses().len(), 2);
        // The ball's file is as it was saved: its change was dropped.
        assert_eq!(features_in(&own), 1);
        // Asked again: Don't Save for the block too, and the window goes;
        // nothing unsaved is left.
        assert!(!desk.window_close());
        until_asked(&mut desk);
        desk.host.shell.answer_closing(CloseAnswer::Discard);
        settle(&mut desk);
        until_asked(&mut desk);
        assert!(desk.host.shell.quit_requested() && desk.may_end_recovery());
        // A window with nothing unsaved closes at once.
        let mut clean = Session::new(Workspace::none());
        run(&mut clean, "file.new", json!({}));
        let mut quiet = self::desk(clean, Vec::new());
        settle(&mut quiet);
        assert!(quiet.window_close());
        assert!(quiet.host.shell.quit_requested());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Save for a document never saved whose save dialog is cancelled
    /// closes nothing, and the window stays: it does not ask again and
    /// again.
    #[test]
    fn a_cancelled_save_dialog_keeps_the_window() {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({}));
        run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
        let mut desk = desk(s, vec![None]);
        settle(&mut desk);
        assert!(!desk.window_close());
        until_asked(&mut desk);
        desk.host.shell.answer_closing(CloseAnswer::Save);
        let deadline = Instant::now() + Duration::from_secs(20);
        while desk.host.shell.quitting() {
            assert!(Instant::now() < deadline, "still closing");
            let _ = desk.turn();
            std::thread::sleep(Duration::from_millis(2));
        }
        settle(&mut desk);
        assert!(!desk.host.shell.quit_requested());
        assert!(desk.host.shell.closing().is_none());
        assert_eq!(desk.host.session.document_statuses().len(), 1);
        assert!(!desk.may_end_recovery());
    }

    /// The close button while a command runs that leaves a document with
    /// unsaved changes: nothing is decided on the tabs from before the
    /// command; once it is done the window asks about that document.
    #[test]
    fn a_close_while_a_command_runs_is_decided_when_it_is_done() {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({}));
        let (requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(Answers(Mutex::new(Vec::new()))), Arc::new(|| {}));
        let mut host = Host::new(s);
        host.shell.sync(&mut host.session);
        let mut desk = Desk::new(host, incoming, picker)
            .with_engine_thread(Arc::new(|| {}))
            .with_patience(Duration::ZERO);
        settle(&mut desk);
        assert!(!desk.host.shell.tabs()[0].asks_to_close);
        let (reply, _answer) = mpsc::channel();
        requests
            .send(crate::host::Request {
                line: r#"{"id":1,"method":"engine.execute","params":{"command":"feature.sphere","params":{"radius":2}}}"#.to_string(),
                reply,
            })
            .unwrap();
        let _ = desk.turn();
        assert!(desk.away(), "the command runs on the engine thread");
        assert!(!desk.window_close(), "not decided while it runs");
        until_asked(&mut desk);
        assert!(!desk.host.shell.quit_requested());
        assert_eq!(desk.host.shell.closing().unwrap().name, "Untitled 1");
    }

    /// One request over the control channel, answered by the desk.
    fn ask(desk: &mut Desk, requests: &mpsc::Sender<crate::host::Request>, line: &str) -> Value {
        let (reply, answer) = mpsc::channel();
        requests
            .send(crate::host::Request {
                line: line.to_string(),
                reply,
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let _ = desk.turn();
            if let Ok(reply) = answer.try_recv() {
                return serde_json::from_str(&reply.unwrap()).unwrap();
            }
            assert!(Instant::now() < deadline, "no reply to {line}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// `ui.quit` over the control channel: `save: "ask"` asks as the
    /// close button does, `save: "save"` saves every document to its own
    /// file and stops (refused, nothing saved, while one was never saved),
    /// the default drops what is unsaved, and anything else is an error
    /// that changes nothing.
    #[test]
    fn ui_quit_asks_saves_or_discards_over_the_control_channel() {
        let dir = workdir("quit-channel");
        let (s, own) = three(&dir);
        let (requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(Answers(Mutex::new(Vec::new()))), Arc::new(|| {}));
        let mut host = Host::new(s);
        host.shell.sync(&mut host.session);
        let mut desk = Desk::new(host, incoming, picker)
            .with_engine_thread(Arc::new(|| {}))
            .with_patience(Duration::ZERO);
        settle(&mut desk);
        for bad in [
            r#"{"id":1,"method":"ui.quit","params":{"save":"maybe"}}"#,
            r#"{"id":1,"method":"ui.quit","params":{"save":true}}"#,
            r#"{"id":1,"method":"ui.quit","params":{"keep":1}}"#,
            r#"{"id":1,"method":"ui.quit","params":[]}"#,
        ] {
            let reply = ask(&mut desk, &requests, bad);
            assert_eq!(reply["ok"], false, "{bad}: {reply}");
            assert!(!desk.host.shell.quit_requested() && !desk.host.shell.quitting());
        }
        // Save: refused while the block was never saved; nothing saved.
        let reply = ask(
            &mut desk,
            &requests,
            r#"{"id":2,"method":"ui.quit","params":{"save":"save"}}"#,
        );
        assert_eq!(reply["ok"], false, "{reply}");
        let error = reply["error"].as_str().unwrap();
        assert!(
            error.contains("Untitled 1") && error.contains("never saved"),
            "{error}"
        );
        assert!(!desk.host.shell.quit_requested());
        assert!(desk.host.session.document_statuses()[0].dirty);
        // Ask: the window asks, as its close button does.
        let reply = ask(
            &mut desk,
            &requests,
            r#"{"id":3,"method":"ui.quit","params":{"save":"ask"}}"#,
        );
        assert_eq!(reply["result"]["quitting"], false, "{reply}");
        // The can is shown: the ball is shown first, then asked about.
        assert_eq!(reply["result"]["asking"], Value::Null, "{reply}");
        until_asked(&mut desk);
        let got = ask(&mut desk, &requests, r#"{"id":4,"method":"ui.get"}"#);
        assert_eq!(got["result"]["quitting"], true, "{got}");
        assert_eq!(got["result"]["closing"]["name"], "ball", "{got}");
        assert_eq!(got["result"]["closing"]["quitting"], true, "{got}");
        assert_eq!(active(&desk), Some(0));
        // Asked while it asks: the same question.
        let reply = ask(
            &mut desk,
            &requests,
            r#"{"id":3,"method":"ui.quit","params":{"save":"ask"}}"#,
        );
        assert_eq!(reply["result"]["asking"]["name"], "ball", "{reply}");
        desk.host.shell.answer_closing(CloseAnswer::Cancel);
        // The block gets a file and another change; then Save saves the
        // ball and the block, and stops.
        let block = dir.join("block.3dcraft");
        run(
            &mut desk.host.session,
            "file.activate",
            json!({"document": 1}),
        );
        files::run_as_user(&mut desk.host.session, "file.save", &block, true, None).unwrap();
        run(
            &mut desk.host.session,
            "feature.sphere",
            json!({"radius": 1}),
        );
        // Both files were chosen by the person, and the channel has no
        // write root: the channel may not overwrite them. Refused,
        // nothing saved, the window stays.
        let quit_save = r#"{"id":5,"method":"ui.quit","params":{"save":"save"}}"#;
        let reply = ask(&mut desk, &requests, quit_save);
        assert_eq!(reply["ok"], false, "{reply}");
        let error = reply["error"].as_str().unwrap();
        assert!(
            error.contains("outside the automation write root"),
            "{error}"
        );
        assert!(!desk.host.shell.quit_requested());
        assert_eq!(features_in(&own), 1);
        assert_eq!(features_in(&block), 1);
        // A write root the files lie beneath: saved through it, and the
        // application stops.
        desk.host.session.set_workspace(Workspace {
            read: craft3d_engine::Access::None,
            write: craft3d_engine::Access::Root(dir.clone()),
        });
        let reply = ask(&mut desk, &requests, quit_save);
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(
            reply["result"]["saved"].as_array().unwrap().len(),
            2,
            "{reply}"
        );
        assert!(desk.host.shell.quit_requested() && !desk.host.shell.quit_discards());
        assert!(desk.may_end_recovery());
        assert_eq!(active(&desk), Some(1), "the document shown stays shown");
        assert_eq!(features_in(&own), 2);
        assert_eq!(features_in(&block), 2);
        // The default drops what is unsaved, as before.
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({}));
        run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
        let (requests, incoming) = mpsc::channel();
        let picker = Picker::new(Arc::new(Answers(Mutex::new(Vec::new()))), Arc::new(|| {}));
        let mut desk = Desk::new(Host::new(s), incoming, picker);
        let reply = ask(&mut desk, &requests, r#"{"id":6,"method":"ui.quit"}"#);
        assert_eq!(reply["result"]["quitting"], true, "{reply}");
        assert!(desk.host.shell.quit_discards() && desk.may_end_recovery());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
