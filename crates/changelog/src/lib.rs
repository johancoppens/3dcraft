//! Release notes read from a changelog in the form of
//! [Keep a Changelog](https://keepachangelog.com/en/1.1.0/): 3DCraft's own
//! `CHANGELOG.md`, which the programs carry so that they can say after an
//! update what is new and which results may have changed (the card *New
//! in …* on the start screen, Help › What's new, `session.whatsNew`).
//!
//! A standalone crate: it depends on nothing, so a program's build script
//! reads the file with it ([`build::emit`]), the engine reads the embedded
//! text with it again at run time, and `xtask` holds its reading to the
//! Release workflow's.
//!
//! **One reading of the file.** The Release workflow takes a version's
//! notes with a small `awk` program: from the line that starts with
//! `## [<version>]` to the next line that starts with `## [`, or to the
//! link definitions at the end (`[0.1.0]: https://…`). [`release_section`]
//! is that reading, and [`Changelog::parse`] splits the file into its
//! sections the same way, so what the draft release says and what the
//! program shows cannot drift apart (an `xtask` test runs the workflow's
//! own `awk` program against [`release_section`]).
//!
//! **The form.** A section is `## [Unreleased]` or `## [1.2.3] - 2026-11-12`
//! (newest first, each version once), an optional paragraph that sums the
//! release up, and groups `### Added`, `### Changed`, `### Fixed`, … of
//! entries `- …`. An entry that begins with a bold sentence has that
//! sentence as its title. The group [`RESULTS_HEADING`] holds the entries
//! that change an answer: a measure, a proof, a check, or a body that was
//! built wrong while it looked right. Someone who relied on such an answer
//! must look again, so those come first wherever notes are shown.
//!
//! **Nothing under a group is dropped without a word.** What stands under
//! a group and is not an entry `- …` (a paragraph, a numbered list, a
//! table) is the group's own note ([`Note`]): the command's answer carries
//! it and the card shows it under the group's heading. Today that is the
//! sentence under [`RESULTS_HEADING`] that says what such an entry means.
//!
//! Everything is read as text a person can read: the Markdown markup
//! (bold, code, links) is taken off ([`plain`]). **A link keeps its words
//! and loses its address**: the notes are read in a window and by an
//! agent, neither of which follows a link from them, and an entry whose
//! point is an address should say the address in words (the full notes
//! with their links are the changelog on the website). Nothing here
//! panics, whatever the file holds, and the reading takes time in
//! proportion to the file's length; a file that is not in this form is an
//! error that names the line.

use std::fmt;

pub mod build;

/// The heading of the group whose entries change an answer.
pub const RESULTS_HEADING: &str = "Results that changed";

/// Most bytes of a changelog read.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Most entries kept, all sections together.
const MAX_ENTRIES: usize = 50_000;
/// Most characters of a version's text looked at.
const MAX_VERSION_CHARS: usize = 40;
/// The largest number a version's part may be.
const MAX_VERSION_PART: u32 = 999_999;
/// Most characters between a link's brackets or parentheses.
const MAX_BRACKETED_CHARS: usize = 1024;
/// Most bytes of a code span, of a reference link's name and of an
/// address between angle brackets: what closes them is looked for no
/// further, so text that opens such a thing over and over without closing
/// it is not looked through once per opening.
const MAX_SPAN_BYTES: usize = 4096;
/// How deep markup inside a link's text is followed.
const MAX_MARKUP_DEPTH: u8 = 4;

/// Why a changelog, or a version, could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// A released version: `major.minor.patch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Reads `1.2.3` (or `v1.2.3`): three whole numbers, nothing after
    /// them. The error says what was given, shortened.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let shown: String = text.chars().take(MAX_VERSION_CHARS).collect();
        let bad = || {
            Error::new(format!(
                "`{shown}` is not a version: a version is three numbers, as in 0.1.2"
            ))
        };
        if text.len() > MAX_VERSION_CHARS {
            return Err(bad());
        }
        let digits = text.strip_prefix('v').unwrap_or(text);
        let mut parts = digits.split('.');
        let mut part = || -> Result<u32, Error> {
            let word = parts.next().ok_or_else(bad)?;
            if word.is_empty() || !word.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            let number: u32 = word.parse().map_err(|_| bad())?;
            if number > MAX_VERSION_PART {
                return Err(bad());
            }
            Ok(number)
        };
        let version = Self {
            major: part()?,
            minor: part()?,
            patch: part()?,
        };
        if parts.next().is_some() {
            return Err(bad());
        }
        Ok(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What kind of change an entry is: the group it stands in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// It changes an answer ([`RESULTS_HEADING`]).
    Results,
    Added,
    Changed,
    Deprecated,
    Removed,
    Fixed,
    Security,
    /// *Known limitations*.
    Limitations,
    /// A group with a heading of its own (the first release's chapters).
    Other,
}

