//! The session and the shell of the page, and what the File menu, the
//! tabs and the files the person hands the page do with them. Nothing
//! here touches the browser: the page ([`crate::page`]) asks the person
//! for a file when [`Outcome::ask`] says so, hands the bytes over with
//! [`WebHost::receive`], and offers each [`Download`] to the person. So
//! the whole flow runs in native tests.
//!
//! Where the browser has the File System Access API (Chrome, Edge,
//! Chromebooks) the page asks with the system's file dialogs
//! ([`WebHost::pickers`]): Save, Save As and Export name the file in a
//! save dialog ([`Outcome::pick`]), and a document opened or saved through
//! one is saved back to that file without asking again
//! ([`WebHost::writable`]). Elsewhere a saved or exported file is a
//! download and the dialog over the page ([`NameDialog`]) names it.
//!
//! The session keeps its files in memory (`craft3d_engine::Access::Memory`):
//! a file the person chose is put there under its name, the same command
//! as on the desktop reads it (`file.open`, `file.import`), and what
//! `file.save` and `file.export` write there is taken out again as a
//! download. Nothing stays in memory after an action.

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::files::{
    EXPORT_FORMATS, EXTENSION, export_kind, export_selection, extension_of, guarded,
    suggested_export_name, suggested_name,
};
use craft3d_ui_egui::{CommandReport, DocAction, FileAction, Shell};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// A file for the person to save: its name and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub name: String,
    pub bytes: Vec<u8>,
    /// The document this is the saved file of (its id in the session),
    /// for the page to write it back to the file it came from; `None`
    /// for an export.
    pub document: Option<u64>,
}

/// What the page names a file for with the system's save dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The active document (`document`, its id), saved as a `.3dcraft`
    /// file; with the tab to close once it is saved.
    Save {
        document: u64,
        suggested: String,
        close: Option<usize>,
    },
    /// An export: the format follows the extension of the name chosen.
    Export { suggested: String },
}

impl Pick {
    /// The kinds of file the dialog offers, the one proposed first.
    pub fn types(&self) -> Vec<PickerType> {
        match self {
            Pick::Save { .. } => vec![PickerType {
                description: "3DCraft document",
                mime: "application/x-3dcraft",
                extensions: vec![EXTENSION],
            }],
            Pick::Export { suggested } => export_types(extension_of(suggested).as_deref()),
        }
    }

    /// The name the dialog proposes.
    pub fn suggested(&self) -> &str {
        match self {
            Pick::Save { suggested, .. } | Pick::Export { suggested } => suggested,
        }
    }
}

/// A kind of file in the system's file dialog: a description, a media
/// type and its extensions (without the dot).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerType {
    pub description: &'static str,
    pub mime: &'static str,
    pub extensions: Vec<&'static str>,
}

/// The kinds of file Open and Import choose from.
pub fn open_types(ask: Ask) -> Vec<PickerType> {
    match ask {
        Ask::Open => vec![PickerType {
            description: "3DCraft document",
            mime: "application/x-3dcraft",
            extensions: vec![EXTENSION],
        }],
        Ask::Import => vec![PickerType {
            description: "STEP",
            mime: "model/step",
            extensions: vec!["step", "stp"],
        }],
    }
}

/// The media type the file dialog files a format under (the dialog
/// filters by the extensions; the type only has to be well formed).
fn export_mime(extension: &str) -> &'static str {
    match extension {
        "step" | "stp" => "model/step",
        "stl" => "model/stl",
        "3mf" => "model/3mf",
        "obj" => "model/obj",
        "dxf" => "image/vnd.dxf",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Every export format as a kind of file for the save dialog, the one
/// with extension `first` (the last format exported) first.
pub fn export_types(first: Option<&str>) -> Vec<PickerType> {
    let mut types: Vec<PickerType> = EXPORT_FORMATS
        .iter()
        .map(|(format, extensions)| PickerType {
            description: format,
            mime: export_mime(extensions.first().copied().unwrap_or_default()),
            extensions: extensions.to_vec(),
        })
        .collect();
    if let Some(at) = types
        .iter()
        .position(|t| first.is_some_and(|f| t.extensions.contains(&f)))
    {
        let chosen = types.remove(at);
        types.insert(0, chosen);
    }
    types
}

/// What the page asks the person for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// A document to open (`.3dcraft`).
    Open,
    /// A STEP file to import.
    Import,
}

