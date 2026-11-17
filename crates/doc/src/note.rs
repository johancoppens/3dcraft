//! Notes pinned to the model (2026-11-17): a sentence written at a place
//! of the model, by the person ("2 mm thicker here", "this hole can go")
//! or by an agent (a question about that place), which an agent reads,
//! works through and marks done with a sentence of reply.
//!
//! A note is pinned to a body, a face, an edge or a vertex the way a
//! selection keeps what it holds: by the feature whose body has it and,
//! for a face, an edge or a vertex, its persistent name there (rule 4,
//! docs/design/names.md), and at a point in model space. Whether that
//! entity is still there, and what it became in later features, is
//! decided where the bodies are known (`craft3d-engine`, `note.list`): a
//! note whose entity is gone is not changed, it stays at its point and
//! says so, and is pinned again when the entity comes back (an undo).
//!
//! Like the assertions, the named sets and the appearance of bodies,
//! notes are beside the history, not in it: regeneration does not read
//! them, so they change no body, no cache key and no assertion. Changing
//! them is an undo step all the same, as every change of the document is.
//! They are saved with the document (format version 75) and left out of
//! everything published: a `.3dcraftjig` holds none (`jig.export`), nor a
//! bundled example (`cargo xtask bundle-examples` refuses a file with
//! any).

use crate::{DocError, Document, FeatureId, PersistentName};
use serde::{Deserialize, Serialize};

/// Most notes a document holds.
pub const MAX_NOTES: usize = 1000;
/// Longest text of a note, and of a reply, in characters.
pub const MAX_NOTE_CHARS: usize = 2000;
/// Longest name of who wrote a note or closed it, in characters.
pub const MAX_NOTE_AUTHOR_CHARS: usize = 200;

/// What a note is pinned to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NoteKind {
    Body,
    Face,
    Edge,
    Vertex,
}

impl NoteKind {
    /// Every kind.
    pub const ALL: [NoteKind; 4] = [
        NoteKind::Body,
        NoteKind::Face,
        NoteKind::Edge,
        NoteKind::Vertex,
    ];

    /// The word the commands and the file use.
    pub fn name(self) -> &'static str {
        match self {
            NoteKind::Body => "body",
            NoteKind::Face => "face",
            NoteKind::Edge => "edge",
            NoteKind::Vertex => "vertex",
        }
    }

    /// The kind a word names.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == word)
    }
}

/// The entity a note is pinned to, as it was when the note was put there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteOn {
    pub kind: NoteKind,
    /// The feature whose body has it (for a body: that feature).
    pub feature: FeatureId,
    /// Its persistent name in that body; `None` for a body.
    pub name: Option<PersistentName>,
}

/// Who closed a note, when, and what they answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteDone {
    /// The agent that closed it, by the name the agent log knows; `None`
    /// for the person.
    pub by: Option<String>,
    /// Milliseconds since the Unix epoch (0 where there is no clock).
    pub time_ms: u64,
    pub reply: Option<String>,
}

/// One note.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Never given to another note of the document ([`Notes::next_id`]).
    pub id: u64,
    pub text: String,
    /// What it is pinned to; `None` for a note at its point alone.
    pub on: Option<NoteOn>,
    /// Where it was put, in model space (the document's length unit).
    pub point: [f64; 3],
    /// The agent that wrote it, by the name the agent log knows; `None`
    /// for the person.
    pub by: Option<String>,
    /// When it was written: milliseconds since the Unix epoch (0 where
    /// there is no clock).
    pub time_ms: u64,
    /// `None` while it is open.
    pub done: Option<NoteDone>,
}

/// The notes of a document, in the order they were written.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notes {
    pub items: Vec<Note>,
    /// The id the next note gets (0 reads as 1): ids are never given out
    /// twice, so a note deleted is never taken for a later one.
    pub next_id: u64,
}

impl Notes {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many are open.
    pub fn open(&self) -> usize {
        self.items.iter().filter(|n| n.done.is_none()).count()
    }
}

