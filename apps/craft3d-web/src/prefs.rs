//! The preferences the page keeps between visits, in the browser's
//! `localStorage` (as JSON under [`KEY`]): which panels are open, whether
//! sketches and datums are shown, the appearance (system, light or dark),
//! the format of the last export, and the newest version whose release
//! notes were seen here (What's new: [`seen`], [`notes_at_start`]).
//! Storage may be missing, blocked or full (a private window, a sandboxed
//! iframe, a browser that partitions third-party storage): the page then
//! starts with the defaults and keeps nothing, without an error. Whatever
//! is stored is read as hostile input: what does not read is the default.

use craft3d_engine::changelog::{Seen, Version};
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
    /// The newest version whose release notes were seen in this browser
    /// (`0.2.0`); `None` while that is not known: preferences an earlier
    /// version stored have no such field.
    pub seen_version: Option<String>,
}

/// What the stored preferences say of the release notes seen in this
/// browser. Nothing stored (a first visit, or storage that is missing or
/// blocked, which looks the same) is [`Seen::Never`]: nothing is old, and
/// nothing can be remembered. Preferences without a version (none, or
/// `null`, which a page writes while its card is still open) are an
/// earlier 3DCraft's ([`Seen::Earlier`]). A version that is not one (a
/// number, a word) is read as the desktop reads a note that does not
/// read: nothing is offered, and this version is written as seen. What
/// does not read as preferences at all is nothing stored.
pub fn seen(stored: Option<&str>) -> Seen {
    let Some(text) = stored.filter(|text| text.len() <= MAX_STORED) else {
        return Seen::Never;
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
        return Seen::Never;
    };
    match map.get("seenVersion") {
        None | Some(Value::Null) => Seen::Earlier,
        Some(Value::String(version)) => Version::parse(version).map_or(Seen::Never, Seen::Version),
        Some(_) => Seen::Never,
    }
}

/// The version to write with the preferences: the newer of the one this
/// page holds (`held`) and the one stored now (`stored`, the preferences
/// as they are in the browser's storage at this moment). Another tab may
/// have closed the notes since this one read them: its newer version is
/// kept, where writing this page's own would bring the card back at the
/// next visit (two tabs, 2026-11-15).
pub fn newest_seen(held: Option<&str>, stored: Option<&str>) -> Option<String> {
    let version = |text: Option<&str>| text.and_then(|text| Version::parse(text).ok());
    let there = match seen(stored) {
        Seen::Version(version) => Some(version),
        Seen::Never | Seen::Earlier => None,
    };
    match (version(held), there) {
        (Some(ours), Some(theirs)) if theirs > ours => Some(theirs.to_string()),
        (None, Some(theirs)) => Some(theirs.to_string()),
        _ => held.map(str::to_string),
    }
}

/// What a page of version `running` knows as it starts, from the stored
/// preferences (`stored`) and whether they could be written back just
/// now (`writable`): what was seen, for the card *New in …*, and the
/// version to keep in the preferences until the notes are closed.
///
/// A first visit is seen as `running` at once. Where the preferences
/// cannot be written, nothing is offered, rather than the same card at
/// every visit: the answer is [`Seen::Never`] and the version stays what
/// it was.
pub fn notes_at_start(
    stored: Option<&str>,
    writable: bool,
    running: Version,
) -> (Seen, Option<String>) {
    match seen(stored) {
        Seen::Never => (Seen::Never, Some(running.to_string())),
        Seen::Earlier if writable => (Seen::Earlier, None),
        Seen::Earlier => (Seen::Never, None),
        Seen::Version(version) if writable => (Seen::Version(version), Some(version.to_string())),
        Seen::Version(version) => (Seen::Never, Some(version.to_string())),
    }
}

/// The version to keep once the person closed the notes of `running`:
/// `running`, or the newer one seen here before (a page served again in
/// an older version).
pub fn seen_after_closing(kept: Option<&str>, running: Version) -> String {
    kept.and_then(|version| Version::parse(version).ok())
        .filter(|version| *version > running)
        .unwrap_or(running)
        .to_string()
}

impl Prefs {
    /// The preferences as the shell has them now.
    pub fn of(state: &UiState, export_extension: Option<&str>) -> Self {
        Self {
            panels: state.panels,
            show: state.show,
            appearance: state.appearance,
            export_extension: export_extension.map(str::to_string),
            seen_version: None,
        }
    }