impl Kind {
    /// Every kind, in the order notes are shown: what changes an answer
    /// first, then what is new, then the rest.
    pub const ALL: [Kind; 9] = [
        Kind::Results,
        Kind::Added,
        Kind::Changed,
        Kind::Deprecated,
        Kind::Removed,
        Kind::Fixed,
        Kind::Security,
        Kind::Limitations,
        Kind::Other,
    ];

    /// The kind of the group with this heading.
    pub fn of_heading(heading: &str) -> Self {
        let heading = heading.trim();
        let is = |name: &str| heading.eq_ignore_ascii_case(name);
        if is(RESULTS_HEADING) {
            Kind::Results
        } else if is("Added") {
            Kind::Added
        } else if is("Changed") {
            Kind::Changed
        } else if is("Deprecated") {
            Kind::Deprecated
        } else if is("Removed") {
            Kind::Removed
        } else if is("Fixed") {
            Kind::Fixed
        } else if is("Security") {
            Kind::Security
        } else if is("Known limitations") {
            Kind::Limitations
        } else {
            Kind::Other
        }
    }

    /// The word data carries for it.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Results => "results",
            Kind::Added => "added",
            Kind::Changed => "changed",
            Kind::Deprecated => "deprecated",
            Kind::Removed => "removed",
            Kind::Fixed => "fixed",
            Kind::Security => "security",
            Kind::Limitations => "limitations",
            Kind::Other => "other",
        }
    }

    /// True for the entries that change an answer.
    pub fn changes_answer(self) -> bool {
        self == Kind::Results
    }
}

/// One change, as text without markup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    /// The heading of its group as the file writes it (`Fixed`).
    pub heading: String,
    /// Its first, bold sentence; without one, its first sentence.
    pub title: String,
    /// What follows the title; empty when the title says it all.
    pub text: String,
}

impl Entry {
    pub fn changes_answer(&self) -> bool {
        self.kind.changes_answer()
    }
}

/// What a group says of itself: the text under its heading that is not an
/// entry (a paragraph, a numbered list, a table), without markup, its
/// paragraphs joined by a line break.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub kind: Kind,
    /// The heading of the group as the file writes it.
    pub heading: String,
    pub text: String,
}

/// The notes of one version, or of what is not released yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// `None` for `## [Unreleased]`.
    pub version: Option<Version>,
    /// The day of the release as the heading writes it (`2026-11-12`).
    pub date: Option<String>,
    /// The paragraphs before the first group, joined by a line break.
    pub summary: String,
    pub entries: Vec<Entry>,
    /// What its groups say of themselves, in the file's order; a group
    /// with entries only has none.
    pub notes: Vec<Note>,
}

impl Section {
    /// True when the section says nothing: no summary, no entry and no
    /// group's note.
    pub fn is_blank(&self) -> bool {
        self.summary.is_empty() && self.entries.is_empty() && self.notes.is_empty()
    }

    /// What its groups of one kind say of themselves.
    pub fn notes_of(&self, kind: Kind) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(move |note| note.kind == kind)
    }

    /// How people name it: the version, or *Not yet released*.
    pub fn label(&self) -> String {
        match self.version {
            Some(version) => version.to_string(),
            None => "Not yet released".to_string(),
        }
    }

    /// Its entries of one kind, in the file's order.
    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |entry| entry.kind == kind)
    }

    /// How many of its entries change an answer.
    pub fn changing_answers(&self) -> usize {
        self.of_kind(Kind::Results).count()
    }
}