/// What an action leaves for the page to do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub ask: Option<Ask>,
    /// A file to name with the system's save dialog (with
    /// [`WebHost::pickers`] only).
    pub pick: Option<Pick>,
    pub downloads: Vec<Download>,
}

/// What the dialog over the page names a file for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Save As (or Save of a document without a name): a `.3dcraft`
    /// download. With the document to close once it is saved (Save in
    /// the question before closing a tab).
    Save { close: Option<usize> },
    /// Export: the format follows the extension.
    Export,
}

/// The dialog in which the person names the file, while it is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameDialog {
    pub purpose: Purpose,
    pub name: String,
}

/// The most bytes of a name, as a file the person hands in or names.
const MAX_NAME: usize = 200;

/// A file name from the browser (a dropped or chosen file's, or one the
/// person typed) as a name in the session's memory: its last part, without
/// control characters, at most [`MAX_NAME`] bytes; `None` when nothing is
/// left or it is only dots.
pub fn memory_name(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut clean: String = last.chars().filter(|c| !c.is_control()).collect();
    clean = clean.trim().to_string();
    while clean.len() > MAX_NAME {
        clean.pop();
    }
    (!clean.is_empty() && !clean.chars().all(|c| c == '.')).then_some(clean)
}

/// `name` ending in `.extension`: as it is when it has that extension
/// (in any case), else with it appended, so a document saved as `x.stl`
/// is `x.stl.3dcraft` and never a `.3dcraft` file under another format's
/// name.
fn with_extension(name: &str, extension: &str) -> String {
    if extension_of(name).as_deref() == Some(extension) {
        name.to_string()
    } else {
        format!("{name}.{extension}")
    }
}

/// What a file handed to the page is, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Received {
    /// A `.3dcraft` document: opened.
    Document,
    /// A STEP file: imported into the active document.
    Step,
}

pub fn received_kind(name: &str) -> Option<Received> {
    match extension_of(name)?.as_str() {
        EXTENSION => Some(Received::Document),
        "step" | "stp" => Some(Received::Step),
        _ => None,
    }
}

pub struct WebHost {
    pub session: Session,
    pub shell: Shell,
    /// The extension of the last export, for the next one's name.
    pub export_extension: Option<String>,
    pub dialog: Option<NameDialog>,
    /// True when the page names files with the system's file dialogs
    /// (the File System Access API): Save, Save As and Export then ask
    /// through [`Outcome::pick`], not [`NameDialog`].
    pub pickers: bool,
    /// The documents (by id) whose file the page can write to again: Save
    /// goes there without asking. Only with [`WebHost::pickers`].
    pub writable: BTreeSet<u64>,
}

impl WebHost {
    /// A host whose session keeps its files in memory, around `shell`.
    pub fn new(shell: Shell) -> Self {
        Self {
            session: Session::new(Workspace::memory()),
            shell,
            export_extension: None,
            dialog: None,
            pickers: false,
            writable: BTreeSet::new(),
        }
    }

    /// Shows `text` in the status bar under `label`, as a command's
    /// report.
    pub fn note(&mut self, label: &str, ok: bool, text: String) {
        self.shell.last_command = Some(CommandReport {
            label: label.to_string(),
            ok,
            text,
        });
    }

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

    fn run(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        guarded(&mut self.session, command, params)
    }

    /// Reports `outcome` under `label`, empties the memory and shows what
    /// the session holds now.
    fn finish(&mut self, label: &str, outcome: &Result<Value, String>) {
        self.report(label, outcome);
        self.forget_files();
        self.shell.sync(&mut self.session);
    }

    /// Takes every file out of the session's memory: nothing stays there
    /// after an action.
    fn forget_files(&mut self) -> Vec<Download> {
        let names = self.session.file_names();
        names
            .into_iter()
            .filter_map(|name| {
                let bytes = self.session.take_file(&name)?;
                Some(Download {
                    name,
                    bytes,
                    document: None,
                })
            })
            .collect()
    }

    /// True when a document has changes that are not saved (downloaded).
    pub fn has_unsaved(&self) -> bool {
        self.session
            .document_statuses()
            .iter()
            .any(|status| status.close_asks)
    }

