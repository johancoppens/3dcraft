//! What's new: the release notes the program carries, and the command
//! `session.whatsNew` that gives an agent what changed since a version as
//! data (the versions, their entries by kind, which of them change an
//! answer). The window shows the same notes on its start screen after an
//! update and under Help › What's new.
//!
//! The notes are the workspace's `CHANGELOG.md` as it was when the program
//! was built (`craft3d-changelog`): a program's build script reads it, its
//! `main` hands the text over with [`set_release_notes`], as it hands over
//! the commit. Nothing is fetched, and nothing looks online for a newer
//! version. A library user that hands over no notes has none: the command
//! says so.
//!
//! **What changes an answer.** A fix release can change a result someone
//! relied on (a clearance reported as proven for parts that overlap, a
//! volume measured wrong). Such entries stand in the changelog under
//! `### Results that changed` (`craft3d_changelog::RESULTS_HEADING`); they
//! are `changesAnswer: true` here and come first in every version.

use crate::commands::{Run, fail, spec};
use crate::params::Params;
use crate::report::VERSION;
use crate::{CommandError, CommandSpec, Session};
use craft3d_changelog::{Changelog, Kind, Section, Version};
use serde_json::{Value, json};
use std::sync::OnceLock;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("session.whatsNew", "What's New", r#"{"since": string = the version before this one}"#,
        "Returns what changed in this 3DCraft since a version, from the release notes the program carries (its changelog at build time; nothing is fetched): version (this program's), since (as given, else null), versions, newest first: [{version (\"unreleased\" for what a build between two releases has that no release has yet), date, summary, changesAnswers: how many of its entries change an answer, entries: [{kind: results|added|changed|deprecated|removed|fixed|security|limitations|other, heading: its group in the changelog, changesAnswer, title, text}], those that change an answer first, notes: [{kind, heading, text}]: what a group says of itself, the text under its heading that is not an entry (the sentence under Results that changed)}], changesAnswers (the total) and note. The texts are the changelog's without its markup; a link keeps its words and loses its address. An entry with changesAnswer true (kind results) says that a result 3DCraft gave before can differ now: a measure, a proof, a check, or a body that was built wrong while it looked right; what was measured or checked with a version before it is worth checking again. since is a version as 0.1.1 (v0.1.1 too); without it the answer is this version's own notes. A since that is this version gives no versions (but what is not released yet, in a build between two releases); a since that is not a version, or that is newer than this program, is an error. A since older than the first release gives everything the program carries (the first release's own notes are not carried: nothing is older; note says so). Running it changes nothing.", doc: false, session_whats_new),
];

/// The notes the running program was built with, as its `main` gave them
/// ([`set_release_notes`]).
static NOTES: OnceLock<Changelog> = OnceLock::new();

/// Tells the engine the release notes the program was built with: a
/// program's `main` passes the text its build script took from
/// `CHANGELOG.md` (`craft3d_changelog::build::emit`). The text is embedded
/// in the program, not in this library, so a changed changelog rebuilds
/// only the programs. The first call wins. An error when the text does
/// not read as a changelog (the build script read it before, so this is
/// not met by a program that was built).
pub fn set_release_notes(text: &str) -> Result<(), String> {
    if NOTES.get().is_some() {
        return Ok(());
    }
    let notes = Changelog::parse(text).map_err(|e| e.to_string())?;
    let _ = NOTES.set(notes);
    Ok(())
}

/// The notes [`set_release_notes`] was given, or `None` when the program
/// handed none over.
pub fn release_notes() -> Option<&'static Changelog> {
    NOTES.get()
}

/// The version of this program, as the notes count versions.
pub fn running_version() -> Option<Version> {
    Version::parse(VERSION).ok()
}

/// One section as `session.whatsNew` gives it: what changes an answer
/// first, then the other kinds in the order notes are shown, each kind in
/// the changelog's order.
fn section_json(section: &Section) -> Value {
    let entries: Vec<Value> = Kind::ALL
        .into_iter()
        .flat_map(|kind| section.of_kind(kind))
        .map(|entry| {
            json!({
                "kind": entry.kind.word(),
                "heading": entry.heading,
                "changesAnswer": entry.changes_answer(),
                "title": entry.title,
                "text": entry.text,
            })
        })
        .collect();
    // What a group says of itself, in the same order as the entries.
    let notes: Vec<Value> = Kind::ALL
        .into_iter()
        .flat_map(|kind| section.notes_of(kind))
        .map(|note| {
            json!({
                "kind": note.kind.word(),
                "heading": note.heading,
                "text": note.text,
            })
        })
        .collect();
    json!({
        "version": section.version.map_or_else(|| "unreleased".to_string(), |v| v.to_string()),
        "date": section.date,
        "summary": section.summary,
        "changesAnswers": section.changing_answers(),
        "entries": entries,
        "notes": notes,
    })
}