/// What a program knows of the notes its person has seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// Nothing was ever kept here: a first start. Nothing is old.
    Never,
    /// An earlier 3DCraft ran here that kept no note of its version (the
    /// 0.1 releases): which one is not known.
    Earlier,
    /// The newest version whose notes were seen.
    Version(Version),
}

/// A changelog: its sections, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Changelog {
    sections: Vec<Section>,
}

/// One section as the file has it: its heading line and the lines under
/// it, up to the next section or the link definitions.
struct Raw<'a> {
    /// The 1-based line of the heading.
    line: usize,
    heading: &'a str,
    lines: Vec<&'a str>,
}

/// The lines of `text` as `awk` reads records: split at line feeds, a
/// last line feed ending the last line rather than starting an empty one.
fn records(text: &str) -> impl Iterator<Item = &str> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    let empty = text.is_empty();
    body.split('\n').filter(move |_| !empty)
}

/// True for a link definition at the start of a line (`[0.1.0]: https…`):
/// the Release workflow's `/^\[[^]]+\]: /`.
fn is_link_definition(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('[') else {
        return false;
    };
    match rest.find(']') {
        Some(0) | None => false,
        Some(end) => rest.get(end..).is_some_and(|tail| tail.starts_with("]: ")),
    }
}

/// The file's sections, by the Release workflow's reading: a line that
/// starts with `## [` begins one, and it runs to the next such line or to
/// a link definition.
fn raw_sections(text: &str) -> Vec<Raw<'_>> {
    let mut sections: Vec<Raw<'_>> = Vec::new();
    let mut open = false;
    for (index, line) in records(text).enumerate() {
        if line.starts_with("## [") {
            sections.push(Raw {
                line: index.saturating_add(1),
                heading: line,
                lines: Vec::new(),
            });
            open = true;
        } else if is_link_definition(line) {
            open = false;
        } else if open && let Some(section) = sections.last_mut() {
            section.lines.push(line);
        }
    }
    sections
}

/// The notes of `version` exactly as the Release workflow puts them on a
/// draft release: the lines under the first heading that starts with
/// `## [<version>]`, each ended by a line feed. `None` when the file has
/// no such heading.
pub fn release_section(text: &str, version: &str) -> Option<String> {
    let head = format!("## [{version}]");
    let section = raw_sections(text)
        .into_iter()
        .find(|section| section.heading.starts_with(&head))?;
    let mut notes = String::new();
    for line in section.lines {
        notes.push_str(line);
        notes.push('\n');
    }
    Some(notes)
}

/// What a section's heading names: `None` for *Unreleased*, else the
/// version; and the date after ` - `, when there is one.
fn heading_of(raw: &Raw<'_>) -> Result<(Option<Version>, Option<String>), Error> {
    let at = raw.line;
    let rest = raw.heading.strip_prefix("## [").unwrap_or(raw.heading);
    let Some((name, after)) = rest.split_once(']') else {
        return Err(Error::new(format!(
            "line {at}: a section's heading is `## [<version>] - <date>` or `## [Unreleased]`; this one has no `]`"
        )));
    };
    let version = if name.eq_ignore_ascii_case("unreleased") {
        None
    } else {
        Some(Version::parse(name).map_err(|e| Error::new(format!("line {at}: {e}")))?)
    };
    let after = after.trim();
    let date = match after.strip_prefix('-') {
        Some(date) => {
            let date = date.trim();
            if date.is_empty() || date.chars().count() > 40 {
                return Err(Error::new(format!(
                    "line {at}: after ` - ` comes the day of the release, as in 2026-11-12"
                )));
            }
            Some(date.to_string())
        }
        None if after.is_empty() => None,
        None => {
            return Err(Error::new(format!(
                "line {at}: after `## [<version>]` comes ` - <date>` or nothing"
            )));
        }
    };
    Ok((version, date))
}

