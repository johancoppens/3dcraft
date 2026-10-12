//! New, Open, Save and Save As for the person at the window.
//!
//! The engine's file commands run under the session's [`Workspace`]: the
//! automation roots, which keep a script or an agent on the control
//! channel inside the directories it was given. A person who picks a file
//! in the system's dialog has chosen that file themselves, and may open or
//! save it wherever they can: so the same commands (`file.open`,
//! `file.save`; everything is a command) are run for them with a grant
//! for that one choice.
//!
//! The grant is as narrow and as short as the engine can express:
//!
//! - one direction (reading to open, writing to save), the other denied;
//! - one directory, the one the chosen file is in;
//! - one command: the automation workspace is put back before this
//!   module returns, whether the command worked, failed or panicked.
//!
//! It cannot be reached from the control channel. Requests are answered on
//! the thread that owns the session, between frames, one at a time, and so
//! is this: no request runs while the grant is in place. There is no
//! method, parameter or MCP tool that asks for it; only a file action of
//! the shell (its menu and shortcuts) and a path that came out of the
//! dialog lead here. What a request can do afterwards is what it could do
//! before: a document that was opened from outside the roots can be
//! edited by an agent (it is the document in the window), but saved by one
//! only to a path beneath the write root.
//!
//! The dialog itself runs on its own thread, so the window keeps drawing
//! and answering requests while it is open ([`Picker`]). It is behind a
//! trait ([`Dialogs`]): tests answer for the person.

use crate::control::Wake;
use crate::host::Host;
use craft3d_engine::{Access, CommandError, Workspace};
use craft3d_ui_egui::{CommandReport, FileAction};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// The extension of the native format.
pub const EXTENSION: &str = "3dcraft";

/// The system's file dialogs. Each call blocks until the person chose or
/// cancelled (`None`).
pub trait Dialogs: Send + Sync {
    /// Asks for a file to open.
    fn open(&self) -> Option<PathBuf>;
    /// Asks where to save, proposing `suggested` as the file's name.
    fn save(&self, suggested: &str) -> Option<PathBuf>;
}

/// The dialogs of the desktop (the portal's on Linux).
pub struct SystemDialogs;

impl Dialogs for SystemDialogs {
    fn open(&self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Open")
            .add_filter("3DCraft document", &[EXTENSION])
            .add_filter("All files", &["*"])
            .pick_file()
    }

    fn save(&self, suggested: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Save As")
            .add_filter("3DCraft document", &[EXTENSION])
            .set_file_name(suggested)
            .save_file()
    }
}

/// What came out of a dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Open(PathBuf),
    Save(PathBuf),
    /// The dialog was closed without a choice.
    Cancelled(&'static str),
}

/// Shows one dialog at a time on a thread of its own and hands back what
/// was chosen.
pub struct Picker {
    dialogs: Arc<dyn Dialogs>,
    /// Called when a choice is ready, to have it collected.
    wake: Wake,
    waiting: Option<Receiver<Choice>>,
}

impl Picker {
    pub fn new(dialogs: Arc<dyn Dialogs>, wake: Wake) -> Self {
        Self {
            dialogs,
            wake,
            waiting: None,
        }
    }

    /// True while a dialog is open.
    pub fn busy(&self) -> bool {
        self.waiting.is_some()
    }

