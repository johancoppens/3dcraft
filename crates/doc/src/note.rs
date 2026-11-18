//! Notes pinned to the model (2026-11-17): a sentence written at a place
//! of the model ("2 mm thicker here", "this hole can go", or an agent's
//! question about that place), which whoever works on the model next
//! reads, works through and marks done with a sentence of reply.
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
//! Who wrote a note, changed its text and closed it is kept as how it
//! came in ([`NoteAuthor`]): typed by the person in the window, sent by an
//! agent under the name its client gave, or over a channel that says no
//! one (a script, the command line): never a proof of who wrote it, as a
//! file can be edited by hand.
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
/// Longest text of a note, and of a reply, in Unicode code points.
pub const MAX_NOTE_CHARS: usize = 2000;
/// Longest name of an agent that wrote, changed or closed a note, in
/// Unicode code points.
pub const MAX_NOTE_AUTHOR_CHARS: usize = 200;
/// Most replies a note takes after it was closed.
pub const MAX_NOTE_REPLIES: usize = 20;
/// Most bytes of text the notes of a document hold together, in UTF-8:
/// their texts, replies and the names of those who wrote, changed, closed
/// and replied ([`Note::bytes`]). A document at this cap always saves:
/// JSON at most doubles a note's text (a quote, a backslash or a line
/// break is written as two bytes; nothing else is escaped, and other
/// characters are written as their UTF-8), so the notes' strings take at
/// most 4 MiB of the manifest's 64 MiB, and what the manifest writes
/// around them (keys, numbers, points, names) less than 1 KiB a note and
/// 200 bytes a reply: under 10 MiB for 1000 notes of 20 replies each. Each
/// change to the notes is an undo step that keeps a copy of them all: at
/// the cap about 2 MiB a step.
pub const MAX_NOTES_BYTES: usize = 2 * 1024 * 1024;

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

/// How a note, its new text or its closing came in: what the program
/// knew of who sent it, not a proof of who wrote it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteAuthor {
    /// Typed by the person at the window.
    Person,
    /// Sent by an agent, under the name its client gave (the agent log's);
    /// empty for an agent that gave none (a control connection of the
    /// window without a name, an MCP client without one).
    Agent(String),
    /// Over a channel that is not an agent's: a script, the command line,
    /// `serve`.
    Unknown,
}

impl NoteAuthor {
    /// `person`, `agent` or `unknown`, as the commands say it.
    pub fn word(&self) -> &'static str {
        match self {
            NoteAuthor::Person => "person",
            NoteAuthor::Agent(_) => "agent",
            NoteAuthor::Unknown => "unknown",
        }
    }

    /// The agent's name, for an agent that gave one.
    pub fn agent(&self) -> Option<&str> {
        match self {
            NoteAuthor::Agent(name) if !name.is_empty() => Some(name),
            NoteAuthor::Agent(_) | NoteAuthor::Person | NoteAuthor::Unknown => None,
        }
    }

    /// An agent named `name` as a note keeps it ([`clean_note_author`]);
    /// an agent without a name when nothing of the name is left (it is an
    /// agent's channel all the same).
    pub fn agent_named(name: &str) -> NoteAuthor {
        NoteAuthor::Agent(clean_note_author(name).unwrap_or_default())
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

/// Who did something to a note, and when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteStamp {
    pub by: NoteAuthor,
    /// Milliseconds since the Unix epoch (0 where there is no clock).
    pub time_ms: u64,
}

/// Who closed a note, when, and what they answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteDone {
    pub by: NoteAuthor,
    /// Milliseconds since the Unix epoch (0 where there is no clock).
    pub time_ms: u64,
    pub reply: Option<String>,
}

/// A reply given to a note after it was closed: who, when, what.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteReply {
    pub by: NoteAuthor,
    /// Milliseconds since the Unix epoch (0 where there is no clock).
    pub time_ms: u64,
    pub text: String,
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
    /// How it came in.
    pub by: NoteAuthor,
    /// When it was written: milliseconds since the Unix epoch (0 where
    /// there is no clock).
    pub time_ms: u64,
    /// Who last changed its text, and when; `None` while it has the text
    /// it was written with.
    pub edited: Option<NoteStamp>,
    /// `None` while it is open.
    pub done: Option<NoteDone>,
    /// The replies given to it after it was closed (an agent's account of
    /// what it did when the person closed the note first), in the order
    /// they came; kept when the note is opened again. At most
    /// [`MAX_NOTE_REPLIES`].
    #[serde(default)]
    pub later: Vec<NoteReply>,
}

