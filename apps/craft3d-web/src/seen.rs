//! What's new in the browser: the newest version whose release notes were
//! seen here, kept in `localStorage` under a key of its own ([`KEY`]),
//! beside the preferences (`prefs.rs`) and never inside them. A tab of an
//! earlier version that is still open writes its preferences without
//! knowing of it, and a tab that changes a preference does not touch it;
//! only the person's close of the notes writes it ([`at_close`]).
//!
//! At a start ([`at_start`]): a version there is the one seen; a value
//! that is not a version is read as the desktop reads a note that does
//! not read (a first visit: nothing offered, this version written); no
//! value, but preferences an earlier 3DCraft stored, is an update from it
//! ([`Seen::Earlier`]); nothing stored at all is a first visit, this
//! version written at once. **Nothing is offered that cannot be
//! remembered:** before the card is offered, the page writes under the
//! key what a close will write and puts back what was there; where that
//! fails (storage full, blocked, or throwing), no card is shown, rather
//! than the same card at every visit.
//!
//! Until 2026-11-15 the version was a field of the preferences: an open
//! tab of 0.1 then wrote it away (the card came back once), and a tab
//! that changed a preference had to merge it with what another tab
//! closed. With a key of its own neither is needed. No released version
//! kept it there, so it is not read from there.

use craft3d_engine::changelog::{Seen, Version};

/// The key in `localStorage`.
pub const KEY: &str = "3dcraft.seenVersion";

/// The most bytes of a stored version read.
const MAX_STORED: usize = 64;

/// The browser's storage as the page uses it here; tests give one of
/// their own.
pub trait Store {
    /// What is stored under `key`, where it can be read.
    fn get(&mut self, key: &str) -> Option<String>;
    /// Stores `value` under `key`; false where the storage refused.
    fn set(&mut self, key: &str, value: &str) -> bool;
    fn remove(&mut self, key: &str);
}

/// What is stored of the notes seen, without writing anything.
pub fn stored(store: &mut dyn Store) -> Seen {
    match store.get(KEY) {
        Some(text) if text.len() <= MAX_STORED => {
            Version::parse(&text).map_or(Seen::Never, Seen::Version)
        }
        Some(_) => Seen::Never,
        None if store.get(crate::prefs::KEY).is_some() => Seen::Earlier,
        None => Seen::Never,
    }
}

/// What a page of version `running` knows of the notes seen as it
/// starts, for the card *New in …*: see the module's documentation.
pub fn at_start(store: &mut dyn Store, running: Version) -> Seen {
    let seen = stored(store);
    match seen {
        Seen::Never => {
            store.set(KEY, &running.to_string());
            Seen::Never
        }
        // The same version, or a newer one (a page served again in an
        // older version): nothing to offer, nothing written.
        Seen::Version(version) if version >= running => seen,
        Seen::Earlier | Seen::Version(_) => {
            let before = store.get(KEY);
            if !store.set(KEY, &closed(before.as_deref(), running)) {
                return Seen::Never;
            }
            match before {
                Some(text) => {
                    store.set(KEY, &text);
                }
                None => store.remove(KEY),
            }
            seen
        }
    }
}

/// What a close of the notes of `running` writes, with `stored` under
/// the key: `running`, or the newer version stored there (a tab of a
/// newer version closed them since).
fn closed(stored: Option<&str>, running: Version) -> String {
    stored
        .filter(|text| text.len() <= MAX_STORED)
        .and_then(|text| Version::parse(text).ok())
        .filter(|version| *version > running)
        .unwrap_or(running)
        .to_string()
}