/// What `session.whatsNew` answers for a program of version `running`
/// that carries `notes`, asked since `since` (`None`: the version's own
/// notes). The error is the reason, without the command's name.
pub fn whats_new(notes: &Changelog, running: &str, since: Option<&str>) -> Result<Value, String> {
    let running = Version::parse(running).map_err(|e| e.to_string())?;
    let (sections, since, note) = match since {
        None => (notes.of(running), None, None),
        Some(text) => {
            let since = Version::parse(text).map_err(|e| e.to_string())?;
            if since > running {
                return Err(format!(
                    "{since} is newer than this 3DCraft, {running}: it carries the notes up to its own version only"
                ));
            }
            let note = notes.oldest().filter(|first| since < *first).map(|first| {
                format!(
                    "the notes start after {first}, the first release: nothing is older, so its own notes are not carried"
                )
            });
            (notes.after(since, running), Some(since), note)
        }
    };
    let changes: usize = sections
        .iter()
        .map(|section| section.changing_answers())
        .fold(0, usize::saturating_add);
    Ok(json!({
        "version": running.to_string(),
        "since": since.map(|v| v.to_string()),
        "changesAnswers": changes,
        "versions": sections.iter().map(|s| section_json(s)).collect::<Vec<_>>(),
        "note": note,
    }))
}