impl Note {
    /// The bytes of text it holds, in UTF-8: its text, its reply and the
    /// replies after its closing, and the names of the agents that wrote,
    /// changed, closed it or replied.
    pub fn bytes(&self) -> usize {
        let name = |by: &NoteAuthor| by.agent().map_or(0, str::len);
        let mut sum = self.text.len().saturating_add(name(&self.by));
        if let Some(edited) = &self.edited {
            sum = sum.saturating_add(name(&edited.by));
        }
        if let Some(done) = &self.done {
            sum = sum
                .saturating_add(name(&done.by))
                .saturating_add(done.reply.as_ref().map_or(0, String::len));
        }
        for reply in &self.later {
            sum = sum
                .saturating_add(name(&reply.by))
                .saturating_add(reply.text.len());
        }
        sum
    }
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

    /// How many are open that an agent wrote: its questions.
    pub fn open_from_agents(&self) -> usize {
        self.items
            .iter()
            .filter(|n| n.done.is_none() && matches!(n.by, NoteAuthor::Agent(_)))
            .count()
    }

    /// The bytes of text they hold together ([`Note::bytes`]).
    pub fn bytes(&self) -> usize {
        self.items
            .iter()
            .fold(0usize, |sum, n| sum.saturating_add(n.bytes()))
    }
}

/// A note's text, or a reply, as it is kept: trimmed, line breaks as
/// `\n` (a carriage return, CR LF, and the line and paragraph separators
/// U+2028 and U+2029 read as one), a tab as a space, and the
/// bidirectional formatting characters left out ([`deceives`]: they make
/// a text read otherwise where it is shown); every other character as it
/// was (zero-width joiners and non-joiners included). Refused: empty,
/// longer than [`MAX_NOTE_CHARS`] Unicode code points, or holding another
/// control character.
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
            '\u{2028}' | '\u{2029}' => '\n',
            '\t' => ' ',
            c if deceives(c) => continue,
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
            "a note's text is at most {MAX_NOTE_CHARS} Unicode code points long (this one has {})",
            text.chars().count()
        ));
    }
    Ok(trimmed.to_string())
}

/// An agent's name as a note keeps it: one line of at most
/// [`MAX_NOTE_AUTHOR_CHARS`] Unicode code points, control characters made
/// spaces and the bidirectional formatting characters left out;
/// `None` when nothing is left.
pub fn clean_note_author(name: &str) -> Option<String> {
    let flat: String = name
        .chars()
        .filter(|c| !deceives(*c) && !unseen(*c))
        .map(|c| {
            if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                ' '
            } else {
                c
            }
        })
        .take(MAX_NOTE_AUTHOR_CHARS.saturating_mul(2))
        .collect();
    let flat: String = flat.trim().chars().take(MAX_NOTE_AUTHOR_CHARS).collect();
    let flat = flat.trim_end().to_string();
    (!flat.is_empty()).then_some(flat)
}

/// A character a name shows nothing of: the soft hyphen (U+00AD), the
/// Mongolian vowel separator (U+180E), the zero-width space, non-joiner
/// and joiner (U+200B to U+200D), the word joiner and invisible
/// operators (U+2060 to U+2064), the byte-order mark (U+FEFF), the
/// interlinear annotation marks (U+FFF9 to U+FFFB) and the tag characters
/// (U+E0001, U+E0020 to U+E007F). An author's name leaves them out, so
/// `Claude\u{200B} Code` cannot pass for `Claude Code` (a note's text
/// keeps joiners: they spell words and emoji).
pub fn unseen(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200D}'
            | '\u{2060}'..='\u{2064}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// A bidirectional formatting character: the marks (U+061C, U+200E,