    fn ask(
        &mut self,
        name: &str,
        ask: impl FnOnce(&dyn Dialogs) -> Choice + Send + 'static,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("a file dialog is already open".to_string());
        }
        let (send, receive) = mpsc::channel();
        let (dialogs, wake) = (Arc::clone(&self.dialogs), Arc::clone(&self.wake));
        std::thread::Builder::new()
            .name(format!("craft3d-dialog-{name}"))
            .spawn(move || {
                // If the dialog panics the channel closes, which `poll`
                // reports; nothing else is lost.
                let choice = ask(dialogs.as_ref());
                let _ = send.send(choice);
                wake();
            })
            .map_err(|e| format!("the file dialog could not be opened: {e}"))?;
        self.waiting = Some(receive);
        Ok(())
    }

    /// Opens the dialog for a file to open. An error when one is open
    /// already or it cannot be shown.
    pub fn ask_open(&mut self) -> Result<(), String> {
        self.ask("open", |dialogs| {
            dialogs
                .open()
                .map_or(Choice::Cancelled("file.open"), Choice::Open)
        })
    }

    /// Opens the dialog for where to save.
    pub fn ask_save(&mut self, suggested: String) -> Result<(), String> {
        self.ask("save", move |dialogs| {
            dialogs
                .save(&suggested)
                .map_or(Choice::Cancelled("file.save"), |path| {
                    Choice::Save(with_extension(path))
                })
        })
    }

    fn settle(&mut self, received: Result<Choice, bool>) -> Option<Result<Choice, String>> {
        match received {
            Ok(choice) => {
                self.waiting = None;
                Some(Ok(choice))
            }
            // Still open.
            Err(false) => None,
            Err(true) => {
                self.waiting = None;
                Some(Err("the file dialog closed unexpectedly".to_string()))
            }
        }
    }

    /// The choice, once the dialog was closed; `None` while it is open or
    /// when none was asked for.
    pub fn poll(&mut self) -> Option<Result<Choice, String>> {
        let received = match self.waiting.as_ref()?.try_recv() {
            Ok(choice) => Ok(choice),
            Err(TryRecvError::Empty) => Err(false),
            Err(TryRecvError::Disconnected) => Err(true),
        };
        self.settle(received)
    }

    /// Like [`Self::poll`], but waits for the dialog to close.
    pub fn wait(&mut self) -> Option<Result<Choice, String>> {
        let received = self.waiting.as_ref()?.recv().map_err(|_| true);
        self.settle(received)
    }
}

/// A file saved without an extension gets the native one: not every
/// dialog adds it.
pub fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        path
    } else {
        path.with_extension(EXTENSION)
    }
}

/// The name a dialog proposes for a document saved for the first time or
/// under another name.
pub fn suggested_name(path: Option<&str>) -> String {
    path.and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| format!("untitled.{EXTENSION}"), str::to_string)
}

/// The grant for one file the person chose: its directory, in one
/// direction. With the path as the command is to be given it.
fn grant(path: &Path, write: bool) -> Result<(Workspace, String), String> {
    let shown = path.display();
    let absolute = std::path::absolute(path)
        .map_err(|e| format!("`{shown}` cannot be used as a file path: {e}"))?;
    let (Some(directory), Some(_)) = (absolute.parent(), absolute.file_name()) else {
        return Err(format!("`{shown}` is not a file path"));
    };
    // The engine takes paths as text.
    let Some(text) = absolute.to_str() else {
        return Err(format!(
            "`{shown}` is not valid Unicode; rename the file or its folder to open or save it"
        ));
    };
    let root = Access::Root(directory.to_path_buf());
    let workspace = if write {
        Workspace {
            read: Access::None,
            write: root,
        }
    } else {
        Workspace {
            read: root,
            write: Access::None,
        }
    };
    Ok((workspace, text.to_string()))
}

impl Host {
    fn report(&mut self, label: &str, outcome: &Result<Value, String>) {
        let (ok, text) = match outcome {
            Ok(result) => (true, result.to_string()),
            Err(error) => (false, error.clone()),
        };
        self.shell.last_command = Some(CommandReport {
            label: label.to_string(),
            ok,
            text: text.chars().take(160).collect(),
        });
    }