/// A note's text, or a reply, as it is kept: line breaks as `\n`, other
/// white space as it was, invisible format characters (direction marks
/// and overrides) left out so a text cannot read otherwise where it is
/// shown, trimmed. Refused: empty, longer than [`MAX_NOTE_CHARS`]
/// characters, or holding another control character.
pub fn clean_note_text(text: &str) -> Result<String, String> {
    // Read no further than twice the cap: what is longer is refused
    // whatever trimming would take away.
    let limit = MAX_NOTE_CHARS.saturating_mul(2);
    let mut out = String::new();
    let mut kept = 0usize;
    let mut cut = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let c = match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                '\n'
            }
            '\t' => ' ',
            c if invisible(c) => continue,
            c if c.is_control() && c != '\n' => {
                return Err(format!(
                    "a note's text must not hold the control character U+{:04X}",
                    u32::from(c)
                ));
            }
            c => c,
        };
        if kept == limit {
            cut = true;
            break;
        }
        out.push(c);
        kept += 1;
    }
    let trimmed = out.trim();
    if trimmed.is_empty() && !cut {
        return Err("a note's text must not be empty".to_string());
    }
    if cut || trimmed.chars().count() > MAX_NOTE_CHARS {
        return Err(format!(
            "a note's text is at most {MAX_NOTE_CHARS} characters long (this one is {})",
            text.chars().count()
        ));
    }
    Ok(trimmed.to_string())
}