fn session_whats_new(_session: &mut Session, params: &Value) -> Run {
    let command = "session.whatsNew";
    let p = Params::new(command, params, &["since"])?;
    let since = p.str("since")?;
    let Some(notes) = release_notes() else {
        return fail(format!(
            "{command}: this program carries no release notes (the programs 3DCraft is released as do; this one was built without them)"
        ));
    };
    whats_new(notes, VERSION, since).map_err(|e| CommandError::new(format!("{command}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Workspace;

    const SAMPLE: &str = "\
## [Unreleased]

### Added

- **Not released yet.**

## [0.3.0] - 2026-12-20

Three things.

### Added

- **A fillet takes one edge by its own name.** Give `edges`.

### Fixed

- **A shell was refused.** It builds.

### Results that changed

- **A measured volume could be wrong.** By *10 mm³*.
- **A clearance could be proven for parts that overlap.**

## [0.2.1] - 2026-12-05

### Fixed

- **Windows: no console window.**

## [0.2.0] - 2026-12-01

### Results that changed

- **A proven distance could be too high.**

### Added

- **Embed.**

## [0.1.0] - 2026-11-11
";

    fn sample() -> Changelog {
        Changelog::parse(SAMPLE).unwrap()
    }

    fn versions(answer: &Value) -> Vec<&str> {
        answer["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["version"].as_str().unwrap())
            .collect()
    }

    /// The repository's changelog as a program built today carries it.
    fn carried() -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        craft3d_changelog::build::notes(&root, VERSION).unwrap()
    }

    #[test]
    fn a_version_newer_than_the_one_asked_since_is_listed_with_its_entries_by_kind() {
        let answer = whats_new(&sample(), "0.3.0", Some("0.2.1")).unwrap();
        assert_eq!(answer["version"], "0.3.0");
        assert_eq!(answer["since"], "0.2.1");
        assert_eq!(versions(&answer), ["unreleased", "0.3.0"]);
        let release = &answer["versions"][1];
        assert_eq!(release["date"], "2026-12-20");
        assert_eq!(release["summary"], "Three things.");
        assert_eq!(release["changesAnswers"], 2);
        // What changes an answer comes first, whatever the file's order.
        let kinds: Vec<&str> = release["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["results", "results", "added", "fixed"]);
        let first = &release["entries"][0];
        assert_eq!(
            first,
            &json!({
                "kind": "results",
                "heading": "Results that changed",
                "changesAnswer": true,
                "title": "A measured volume could be wrong.",
                "text": "By 10 mm³.",
            })
        );
        assert_eq!(release["entries"][2]["changesAnswer"], false);
        assert_eq!(release["entries"][2]["text"], "Give edges.");
        assert_eq!(answer["changesAnswers"], 2);
        assert_eq!(answer["note"], Value::Null);
        // What is not released has no date.
        assert_eq!(answer["versions"][0]["date"], Value::Null);
    }

    #[test]
    fn the_same_version_has_nothing_new_but_what_is_not_released() {
        let answer = whats_new(&sample(), "0.3.0", Some("0.3.0")).unwrap();
        assert_eq!(versions(&answer), ["unreleased"]);
        assert_eq!(answer["changesAnswers"], 0);
        let released = Changelog::parse("## [0.3.0]\n\n- x\n\n## [0.2.0]\n\n- y\n").unwrap();
        let answer = whats_new(&released, "0.3.0", Some("0.3.0")).unwrap();
        assert_eq!(answer["versions"], json!([]));
    }

    #[test]
    fn several_skipped_versions_are_all_listed_newest_first() {
        let answer = whats_new(&sample(), "0.3.0", Some("0.1.0")).unwrap();
        assert_eq!(versions(&answer), ["unreleased", "0.3.0", "0.2.1", "0.2.0"]);
        assert_eq!(answer["changesAnswers"], 3);
        assert_eq!(answer["versions"][3]["changesAnswers"], 1);
        // A version the notes do not name is a place in the order.
        let answer = whats_new(&sample(), "0.3.0", Some("0.2.5")).unwrap();
        assert_eq!(versions(&answer), ["unreleased", "0.3.0"]);
    }

    #[test]
    fn without_since_the_answer_is_the_versions_own_notes() {
        let answer = whats_new(&sample(), "0.2.1", None).unwrap();
        assert_eq!(answer["since"], Value::Null);
        assert_eq!(versions(&answer), ["unreleased", "0.2.1"]);
    }

    /// What stands under a group and is no entry is not dropped: it is
    /// the group's note in the answer, what changes an answer first.
    #[test]
    fn what_a_group_says_of_itself_is_in_the_answer() {
        let notes = Changelog::parse(
            "## [0.3.0]\n\n### Fixed\n\nTwo fixes of the *window*.\n\n- **A fix.**\n\n### Results that changed\n\nCheck [again](https://example.org) what you measured.\n\n- **A measure was wrong.**\n\n## [0.2.0]\n\n- x\n",
        )
        .unwrap();
        let answer = whats_new(&notes, "0.3.0", Some("0.2.0")).unwrap();
        assert_eq!(
            answer["versions"][0]["notes"],
            json!([
                {"kind": "results", "heading": "Results that changed", "text": "Check again what you measured."},
                {"kind": "fixed", "heading": "Fixed", "text": "Two fixes of the window."},
            ])
        );
        // A version whose groups have entries only has none.
        let answer = whats_new(&sample(), "0.3.0", Some("0.2.1")).unwrap();
        assert_eq!(answer["versions"][1]["notes"], json!([]));
    }

    #[test]
    fn since_before_the_first_release_gives_everything_and_says_where_the_notes_start() {
        let answer = whats_new(&sample(), "0.3.0", Some("0.0.1")).unwrap();
        assert_eq!(versions(&answer), ["unreleased", "0.3.0", "0.2.1", "0.2.0"]);
        assert!(
            answer["note"]
                .as_str()
                .unwrap()
                .contains("the notes start after 0.1.0, the first release")
        );
    }

    #[test]
    fn a_since_that_is_not_a_version_or_is_a_future_one_is_an_error() {
        let long = "9".repeat(5000);
        for junk in [
            "",
            " ",
            "next",
            "latest",
            "1",
            "1.2",
            "1.2.3.4",
            "0.1.x",
            "-1.0.0",
            "1.2.3-rc1",
            "0.1.2 ",
            "\u{0}",
            "4294967296.0.0",
            &long,
        ] {
            let error = whats_new(&sample(), "0.3.0", Some(junk)).unwrap_err();
            assert!(error.contains("is not a version"), "{junk:?}: {error}");
            assert!(error.chars().count() < 200, "{error}");
        }
        for future in ["0.3.1", "0.4.0", "1.0.0", "999999.0.0"] {
            let error = whats_new(&sample(), "0.3.0", Some(future)).unwrap_err();
            assert!(
                error.contains("is newer than this 3DCraft, 0.3.0"),
                "{future}: {error}"
            );
        }
        // A downgrade asks since a newer version: the same error.
        assert!(whats_new(&sample(), "0.2.0", Some("0.3.0")).is_err());
    }

    /// The command itself, on the notes a program built from this
    /// repository carries today: hostile params are errors that name the
    /// command, and an answer has the documented keys.
    #[test]
    fn the_command_answers_from_the_notes_the_program_carries() {
        set_release_notes(&carried()).unwrap();
        // The first call wins; a second one changes nothing and is no error.
        set_release_notes("## [banana]").unwrap();
        let mut session = Session::new(Workspace::none());
        let answer = session.execute("session.whatsNew", &json!({})).unwrap();
        assert_eq!(answer["version"], VERSION);
        assert_eq!(answer["since"], Value::Null);
        assert!(versions(&answer).contains(&VERSION));
        for version in answer["versions"].as_array().unwrap() {
            for entry in version["entries"].as_array().unwrap() {
                assert_eq!(
                    entry["changesAnswer"].as_bool().unwrap(),
                    entry["kind"] == "results"
                );
                assert!(!entry["title"].as_str().unwrap().is_empty());
            }
        }
        // Null params are none.
        assert_eq!(
            session.execute("session.whatsNew", &Value::Null).unwrap(),
            answer
        );
        for (params, says) in [
            (json!({"since": 7}), "`since` must be a string"),
            (json!({"since": ["0.1.0"]}), "`since` must be a string"),
            (json!({"since": "soon"}), "`soon` is not a version"),
            (json!({"since": "99.0.0"}), "is newer than this 3DCraft"),
            (json!({"from": "0.1.0"}), "unknown parameter `from`"),
            (json!([1]), "params must be a JSON object"),
        ] {
            let error = session.execute("session.whatsNew", &params).unwrap_err();
            assert!(
                error.message().starts_with("session.whatsNew: "),
                "{params}: {error:?}"
            );
            assert!(error.message().contains(says), "{params}: {error:?}");
        }
        // It needs no document and changes nothing.
        assert!(session.document_status().is_none());
    }
}