/// Splits an entry's source into its title and what follows: the bold
/// sentence it begins with, else its first sentence.
fn title_and_text(source: &str) -> (String, String) {
    let source = source.trim();
    if let Some(rest) = source.strip_prefix("**")
        && let Some((lead, after)) = rest.split_once("**")
        && !lead.trim().is_empty()
    {
        return (plain(lead), plain(after));
    }
    let whole = plain(source);
    // The first sentence: up to a full stop, question mark or exclamation
    // mark that a space follows.
    let mut end = None;
    let mut chars = whole.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if matches!(c, '.' | '?' | '!') && chars.peek().is_some_and(|(_, next)| *next == ' ') {
            end = Some(at.saturating_add(c.len_utf8()));
            break;
        }
    }
    match end.and_then(|end| whole.split_at_checked(end)) {
        Some((title, text)) => (title.trim().to_string(), text.trim().to_string()),
        None => (whole, String::new()),
    }
}

impl Changelog {
    /// Reads a changelog. An error names the line and what is expected
    /// there: a heading that is not a version, versions that are not in
    /// descending order, a version twice, *Unreleased* anywhere but first.
    pub fn parse(text: &str) -> Result<Self, Error> {
        if text.len() > MAX_BYTES {
            return Err(Error::new(format!(
                "the changelog is larger than {MAX_BYTES} bytes"
            )));
        }
        let mut sections: Vec<Section> = Vec::new();
        let mut entries = 0usize;
        let mut last: Option<Version> = None;
        for (index, raw) in raw_sections(text).iter().enumerate() {
            let (version, date) = heading_of(raw)?;
            match version {
                None if index > 0 => {
                    return Err(Error::new(format!(
                        "line {}: `## [Unreleased]` comes first, above every version",
                        raw.line
                    )));
                }
                None => {}
                Some(version) => {
                    if let Some(newer) = last
                        && version >= newer
                    {
                        return Err(Error::new(format!(
                            "line {}: version {version} stands below {newer}; the versions go newest first, each once",
                            raw.line
                        )));
                    }
                    last = Some(version);
                }
            }
            let section = Self::section(version, date, raw, &mut entries)?;
            sections.push(section);
        }
        Ok(Self { sections })
    }

