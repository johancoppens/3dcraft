//! The built-in fonts, fetched when a text first asks for one
//! (docs/design/text.md › *Built-in fonts*): they are files of the site
//! (`fonts/<file>`, copied there by `cargo xtask package --formats web`),
//! never part of the module, so a page that sets no text never loads one.
//!
//! The engine lists the built-in fonts a text asked for that are not
//! here (`craft3d_engine::fonts::wanted`); each is fetched once, and its
//! bytes handed to `craft3d_engine::fonts::register`, which checks them
//! against the pinned SHA-256. The command that asked is then run again.

use std::collections::BTreeSet;

/// Whether `name` is the interface's own font (Inter, fetched at start-up
/// for the shell: `craft3d_ui_egui::fonts`), which no text uses.
pub fn is_interface_font(name: &str) -> bool {
    name == craft3d_ui_egui::fonts::TEXT
}

/// Which built-in fonts were asked for already.
#[derive(Debug, Default)]
pub struct FontFetches {
    asked: BTreeSet<&'static str>,
}

impl FontFetches {
    /// The fonts to fetch now, each with the address of its file beside
    /// the page: every font the engine wants that was not asked for yet.
    pub fn due(&mut self) -> Vec<(&'static str, String)> {
        craft3d_engine::fonts::wanted()
            .into_iter()
            .filter(|(name, _)| self.asked.insert(name))
            .map(|(name, file)| (name, format!("fonts/{file}")))
            .collect()
    }

    /// A fetch that failed may be tried again, by the next text.
    pub fn failed(&mut self, name: &str) {
        self.asked.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_is_fetched_once_and_only_when_a_text_wants_it() {
        // The engine's wanted fonts are process-wide, and other tests in
        // this binary want fonts of their own at the same time: only the
        // mono entries are this test's to count.
        let mono = |due: Vec<(&'static str, String)>| -> Vec<(&'static str, String)> {
            due.into_iter()
                .filter(|(name, _)| *name == "mono")
                .collect()
        };
        let mut fetches = FontFetches::default();
        // Nothing is fetched until a text asks for a font.
        assert!(mono(fetches.due()).is_empty());
        craft3d_engine::fonts::want("mono").unwrap();
        let due = mono(fetches.due());
        // Another test of this binary may have brought the font in; then
        // it is rightly not wanted, and there is nothing to count here.
        if craft3d_engine::fonts::ready("mono") {
            return;
        }
        assert_eq!(
            due,
            vec![("mono", "fonts/JetBrainsMono-Regular.ttf".to_string())]
        );
        // Once: asking again does not fetch it again.
        assert!(mono(fetches.due()).is_empty());
        // After a failure the next text tries again.
        fetches.failed("mono");
        assert_eq!(mono(fetches.due()).len(), 1);
        // Bytes that are not the pinned file are refused.
        assert!(craft3d_engine::fonts::register("mono", b"not a font").is_err());
        assert!(craft3d_engine::fonts::want("nope").is_err());
    }
}