/// The person closed the notes of `running`: they are seen from now on.
pub fn at_close(store: &mut dyn Store, running: Version) {
    let stored = store.get(KEY);
    store.set(KEY, &closed(stored.as_deref(), running));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    /// A browser's storage in memory: `limit` is the most bytes of keys
    /// and values it holds (a full storage refuses what would grow it);
    /// `throws` refuses every write.
    #[derive(Clone, Default)]
    struct Memory {
        items: BTreeMap<String, String>,
        limit: Option<usize>,
        throws: bool,
    }

    impl Memory {
        fn size(items: &BTreeMap<String, String>) -> usize {
            items.iter().map(|(k, v)| k.len() + v.len()).sum()
        }
    }

    impl Store for Memory {
        fn get(&mut self, key: &str) -> Option<String> {
            self.items.get(key).cloned()
        }
        fn set(&mut self, key: &str, value: &str) -> bool {
            if self.throws {
                return false;
            }
            let mut after = self.items.clone();
            after.insert(key.to_string(), value.to_string());
            if self.limit.is_some_and(|limit| Memory::size(&after) > limit) {
                return false;
            }
            self.items = after;
            true
        }
        fn remove(&mut self, key: &str) {
            self.items.remove(key);
        }
    }

    /// What 0.1's page stored: its preferences, no version.
    const EARLIER: &str =
        "{\"panels\": {\"tree\": true}, \"appearance\": \"dark\", \"exportExtension\": \"step\"}";

    fn with(items: &[(&str, &str)]) -> Memory {
        Memory {
            items: items
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Memory::default()
        }
    }

    /// Whether the card comes up at a visit of `running` with `store`.
    fn offered(store: &mut Memory, running: Version) -> bool {
        match at_start(store, running) {
            Seen::Earlier => true,
            Seen::Version(seen) => seen < running,
            Seen::Never => false,
        }
    }

    #[test]
    fn a_first_visit_an_earlier_version_and_a_version_seen_are_told_apart() {
        let running = v("0.2.0");
        // Nothing stored: a first visit; this version is seen at once.
        let mut store = Memory::default();
        assert_eq!(at_start(&mut store, running), Seen::Never);
        assert_eq!(store.get(KEY).as_deref(), Some("0.2.0"));
        // Preferences of a 3DCraft that kept no version: an earlier one,
        // offered, and nothing kept under the key until it is closed.
        let mut store = with(&[(crate::prefs::KEY, EARLIER)]);
        assert_eq!(at_start(&mut store, running), Seen::Earlier);
        assert_eq!(store.get(KEY), None);
        assert_eq!(store.get(crate::prefs::KEY).as_deref(), Some(EARLIER));
        // A version seen: offered after it, kept as it was until closed.
        let mut store = with(&[(KEY, "0.1.1")]);
        assert_eq!(at_start(&mut store, running), Seen::Version(v("0.1.1")));
        assert_eq!(store.get(KEY).as_deref(), Some("0.1.1"));
        // The same version, or a newer one: nothing to offer or write.
        for same in ["0.2.0", "0.3.0"] {
            let mut store = with(&[(KEY, same)]);
            assert_eq!(at_start(&mut store, running), Seen::Version(v(same)));
            assert_eq!(store.get(KEY).as_deref(), Some(same));
        }
        // A value that is not a version: as the desktop reads a note that
        // does not read, a first visit; this version written.
        let long = "1".repeat(MAX_STORED + 1);
        for junk in [
            "soon",
            "banana",
            "7",
            "",
            "12345678901234567890.0.0",
            "[\"0.1.0\"]",
            "null",
            " 0.1.1",
            long.as_str(),
        ] {
            let mut store = with(&[(KEY, junk), (crate::prefs::KEY, EARLIER)]);
            assert_eq!(at_start(&mut store, running), Seen::Never, "{junk:?}");
            assert_eq!(store.get(KEY).as_deref(), Some("0.2.0"), "{junk:?}");
        }
    }

    /// Closing keeps this version, or a newer one stored by another tab.
    #[test]
    fn a_close_keeps_this_version_or_a_newer_one() {
        let running = v("0.2.0");
        for (before, after) in [
            (None, "0.2.0"),
            (Some("0.1.1"), "0.2.0"),
            (Some("0.3.0"), "0.3.0"),
            (Some("junk"), "0.2.0"),
        ] {
            let mut store = Memory::default();
            if let Some(before) = before {
                store.set(KEY, before);
            }
            at_close(&mut store, running);
            assert_eq!(store.get(KEY).as_deref(), Some(after), "{before:?}");
        }
    }

    /// The reviewer's two findings of round 2 (b): an open tab of 0.1
    /// writes its preferences without a version; and two tabs of this one,
    /// the second changing a preference after the first closed the card.
    /// With the version under a key of its own, neither writes it away:
    /// the next visit has no card.
    #[test]
    fn a_tab_that_writes_its_preferences_never_writes_the_version_away() {
        let running = v("0.2.0");
        let mut store = with(&[(crate::prefs::KEY, EARLIER), (KEY, "0.1.1")]);
        // Tabs A and B load: both offer the card.
        assert!(offered(&mut store, running));
        assert!(offered(&mut store, running));
        // A closes it.
        at_close(&mut store, running);
        // B changes a preference; an open tab of 0.1 writes its own.
        store.set(crate::prefs::KEY, "{\"appearance\": \"dark\"}");
        store.set(crate::prefs::KEY, EARLIER);
        // A's next visit: nothing offered.
        assert_eq!(at_start(&mut store, running), Seen::Version(running));
        assert!(!offered(&mut store, running));
    }

    /// Where the key cannot be written, no card is shown, rather than the
    /// same card at every visit. The reviewer saw a full storage (it
    /// refused only what would grow it) bring the card back at three
    /// visits after a 0.1 page; here three visits offer nothing. A storage
    /// that refuses every write: the same. A full storage with a version
    /// stored: the close writes as many bytes as the probe, so it can be
    /// offered and remembered.
    #[test]
    fn where_the_version_cannot_be_written_no_card_is_shown() {
        let running = v("0.2.0");
        let mut full = with(&[(crate::prefs::KEY, EARLIER)]);
        full.limit = Some(Memory::size(&full.items));
        for visit in 0..3 {
            assert_eq!(at_start(&mut full, running), Seen::Never, "visit {visit}");
            assert_eq!(full.get(KEY), None);
        }
        let mut throws = with(&[(crate::prefs::KEY, EARLIER), (KEY, "0.1.1")]);
        throws.throws = true;
        for visit in 0..3 {
            assert_eq!(at_start(&mut throws, running), Seen::Never, "visit {visit}");
        }
        assert_eq!(throws.get(KEY).as_deref(), Some("0.1.1"));
        let mut full = with(&[(crate::prefs::KEY, EARLIER), (KEY, "0.1.1")]);
        full.limit = Some(Memory::size(&full.items));
        assert_eq!(at_start(&mut full, running), Seen::Version(v("0.1.1")));
        assert_eq!(full.get(KEY).as_deref(), Some("0.1.1"));
        at_close(&mut full, running);
        assert_eq!(at_start(&mut full, running), Seen::Version(running));
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