    fn section(
        version: Option<Version>,
        date: Option<String>,
        raw: &Raw<'_>,
        count: &mut usize,
    ) -> Result<Section, Error> {
        // The paragraphs of what is being read: the section's summary
        // above the first group, then each group's own note.
        let mut paragraphs: Vec<String> = Vec::new();
        let mut paragraph = String::new();
        let mut summary = String::new();
        let mut entries: Vec<Entry> = Vec::new();
        let mut notes: Vec<Note> = Vec::new();
        // The group being read, and the entry being gathered in it.
        let mut group: Option<(Kind, String)> = None;
        let mut entry: Option<String> = None;
        let mut close = |entry: &mut Option<String>,
                         group: &Option<(Kind, String)>,
                         entries: &mut Vec<Entry>|
         -> Result<(), Error> {
            let Some(source) = entry.take() else {
                return Ok(());
            };
            let Some((kind, heading)) = group else {
                return Ok(());
            };
            *count = count.saturating_add(1);
            if *count > MAX_ENTRIES {
                return Err(Error::new(format!(
                    "the changelog has more than {MAX_ENTRIES} entries"
                )));
            }
            let (title, text) = title_and_text(&source);
            if !title.is_empty() {
                entries.push(Entry {
                    kind: *kind,
                    heading: heading.clone(),
                    title,
                    text,
                });
            }
            Ok(())
        };
        // The paragraphs gathered so far, as text: the summary before the
        // first group, a group's note after it.
        let said = |paragraphs: &mut Vec<String>, paragraph: &mut String| -> String {
            if !paragraph.is_empty() {
                paragraphs.push(std::mem::take(paragraph));
            }
            std::mem::take(paragraphs)
                .iter()
                .map(|paragraph| plain(paragraph))
                .filter(|paragraph| !paragraph.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let end_group = |group: &mut Option<(Kind, String)>,
                         text: String,
                         summary: &mut String,
                         notes: &mut Vec<Note>| {
            match group.take() {
                None => *summary = text,
                Some((kind, heading)) if !text.is_empty() => notes.push(Note {
                    kind,
                    heading,
                    text,
                }),
                Some(_) => {}
            }
        };
        for line in &raw.lines {
            let line = line.trim_end();
            if let Some(heading) = line.strip_prefix("### ") {
                close(&mut entry, &group, &mut entries)?;
                let text = said(&mut paragraphs, &mut paragraph);
                end_group(&mut group, text, &mut summary, &mut notes);
                let heading = plain(heading);
                group = Some((Kind::of_heading(&heading), heading));
            } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
                close(&mut entry, &group, &mut entries)?;
                if group.is_none() {
                    // A list above the first group belongs to the summary.
                    if !paragraph.is_empty() {
                        paragraph.push(' ');
                    }
                    paragraph.push_str(item);
                } else {
                    if !paragraph.is_empty() {
                        paragraphs.push(std::mem::take(&mut paragraph));
                    }
                    entry = Some(item.to_string());
                }
            } else if line.trim().is_empty() {
                // An empty line ends a paragraph; an entry goes on only
                // with an indented line.
                if !paragraph.is_empty() {
                    paragraphs.push(std::mem::take(&mut paragraph));
                }
            } else if let Some(open) = entry.as_mut()
                && line.starts_with([' ', '\t'])
            {
                open.push(' ');
                open.push_str(line.trim());
            } else {
                // Not an entry and not the rest of one: above the first
                // group it is the section's summary; under a group it is
                // what the group says of itself (a paragraph, a numbered
                // list, a table), kept as the group's note.
                close(&mut entry, &group, &mut entries)?;
                if !paragraph.is_empty() {
                    paragraph.push(' ');
                }
                paragraph.push_str(line.trim());
            }
        }
        close(&mut entry, &group, &mut entries)?;
        let text = said(&mut paragraphs, &mut paragraph);
        end_group(&mut group, text, &mut summary, &mut notes);
        Ok(Section {
            version,
            date,
            summary,
            entries,
            notes,
        })
    }

    /// Every section, newest first; *Unreleased*, when the file has it,
    /// before the versions.
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The first release the file names: nothing is older.
    pub fn oldest(&self) -> Option<Version> {
        self.sections.iter().filter_map(|s| s.version).min()
    }

    /// The newest release the file names.
    pub fn newest(&self) -> Option<Version> {
        self.sections.iter().filter_map(|s| s.version).max()
    }

    /// The section of `version`.
    pub fn version(&self, version: Version) -> Option<&Section> {
        self.sections.iter().find(|s| s.version == Some(version))
    }

    /// What a program of version `running` has that one of version
    /// `since` had not: the sections that say something, of every version
    /// after `since` up to `running`, newest first, with what is not
    /// released yet before them (a build between two releases has it).
    pub fn after(&self, since: Version, running: Version) -> Vec<&Section> {
        self.sections
            .iter()
            .filter(|section| match section.version {
                None => true,
                Some(version) => version > since && version <= running,
            })
            .filter(|section| !section.is_blank())
            .collect()
    }

    /// The notes of `running` itself, with what is not released yet
    /// before them.
    pub fn of(&self, running: Version) -> Vec<&Section> {
        self.sections
            .iter()
            .filter(|section| section.version.is_none_or(|version| version == running))
            .filter(|section| !section.is_blank())
            .collect()
    }

    /// What to offer by itself when a program of version `running` starts
    /// and `seen` is what was seen here before; `None` when nothing is
    /// new to this person: a first start, the same version again, an
    /// older one (a downgrade), or no released notes to show. After an
    /// earlier 3DCraft that kept no note of its version, everything after
    /// the first release is offered.
    pub fn offer(&self, running: Version, seen: Seen) -> Option<Vec<&Section>> {
        let since = match seen {
            Seen::Never => return None,
            Seen::Earlier => self.oldest()?,
            Seen::Version(version) => version,
        };
        if since >= running {
            return None;
        }
        let sections = self.after(since, running);
        sections
            .iter()
            .any(|section| section.version.is_some())
            .then_some(sections)
    }
}

/// The part of a changelog a program of version `running` carries: the
/// file's own text of every section that can be shown as new (all but the
/// first release's, whose notes are new to nobody: only its heading is
/// kept, so that the first release stays known), without the file's
/// preamble and link definitions. The text reads back with
/// [`Changelog::parse`].
///
/// An error when the file does not read, when it has no notes for
/// `running` (the Release workflow refuses such a tag too), or when it
/// names a version newer than `running`: a program must not carry notes
/// of a version it is not.
pub fn embedded(text: &str, running: &str) -> Result<String, Error> {
    let running = Version::parse(running)?;
    let changelog = Changelog::parse(text)?;
    if !changelog
        .version(running)
        .is_some_and(|section| !section.is_blank())
    {
        return Err(Error::new(format!(
            "there are no notes under `## [{running}]`: the notes of this version are what 3DCraft shows after an update, and what the Release workflow puts on the release"
        )));
    }
    if let Some(newest) = changelog.newest()
        && newest > running
    {
        return Err(Error::new(format!(
            "it names version {newest}, which is newer than this one, {running}: the version in Cargo.toml and the heading in the changelog change together"
        )));
    }
    let oldest = changelog.oldest();
    let mut out = String::new();
    for (raw, section) in raw_sections(text).iter().zip(changelog.sections()) {
        let first_release = section.version.is_some() && section.version == oldest;
        let kept = !first_release || section.version == Some(running);
        if section.version.is_none() && section.is_blank() {
            continue;
        }
        out.push_str(raw.heading.trim_end());
        out.push('\n');
        if kept {
            for line in &raw.lines {
                out.push_str(line.trim_end());
                out.push('\n');
            }
        }
    }
    Ok(out)
}

/// `markup` as text a person can read: the Markdown of a changelog entry
/// taken off. Bold and emphasis lose their stars, code its backticks, a
/// link keeps its words and loses its address, an escaped character
/// stands for itself, and white space runs become one space. What is
/// inside backticks is kept as it is written.
pub fn plain(markup: &str) -> String {
    let mut out = String::new();
    plain_into(markup, &mut out, 0);
    let mut text = String::with_capacity(out.len());
    for word in out.split_whitespace() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(word);
    }
    text
}

