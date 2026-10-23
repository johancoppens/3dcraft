//! The files opened or saved lately: in the File menu and on the start
//! screen, newest first, at most [`MAX_RECENT`].
//!
//! The shell notes a path whenever a document shows up with it (opened or
//! saved, from the menu, the command line or an agent): it follows the
//! open documents, not the person's clicks. A path beneath an automation
//! root is kept absolute, so the list means the same file to the menu. The
//! host keeps the list between runs ([`RecentFiles::from_json`],
//! [`Shell::take_recent_changed`]); what it reads back is hostile input,
//! and anything that does not read is left out. In the browser there are
//! no paths and the list stays empty.

use super::Shell;
use crate::theme::color;
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Most files the list keeps.
pub const MAX_RECENT: usize = 10;
/// Most bytes of a stored list read.
const MAX_STORED: usize = 64 * 1024;
/// Longest path kept, in bytes.
const MAX_PATH: usize = 4096;

/// The list itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentFiles {
    paths: Vec<String>,
}

impl RecentFiles {
    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// Puts `path` first (once).
    pub fn note(&mut self, path: &str) -> bool {
        if path.is_empty() || path.len() > MAX_PATH || path.contains('\0') {
            return false;
        }
        if self.paths.first().map(String::as_str) == Some(path) {
            return false;
        }
        self.paths.retain(|kept| kept != path);
        self.paths.insert(0, path.to_string());
        self.paths.truncate(MAX_RECENT);
        true
    }

    pub fn clear(&mut self) -> bool {
        let had = !self.paths.is_empty();
        self.paths.clear();
        had
    }

    /// As the host stores it: `{"recent": [path, ...]}`.
    pub fn to_json(&self) -> String {
        json!({ "recent": self.paths }).to_string()
    }

    /// A stored list. What does not read (too long, not JSON, not a list
    /// of paths) gives what does: an empty list at worst, never an error.
    pub fn from_json(text: &str) -> Self {
        let mut list = Self::default();
        if text.len() > MAX_STORED {
            return list;
        }
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
            return list;
        };
        let Some(Value::Array(items)) = map.get("recent") else {
            return list;
        };
        // Oldest first, so the newest ends up in front.
        for item in items.iter().take(MAX_RECENT).rev() {
            if let Value::String(path) = item {
                list.note(path);
            }
        }
        list
    }
}

/// One entry as the menu and `ui.get` show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentEntry {
    pub path: String,
    /// The file's name, without its folders.
    pub name: String,
    /// False when there is no such file any more: shown, but not offered.
    pub exists: bool,
}

/// The shell's part: the list, and what follows from it.
#[derive(Default)]
pub(crate) struct RecentMemory {
    list: RecentFiles,
    /// Changed since the host last took it.
    changed: bool,
    /// The paths of the documents open at the last sync.
    open: BTreeSet<String>,
    /// A recent file chosen, until the host opens it.
    pub(super) chosen: Option<String>,
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
        .to_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn exists(path: &str) -> bool {
    std::path::Path::new(path).is_file()
}

#[cfg(target_arch = "wasm32")]
fn exists(_path: &str) -> bool {
    false
}

/// The path of a document as a file on disk: as it is when absolute, else
/// beneath the session's read root (or write root). `None` for a document
/// in memory or a path that names no file.
#[cfg(not(target_arch = "wasm32"))]
fn on_disk(session: &Session, path: &str) -> Option<String> {
    use craft3d_engine::Access;
    let given = std::path::Path::new(path);
    if given.is_absolute() {
        return Some(path.to_string());
    }
    let workspace = session.workspace();
    let resolved =
        [&workspace.read, &workspace.write]
            .into_iter()
            .find_map(|access| match access {
                Access::Root(root) => Some(root.join(given)),
                Access::Unrestricted => Some(given.to_path_buf()),
                Access::None | Access::Memory => None,
            })?;
    let absolute = std::path::absolute(resolved).ok()?;
    absolute.to_str().map(str::to_string)
}

#[cfg(target_arch = "wasm32")]
fn on_disk(_session: &Session, _path: &str) -> Option<String> {
    None
}

impl Shell {
    /// The recent files, newest first, with whether each is still there.
    pub fn recent_files(&self) -> Vec<RecentEntry> {
        self.recent
            .list
            .paths()
            .iter()
            .map(|path| RecentEntry {
                path: path.clone(),
                name: file_name(path),
                exists: exists(path),
            })
            .collect()
    }

    /// The list as `ui.get` gives it.
    pub fn recent_json(&self) -> Value {
        Value::Array(
            self.recent_files()
                .into_iter()
                .map(
                    |entry| json!({"path": entry.path, "name": entry.name, "exists": entry.exists}),
                )
                .collect(),
        )
    }

    /// Starts with a list the host kept; what the window noted before is
    /// put in front of it.
    pub fn set_recent_files(&mut self, kept: RecentFiles) {
        let noted = std::mem::replace(&mut self.recent.list, kept);
        for path in noted.paths().iter().rev() {
            self.recent.list.note(path);
        }
    }