/// Who wrote or closed a note, as it is kept: one line of at most
/// [`MAX_NOTE_AUTHOR_CHARS`] characters, control and invisible
/// characters made spaces or left out; `None` when nothing is left.
pub fn clean_note_author(name: &str) -> Option<String> {
    let flat: String = name
        .chars()
        .filter(|c| !invisible(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_NOTE_AUTHOR_CHARS.saturating_mul(2))
        .collect();
    let flat: String = flat.trim().chars().take(MAX_NOTE_AUTHOR_CHARS).collect();
    let flat = flat.trim_end().to_string();
    (!flat.is_empty()).then_some(flat)
}

/// A direction mark or override, or an invisible format character (as the
/// agent log leaves out of an intent).
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

fn check_author(who: &str, what: &str) -> Result<(), String> {
    if clean_note_author(who).as_deref() != Some(who) {
        return Err(format!(
            "{what} must be one line of 1 to {MAX_NOTE_AUTHOR_CHARS} characters without control characters"
        ));
    }
    Ok(())
}

fn check_text(text: &str, what: &str) -> Result<(), String> {
    match clean_note_text(text) {
        Ok(clean) if clean == text => Ok(()),
        Ok(_) => Err(format!(
            "{what} is not as a note keeps it (trimmed, line breaks as \\n, no invisible format characters)"
        )),
        Err(e) => Err(format!("{what}: {e}")),
    }
}

/// Why `on` cannot be what a note is pinned to in a document whose
/// features have ids below `next_feature_id`.
fn check_on(on: &NoteOn, next_feature_id: u64) -> Result<(), String> {
    if on.feature.0 == 0 || on.feature.0 >= next_feature_id {
        return Err(format!(
            "it is pinned to feature {}, an id that was never given out",
            on.feature
        ));
    }
    match (&on.name, on.kind) {
        (None, NoteKind::Body) => Ok(()),
        (Some(_), NoteKind::Body) => Err("it is pinned to a body, which has no name".into()),
        (None, kind) => Err(format!("it is pinned to a {} without a name", kind.name())),
        (Some(name), kind) => {
            name.check()
                .map_err(|reason| format!("`{name}` is not a name: {reason}"))?;
            let fits = match kind {
                NoteKind::Face => name.is_face(),
                NoteKind::Edge => name.is_edge(),
                NoteKind::Vertex => name.is_vertex(),
                NoteKind::Body => false,
            };
            if fits {
                Ok(())
            } else {
                Err(format!("`{name}` is not the name of a {}", kind.name()))
            }
        }
    }
}

/// Why `point` cannot be where a note is.
pub fn check_note_point(point: [f64; 3]) -> Result<(), String> {
    if point.iter().all(|c| c.is_finite()) {
        Ok(())
    } else {
        Err("a note's point must be three finite numbers".to_string())
    }
}

impl Document {
    /// The note with id `id`.
    pub fn note(&self, id: u64) -> Option<&Note> {
        self.notes.items.iter().find(|n| n.id == id)
    }

    /// Adds a note at the end with the next id, and returns that id. The
    /// text is kept as [`clean_note_text`] gives it, who wrote it as
    /// [`clean_note_author`] does.
    pub fn add_note(
        &mut self,
        text: &str,
        on: Option<NoteOn>,
        point: [f64; 3],
        by: Option<&str>,
        time_ms: u64,
    ) -> Result<u64, DocError> {
        if self.notes.items.len() >= MAX_NOTES {
            return Err(DocError::Invalid(format!(
                "a document holds at most {MAX_NOTES} notes; delete some first (note.delete with state: done takes away those done)"
            )));
        }
        let text = clean_note_text(text).map_err(DocError::Invalid)?;
        check_note_point(point).map_err(DocError::Invalid)?;
        if let Some(on) = &on {
            check_on(on, self.next_feature_id)
                .map_err(|e| DocError::Invalid(format!("a new note: {e}")))?;
        }
        let id = self.notes.next_id.max(1);
        let next = id
            .checked_add(1)
            .ok_or_else(|| DocError::Invalid("no note id is left to give out".to_string()))?;
        self.notes.items.push(Note {
            id,
            text,
            on,
            point,
            by: by.and_then(clean_note_author),
            time_ms,
            done: None,
        });
        self.notes.next_id = next;
        Ok(id)
    }

    fn note_slot(&mut self, id: u64) -> Result<&mut Note, DocError> {
        self.notes
            .items
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or_else(|| DocError::Invalid(format!("there is no note {id}")))
    }

    /// Gives note `id` a new text.
    pub fn edit_note_text(&mut self, id: u64, text: &str) -> Result<(), DocError> {
        let text = clean_note_text(text).map_err(DocError::Invalid)?;
        self.note_slot(id)?.text = text;
        Ok(())
    }

    /// Pins note `id` elsewhere: to `on`, at `point`.
    pub fn repin_note(
        &mut self,
        id: u64,
        on: Option<NoteOn>,
        point: [f64; 3],
    ) -> Result<(), DocError> {
        check_note_point(point).map_err(DocError::Invalid)?;
        if let Some(on) = &on {
            check_on(on, self.next_feature_id)
                .map_err(|e| DocError::Invalid(format!("note {id}: {e}")))?;
        }
        let note = self.note_slot(id)?;
        note.on = on;
        note.point = point;
        Ok(())
    }

    /// Marks note `id` done by `by` (an agent's name, `None` for the
    /// person) with `reply`, or open again (`done` false: who closed it
    /// and the reply are taken away).
    pub fn resolve_note(
        &mut self,
        id: u64,
        done: bool,
        by: Option<&str>,
        reply: Option<&str>,
        time_ms: u64,
    ) -> Result<(), DocError> {
        let reply = reply
            .map(|r| {
                clean_note_text(r).map_err(|e| DocError::Invalid(e.replace("note's text", "reply")))
            })
            .transpose()?;
        let note = self.note_slot(id)?;
        if done {
            note.done = Some(NoteDone {
                by: by.and_then(clean_note_author),
                time_ms,
                reply,
            });
        } else {
            note.done = None;
        }
        Ok(())
    }

    /// Takes the notes `ids` away and returns them, in the order they
    /// were written. Every id must be a note's; then all go, or none.
    pub fn remove_notes(&mut self, ids: &[u64]) -> Result<Vec<Note>, DocError> {
        if let Some(missing) = ids.iter().find(|id| self.note(**id).is_none()) {
            return Err(DocError::Invalid(format!("there is no note {missing}")));
        }
        let (gone, kept): (Vec<Note>, Vec<Note>) = std::mem::take(&mut self.notes.items)
            .into_iter()
            .partition(|n| ids.contains(&n.id));
        self.notes.items = kept;
        Ok(gone)
    }

    /// The document without its notes, as anything published holds it
    /// (a Jig, a bundled example), and how many there were.
    pub fn without_notes(&self) -> (Document, usize) {
        let mut copy = self.clone();
        let count = copy.notes.items.len();
        copy.notes = Notes::default();
        (copy, count)
    }

    pub(crate) fn validate_notes(&self) -> Result<(), DocError> {
        let notes = &self.notes.items;
        if notes.len() > MAX_NOTES {
            return Err(DocError::Invalid(format!(
                "the document has {} notes; at most {MAX_NOTES} are read",
                notes.len()
            )));
        }
        let mut last = 0u64;
        for note in notes {
            let bad = |reason: String| DocError::Invalid(format!("note {}: {reason}", note.id));
            if note.id <= last {
                return Err(bad(
                    "the notes are not in ascending order of their ids, or one is 0 or listed twice"
                        .to_string(),
                ));
            }
            last = note.id;
            if note.id >= self.notes.next_id.max(1) {
                return Err(bad(format!(
                    "its id is not below the next id to give out ({})",
                    self.notes.next_id
                )));
            }
            check_text(&note.text, "its text").map_err(bad)?;
            check_note_point(note.point).map_err(bad)?;
            if let Some(on) = &note.on {
                check_on(on, self.next_feature_id).map_err(bad)?;
            }
            if let Some(by) = &note.by {
                check_author(by, "who wrote it").map_err(bad)?;
            }
            if let Some(done) = &note.done {
                if let Some(by) = &done.by {
                    check_author(by, "who closed it").map_err(bad)?;
                }
                if let Some(reply) = &done.reply {
                    check_text(reply, "its reply").map_err(bad)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::{Block, Expr, FeatureKind, NameRole, Orientation, Units};

    fn face(feature: u64, n: u64) -> NoteOn {
        NoteOn {
            kind: NoteKind::Face,
            feature: FeatureId(feature),
            name: Some(PersistentName::new(
                FeatureId(feature),
                NameRole::Face,
                vec![n],
            )),
        }
    }

    fn document() -> Document {
        let mut doc = Document::new(Units::default());
        doc.add_feature(
            Some("Plate"),
            FeatureKind::Block(Block {
                position: [Expr::new("0"), Expr::new("0"), Expr::new("0")],
                size: [Expr::new("10"), Expr::new("20"), Expr::new("5")],
                orientation: Orientation::default(),
            }),
        )
        .unwrap();
        doc
    }

    #[test]
    fn texts_are_kept_clean_and_bounded() {
        assert_eq!(
            clean_note_text("  2 mm thicker here \n").unwrap(),
            "2 mm thicker here"
        );
        assert_eq!(clean_note_text("a\r\nb\rc\td").unwrap(), "a\nb\nc d");
        assert_eq!(clean_note_text("a\u{202E}b\u{200B}").unwrap(), "ab");
        for bad in ["", " \n\t ", "a\u{7}b", "a\0b", "\u{202E}"] {
            assert!(clean_note_text(bad).is_err(), "{bad:?}");
        }
        assert!(clean_note_text(&"x".repeat(MAX_NOTE_CHARS)).is_ok());
        assert!(clean_note_text(&"x".repeat(MAX_NOTE_CHARS + 1)).is_err());
        let megabyte = "é".repeat(1 << 20);
        let e = clean_note_text(&megabyte).unwrap_err();
        assert!(e.contains("at most 2000"), "{e}");
        assert_eq!(
            clean_note_author(" Claude\nCode ").as_deref(),
            Some("Claude Code")
        );
        assert_eq!(clean_note_author("\u{202E}"), None);
        assert_eq!(
            clean_note_author(&"a".repeat(1000))
                .unwrap()
                .chars()
                .count(),
            MAX_NOTE_AUTHOR_CHARS
        );
    }

    #[test]
    fn notes_are_added_edited_resolved_and_removed_and_ids_are_never_reused() {
        let mut doc = document();
        let id = doc.features[0].id.0;
        let a = doc
            .add_note("thicker", Some(face(id, 1)), [0.0, 0.0, 5.0], None, 7)
            .unwrap();
        let b = doc
            .add_note(
                "why?",
                Some(face(id, 0)),
                [1.0, 1.0, 0.0],
                Some("Claude Code"),
                8,
            )
            .unwrap();
        assert_eq!((a, b), (1, 2));
        assert!(doc.validate().is_ok());
        doc.edit_note_text(a, " 2 mm thicker ").unwrap();
        assert_eq!(doc.note(a).unwrap().text, "2 mm thicker");
        doc.resolve_note(a, true, Some("Claude Code"), Some("Done: 7 mm now."), 9)
            .unwrap();
        assert_eq!(doc.notes.open(), 1);
        assert_eq!(
            doc.note(a).unwrap().done.as_ref().unwrap().reply.as_deref(),
            Some("Done: 7 mm now.")
        );
        doc.resolve_note(a, false, None, None, 10).unwrap();
        assert!(doc.note(a).unwrap().done.is_none());
        assert!(doc.remove_notes(&[b, 99]).is_err());
        assert_eq!(doc.notes.items.len(), 2, "all or none");
        assert_eq!(doc.remove_notes(&[b]).unwrap().len(), 1);
        let c = doc.add_note("again", None, [0.0; 3], None, 11).unwrap();
        assert_eq!(c, 3, "a deleted note's id is not given out again");
        assert!(doc.validate().is_ok());
        let (published, count) = doc.without_notes();
        assert_eq!(count, 2);
        assert!(published.notes.is_empty() && published.notes == Notes::default());
    }

    #[test]
    fn hostile_notes_are_refused_and_a_bad_file_is_too() {
        let mut doc = document();
        let id = doc.features[0].id.0;
        assert!(
            doc.add_note("x", Some(face(id, 1)), [f64::NAN, 0.0, 0.0], None, 0)
                .is_err()
        );
        assert!(
            doc.add_note("x", Some(face(id + 5, 1)), [0.0; 3], None, 0)
                .is_err()
        );
        let mut wrong = face(id, 1);
        wrong.kind = NoteKind::Edge;
        assert!(doc.add_note("x", Some(wrong), [0.0; 3], None, 0).is_err());
        assert!(
            doc.add_note("", Some(face(id, 1)), [0.0; 3], None, 0)
                .is_err()
        );
        assert!(doc.edit_note_text(5, "x").is_err());
        assert!(doc.resolve_note(5, true, None, None, 0).is_err());
        assert!(doc.notes.is_empty());
        doc.add_note("x", Some(face(id, 1)), [0.0; 3], None, 0)
            .unwrap();
        // As a file could hold it: each breaks validation.
        let breaks: [fn(&mut Document); 6] = [
            |d| d.notes.next_id = 1,
            |d| d.notes.items[0].text = " x".into(),
            |d| d.notes.items[0].point[2] = f64::INFINITY,
            |d| d.notes.items[0].by = Some("a\nb".into()),
            |d| d.notes.items.push(d.notes.items[0].clone()),
            |d| d.notes.items[0].on.as_mut().unwrap().name = None,
        ];
        for (k, broken) in breaks.iter().enumerate() {
            let mut bad = doc.clone();
            broken(&mut bad);
            assert!(bad.validate().is_err(), "case {k}");
        }
        for _ in 1..MAX_NOTES {
            doc.add_note("x", None, [0.0; 3], None, 0).unwrap();
        }
        let e = doc.add_note("x", None, [0.0; 3], None, 0).unwrap_err();
        assert!(e.to_string().contains("at most 1000"), "{e}");
    }
}