    /// Runs a command with the session's own (automation) file access,
    /// guarded like a request is.
    fn run(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        let session = &mut self.session;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.execute(command, params)
        }))
        .unwrap_or_else(|_| {
            Err(CommandError::new(
                "internal error (a bug in 3DCraft, please report it); the document was kept",
            ))
        })
        .map_err(|e| e.message().to_string())
    }

    /// Runs `command` on the file at `path` with the person's own access
    /// to it (see the module's documentation). The automation workspace is
    /// back in place when this returns, whatever happened.
    fn run_as_user(&mut self, command: &str, path: &Path, write: bool) -> Result<Value, String> {
        let (grant, text) = grant(path, write).map_err(|e| format!("{command}: {e}"))?;
        let automation = self.session.workspace().clone();
        self.session.set_workspace(grant);
        let outcome = self.run(command, &json!({ "path": text }));
        self.session.set_workspace(automation);
        outcome
    }

    /// Opens the file the person chose and makes it the active document.
    pub fn open_as_user(&mut self, path: &Path) -> Result<Value, String> {
        let outcome = self.run_as_user("file.open", path, false);
        self.report("file.open", &outcome);
        self.shell.sync(&mut self.session);
        outcome
    }

    /// Saves the active document to the file the person chose, which
    /// becomes its own file.
    pub fn save_as_user(&mut self, path: &Path) -> Result<Value, String> {
        let outcome = self.run_as_user("file.save", path, true);
        self.report("file.save", &outcome);
        self.shell.sync(&mut self.session);
        outcome
    }

    /// Does what the File menu asked for, as far as that goes without a
    /// dialog; opens the dialog otherwise, whose answer goes to
    /// [`Self::file_choice`].
    pub fn file_action(&mut self, action: FileAction, picker: &mut Picker) {
        let asked = match action {
            FileAction::New => {
                let outcome = self.run("file.new", &json!({}));
                self.report("file.new", &outcome);
                self.shell.sync(&mut self.session);
                return;
            }
            FileAction::Open => picker.ask_open().map_err(|e| ("file.open", e)),
            FileAction::Save | FileAction::SaveAs => {
                let Some(status) = self.session.document_status() else {
                    self.report(
                        "file.save",
                        &Err("there is no document to save".to_string()),
                    );
                    return;
                };
                let own = status
                    .path
                    .as_deref()
                    .filter(|_| action == FileAction::Save);
                match own {
                    // Its own file, chosen by the person before (in a
                    // dialog or on the command line): saved there again.
                    Some(path) if Path::new(path).is_absolute() => {
                        let _ = self.save_as_user(Path::new(path));
                        return;
                    }
                    // A path beneath the automation roots, as a request
                    // gave it: saved the way a request would. When that
                    // is refused (no write root), the person chooses.
                    Some(_) => {
                        let outcome = self.run("file.save", &json!({}));
                        if outcome.is_ok() {
                            self.report("file.save", &outcome);
                            self.shell.sync(&mut self.session);
                            return;
                        }
                    }
                    None => {}
                }
                picker
                    .ask_save(suggested_name(status.path.as_deref()))
                    .map_err(|e| ("file.save", e))
            }
        };
        if let Err((label, error)) = asked {
            self.report(label, &Err(error));
        }
    }

    /// Does what the person chose in a dialog.
    pub fn file_choice(&mut self, choice: Result<Choice, String>) {
        match choice {
            Ok(Choice::Open(path)) => {
                let _ = self.open_as_user(&path);
            }
            Ok(Choice::Save(path)) => {
                let _ = self.save_as_user(&path);
            }
            Ok(Choice::Cancelled(label)) => self.report(label, &Err("cancelled".to_string())),
            Err(error) => self.report("file", &Err(error)),
        }
    }

    /// One step of the file actions, between two frames: the answer of a
    /// dialog that was closed, then what the menu asked for. True when
    /// something was done.
    pub fn pump_files(&mut self, picker: &mut Picker) -> bool {
        let mut did = false;
        if let Some(choice) = picker.poll() {
            self.file_choice(choice);
            did = true;
        }
        if let Some(action) = self.shell.take_file_action() {
            self.file_action(action, picker);
            did = true;
        }
        did
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_engine::Session;
    use craft3d_ui_egui::window_title;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An empty directory of this test's own, by its one true name.
    fn workdir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/tmp/craft3d-files-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(dir).unwrap()
    }

    fn subdir(dir: &Path, name: &str) -> PathBuf {
        let sub = dir.join(name);
        std::fs::create_dir_all(&sub).unwrap();
        sub
    }

    fn text(path: &Path) -> &str {
        path.to_str().unwrap()
    }

    /// The names in a directory, sorted.
    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// Writes a document with one block of the given height, as a person
    /// with a file of their own would have it.
    fn write_part(path: &Path, height: f64) {
        let mut s = Session::new(Workspace::unrestricted());
        s.execute("file.new", &json!({})).unwrap();
        s.execute("feature.block", &json!({"size": [10, 10, height]}))
            .unwrap();
        s.execute("file.save", &json!({"path": text(path)}))
            .unwrap();
    }

    /// A measured volume to the millionth: the blocks here have whole
    /// volumes, and the measure is an integral, exact to rounding.
    fn whole(measured: &Value) -> f64 {
        (measured["volume"].as_f64().unwrap() * 1e6).round() / 1e6
    }

    fn volume(host: &mut Host) -> f64 {
        whole(
            &host
                .session
                .execute("document.measure", &json!({}))
                .unwrap(),
        )
    }

    /// The volume of the document in a file, read by a session of its own.
    fn volume_on_disk(path: &Path) -> f64 {
        let mut s = Session::new(Workspace::unrestricted());
        s.execute("file.open", &json!({"path": text(path)}))
            .unwrap();
        whole(&s.execute("document.measure", &json!({})).unwrap())
    }

    /// A request as an agent on the control channel sends it. The error,
    /// or the result.
    fn agent(host: &mut Host, command: &str, params: Value) -> Result<Value, String> {
        let line = json!({"id": 1, "method": "engine.execute",
                          "params": {"command": command, "params": params}})
        .to_string();
        let reply: Value = serde_json::from_str(&host.handle_line(&line).unwrap()).unwrap();
        if reply["ok"] == true {
            Ok(reply["result"].clone())
        } else {
            Err(reply["error"].as_str().unwrap().to_string())
        }
    }

    fn rooted(read: Option<&Path>, write: Option<&Path>) -> Workspace {
        let access =
            |dir: Option<&Path>| dir.map_or(Access::None, |d| Access::Root(d.to_path_buf()));
        Workspace {
            read: access(read),
            write: access(write),
        }
    }

    /// Answers the dialogs for the person, in order, and notes what was
    /// asked.
    #[derive(Default)]
    struct Person {
        answers: Mutex<Vec<Option<PathBuf>>>,
        asked: Mutex<Vec<String>>,
    }

    impl Person {
        fn who_answers(answers: &[Option<&Path>]) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers.iter().map(|a| a.map(Path::to_path_buf)).collect()),
                asked: Mutex::default(),
            })
        }

        fn answer(&self, question: String) -> Option<PathBuf> {
            self.asked.lock().unwrap().push(question);
            let mut answers = self.answers.lock().unwrap();
            assert!(!answers.is_empty(), "a dialog nobody expected");
            answers.remove(0)
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Dialogs for Person {
        fn open(&self) -> Option<PathBuf> {
            self.answer("open".to_string())
        }

        fn save(&self, suggested: &str) -> Option<PathBuf> {
            self.answer(format!("save {suggested}"))
        }
    }

    fn picker(person: &Arc<Person>) -> Picker {
        Picker::new(Arc::clone(person) as Arc<dyn Dialogs>, Arc::new(|| {}))
    }

    /// Does a file action and, when it opened a dialog, waits for the
    /// person and does what they chose.
    fn act(host: &mut Host, picker: &mut Picker, action: FileAction) {
        host.file_action(action, picker);
        if let Some(choice) = picker.wait() {
            host.file_choice(choice);
        }
    }

    fn last(host: &Host) -> (String, bool, String) {
        let report = host.shell.last_command.clone().unwrap();
        (report.label, report.ok, report.text)
    }

    #[test]
    fn the_person_opens_a_file_outside_the_roots_and_a_request_still_cannot() {
        let dir = workdir("open");
        let (mine, shared) = (subdir(&dir, "mine"), subdir(&dir, "shared"));
        let part = mine.join("part.3dcraft");
        write_part(&part, 5.0);
        let on_disk = std::fs::read(&part).unwrap();
        let automation = rooted(Some(&shared), Some(&shared));
        let mut host = Host::new(Session::new(automation.clone()));

        // A request cannot reach the file.
        let refused = agent(&mut host, "file.open", json!({"path": text(&part)})).unwrap_err();
        assert!(refused.contains("outside the read root"), "{refused}");
        assert!(host.session.document_status().is_none());

        // The person can: it is the file they chose.
        let result = host.open_as_user(&part).unwrap();
        assert_eq!(result["features"], 1);
        let status = host.session.document_status().unwrap();
        assert_eq!(status.path.as_deref(), Some(text(&part)));
        assert!(!status.dirty);
        assert_eq!(volume(&mut host), 500.0);
        assert_eq!(host.shell.tree_rows().len(), 1, "the tree shows it already");
        let (label, ok, _) = last(&host);
        assert_eq!((label.as_str(), ok), ("file.open", true));

        // Nothing was widened: the session is under the automation roots
        // exactly as before, and a request is refused exactly as before.
        assert_eq!(*host.session.workspace(), automation);
        let refused = agent(&mut host, "file.open", json!({"path": text(&part)})).unwrap_err();
        assert!(refused.contains("outside the read root"), "{refused}");
        let sibling = mine.join("other.3dcraft");
        for params in [
            json!({}),
            json!({"path": text(&part)}),
            json!({"path": text(&sibling)}),
        ] {
            let refused = agent(&mut host, "file.save", params.clone()).unwrap_err();
            assert!(
                refused.contains("outside the write root"),
                "{params}: {refused}"
            );
        }
        let refused = agent(
            &mut host,
            "file.export",
            json!({"path": text(&mine.join("a.stl"))}),
        )
        .unwrap_err();
        assert!(refused.contains("outside the write root"), "{refused}");
        // A request may edit the document in the window, and save it
        // where it may write.
        agent(
            &mut host,
            "feature.edit",
            json!({"feature": "F1", "size": [10, 10, 7]}),
        )
        .unwrap();
        assert!(host.session.document_status().unwrap().dirty);
        agent(&mut host, "file.save", json!({"path": "copy.3dcraft"})).unwrap();
        assert_eq!(volume_on_disk(&shared.join("copy.3dcraft")), 700.0);
        // The person's file and folder are as they were.
        assert_eq!(std::fs::read(&part).unwrap(), on_disk);
        assert_eq!(listing(&mine), ["part.3dcraft"]);

        // A file that is not a document, and one that is not there: an
        // error for the status bar, and the roots are back in place.
        let junk = mine.join("junk.3dcraft");
        std::fs::write(&junk, b"not a document").unwrap();
        for bad in [junk, mine.join("missing.3dcraft"), mine.clone()] {
            let error = host.open_as_user(&bad).unwrap_err();
            assert!(error.contains("file.open"), "{error}");
            assert_eq!(*host.session.workspace(), automation);
            let (label, ok, text) = last(&host);
            assert_eq!((label.as_str(), ok), ("file.open", false));
            assert!(!text.is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_as_writes_the_chosen_file_and_nothing_else() {
        let dir = workdir("save-as");
        let (mine, elsewhere) = (subdir(&dir, "mine"), subdir(&dir, "elsewhere"));
        // No file access at all for requests.
        let mut host = Host::new(Session::new(Workspace::none()));
        host.session.execute("file.new", &json!({})).unwrap();
        host.session
            .execute("feature.block", &json!({"size": [10, 10, 3]}))
            .unwrap();
        assert!(host.session.document_status().unwrap().dirty);

        let chosen = mine.join("bracket.3dcraft");
        let result = host.save_as_user(&chosen).unwrap();
        assert_eq!(result["path"], text(&chosen));
        // That file, with this document in it; nothing beside it, nothing
        // anywhere else.
        assert_eq!(listing(&mine), ["bracket.3dcraft"]);
        assert!(listing(&elsewhere).is_empty());
        assert_eq!(volume_on_disk(&chosen), 300.0);
        let status = host.session.document_status().unwrap();
        assert_eq!(status.path.as_deref(), Some(text(&chosen)));
        assert!(!status.dirty);
        // The window's title: the file's name, and no mark.
        let title = |host: &Host| window_title(host.session.document_status().as_ref());
        assert_eq!(title(&host), "bracket.3dcraft – 3DCraft");
        let (label, ok, _) = last(&host);
        assert_eq!((label.as_str(), ok), ("file.save", true));
        // Requests are where they were: nowhere.
        assert_eq!(*host.session.workspace(), Workspace::none());
        for command in ["file.save", "file.open"] {
            let refused = agent(&mut host, command, json!({"path": text(&chosen)})).unwrap_err();
            assert!(refused.contains("access"), "{command}: {refused}");
        }
        assert!(agent(&mut host, "file.save", json!({})).is_err());

        // Under another name, in another folder: the first file stays as
        // it is and the document moves to the new one.
        host.session
            .execute(
                "feature.edit",
                &json!({"feature": "F1", "size": [10, 10, 4]}),
            )
            .unwrap();
        assert_eq!(title(&host), "bracket.3dcraft* – 3DCraft", "changed");
        let second = elsewhere.join("bracket-v2.3dcraft");
        host.save_as_user(&second).unwrap();
        assert_eq!(volume_on_disk(&chosen), 300.0);
        assert_eq!(volume_on_disk(&second), 400.0);
        assert_eq!(
            host.session.document_status().unwrap().path.as_deref(),
            Some(text(&second))
        );

        // Where nothing can be saved: an error, and the document is still
        // the one of the second file, changed.
        host.session
            .execute(
                "feature.edit",
                &json!({"feature": "F1", "size": [10, 10, 9]}),
            )
            .unwrap();
        let nowhere = dir.join("no-such-folder").join("x.3dcraft");
        let error = host.save_as_user(&nowhere).unwrap_err();
        assert!(error.contains("file.save"), "{error}");
        assert!(!nowhere.exists());
        let status = host.session.document_status().unwrap();
        assert!(status.dirty);
        assert_eq!(status.path.as_deref(), Some(text(&second)));
        assert_eq!(*host.session.workspace(), Workspace::none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_goes_to_the_documents_own_file_and_asks_only_when_there_is_none() {
        let dir = workdir("save");
        let mine = subdir(&dir, "mine");
        let mut host = Host::new(Session::new(Workspace::none()));

        // Nothing to save: no dialog, an error in the status bar.
        let nobody = Person::who_answers(&[]);
        let mut dialogs = picker(&nobody);
        for action in [FileAction::Save, FileAction::SaveAs] {
            act(&mut host, &mut dialogs, action);
            let (label, ok, text) = last(&host);
            assert_eq!((label.as_str(), ok), ("file.save", false));
            assert!(text.contains("no document"), "{text}");
        }

        // New: a document without a file. Save asks where, proposing a
        // name; the answer has no extension and gets the native one.
        let person = Person::who_answers(&[
            Some(&mine.join("gear")),
            None,
            Some(&mine.join("gear2.3dcraft")),
        ]);
        let mut dialogs = picker(&person);
        act(&mut host, &mut dialogs, FileAction::New);
        assert_eq!(last(&host).0, "file.new");
        host.session
            .execute("feature.block", &json!({"size": [10, 10, 2]}))
            .unwrap();
        act(&mut host, &mut dialogs, FileAction::Save);
        assert_eq!(person.asked(), ["save untitled.3dcraft"]);
        let gear = mine.join("gear.3dcraft");
        assert_eq!(listing(&mine), ["gear.3dcraft"]);
        assert_eq!(volume_on_disk(&gear), 200.0);
        assert!(!host.session.document_status().unwrap().dirty);

        // Now it has a file: Save writes it again without asking.
        host.session
            .execute(
                "feature.edit",
                &json!({"feature": "F1", "size": [10, 10, 6]}),
            )
            .unwrap();
        act(&mut host, &mut dialogs, FileAction::Save);
        assert_eq!(person.asked().len(), 1, "no second dialog");
        assert_eq!(volume_on_disk(&gear), 600.0);
        assert!(!host.session.document_status().unwrap().dirty);
        assert_eq!(listing(&mine), ["gear.3dcraft"]);

        // Save As asks even so, proposing the file's name. Cancelled:
        // nothing is written and the document is as it was.
        host.session
            .execute(
                "feature.edit",
                &json!({"feature": "F1", "size": [10, 10, 8]}),
            )
            .unwrap();
        act(&mut host, &mut dialogs, FileAction::SaveAs);
        assert_eq!(person.asked()[1], "save gear.3dcraft");
        let (label, ok, text) = last(&host);
        assert_eq!(
            (label.as_str(), ok, text.as_str()),
            ("file.save", false, "cancelled")
        );
        assert_eq!(volume_on_disk(&gear), 600.0);
        assert_eq!(listing(&mine), ["gear.3dcraft"]);
        let status = host.session.document_status().unwrap();
        assert!(status.dirty);
        assert_eq!(status.path.as_deref(), Some(text_of(&gear)));
        // And answered: the new file is written, the old one stays.
        act(&mut host, &mut dialogs, FileAction::SaveAs);
        assert_eq!(volume_on_disk(&mine.join("gear2.3dcraft")), 800.0);
        assert_eq!(volume_on_disk(&gear), 600.0);
        assert_eq!(person.asked().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn text_of(path: &Path) -> &str {
        text(path)
    }

    #[test]
    fn open_asks_for_the_file_and_opens_what_was_chosen() {
        let dir = workdir("open-dialog");
        let mine = subdir(&dir, "mine");
        let part = mine.join("part.3dcraft");
        write_part(&part, 5.0);
        let person = Person::who_answers(&[None, Some(&part)]);
        let mut dialogs = picker(&person);
        let mut host = Host::new(Session::new(Workspace::none()));
        // Cancelled: no document.
        act(&mut host, &mut dialogs, FileAction::Open);
        assert_eq!(
            last(&host),
            ("file.open".to_string(), false, "cancelled".to_string())
        );
        assert!(host.session.document_status().is_none());
        // Chosen: opened, through the shell's own request as the menu
        // makes it.
        host.shell.request_file_action(FileAction::Open);
        assert!(host.pump_files(&mut dialogs));
        assert!(dialogs.busy());
        let choice = dialogs.wait().unwrap();
        assert_eq!(choice, Ok(Choice::Open(part.clone())));
        host.file_choice(choice);
        assert_eq!(volume(&mut host), 500.0);
        assert_eq!(person.asked(), ["open", "open"]);
        // Nothing more to do.
        assert!(!host.pump_files(&mut dialogs));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_document_that_lives_under_the_roots_is_saved_there_without_asking() {
        let dir = workdir("rooted");
        let (shared, mine) = (subdir(&dir, "shared"), subdir(&dir, "mine"));
        let mut host = Host::new(Session::new(rooted(Some(&shared), Some(&shared))));
        // A request made and saved it, by a path relative to its root.
        agent(&mut host, "file.new", json!({})).unwrap();
        agent(&mut host, "feature.block", json!({"size": [10, 10, 1]})).unwrap();
        agent(&mut host, "file.save", json!({"path": "made.3dcraft"})).unwrap();
        agent(
            &mut host,
            "feature.edit",
            json!({"feature": "F1", "size": [10, 10, 2]}),
        )
        .unwrap();
        // The person presses Ctrl+S: saved where it lives, no dialog.
        let nobody = Person::who_answers(&[]);
        let mut dialogs = picker(&nobody);
        act(&mut host, &mut dialogs, FileAction::Save);
        assert!(last(&host).1, "{:?}", last(&host));
        assert_eq!(volume_on_disk(&shared.join("made.3dcraft")), 200.0);
        assert!(!host.session.document_status().unwrap().dirty);
        assert!(nobody.asked().is_empty());

        // Where requests may read but not write, that path cannot be
        // saved to by them: the person is asked instead.
        let mut host = Host::new(Session::new(rooted(Some(&shared), None)));
        agent(&mut host, "file.open", json!({"path": "made.3dcraft"})).unwrap();
        agent(
            &mut host,
            "feature.edit",
            json!({"feature": "F1", "size": [10, 10, 3]}),
        )
        .unwrap();
        let person = Person::who_answers(&[Some(&mine.join("kept.3dcraft"))]);
        let mut dialogs = picker(&person);
        act(&mut host, &mut dialogs, FileAction::Save);
        assert_eq!(person.asked(), ["save made.3dcraft"]);
        assert_eq!(volume_on_disk(&mine.join("kept.3dcraft")), 300.0);
        assert_eq!(
            volume_on_disk(&shared.join("made.3dcraft")),
            200.0,
            "untouched"
        );
        assert_eq!(*host.session.workspace(), rooted(Some(&shared), None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_dialog_at_a_time_and_the_window_is_woken_when_it_closes() {
        /// A person who takes their time: the dialog stays open until the
        /// test says so.
        struct Slow {
            close: Mutex<mpsc::Receiver<Option<PathBuf>>>,
        }
        impl Dialogs for Slow {
            fn open(&self) -> Option<PathBuf> {
                self.close.lock().unwrap().recv().unwrap()
            }
            fn save(&self, _: &str) -> Option<PathBuf> {
                panic!("the save dialog fell over")
            }
        }
        let (close, closing) = mpsc::channel();
        let woken = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&woken);
        let mut picker = Picker::new(
            Arc::new(Slow {
                close: Mutex::new(closing),
            }),
            Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert!(!picker.busy() && picker.poll().is_none() && picker.wait().is_none());
        picker.ask_open().unwrap();
        assert!(picker.busy());
        // While it is open: nothing to collect, and no second dialog.
        assert!(picker.poll().is_none());
        let error = picker.ask_open().unwrap_err();
        assert!(error.contains("already open"), "{error}");
        assert!(picker.ask_save("x".to_string()).is_err());
        let mut host = Host::new(Session::new(Workspace::none()));
        host.file_action(FileAction::Open, &mut picker);
        assert!(!last(&host).1);
        assert!(last(&host).2.contains("already open"));
        assert_eq!(woken.load(Ordering::SeqCst), 0);
        // The person chooses.
        close
            .send(Some(PathBuf::from("/somewhere/a.3dcraft")))
            .unwrap();
        assert_eq!(
            picker.wait(),
            Some(Ok(Choice::Open(PathBuf::from("/somewhere/a.3dcraft"))))
        );
        assert!(!picker.busy());
        // A dialog that falls over is an error, and the next one works.
        picker.ask_save("x".to_string()).unwrap();
        let error = picker.wait().unwrap().unwrap_err();
        assert!(error.contains("unexpectedly"), "{error}");
        host.file_choice(Err(error));
        assert!(!last(&host).1);
        picker.ask_open().unwrap();
        close.send(None).unwrap();
        assert_eq!(picker.wait(), Some(Ok(Choice::Cancelled("file.open"))));
        // Woken once per dialog that gave an answer.
        for _ in 0..2000 {
            if woken.load(Ordering::SeqCst) == 2 {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(woken.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_grant_is_one_folder_in_one_direction() {
        let dir = workdir("grant");
        let file = dir.join("a.3dcraft");
        let (workspace, path) = grant(&file, false).unwrap();
        assert_eq!(workspace, rooted(Some(&dir), None));
        assert_eq!(path, text(&file));
        let (workspace, path) = grant(&file, true).unwrap();
        assert_eq!(workspace, rooted(None, Some(&dir)));
        // The engine accepts that path under that grant, and no other
        // folder: not the one above, not one beside it.
        assert_eq!(workspace.resolve_write(&path).unwrap(), file);
        assert!(workspace.resolve_read(&path).is_err());
        assert!(
            workspace
                .resolve_write(text(&dir.parent().unwrap().join("b.3dcraft")))
                .is_err()
        );
        assert!(
            workspace
                .resolve_write(text(&dir.join("../b.3dcraft")))
                .is_err()
        );
        // A relative path is the person's, from where the program runs:
        // made absolute, never left to the roots to place.
        let (workspace, path) = grant(Path::new("rel.3dcraft"), true).unwrap();
        let here = std::env::current_dir().unwrap();
        assert_eq!(path, text(&here.join("rel.3dcraft")));
        assert_eq!(workspace.write, Access::Root(here));
        // No file name, nothing to grant.
        for bad in ["", "/"] {
            assert!(grant(Path::new(bad), true).is_err(), "`{bad}`");
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let odd = dir.join(std::ffi::OsStr::from_bytes(b"caf\xe9.3dcraft"));
            let error = grant(&odd, true).unwrap_err();
            assert!(error.contains("Unicode"), "{error}");
            let mut host = Host::new(Session::new(Workspace::none()));
            assert!(host.open_as_user(&odd).unwrap_err().contains("Unicode"));
            assert_eq!(*host.session.workspace(), Workspace::none());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_for_the_dialog_and_the_extension() {
        assert_eq!(suggested_name(None), "untitled.3dcraft");
        assert_eq!(suggested_name(Some("")), "untitled.3dcraft");
        assert_eq!(
            suggested_name(Some("/home/me/parts/gear.3dcraft")),
            "gear.3dcraft"
        );
        assert_eq!(suggested_name(Some("rel/gear.3dcraft")), "gear.3dcraft");
        assert_eq!(suggested_name(Some("gear")), "gear");
        let ext = |p: &str| with_extension(PathBuf::from(p));
        assert_eq!(ext("/a/gear"), PathBuf::from("/a/gear.3dcraft"));
        assert_eq!(ext("/a/gear.3dcraft"), PathBuf::from("/a/gear.3dcraft"));
        // Another extension is the person's business.
        assert_eq!(ext("/a/gear.bak"), PathBuf::from("/a/gear.bak"));
        assert_eq!(ext("/a.b/gear"), PathBuf::from("/a.b/gear.3dcraft"));
    }
}