    /// Opens or imports a file the person handed the page (chosen, dropped,
    /// or named in the address): a `.3dcraft` document is opened, a STEP
    /// file imported into the active document (a new one when there is
    /// none, closed again when the file cannot be imported). `wanted` is
    /// what the person asked for, when they asked (Open or Import): a file
    /// of the other kind is refused then.
    pub fn receive(
        &mut self,
        name: &str,
        bytes: Vec<u8>,
        wanted: Option<Ask>,
    ) -> Result<Value, String> {
        let outcome = self.receive_inner(name, bytes, wanted);
        let label = match received_kind(name) {
            Some(Received::Step) => "file.import",
            _ => "file.open",
        };
        self.finish(label, &outcome);
        outcome
    }

    fn receive_inner(
        &mut self,
        name: &str,
        bytes: Vec<u8>,
        wanted: Option<Ask>,
    ) -> Result<Value, String> {
        let Some(name) = memory_name(name) else {
            return Err(format!("`{name}` is not a file name"));
        };
        let kind = received_kind(&name).ok_or_else(|| {
            format!("`{name}` is not a 3DCraft document (.3dcraft) or a STEP file (.step, .stp)")
        })?;
        match (wanted, kind) {
            (Some(Ask::Open), Received::Step) => {
                return Err(format!(
                    "`{name}` is a STEP file; import it (File, Import…) to add its solids to a document"
                ));
            }
            (Some(Ask::Import), Received::Document) => {
                return Err(format!(
                    "`{name}` is a 3DCraft document; open it (File, Open…)"
                ));
            }
            _ => {}
        }
        self.session
            .put_file(&name, bytes)
            .map_err(|e| e.message().to_string())?;
        match kind {
            Received::Document => self.run("file.open", &json!({ "path": name })),
            Received::Step => {
                let made = if self.session.document_status().is_none() {
                    self.run("file.new", &json!({}))
                        .map_err(|e| format!("file.new: {e}"))?;
                    true
                } else {
                    false
                };
                let imported = self.run("file.import", &json!({ "path": name }));
                if imported.is_err() && made {
                    let _ = self.run("file.close", &json!({ "discard": true }));
                }
                imported
            }
        }
    }

    /// Does what the File menu, its shortcuts and the tabs asked for since
    /// the last frame.
    pub fn pump(&mut self) -> Outcome {
        let mut outcome = Outcome::default();
        if let Some(action) = self.shell.take_file_action() {
            outcome = self.file_action(action);
        }
        if let Some(action) = self.shell.take_doc_action() {
            let more = self.doc_action(action);
            outcome.downloads.extend(more.downloads);
            outcome.ask = outcome.ask.or(more.ask);
            outcome.pick = outcome.pick.or(more.pick);
        }
        outcome
    }

    /// Does what the File menu asked for, as far as that goes without the
    /// person: a file to choose is asked of the page, a name of the
    /// dialog.
    pub fn file_action(&mut self, action: FileAction) -> Outcome {
        let mut outcome = Outcome::default();
        match action {
            FileAction::New => {
                let made = self.run("file.new", &json!({}));
                self.finish("file.new", &made);
            }
            FileAction::Open => outcome.ask = Some(Ask::Open),
            FileAction::Import => outcome.ask = Some(Ask::Import),
            FileAction::Save | FileAction::SaveAs => {
                let Some(status) = self.session.document_status() else {
                    self.report(
                        "file.save",
                        &Err("there is no document to save".to_string()),
                    );
                    return outcome;
                };
                // Save goes to the file the document has without asking:
                // a download of it again, or (with the file dialogs) the
                // file it was opened from or saved to, when the page can
                // write there.
                let own = status.path.clone().filter(|_| {
                    action == FileAction::Save
                        && (!self.pickers || self.writable.contains(&status.id))
                });
                match own {
                    Some(own) => outcome.downloads = self.save_as(&own, None),
                    None if self.pickers => {
                        outcome.pick = Some(Pick::Save {
                            document: status.id,
                            suggested: suggested_name(status.path.as_deref()),
                            close: None,
                        });
                    }
                    None => {
                        self.dialog = Some(NameDialog {
                            purpose: Purpose::Save { close: None },
                            name: suggested_name(status.path.as_deref()),
                        });
                    }
                }
            }
            FileAction::Export => {
                let Some(status) = self.session.document_status() else {
                    self.report(
                        "file.export",
                        &Err("there is no document to export".to_string()),
                    );
                    return outcome;
                };
                let name =
                    suggested_export_name(status.path.as_deref(), self.export_extension.as_deref());
                if self.pickers {
                    outcome.pick = Some(Pick::Export { suggested: name });
                } else {
                    self.dialog = Some(NameDialog {
                        purpose: Purpose::Export,
                        name,
                    });
                }
            }
        }
        outcome
    }

