//! The files opened or saved lately: in the File menu and on the start
//! screen, newest first, at most [`MAX_RECENT`].
//!
//! The shell notes a file whenever a document shows up with it (opened or
//! saved, from the menu, the command line or an agent): it follows the
//! open documents, not the person's clicks. It keeps the file on disk the
//! engine reports, absolute, whichever root a request named it beneath.
//! `ui.get` tells an agent only of the files beneath the session's roots. The
//! host keeps the list between runs ([`RecentFiles::from_json`],
//! [`Shell::take_recent_changed`]); what it reads back is hostile input,
//! and anything that does not read is left out. In the browser there are
//! no paths and the list stays empty.

use super::Shell;
use crate::theme::color;
use craft3d_engine::{Access, Session};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

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
    /// When the file was last changed, in seconds since the Unix epoch
    /// (none in the browser or when the file system does not say).
    pub modified: Option<u64>,
}

/// The shell's part: the list, and what follows from it.
#[derive(Default)]
pub(crate) struct RecentMemory {
    list: RecentFiles,
    /// Changed since the host last took it.
    changed: bool,
    /// The files of the documents open at the last sync.
    open: BTreeSet<String>,
    /// A recent file chosen, until the host opens it.
    pub(super) chosen: Option<String>,
    /// The entries with whether each file is there, as last looked at:
    /// `None` until the list changes are looked at again. Looked at when
    /// the list changes, when the start screen comes up and when the
    /// menu opens, not every frame.
    entries: Option<Vec<RecentEntry>>,
    /// The frame the Recent files menu was last drawn in.
    menu_pass: Option<u64>,
    /// The session's roots as of the last sync, absolute: what `ui.get`
    /// may tell an agent about. `None` for unrestricted access.
    roots: Option<Vec<PathBuf>>,
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

#[cfg(not(target_arch = "wasm32"))]
fn modified(path: &str) -> Option<u64> {
    let at = std::fs::metadata(path).ok()?.modified().ok()?;
    // native clock: a file's time, read off the browser only (cfg above).
    let since = at.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(since.as_secs())
}

#[cfg(target_arch = "wasm32")]
fn modified(_path: &str) -> Option<u64> {
    None
}

/// How long ago `at` was at `now` (seconds since the epoch), as the start
/// screen's recent files say it: "2 min ago", "1 h ago", "yesterday",
/// "4 days ago"; empty without a time.
pub fn ago(at: Option<u64>, now: Option<u64>) -> String {
    let (Some(at), Some(now)) = (at, now) else {
        return String::new();
    };
    let age = now.saturating_sub(at);
    match age {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", age / 60),
        3600..86_400 => format!("{} h ago", age / 3600),
        86_400..172_800 => "yesterday".to_string(),
        _ => format!("{} days ago", age / 86_400),
    }
}

/// The line icon of a file's kind of document, by its extension.
pub fn file_glyph(path: &str) -> crate::kit::Glyph {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".3dcraftasm") {
        crate::kit::Glyph::Assembly
    } else if lower.ends_with(".3dcraftjig") {
        crate::kit::Glyph::Jig
    } else {
        crate::kit::Glyph::Part
    }
}

/// The roots of `session`'s workspace, absolute; `None` when one way is
/// unrestricted (every file is the agent's to name).
fn roots_of(session: &Session) -> Option<Vec<PathBuf>> {
    let workspace = session.workspace();
    let mut roots = Vec::new();
    for access in [&workspace.read, &workspace.write] {
        match access {
            Access::Unrestricted => return None,
            Access::Root(_) | Access::Roots(_) | Access::Grant { .. } => {
                for root in access.roots() {
                    if let Ok(root) = std::path::absolute(root) {
                        roots.push(root);
                    }
                }
            }
            Access::None | Access::Memory => {}
        }
    }
    Some(roots)
}

impl Shell {
    /// The recent files, newest first, with whether each is still there
    /// (as last looked at).
    pub fn recent_files(&mut self) -> Vec<RecentEntry> {
        if self.recent.entries.is_none() {
            self.refresh_recent();
        }
        self.recent.entries.clone().unwrap_or_default()
    }

    /// Looks again whether each recent file is there.
    pub fn refresh_recent(&mut self) {
        self.recent.entries = Some(
            self.recent
                .list
                .paths()
                .iter()
                .map(|path| RecentEntry {
                    path: path.clone(),
                    name: file_name(path),
                    exists: exists(path),
                    modified: modified(path),
                })
                .collect(),
        );
    }

