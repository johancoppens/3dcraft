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
        let mut fetches = FontFetches::default();
        // Nothing is fetched until a text asks for a font.
        assert!(fetches.due().iter().all(|(name, _)| *name != "mono"));
        craft3d_engine::fonts::want("mono").unwrap();
        assert_eq!(
            fetches.due(),
            vec![("mono", "fonts/JetBrainsMono-Regular.ttf".to_string())]
        );
        // Once: asking again does not fetch it again.
        assert!(fetches.due().is_empty());
        // After a failure the next text tries again.
        fetches.failed("mono");
        assert_eq!(fetches.due().len(), 1);
        // Bytes that are not the pinned file are refused.
        assert!(craft3d_engine::fonts::register("mono", b"not a font").is_err());
        assert!(craft3d_engine::fonts::want("nope").is_err());
    }
}