    /// What a tab asked for: show a document, close it, or save it (a
    /// download) and close it.
    pub fn doc_action(&mut self, action: DocAction) -> Outcome {
        let mut outcome = Outcome::default();
        match action {
            DocAction::Activate(document) => {
                let done = self.run("file.activate", &json!({ "document": document }));
                self.finish("file.activate", &done);
            }
            DocAction::Close { document, discard } => {
                let done = self.run(
                    "file.close",
                    &json!({ "document": document, "discard": discard }),
                );
                self.finish("file.close", &done);
            }
            DocAction::SaveAndClose(document) => {
                let shown = self.run("file.activate", &json!({ "document": document }));
                if let Err(e) = shown {
                    self.finish("file.activate", &Err(e));
                    return outcome;
                }
                let Some(status) = self.session.document_status() else {
                    return outcome;
                };
                let own = status
                    .path
                    .clone()
                    .filter(|_| !self.pickers || self.writable.contains(&status.id));
                match own {
                    Some(own) => outcome.downloads = self.save_as(&own, Some(document)),
                    None if self.pickers => {
                        self.shell.sync(&mut self.session);
                        outcome.pick = Some(Pick::Save {
                            document: status.id,
                            suggested: suggested_name(status.path.as_deref()),
                            close: Some(document),
                        });
                    }
                    None => {
                        self.shell.sync(&mut self.session);
                        self.dialog = Some(NameDialog {
                            purpose: Purpose::Save {
                                close: Some(document),
                            },
                            name: suggested_name(None),
                        });
                    }
                }
            }
        }
        outcome
    }

    /// Saves the active document as `name` (`.3dcraft` added when it has no
    /// extension), which becomes its name: the file is the download. With
    /// `close`, the document is closed once it is saved.
    pub fn save_as(&mut self, name: &str, close: Option<usize>) -> Vec<Download> {
        let Some(name) = memory_name(name).map(|n| with_extension(&n, EXTENSION)) else {
            self.finish("file.save", &Err(format!("`{name}` is not a file name")));
            return Vec::new();
        };
        let id = self.session.document_status().map(|s| s.id);
        let saved = self.run("file.save", &json!({ "path": name, "overwrite": true }));
        let file = self.session.take_file(&name);
        let mut downloads = Vec::new();
        let outcome = match (saved, file) {
            (Ok(result), Some(bytes)) => {
                downloads.push(Download {
                    name: name.clone(),
                    bytes,
                    document: id,
                });
                match close {
                    Some(_) => {
                        let index = self.session.document_status().map(|s| s.index);
                        self.run("file.close", &json!({ "document": index }))
                            .map(|closed| json!({ "saved": result, "closed": closed }))
                    }
                    None => Ok(result),
                }
            }
            (Ok(_), None) => Err(format!("file.save: `{name}` was not written")),
            (Err(e), _) => Err(e),
        };
        self.finish("file.save", &outcome);
        downloads
    }

