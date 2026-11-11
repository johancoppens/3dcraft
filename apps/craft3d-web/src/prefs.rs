//! The preferences the page keeps between visits, in the browser's
//! `localStorage` (as JSON under [`KEY`]): which panels are open, whether
//! sketches and datums are shown, the appearance (system, light or dark),
//! and the format of the last export.
//! Storage may be missing, blocked or full (a private window, a sandboxed
//! iframe, a browser that partitions third-party storage): the page then
//! starts with the defaults and keeps nothing, without an error. Whatever
//! is stored is read as hostile input: what does not read is the default.

use craft3d_ui_egui::UiState;
use craft3d_ui_egui::state::{Appearance, Panels, Show};
use serde_json::{Value, json};

/// The key in `localStorage`.
pub const KEY: &str = "3dcraft.prefs";

/// The most bytes of stored preferences read.
const MAX_STORED: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub panels: Panels,
    pub show: Show,
    pub appearance: Appearance,
    /// The extension of the last export, lower case (`step`, `stl`, …).
    pub export_extension: Option<String>,
}

impl Prefs {
    /// The preferences as the shell has them now.
    pub fn of(state: &UiState, export_extension: Option<&str>) -> Self {
        Self {
            panels: state.panels,
            show: state.show,
            appearance: state.appearance,
            export_extension: export_extension.map(str::to_string),
        }
    }

    /// Puts them into the shell's state: the format of the last export is
    /// the export dialog's first choice.
    pub fn apply(&self, state: &mut UiState) {
        state.panels = self.panels;
        state.show = self.show;
        state.appearance = self.appearance;
        if let Some(format) = self
            .export_extension
            .as_deref()
            .and_then(craft3d_ui_egui::files::ExportFormat::from_extension)
        {
            state.export.format = format;
        }
    }

    pub fn to_json(&self) -> String {
        json!({
            "panels": self.panels,
            "show": self.show,
            "appearance": self.appearance,
            "exportExtension": self.export_extension,
        })
        .to_string()
    }

    /// Reads stored preferences over `defaults`, field by field: a field
    /// that is missing or does not read keeps its default.
    pub fn from_json(text: &str, defaults: Self) -> Self {
        let mut prefs = defaults;
        if text.len() > MAX_STORED {
            return prefs;
        }
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
            return prefs;
        };
        if let Some(Ok(panels)) = map.get("panels").cloned().map(serde_json::from_value) {
            prefs.panels = panels;
        }
        if let Some(Ok(show)) = map.get("show").cloned().map(serde_json::from_value) {
            prefs.show = show;
        }
        if let Some(Ok(appearance)) = map.get("appearance").cloned().map(serde_json::from_value) {
            prefs.appearance = appearance;
        }
        if let Some(Value::String(extension)) = map.get("exportExtension") {
            let known = craft3d_ui_egui::files::export_kind(&format!("x.{extension}")).is_some();
            if known && extension.len() <= 8 {
                prefs.export_extension = Some(extension.to_ascii_lowercase());
            }
        }
        prefs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Prefs {
        Prefs::of(&UiState::default(), None)
    }

    #[test]
    fn preferences_read_back_as_they_were_written() {
        let mut state = UiState::default();
        state.panels.tree = !state.panels.tree;
        state.show.sketches = !state.show.sketches;
        state.appearance = Appearance::Dark;
        let prefs = Prefs::of(&state, Some("stl"));
        let read = Prefs::from_json(&prefs.to_json(), defaults());
        assert_eq!(read, prefs);
        let mut applied = UiState::default();
        read.apply(&mut applied);
        assert_eq!(applied.panels, state.panels);
        assert_eq!(applied.show, state.show);
        assert_eq!(applied.appearance, Appearance::Dark);
    }

    #[test]
    fn what_does_not_read_is_the_default() {
        for junk in [
            "",
            "null",
            "[1, 2]",
            "{\"panels\": 3, \"show\": {\"bogus\": 1}}",
            "{\"exportExtension\": \"exe\"}",
            "{\"exportExtension\": 7}",
            "{\"appearance\": \"dusk\"}",
            "{{{{",
        ] {
            assert_eq!(Prefs::from_json(junk, defaults()), defaults(), "{junk}");
        }
        let long = format!("{{\"x\": \"{}\"}}", "a".repeat(MAX_STORED));
        assert_eq!(Prefs::from_json(&long, defaults()), defaults());
        // One good field among bad ones is kept.
        let mixed = Prefs::from_json("{\"panels\": 3, \"exportExtension\": \"OBJ\"}", defaults());
        assert_eq!(mixed.export_extension.as_deref(), Some("obj"));
    }

    /// Datums are hidden by default (2026-11-11); a stored switch wins
    /// either way, and stored preferences without it (none at all, no
    /// `show`, or a `show` without `datums`) take the default.
    #[test]
    fn a_stored_datums_switch_wins_and_a_missing_one_is_hidden() {
        assert!(!defaults().show.datums);
        assert!(defaults().show.sketches);
        let shown = |text: &str| {
            let mut state = UiState::default();
            Prefs::from_json(text, defaults()).apply(&mut state);
            state.show
        };
        // Written by an earlier build, which stored the whole of `show`.
        let stored = shown(
            "{\"show\": {\"datums\": true, \"sketches\": true, \"edges\": true, \"mates\": true}}",
        );
        assert!(stored.datums && stored.sketches);
        assert!(!shown("{\"show\": {\"datums\": false, \"sketches\": true}}").datums);
        for missing in [
            "{}",
            "{\"panels\": {\"tree\": false}}",
            "{\"show\": {\"sketches\": false}}",
        ] {
            assert!(!shown(missing).datums, "{missing}");
        }
        assert!(!shown("{\"show\": {\"sketches\": false}}").sketches);
        // Switched on, it is written and read back on.
        let mut state = UiState::default();
        state.show.datums = true;
        let text = Prefs::of(&state, None).to_json();
        assert!(shown(&text).datums);
    }
}