/// The end of the bracketed part that starts after `open` in `text`
/// (which begins just inside the bracket): the byte offset of the closing
/// character at the same depth, when there is one within
/// [`MAX_BRACKETED_CHARS`] (a link's words or its address are shorter;
/// without the cap, text of nothing but brackets would be looked through
/// once per bracket).
fn closing(text: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (at, c) in text.char_indices().take(MAX_BRACKETED_CHARS) {
        if c == open {
            depth = depth.saturating_add(1);
        } else if c == close {
            if depth == 0 {
                return Some(at);
            }
            depth = depth.saturating_sub(1);
        }
    }
    None
}

/// The longest start of `text` of at most `max` bytes that ends between
/// two characters.
fn head(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while end > 0 && !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or("")
}

/// Where a character next stands in a text that is read from its start
/// to its end, looked for once and remembered: asked again further on,
/// the answer is the one found before for as long as it lies ahead. So
/// asking at every position costs the text's length once, where a search
/// of the rest at every position would cost its square (a text of nothing
/// but `<`, each looking for its `>`).
struct Ahead {
    wanted: char,
    /// What the last search found: `None` before the first; `Some(None)`
    /// when there is none from there to the end.
    found: Option<Option<usize>>,
}

impl Ahead {
    fn new(wanted: char) -> Self {
        Self {
            wanted,
            found: None,
        }
    }

    /// The byte offset in `markup` of the next `wanted` at or after
    /// `from`, where `rest` is `markup` from `from` on.
    fn next(&mut self, rest: &str, from: usize) -> Option<usize> {
        match self.found {
            Some(None) => None,
            Some(Some(at)) if at >= from => Some(at),
            _ => {
                let found = rest.find(self.wanted).map(|at| from.saturating_add(at));
                self.found = Some(found);
                found
            }
        }
    }
}