    /// Exports to a file named `name`, in the format its extension names:
    /// every body, or the bodies, faces or sketch selected as far as
    /// `file.export` can take them. The file (and an OBJ's material
    /// library) are the downloads.
    pub fn export_as(&mut self, name: &str) -> Vec<Download> {
        let Some(name) = memory_name(name) else {
            self.finish("file.export", &Err(format!("`{name}` is not a file name")));
            return Vec::new();
        };
        if export_kind(&name).is_some() {
            self.export_extension = extension_of(&name);
        }
        let exported = export_selection(&mut self.session, &name).and_then(|extra| {
            let mut params = json!({ "path": name });
            if let (Some((key, value)), Some(map)) = (extra, params.as_object_mut()) {
                map.insert(key.to_string(), value);
            }
            self.run("file.export", &params)
        });
        let mut downloads = Vec::new();
        if let Ok(result) = &exported {
            if let Some(bytes) = self.session.take_file(&name) {
                downloads.push(Download {
                    name: name.clone(),
                    bytes,
                    document: None,
                });
            }
            if let Some(library) = result.get("mtl").and_then(Value::as_str)
                && let Some(bytes) = self.session.take_file(library)
            {
                downloads.push(Download {
                    name: library.to_string(),
                    bytes,
                    document: None,
                });
            }
        }
        self.finish("file.export", &exported);
        downloads
    }

    /// The dialog's answer: the file named, or nothing when it was
    /// cancelled.
    pub fn answer(&mut self, dialog: NameDialog, confirmed: bool) -> Vec<Download> {
        if !confirmed {
            let label = match dialog.purpose {
                Purpose::Save { .. } => "file.save",
                Purpose::Export => "file.export",
            };
            self.report(label, &Err("cancelled".to_string()));
            return Vec::new();
        }
        match dialog.purpose {
            Purpose::Save { close } => self.save_as(&dialog.name, close),
            Purpose::Export => self.export_as(&dialog.name),
        }
    }

    /// Draws the dialog in which the person names a file, while it is open;
    /// the downloads of its answer.
    pub fn dialog_ui(&mut self, ctx: &egui::Context) -> Vec<Download> {
        let Some(mut dialog) = self.dialog.take() else {
            return Vec::new();
        };
        let mut answer = None;
        let title = match dialog.purpose {
            Purpose::Save { .. } => "Save As",
            Purpose::Export => "Export",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                if dialog.purpose == Purpose::Export {
                    ui.label("Format (the file's extension):");
                    ui.horizontal_wrapped(|ui| {
                        for (format, extensions) in EXPORT_FORMATS {
                            let extension = extensions.first().copied().unwrap_or_default();
                            let current = extension_of(&dialog.name);
                            let chosen =
                                current.as_deref().is_some_and(|e| extensions.contains(&e));
                            if ui.selectable_label(chosen, format).clicked() {
                                dialog.name = renamed(&dialog.name, extension);
                            }
                        }
                    });
                    if matches!(extension_of(&dialog.name).as_deref(), Some("dxf" | "svg")) {
                        ui.label(
                            "DXF and SVG hold the outline of the selected flat faces or sketch.",
                        );
                    }
                }
                ui.label("File name:");
                let field = ui.text_edit_singleline(&mut dialog.name);
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    answer = Some(true);
                }
                ui.horizontal(|ui| {
                    let verb = match dialog.purpose {
                        Purpose::Save { .. } => "Save",
                        Purpose::Export => "Export",
                    };
                    if ui.button(verb).clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(false);
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    answer = Some(false);
                }
            });
        match answer {
            Some(confirmed) => self.answer(dialog, confirmed),
            None => {
                self.dialog = Some(dialog);
                Vec::new()
            }
        }
    }
}