    /// With the newest version whose release notes were seen.
    pub fn with_seen(mut self, version: Option<String>) -> Self {
        self.seen_version = version;
        self
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
            "seenVersion": self.seen_version,
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
        if let Some(Value::String(version)) = map.get("seenVersion")
            && let Ok(version) = Version::parse(version)
        {
            prefs.seen_version = Some(version.to_string());
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

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    /// What 0.1's page stored: no version in it.
    const EARLIER: &str =
        "{\"panels\": {\"tree\": true}, \"appearance\": \"dark\", \"exportExtension\": \"step\"}";

    #[test]
    fn the_version_seen_is_kept_with_the_preferences() {
        let prefs = defaults().with_seen(Some("0.2.0".to_string()));
        let text = prefs.to_json();
        assert_eq!(Prefs::from_json(&text, defaults()), prefs);
        assert_eq!(seen(Some(&text)), Seen::Version(v("0.2.0")));
        // The other preferences are untouched by it, and it by them.
        let read = Prefs::from_json(EARLIER, defaults());
        assert_eq!(read.seen_version, None);
        assert_eq!(read.appearance, Appearance::Dark);
        // A version that is not one is not kept.
        for junk in [
            "{\"seenVersion\": \"soon\"}",
            "{\"seenVersion\": 7}",
            "{\"seenVersion\": [\"0.1.0\"]}",
            "{\"seenVersion\": \"1.2\"}",
        ] {
            assert_eq!(
                Prefs::from_json(junk, defaults()).seen_version,
                None,
                "{junk}"
            );
        }
    }

    #[test]
    fn a_first_visit_an_earlier_version_and_a_version_seen_are_told_apart() {
        // Nothing stored: a first visit, or storage that keeps nothing.
        assert_eq!(seen(None), Seen::Never);
        // What does not read as preferences is nothing stored.
        let long = format!("{{\"x\": \"{}\"}}", "a".repeat(MAX_STORED));
        for junk in ["", "null", "[1, 2]", "7", "{{{{", long.as_str()] {
            assert_eq!(seen(Some(junk)), Seen::Never, "{junk:?}");
        }
        // Preferences of a 3DCraft that kept no version: an earlier one.
        assert_eq!(seen(Some(EARLIER)), Seen::Earlier);
        assert_eq!(seen(Some("{}")), Seen::Earlier);
        assert_eq!(seen(Some("{\"seenVersion\": null}")), Seen::Earlier);
        // A version that is not one: as the desktop reads a note that
        // does not read, a first start; nothing is offered and this
        // version is written (until 2026-11-15 the card came up with
        // everything).
        let running = v("0.2.0");
        for junk in [
            "{\"seenVersion\": \"soon\"}",
            "{\"seenVersion\": \"banana\"}",
            "{\"seenVersion\": 7}",
            "{\"seenVersion\": 12345678901234567890}",
            "{\"seenVersion\": \"12345678901234567890.0.0\"}",
            "{\"seenVersion\": [\"0.1.0\"]}",
            "{\"seenVersion\": {\"v\": 1}}",
            "{\"seenVersion\": true}",
            "{\"seenVersion\": \"\"}",
        ] {
            assert_eq!(seen(Some(junk)), Seen::Never, "{junk}");
            for writable in [true, false] {
                assert_eq!(
                    notes_at_start(Some(junk), writable, running),
                    (Seen::Never, Some("0.2.0".to_string())),
                    "{junk}"
                );
            }
        }
        assert_eq!(
            seen(Some("{\"seenVersion\": \"0.1.2\"}")),
            Seen::Version(v("0.1.2"))
        );
    }

    #[test]
    fn the_page_offers_notes_only_where_it_can_remember_them() {
        let running = v("0.2.0");
        // A first visit: nothing is old; this version is seen at once.
        assert_eq!(
            notes_at_start(None, false, running),
            (Seen::Never, Some("0.2.0".to_string()))
        );
        // After an earlier version: offered, and no version kept until
        // the notes are closed.
        assert_eq!(
            notes_at_start(Some(EARLIER), true, running),
            (Seen::Earlier, None)
        );
        // After a version seen.
        let stored = "{\"seenVersion\": \"0.1.1\"}";
        assert_eq!(
            notes_at_start(Some(stored), true, running),
            (Seen::Version(v("0.1.1")), Some("0.1.1".to_string()))
        );
        // Storage that reads but does not write: nothing is offered, and
        // what is kept stays as it was.
        assert_eq!(
            notes_at_start(Some(EARLIER), false, running),
            (Seen::Never, None)
        );
        assert_eq!(
            notes_at_start(Some(stored), false, running),
            (Seen::Never, Some("0.1.1".to_string()))
        );
        // Closing the notes keeps this version, or a newer one seen here.
        assert_eq!(seen_after_closing(None, running), "0.2.0");
        assert_eq!(seen_after_closing(Some("0.1.1"), running), "0.2.0");
        assert_eq!(seen_after_closing(Some("0.3.0"), running), "0.3.0");
        assert_eq!(seen_after_closing(Some("junk"), running), "0.2.0");
    }

    /// Two tabs at once. Both load with 0.1.1 seen and show the card. The
    /// person closes it in tab A, which stores this version. Tab B, still
    /// open, then changes a preference: it writes the newer version that
    /// is stored, not its own stale one, so A's next visit has no card.
    #[test]
    fn a_tab_that_writes_later_keeps_the_newer_version_another_tab_stored() {
        let running = v("0.2.0");
        let stored = defaults().with_seen(Some("0.1.1".to_string())).to_json();
        let (seen_a, mut held_a) = notes_at_start(Some(&stored), true, running);
        let (seen_b, mut held_b) = notes_at_start(Some(&stored), true, running);
        assert_eq!(seen_a, Seen::Version(v("0.1.1")));
        assert_eq!(seen_b, Seen::Version(v("0.1.1")));
        // What a tab writes: its preferences with the newer of what it
        // holds and what is stored, as the page does before every write.
        let write = |held: &mut Option<String>, storage: &str, appearance: Appearance| {
            *held = newest_seen(held.as_deref(), Some(storage));
            let mut prefs = defaults().with_seen(held.clone());
            prefs.appearance = appearance;
            prefs.to_json()
        };
        // A closes the card.
        held_a = Some(seen_after_closing(held_a.as_deref(), running));
        let storage = write(&mut held_a, &stored, Appearance::System);
        assert_eq!(seen(Some(&storage)), Seen::Version(running));
        // B changes a preference.
        let storage = write(&mut held_b, &storage, Appearance::Dark);
        assert_eq!(
            seen(Some(&storage)),
            Seen::Version(running),
            "not B's 0.1.1"
        );
        assert_eq!(held_b.as_deref(), Some("0.2.0"));
        // A's next visit: nothing offered.
        assert_eq!(
            notes_at_start(Some(&storage), true, running),
            (Seen::Version(running), Some("0.2.0".to_string()))
        );
        // The newer of the two, whichever holds it; what does not read
        // counts for nothing.
        assert_eq!(newest_seen(None, None), None);
        assert_eq!(newest_seen(Some("0.2.0"), None).as_deref(), Some("0.2.0"));
        let earlier = "{\"panels\": {}}";
        assert_eq!(newest_seen(None, Some(earlier)), None);
        assert_eq!(
            newest_seen(Some("0.3.0"), Some(&storage)).as_deref(),
            Some("0.3.0")
        );
        assert_eq!(newest_seen(None, Some(&storage)).as_deref(), Some("0.2.0"));
        assert_eq!(
            newest_seen(Some("0.1.1"), Some("{\"seenVersion\": \"banana\"}")).as_deref(),
            Some("0.1.1")
        );
        assert_eq!(
            newest_seen(Some("0.1.1"), Some("{{{{")).as_deref(),
            Some("0.1.1")
        );
    }

    /// The notes this build carries read, and name this version: what the
    /// page hands the engine.
    #[test]
    fn the_notes_the_page_carries_read() {
        let notes = craft3d_engine::changelog::Changelog::parse(crate::RELEASE_NOTES).unwrap();
        let running = v(craft3d_engine::VERSION);
        assert!(notes.version(running).is_some_and(|s| !s.is_blank()));
        // Nothing of the first release's own notes is carried: nothing is
        // older, so they are new to nobody.
        let first = notes.oldest().unwrap();
        if first != running {
            assert!(notes.version(first).unwrap().is_blank());
        }
    }
}