/// U+200F), the embeddings and overrides (U+202A to U+202E) and the
/// isolates (U+2066 to U+2069). Shown, they turn the text around them,
/// so a note could read otherwise than it is; a note leaves them out.
pub fn deceives(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

fn check_author(who: &NoteAuthor, what: &str) -> Result<(), String> {
    // An empty name is an agent that gave none.
    if let NoteAuthor::Agent(name) = who
        && !name.is_empty()
        && clean_note_author(name).as_deref() != Some(name.as_str())
    {
        return Err(format!(
            "{what} must be one line of 1 to {MAX_NOTE_AUTHOR_CHARS} Unicode code points without control or bidirectional formatting characters"
        ));
    }
    Ok(())
}

fn check_text(text: &str, what: &str) -> Result<(), String> {
    match clean_note_text(text) {
        Ok(clean) if clean == text => Ok(()),
        Ok(_) => Err(format!(
            "{what} is not as a note keeps it (trimmed, line breaks as \\n, no tab, no bidirectional formatting characters)"
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
    /// text is kept as [`clean_note_text`] gives it.
    pub fn add_note(
        &mut self,
        text: &str,
        on: Option<NoteOn>,
        point: [f64; 3],
        by: NoteAuthor,
        time_ms: u64,
    ) -> Result<u64, DocError> {
        if self.notes.items.len() >= MAX_NOTES {
            return Err(DocError::Invalid(format!(
                "a document holds at most {MAX_NOTES} notes; delete some first (note.delete with state: done takes away those done)"
            )));
        }
        let text = clean_note_text(text).map_err(DocError::Invalid)?;
        check_note_point(point).map_err(DocError::Invalid)?;
        check_author(&by, "who wrote it").map_err(DocError::Invalid)?;
        if let Some(on) = &on {
            check_on(on, self.next_feature_id)
                .map_err(|e| DocError::Invalid(format!("a new note: {e}")))?;
        }
        let id = self.notes.next_id.max(1);
        let next = id
            .checked_add(1)
            .ok_or_else(|| DocError::Invalid("no note id is left to give out".to_string()))?;
        let note = Note {
            id,
            text,
            on,
            point,
            by,
            time_ms,
            edited: None,
            done: None,
            later: Vec::new(),
        };
        self.notes_fit(None, &note)?;
        self.notes.items.push(note);
        self.notes.next_id = next;
        Ok(id)
    }

    /// Whether the notes still fit in [`MAX_NOTES_BYTES`] with `changed` in
    /// place of note `id` (`None`: added); refused with what to do.
    fn notes_fit(&self, id: Option<u64>, changed: &Note) -> Result<(), DocError> {
        let now = self.notes.bytes();
        let was = id.and_then(|id| self.note(id)).map_or(0, Note::bytes);
        let after = now.saturating_sub(was).saturating_add(changed.bytes());
        if after > MAX_NOTES_BYTES && after > now {
            return Err(DocError::Invalid(format!(
                "the notes of a document hold at most {MAX_NOTES_BYTES} bytes of text together (so it always saves), and this would make them {after} ({now} now): shorten it, or delete notes that are done (note.delete with state: done)"
            )));
        }
        Ok(())
    }

    fn note_slot(&mut self, id: u64) -> Result<&mut Note, DocError> {
        self.notes
            .items
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or_else(|| DocError::Invalid(format!("there is no note {id}")))
    }

    /// Gives note `id` a new text, noting who gave it and when; a text the
    /// same as the one it has changes nothing.
    pub fn edit_note_text(
        &mut self,
        id: u64,
        text: &str,
        by: NoteAuthor,
        time_ms: u64,
    ) -> Result<(), DocError> {
        let text = clean_note_text(text).map_err(DocError::Invalid)?;
        check_author(&by, "who changed it").map_err(DocError::Invalid)?;
        let mut changed = self
            .note(id)
            .cloned()
            .ok_or_else(|| DocError::Invalid(format!("there is no note {id}")))?;
        if changed.text == text {
            return Ok(());
        }
        changed.text = text;
        changed.edited = Some(NoteStamp { by, time_ms });
        self.notes_fit(Some(id), &changed)?;
        *self.note_slot(id)? = changed;
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

    /// Marks note `id` done by `by` with `reply`, or open again (`done`
    /// false: who closed it and the reply are taken away; the replies given
    /// after its closing stay).
    pub fn resolve_note(
        &mut self,
        id: u64,
        done: bool,
        by: NoteAuthor,
        reply: Option<&str>,
        time_ms: u64,
    ) -> Result<(), DocError> {
        let reply = reply
            .map(|r| {
                clean_note_text(r).map_err(|e| DocError::Invalid(e.replace("note's text", "reply")))
            })
            .transpose()?;
        check_author(&by, "who closed it").map_err(DocError::Invalid)?;
        let mut changed = self
            .note(id)
            .cloned()
            .ok_or_else(|| DocError::Invalid(format!("there is no note {id}")))?;
        changed.done = done.then_some(NoteDone { by, time_ms, reply });
        self.notes_fit(Some(id), &changed)?;
        *self.note_slot(id)? = changed;
        Ok(())
    }

    /// Adds `reply` to note `id`, which is done, as given by `by` at
    /// `time_ms`: kept with the note after its closing, the closing as it
    /// was.
    pub fn reply_to_note(
        &mut self,
        id: u64,
        by: NoteAuthor,
        reply: &str,
        time_ms: u64,
    ) -> Result<(), DocError> {
        let text = clean_note_text(reply)
            .map_err(|e| DocError::Invalid(e.replace("note's text", "reply")))?;
        check_author(&by, "who replied").map_err(DocError::Invalid)?;
        let mut changed = self
            .note(id)
            .cloned()
            .ok_or_else(|| DocError::Invalid(format!("there is no note {id}")))?;
        if changed.done.is_none() {
            return Err(DocError::Invalid(format!(
                "note {id} is open: a reply closes it (note.resolve)"
            )));
        }
        if changed.later.len() >= MAX_NOTE_REPLIES {
            return Err(DocError::Invalid(format!(
                "note {id} has {MAX_NOTE_REPLIES} replies after its closing already"
            )));
        }
        changed.later.push(NoteReply { by, time_ms, text });
        self.notes_fit(Some(id), &changed)?;
        *self.note_slot(id)? = changed;
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
            check_author(&note.by, "who wrote it").map_err(bad)?;
            if let Some(edited) = &note.edited {
                check_author(&edited.by, "who changed it").map_err(bad)?;
            }
            if let Some(done) = &note.done {
                check_author(&done.by, "who closed it").map_err(bad)?;
                if let Some(reply) = &done.reply {
                    check_text(reply, "its reply").map_err(bad)?;
                }
            }
            if note.later.len() > MAX_NOTE_REPLIES {
                return Err(bad(format!(
                    "it has more than {MAX_NOTE_REPLIES} replies after its closing"
                )));
            }
            for later in &note.later {
                check_author(&later.by, "who replied").map_err(bad)?;
                check_text(&later.text, "a reply").map_err(bad)?;
            }
        }
        let bytes = self.notes.bytes();
        if bytes > MAX_NOTES_BYTES {
            return Err(DocError::Invalid(format!(
                "the notes hold {bytes} bytes of text together; at most {MAX_NOTES_BYTES} are read"
            )));
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

    fn agent() -> NoteAuthor {
        NoteAuthor::Agent("Claude Code".into())
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
        assert_eq!(
            clean_note_text("a\r\nb\rc\td\u{2028}e").unwrap(),
            "a\nb\nc d\ne"
        );
        // Bidirectional formatting goes; joiners, non-joiners and the
        // other format characters stay (Persian spelling, emoji
        // sequences).
        assert_eq!(
            clean_note_text("a\u{202E}b\u{2066}c\u{200F}").unwrap(),
            "abc"
        );
        let kept = "می\u{200C}خواهم 👩\u{200D}🔧 a\u{200B}b";
        assert_eq!(clean_note_text(kept).unwrap(), kept);
        for bad in ["", " \n\t ", "a\u{7}b", "a\0b", "\u{202E}"] {
            assert!(clean_note_text(bad).is_err(), "{bad:?}");
        }
        assert!(clean_note_text(&"x".repeat(MAX_NOTE_CHARS)).is_ok());
        assert!(clean_note_text(&"x".repeat(MAX_NOTE_CHARS + 1)).is_err());
        // Code points, not what is drawn: 1000 e with a combining acute
        // are 2000 code points.
        assert!(clean_note_text(&"e\u{301}".repeat(MAX_NOTE_CHARS / 2)).is_ok());
        assert!(clean_note_text(&"e\u{301}".repeat(MAX_NOTE_CHARS / 2 + 1)).is_err());
        let megabyte = "é".repeat(1 << 20);
        let e = clean_note_text(&megabyte).unwrap_err();
        assert!(e.contains("at most 2000 Unicode code points"), "{e}");
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
        // A name of nothing to see is no name: an agent without one (the
        // developer, 2026-11-17: an agent's channel is an agent's); one
        // with a hidden space reads as what it shows.
        let unnamed = NoteAuthor::Agent(String::new());
        assert_eq!(NoteAuthor::agent_named("\u{202E}"), unnamed);
        assert_eq!(NoteAuthor::agent_named("\u{200B}"), unnamed);
        assert_eq!(unnamed.word(), "agent");
        assert_eq!(unnamed.agent(), None);
        assert_eq!(
            clean_note_author("Claude\u{200B} Code").as_deref(),
            Some("Claude Code")
        );
    }

    #[test]
    fn notes_are_added_edited_resolved_and_removed_and_ids_are_never_reused() {
        let mut doc = document();
        let id = doc.features[0].id.0;
        let a = doc
            .add_note(
                "thicker",
                Some(face(id, 1)),
                [0.0, 0.0, 5.0],
                NoteAuthor::Person,
                7,
            )
            .unwrap();
        let b = doc
            .add_note("why?", Some(face(id, 0)), [1.0, 1.0, 0.0], agent(), 8)
            .unwrap();
        assert_eq!((a, b), (1, 2));
        assert!(doc.validate().is_ok());
        // The same text changes nothing; another is stamped with who
        // gave it.
        doc.edit_note_text(a, " thicker ", agent(), 9).unwrap();
        assert_eq!(doc.note(a).unwrap().edited, None);
        doc.edit_note_text(a, "2 mm thicker", agent(), 9).unwrap();
        assert_eq!(doc.note(a).unwrap().text, "2 mm thicker");
        assert_eq!(doc.note(a).unwrap().by, NoteAuthor::Person);
        assert_eq!(
            doc.note(a).unwrap().edited,
            Some(NoteStamp {
                by: agent(),
                time_ms: 9
            })
        );
        doc.resolve_note(a, true, agent(), Some("Done: 7 mm now."), 9)
            .unwrap();
        assert_eq!(doc.notes.open(), 1);
        assert_eq!(doc.notes.open_from_agents(), 1);
        assert_eq!(
            doc.note(a).unwrap().done.as_ref().unwrap().reply.as_deref(),
            Some("Done: 7 mm now.")
        );
        doc.resolve_note(a, false, NoteAuthor::Unknown, None, 10)
            .unwrap();
        assert!(doc.note(a).unwrap().done.is_none());
        assert!(doc.remove_notes(&[b, 99]).is_err());
        assert_eq!(doc.notes.items.len(), 2, "all or none");
        assert_eq!(doc.remove_notes(&[b]).unwrap().len(), 1);
        let c = doc
            .add_note("again", None, [0.0; 3], NoteAuthor::Unknown, 11)
            .unwrap();
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
        let unknown = || NoteAuthor::Unknown;
        assert!(
            doc.add_note("x", Some(face(id, 1)), [f64::NAN, 0.0, 0.0], unknown(), 0)
                .is_err()
        );
        assert!(
            doc.add_note("x", Some(face(id + 5, 1)), [0.0; 3], unknown(), 0)
                .is_err()
        );
        let mut wrong = face(id, 1);
        wrong.kind = NoteKind::Edge;
        assert!(
            doc.add_note("x", Some(wrong), [0.0; 3], unknown(), 0)
                .is_err()
        );
        assert!(
            doc.add_note("", Some(face(id, 1)), [0.0; 3], unknown(), 0)
                .is_err()
        );
        assert!(
            doc.add_note("x", None, [0.0; 3], NoteAuthor::Agent("a\nb".into()), 0)
                .is_err()
        );
        assert!(doc.edit_note_text(5, "x", unknown(), 0).is_err());
        assert!(doc.resolve_note(5, true, unknown(), None, 0).is_err());
        assert!(doc.notes.is_empty());
        doc.add_note("x", Some(face(id, 1)), [0.0; 3], unknown(), 0)
            .unwrap();
        // As a file could hold it: each breaks validation.
        // (An empty agent's name is an agent that gave none, valid.)
        let breaks: [fn(&mut Document); 8] = [
            |d| d.notes.next_id = 1,
            |d| d.notes.items[0].text = " x".into(),
            |d| d.notes.items[0].point[2] = f64::INFINITY,
            |d| d.notes.items[0].by = NoteAuthor::Agent("a\u{202E}b".into()),
            |d| d.notes.items.push(d.notes.items[0].clone()),
            |d| d.notes.items[0].on.as_mut().unwrap().name = None,
            |d| {
                d.notes.items[0].edited = Some(NoteStamp {
                    by: NoteAuthor::Agent(" padded".into()),
                    time_ms: 0,
                })
            },
            |d| {
                d.notes.items[0].done = Some(NoteDone {
                    by: NoteAuthor::Person,
                    time_ms: 0,
                    reply: None,
                });
                d.notes.items[0].later = vec![NoteReply {
                    by: NoteAuthor::Person,
                    time_ms: 0,
                    text: " x".into(),
                }];
            },
        ];
        for (k, broken) in breaks.iter().enumerate() {
            let mut bad = doc.clone();
            broken(&mut bad);
            assert!(bad.validate().is_err(), "case {k}");
        }
        for _ in 1..MAX_NOTES {
            doc.add_note("x", None, [0.0; 3], unknown(), 0).unwrap();
        }
        let e = doc.add_note("x", None, [0.0; 3], unknown(), 0).unwrap_err();
        assert!(e.to_string().contains("at most 1000"), "{e}");
    }

    /// A reply after the closing goes to a done note only, at most
    /// `MAX_NOTE_REPLIES`, the closing kept; an agent without a name is a
    /// valid author.
    #[test]
    fn replies_after_the_closing() {
        let mut doc = document();
        let unnamed = NoteAuthor::Agent(String::new());
        let id = doc
            .add_note("x", None, [0.0; 3], unnamed.clone(), 1)
            .unwrap();
        assert!(doc.reply_to_note(id, agent(), "y", 2).is_err(), "open");
        doc.resolve_note(id, true, NoteAuthor::Person, None, 3)
            .unwrap();
        for k in 0..MAX_NOTE_REPLIES {
            doc.reply_to_note(id, unnamed.clone(), &format!("reply {k}"), 4)
                .unwrap();
        }
        assert!(doc.reply_to_note(id, agent(), "one more", 5).is_err());
        assert!(doc.reply_to_note(99, agent(), "y", 5).is_err());
        let note = doc.note(id).unwrap().clone();
        assert_eq!(
            note.done.unwrap().by,
            NoteAuthor::Person,
            "the closing kept"
        );
        assert_eq!(note.later.len(), MAX_NOTE_REPLIES);
        doc.validate().unwrap();
        // Opened again, the replies stay (review-pins 2b); closed again,
        // they are still there.
        doc.resolve_note(id, false, NoteAuthor::Person, None, 6)
            .unwrap();
        assert!(doc.note(id).unwrap().done.is_none());
        assert_eq!(doc.note(id).unwrap().later.len(), MAX_NOTE_REPLIES);
        doc.resolve_note(id, true, agent(), Some("again"), 7)
            .unwrap();
        assert_eq!(doc.note(id).unwrap().later.len(), MAX_NOTE_REPLIES);
    }

    /// The notes' text together stays within `MAX_NOTES_BYTES`, so a
    /// document full of them still saves (review-pins 2b: 400 notes of 20
    /// replies of 2000 four-byte characters made a 71.9 MB manifest):
    /// adding, changing, closing and replying past it is refused with
    /// what to do, a file past it is refused, and what shrinks the notes
    /// is taken at the cap.
    #[test]
    fn the_notes_text_together_is_capped() {
        let mut doc = document();
        let full = "\u{1F600}".repeat(MAX_NOTE_CHARS);
        let mut last = None;
        let refused = loop {
            match doc.add_note(&full, None, [0.0; 3], agent(), 0) {
                Ok(id) => last = Some(id),
                Err(e) => break e.to_string(),
            }
        };
        assert!(
            refused.contains("note.delete with state: done"),
            "{refused}"
        );
        let id = last.unwrap();
        let bytes = doc.notes.bytes();
        assert!(bytes <= MAX_NOTES_BYTES, "{bytes}");
        assert!(MAX_NOTES_BYTES - bytes < full.len() + 64, "{bytes}");
        doc.validate().unwrap();
        // Closing with a reply, replying, a longer text: refused.
        assert!(doc.resolve_note(id, true, agent(), Some(&full), 1).is_err());
        doc.resolve_note(id, true, NoteAuthor::Person, None, 1)
            .unwrap();
        assert!(doc.reply_to_note(id, agent(), &full, 2).is_err());
        // A shorter text is taken; so is deleting.
        doc.edit_note_text(id, "short", NoteAuthor::Person, 3)
            .unwrap();
        doc.remove_notes(&[id]).unwrap();
        // A file past the cap is refused.
        let mut bad = doc.clone();
        let first = bad.notes.items[0].clone();
        let mut next = bad.notes.next_id;
        for _ in 0..3 {
            let mut copy = first.clone();
            copy.id = next;
            next += 1;
            bad.notes.items.push(copy);
        }
        bad.notes.next_id = next;
        assert!(bad.validate().is_err());
        // At the cap the whole document copies, as an undo step keeps it,
        // in about this long.
        let started = std::time::Instant::now();
        let copy = doc.clone();
        println!(
            "the notes at {} bytes of text: one copy of the document {:?}",
            copy.notes.bytes(),
            started.elapsed()
        );
    }
}