fn plain_into(markup: &str, out: &mut String, depth: u8) {
    let mut rest = markup;
    let mut previous: Option<char> = None;
    let mut closing_angle = Ahead::new('>');
    let mut closing_bracket = Ahead::new(']');
    while let Some(c) = rest.chars().next() {
        let after = rest.get(c.len_utf8()..).unwrap_or("");
        // Where `after` begins in `markup`.
        let at = markup.len().saturating_sub(after.len());
        match c {
            '`' => {
                // A code span: as many backticks close it as open it,
                // within the length a code span has.
                let ticks = rest
                    .len()
                    .saturating_sub(rest.trim_start_matches('`').len());
                let inside = rest.get(ticks..).unwrap_or("");
                let fence = "`".repeat(ticks);
                let reach = MAX_SPAN_BYTES.saturating_add(ticks);
                match head(inside, reach).find(&fence) {
                    Some(end) => {
                        out.push_str(inside.get(..end).unwrap_or(""));
                        rest = inside.get(end.saturating_add(ticks)..).unwrap_or("");
                        previous = Some('`');
                    }
                    None => {
                        out.push_str(&fence);
                        rest = inside;
                        previous = Some('`');
                    }
                }
                continue;
            }
            '\\' => {
                if let Some(next) = after.chars().next()
                    && next.is_ascii_punctuation()
                {
                    out.push(next);
                    rest = after.get(next.len_utf8()..).unwrap_or("");
                    previous = Some(next);
                    continue;
                }
                out.push(c);
            }
            '*' => {
                // A star beside a word is markup; one that stands alone
                // between spaces is a star.
                let stars = rest
                    .len()
                    .saturating_sub(rest.trim_start_matches('*').len());
                let next = rest.get(stars..).and_then(|tail| tail.chars().next());
                let free = |c: Option<char>| c.is_none_or(char::is_whitespace);
                if free(previous) && free(next) {
                    out.push_str(rest.get(..stars).unwrap_or(""));
                }
                rest = rest.get(stars..).unwrap_or("");
                previous = Some('*');
                continue;
            }
            '[' => {
                // Without a `]` ahead within the length of a link's words
                // (a character is at most four bytes) it is no link: not
                // looked into at all.
                if closing_bracket.next(after, at).is_some_and(|close| {
                    close.saturating_sub(at) <= MAX_BRACKETED_CHARS.saturating_mul(4)
                }) && let Some(end) = closing(after, '[', ']')
                {
                    let words = after.get(..end).unwrap_or("");
                    let tail = after.get(end.saturating_add(1)..).unwrap_or("");
                    let target = if let Some(inner) = tail.strip_prefix('(') {
                        closing(inner, '(', ')')
                            .map(|close| inner.get(close.saturating_add(1)..).unwrap_or(""))
                    } else if let Some(inner) = tail.strip_prefix('[') {
                        head(inner, MAX_SPAN_BYTES)
                            .find(']')
                            .map(|close| inner.get(close.saturating_add(1)..).unwrap_or(""))
                    } else {
                        None
                    };
                    if let Some(tail) = target {
                        if depth < MAX_MARKUP_DEPTH {
                            plain_into(words, out, depth.saturating_add(1));
                        } else {
                            out.push_str(words);
                        }
                        rest = tail;
                        previous = Some(']');
                        continue;
                    }
                }
                out.push(c);
            }
            '<' => {
                // An address between angle brackets stands for itself.
                // Its `>` is found once for every `<` before it, and an
                // address is no longer than a span.
                if let Some(end) = closing_angle
                    .next(after, at)
                    .map(|close| close.saturating_sub(at))
                    .filter(|end| *end <= MAX_SPAN_BYTES)
                {
                    let inside = after.get(..end).unwrap_or("");
                    let address = (inside.starts_with("http://") || inside.starts_with("https://"))
                        && !inside.contains(char::is_whitespace);
                    if address {
                        out.push_str(inside);
                        rest = after.get(end.saturating_add(1)..).unwrap_or("");
                        previous = Some('>');
                        continue;
                    }
                }
                out.push(c);
            }
            '&' => {
                let entity = [
                    ("&amp;", '&'),
                    ("&lt;", '<'),
                    ("&gt;", '>'),
                    ("&quot;", '"'),
                    ("&#39;", '\''),
                    ("&nbsp;", ' '),
                ]
                .into_iter()
                .find(|(name, _)| rest.starts_with(name));
                if let Some((name, stands_for)) = entity {
                    out.push(stands_for);
                    rest = rest.get(name.len()..).unwrap_or("");
                    previous = Some(stands_for);
                    continue;
                }
                out.push(c);
            }
            _ => out.push(c),
        }
        previous = Some(c);
        rest = after;
    }
}

#[cfg(test)]
mod tests;