/// `name` with its extension replaced by `extension` (added when it has
/// none).
fn renamed(name: &str, extension: &str) -> String {
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("untitled");
    format!("{stem}.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> WebHost {
        WebHost::new(Shell::new())
    }

    fn modelled() -> WebHost {
        let mut host = host();
        host.session
            .execute("file.new", &json!({"units": "mm"}))
            .unwrap();
        host.session
            .execute(
                "feature.block",
                &json!({"size": [40, 20, 10], "name": "Plate"}),
            )
            .unwrap();
        host.session
            .execute(
                "body.appearance",
                &json!({"body": "Plate", "color": "#2060c0"}),
            )
            .unwrap();
        host
    }

    #[test]
    fn names_from_the_browser_become_names_in_memory() {
        assert_eq!(
            memory_name("C:\\Users\\a\\part.3dcraft").as_deref(),
            Some("part.3dcraft")
        );
        assert_eq!(memory_name("dir/a b.step").as_deref(), Some("a b.step"));
        assert_eq!(memory_name("x\u{0}y.stl").as_deref(), Some("xy.stl"));
        for bad in ["", "  ", "..", ".", "a/", "a/.."] {
            assert_eq!(memory_name(bad), None, "{bad:?}");
        }
        assert!(memory_name(&"a".repeat(1000)).is_some_and(|n| n.len() <= MAX_NAME));
        assert_eq!(received_kind("A.3DCRAFT"), Some(Received::Document));
        assert_eq!(received_kind("a.STP"), Some(Received::Step));
        assert_eq!(received_kind("a.stl"), None);
        assert_eq!(renamed("a.step", "stl"), "a.stl");
        assert_eq!(with_extension("a", EXTENSION), "a.3dcraft");
        assert_eq!(with_extension("a.3DCRAFT", EXTENSION), "a.3DCRAFT");
        assert_eq!(with_extension("x.stl", EXTENSION), "x.stl.3dcraft");
        assert_eq!(renamed("", "obj"), "untitled.obj");
    }

    #[test]
    fn save_downloads_the_document_and_open_reads_it_back() {
        let mut first = modelled();
        // A document without a name asks for one.
        assert!(first.file_action(FileAction::Save).downloads.is_empty());
        let dialog = first.dialog.take().unwrap();
        assert_eq!(dialog.name, "untitled.3dcraft");
        let downloads = first.answer(
            NameDialog {
                name: "plate".to_string(),
                ..dialog
            },
            true,
        );
        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].name, "plate.3dcraft");
        assert!(!first.has_unsaved());
        // Saved under another format's name, it stays a .3dcraft file.
        let renamed = first.save_as("plate.stl", None);
        assert_eq!(renamed[0].name, "plate.stl.3dcraft");
        assert!(renamed[0].bytes.starts_with(b"PK"));
        first.save_as("plate.3dcraft", None);
        // Saved once, Save goes to the same name without asking.
        let again = first.file_action(FileAction::Save).downloads;
        assert_eq!(again, downloads);
        assert!(first.session.file_names().is_empty());

        let mut second = host();
        let opened = second
            .receive("plate.3dcraft", downloads[0].bytes.clone(), Some(Ask::Open))
            .unwrap();
        assert!(opened["features"].as_u64() >= Some(1), "{opened}");
        assert!(!second.shell.tree_rows().is_empty());
        assert!(second.session.file_names().is_empty());
        // A file of the other kind than asked for is refused.
        assert!(second.receive("a.step", vec![], Some(Ask::Open)).is_err());
        assert!(
            second
                .receive("a.3dcraft", vec![], Some(Ask::Import))
                .is_err()
        );
        assert!(second.receive("a.txt", vec![1], None).is_err());
        assert!(
            second
                .receive("junk.3dcraft", b"junk".to_vec(), None)
                .is_err()
        );
        assert_eq!(second.session.document_statuses().len(), 1);
    }

    #[test]
    fn export_downloads_every_format_and_an_obj_its_library() {
        let mut host = modelled();
        for (format, extensions) in EXPORT_FORMATS {
            let extension = extensions[0];
            let downloads = host.export_as(&format!("plate.{extension}"));
            match extension {
                // Outlines need a selected face or sketch.
                "dxf" | "svg" => {
                    assert!(downloads.is_empty(), "{format}");
                    let report = host.shell.last_command.as_ref().unwrap();
                    assert!(
                        !report.ok && report.text.contains("select"),
                        "{}",
                        report.text
                    );
                }
                "obj" => {
                    let names: Vec<&str> = downloads.iter().map(|d| d.name.as_str()).collect();
                    assert_eq!(names, ["plate.obj", "plate.mtl"]);
                }
                _ => {
                    assert_eq!(downloads.len(), 1, "{format}");
                    assert!(!downloads[0].bytes.is_empty());
                }
            }
            assert!(host.session.file_names().is_empty(), "{format}");
        }
        // The next export proposes the last format tried (SVG).
        host.file_action(FileAction::Export);
        assert_eq!(host.dialog.as_ref().unwrap().name, "untitled.svg");
        let cancelled = host.answer(host.dialog.clone().unwrap(), false);
        assert!(cancelled.is_empty());
    }

    #[test]
    fn a_step_file_is_imported_into_a_new_document() {
        let mut source = modelled();
        let step = source.export_as("plate.step").remove(0).bytes;
        let mut host = host();
        let imported = host.receive("plate.step", step, Some(Ask::Import)).unwrap();
        assert_eq!(imported["bodies"].as_array().map(Vec::len), Some(1));
        assert_eq!(host.session.document_statuses().len(), 1);
        // A STEP file that does not read leaves no document behind.
        let mut empty = WebHost::new(Shell::new());
        assert!(empty.receive("bad.step", b"junk".to_vec(), None).is_err());
        assert!(empty.session.document_statuses().is_empty());
    }

    #[test]
    fn tabs_save_and_close_through_the_dialog() {
        let mut host = modelled();
        assert!(host.has_unsaved());
        let outcome = host.doc_action(DocAction::SaveAndClose(0));
        assert!(outcome.downloads.is_empty());
        let dialog = host.dialog.take().unwrap();
        assert_eq!(dialog.purpose, Purpose::Save { close: Some(0) });
        let downloads = host.answer(dialog, true);
        assert_eq!(downloads.len(), 1);
        assert!(host.session.document_statuses().is_empty());
        // Without a document, Save and Export say so.
        host.file_action(FileAction::Export);
        assert!(host.dialog.is_none());
        assert!(!host.shell.last_command.as_ref().unwrap().ok);
        assert_eq!(host.file_action(FileAction::Open).ask, Some(Ask::Open));
        assert_eq!(host.file_action(FileAction::Import).ask, Some(Ask::Import));
    }

    #[test]
    fn with_file_dialogs_save_asks_once_and_then_writes_back() {
        let mut host = modelled();
        host.pickers = true;
        let id = host.session.document_status().unwrap().id;
        // The first Save names the file in the system's dialog, not ours.
        let outcome = host.file_action(FileAction::Save);
        assert!(host.dialog.is_none() && outcome.downloads.is_empty());
        let pick = outcome.pick.unwrap();
        assert_eq!(
            pick,
            Pick::Save {
                document: id,
                suggested: "untitled.3dcraft".to_string(),
                close: None
            }
        );
        assert_eq!(pick.types()[0].extensions, [EXTENSION]);
        // The page saves under the name chosen and can write there again.
        let saved = host.save_as("plate.3dcraft", None);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].document, Some(id));
        host.writable.insert(id);
        // Save now writes back without asking; Save As still asks.
        let again = host.file_action(FileAction::Save);
        assert!(again.pick.is_none());
        assert_eq!(again.downloads[0].name, "plate.3dcraft");
        assert_eq!(again.downloads[0].document, Some(id));
        assert!(host.file_action(FileAction::SaveAs).pick.is_some());
        // A document with a name the page cannot write to (dropped, or
        // opened from the address) asks where to save.
        host.writable.clear();
        let asked = host.file_action(FileAction::Save).pick.unwrap();
        assert_eq!(asked.suggested(), "plate.3dcraft");
        // Closing a tab with Save asks too, with the tab to close.
        let closing = host.doc_action(DocAction::SaveAndClose(0)).pick.unwrap();
        assert!(matches!(closing, Pick::Save { close: Some(0), .. }));
        assert!(host.dialog.is_none());
    }

    #[test]
    fn with_file_dialogs_export_offers_every_format_last_one_first() {
        let mut host = modelled();
        host.pickers = true;
        host.export_extension = Some("stl".to_string());
        let pick = host.file_action(FileAction::Export).pick.unwrap();
        assert_eq!(pick.suggested(), "untitled.stl");
        let types = pick.types();
        assert_eq!(types.len(), EXPORT_FORMATS.len());
        assert_eq!(types[0].extensions, ["stl"]);
        assert_eq!(types[1].extensions, ["step", "stp"]);
        for kind in &types {
            assert!(kind.mime.contains('/'), "{kind:?}");
        }
        // The name chosen decides the format; exports are not saves.
        let downloads = host.export_as("plate.obj");
        assert!(downloads.iter().all(|d| d.document.is_none()));
        assert_eq!(downloads.len(), 2);
        assert_eq!(export_types(None)[0].extensions, ["step", "stp"]);
        assert_eq!(export_types(Some("svg"))[0].description, "SVG");
        assert_eq!(open_types(Ask::Import)[0].extensions, ["step", "stp"]);
        assert_eq!(open_types(Ask::Open)[0].extensions, [EXTENSION]);
    }
}