    /// The list as `ui.get` gives it: only the files beneath the
    /// session's roots, the ones an agent may open (all of them with
    /// unrestricted access).
    pub fn recent_json(&mut self) -> Value {
        let roots = self.recent.roots.clone();
        Value::Array(
            self.recent_files()
                .into_iter()
                .filter(|entry| {
                    roots.as_ref().is_none_or(|roots| {
                        roots
                            .iter()
                            .any(|root| std::path::Path::new(&entry.path).starts_with(root))
                    })
                })
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
        self.recent.entries = None;
    }

    /// The list, as JSON to store, when it changed since the last call.
    pub fn take_recent_changed(&mut self) -> Option<String> {
        std::mem::take(&mut self.recent.changed).then(|| self.recent.list.to_json())
    }

    /// The list as it is now.
    pub fn recent_list(&self) -> &RecentFiles {
        &self.recent.list
    }

    /// The recent file chosen in the menu or on the start screen, for the
    /// host to open (`file.open`, with the person's own access to it).
    pub fn take_open_recent(&mut self) -> Option<String> {
        self.recent.chosen.take()
    }

    /// Empties the list (File, Recent files, Clear recent).
    pub fn clear_recent_files(&mut self) {
        self.recent.changed |= self.recent.list.clear();
        self.recent.entries = None;
    }

    pub(crate) fn choose_recent(&mut self, path: String) {
        self.recent.chosen = Some(path);
    }

    /// Notes the documents that came with a file since the last sync:
    /// opened, or saved under a new name, by the person (an agent's files
    /// are left out). The active one goes first. The
    /// file is the one on disk the engine reports
    /// ([`craft3d_engine::DocumentStatus::file`]), absolute, whichever
    /// root the request named it beneath.
    pub(super) fn note_recent(&mut self, session: &Session) {
        self.recent.roots = roots_of(session);
        let statuses = session.document_statuses();
        let active = session.document_status().map(|status| status.index);
        let mut open = BTreeSet::new();
        let mut new = Vec::new();
        for status in &statuses {
            let Some(file) = status.file.as_deref() else {
                continue;
            };
            // A file an agent opened or wrote is not the person's: left
            // out, so opening it from the list later would not quietly
            // give its folder to agents (2026-11-02).
            if session.person_file_of(status.index).as_deref() != Some(file) {
                continue;
            }
            if !self.recent.open.contains(file) {
                new.push((Some(status.index) == active, file.to_string()));
            }
            open.insert(file.to_string());
        }
        // Inactive ones first, so the active one ends up in front.
        new.sort_by_key(|(active, _)| *active);
        for (_, file) in new {
            if self.recent.list.note(&file) {
                self.recent.changed = true;
                self.recent.entries = None;
            }
        }
        self.recent.open = open;
    }

    /// File › Open recent: the files with their documents' line icons,
    /// and Clear recent. Not in the browser.
    pub(super) fn recent_menu(&mut self, ui: &mut egui::Ui, kit: &crate::kit::Kit) {
        use crate::kit::MenuItem;
        if cfg!(target_arch = "wasm32") {
            return;
        }
        let item = kit.menu_item(ui, MenuItem::new("Open recent").icon_room().submenu());
        egui::containers::menu::SubMenu::new().show(ui, &item, |ui| {
            ui.style_mut().visuals = kit.t.plate_visuals();
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(kit.t.menu_width + 80.0);
            // Looked at again when the menu opens.
            let pass = ui.ctx().cumulative_pass_nr();
            if self
                .recent
                .menu_pass
                .is_none_or(|last| last.saturating_add(1) < pass)
            {
                self.refresh_recent();
            }
            self.recent.menu_pass = Some(pass);
            let entries = self.recent_files();
            if entries.is_empty() {
                kit.menu_item(ui, MenuItem::new("No recent files").enabled(false));
                return;
            }
            for entry in &entries {
                let item = MenuItem::new(&entry.name)
                    .icon(file_glyph(&entry.path))
                    .enabled(entry.exists);
                let hover = if entry.exists {
                    entry.path.clone()
                } else {
                    format!("{} (not there any more)", entry.path)
                };
                if kit.menu_item(ui, item).on_hover_text(hover).clicked() {
                    self.choose_recent(entry.path.clone());
                    ui.close();
                }
            }
            kit.menu_separator(ui);
            if kit
                .menu_item(ui, MenuItem::new("Clear recent").icon_room())
                .clicked()
            {
                self.clear_recent_files();
                ui.close();
            }
        });
    }

    /// The recent files on the start screen (board f1): the document's
    /// line icon, the name as a link and how long ago it changed at the
    /// right; a missing one dimmed.
    pub(super) fn recent_links(&mut self, ui: &mut egui::Ui, kit: &crate::kit::Kit) {
        let t = kit.t;
        let s = kit.skin;
        let entries = self.recent_files();
        if entries.is_empty() {
            kit.paragraph(ui, "Files you open or save show up here.", true);
            return;
        }
        let now = super::epoch_now();
        for entry in entries.iter().take(6) {
            let width = ui.available_width();
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(width.max(0.0), kit.hit(t.row_height + 4.0)),
                if entry.exists {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                },
            );
            let painter = ui.painter();
            if response.hovered() && entry.exists {
                painter.rect_filled(rect, kit.radius(), color(s.hover));
            }
            let side = t.nav_icon_size;
            let icon = egui::Rect::from_center_size(
                egui::pos2(rect.left() + 4.0 + side * 0.5, rect.center().y),
                egui::vec2(side, side),
            );
            crate::kit::glyphs::paint(
                painter,
                file_glyph(&entry.path),
                icon,
                color(s.text_dim),
                &t,
            );
            let when = ago(entry.modified, now);
            let when = kit.galley(ui, when, kit.small_font(), s.text_dim);
            let when_x = rect.right() - 4.0 - when.size().x;
            let x = icon.right() + t.nav_icon_gap + 2.0;
            let ink = if entry.exists { s.info_ink } else { s.text_dim };
            let mut job = egui::text::LayoutJob::simple_singleline(
                entry.name.clone(),
                kit.font(),
                color(ink),
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width((when_x - x - 8.0).max(1.0));
            let name = painter.layout_job(job);
            let name_size = name.size();
            painter.galley(
                egui::pos2(x, rect.center().y - name_size.y * 0.5),
                name,
                color(ink),
            );
            if response.hovered() && entry.exists {
                let y = rect.center().y + name_size.y * 0.5 - 1.0;
                painter.line_segment(
                    [egui::pos2(x, y), egui::pos2(x + name_size.x, y)],
                    egui::Stroke::new(1.0, color(ink)),
                );
            }
            painter.galley(
                egui::pos2(when_x, rect.center().y - when.size().y * 0.5),
                when,
                color(s.text_dim),
            );
            let hover = if entry.exists {
                entry.path.clone()
            } else {
                format!("{} (not there any more)", entry.path)
            };
            if response
                .on_hover_text(hover)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.choose_recent(entry.path.clone());
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