    /// The list, as JSON to store, when it changed since the last call.
    pub fn take_recent_changed(&mut self) -> Option<String> {
        std::mem::take(&mut self.recent.changed).then(|| self.recent.list.to_json())
    }

    /// The recent file chosen in the menu or on the start screen, for the
    /// host to open (`file.open`, with the person's own access to it).
    pub fn take_open_recent(&mut self) -> Option<String> {
        self.recent.chosen.take()
    }

    /// Empties the list (File, Recent files, Clear recent).
    pub fn clear_recent_files(&mut self) {
        self.recent.changed |= self.recent.list.clear();
    }

    pub(crate) fn choose_recent(&mut self, path: String) {
        self.recent.chosen = Some(path);
    }

    /// Notes the documents that came with a path since the last sync:
    /// opened, or saved under a new name. The active one goes first.
    pub(super) fn note_recent(&mut self, session: &Session) {
        let statuses = session.document_statuses();
        let active = session.document_status().map(|status| status.index);
        let mut open = BTreeSet::new();
        let mut new = Vec::new();
        for status in &statuses {
            let Some(path) = status.path.as_deref() else {
                continue;
            };
            if !self.recent.open.contains(path) {
                new.push((Some(status.index) == active, path.to_string()));
            }
            open.insert(path.to_string());
        }
        // Inactive ones first, so the active one ends up in front.
        new.sort_by_key(|(active, _)| *active);
        for (_, path) in new {
            if let Some(full) = on_disk(session, &path) {
                self.recent.changed |= self.recent.list.note(&full);
            }
        }
        self.recent.open = open;
    }

    /// File, Recent files: the list, and Clear. Not in the browser.
    pub(super) fn recent_menu(&mut self, ui: &mut egui::Ui) {
        if cfg!(target_arch = "wasm32") {
            return;
        }
        let entries = self.recent_files();
        ui.menu_button("Recent files", |ui| {
            if entries.is_empty() {
                ui.add_enabled(false, egui::Button::new("No recent files"));
                return;
            }
            for entry in &entries {
                let button = egui::Button::new(&entry.name);
                let response = ui.add_enabled(entry.exists, button);
                let hover = if entry.exists {
                    entry.path.clone()
                } else {
                    format!("{} (not there any more)", entry.path)
                };
                if response.on_hover_text(hover).clicked() {
                    self.choose_recent(entry.path.clone());
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Clear recent").clicked() {
                self.clear_recent_files();
                ui.close();
            }
        });
    }

    /// The recent files on the start screen: a link each, a missing one
    /// dimmed.
    pub(super) fn recent_links(&mut self, ui: &mut egui::Ui) {
        let dim = color(self.tokens.text_dim);
        let entries = self.recent_files();
        if entries.is_empty() {
            ui.colored_label(dim, "Files you open or save show up here.");
            return;
        }
        for entry in entries.iter().take(6) {
            if entry.exists {
                if ui.link(&entry.name).on_hover_text(&entry.path).clicked() {
                    self.choose_recent(entry.path.clone());
                }
            } else {
                ui.colored_label(dim, &entry.name)
                    .on_hover_text(format!("{} (not there any more)", entry.path));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_once_and_at_most_ten() {
        let mut list = RecentFiles::default();
        for i in 0..14 {
            list.note(&format!("/w/{i}.3dcraft"));
        }
        list.note("/w/12.3dcraft");
        assert_eq!(list.paths().len(), MAX_RECENT);
        assert_eq!(list.paths()[0], "/w/12.3dcraft");
        assert_eq!(list.paths()[1], "/w/13.3dcraft");
        assert!(!list.paths().contains(&"/w/0.3dcraft".to_string()));
        assert!(!list.note(""));
        assert!(!list.note("/w/12.3dcraft"), "already first: no change");
    }

    #[test]
    fn a_stored_list_reads_back_and_a_corrupt_one_is_empty() {
        let mut list = RecentFiles::default();
        list.note("/a.3dcraft");
        list.note("/b.3dcraft");
        assert_eq!(RecentFiles::from_json(&list.to_json()), list);
        for bad in [
            "",
            "{",
            "[]",
            "null",
            r#"{"recent": 5}"#,
            r#"{"recent": "x"}"#,
            "\u{0}\u{1}garbage",
        ] {
            assert_eq!(RecentFiles::from_json(bad), RecentFiles::default(), "{bad}");
        }
        let mixed =
            RecentFiles::from_json(r#"{"recent": [1, "/x.3dcraft", null, "", "/x.3dcraft"]}"#);
        assert_eq!(mixed.paths(), ["/x.3dcraft".to_string()]);
        let huge = format!(r#"{{"recent": ["{}"]}}"#, "a".repeat(MAX_STORED));
        assert_eq!(RecentFiles::from_json(&huge), RecentFiles::default());
    }

    #[test]
    fn names_without_their_folders() {
        assert_eq!(file_name("/home/me/part.3dcraft"), "part.3dcraft");
        assert_eq!(file_name(r"C:\work\part.3dcraft"), "part.3dcraft");
        assert_eq!(file_name("part.3dcraft"), "part.3dcraft");
    }
}
