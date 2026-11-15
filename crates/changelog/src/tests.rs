use super::*;

const SAMPLE: &str = "\
# Changelog

All notable changes are listed here.

## [Unreleased]

### Added

- **A thing not released yet.** It is on its way.

## [0.3.0] - 2026-12-20

Three things in one release.

### Results that changed

- **A measured volume could be wrong.** `document.measure` reported 10 mm³ *too little*; see [the guide](https://example.org/guide#measure).
- **A clearance could be reported as proven for parts that overlap.** Such a pair is now an overlap.

### Added

- **A fillet takes one edge by its own name.** Give `\"edges\": [\"F3:between[…]\"]`.

### Changed

- **Files are format version 72.**

### Fixed

- **A shell was refused where four faces meet.** It builds now,
  also on a second line.
- An entry without a bold sentence. It has two sentences.

## [0.2.1] - 2026-12-05

### Fixed

- **Windows: no console window.**

## [0.2.0] - 2026-12-01

### Added

- **Embed.** The page drives the module.

### Known limitations

- The [known limitations of 0.1.0](https://example.org/v0.1.0) still hold.

## [0.1.0] - 2026-11-11

The first release.

### Modelling

- A history of features.

[Unreleased]: https://example.org/compare/v0.3.0...HEAD
[0.3.0]: https://example.org/compare/v0.2.1...v0.3.0
";

fn sample() -> Changelog {
    Changelog::parse(SAMPLE).unwrap()
}

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn labels(sections: &[&Section]) -> Vec<String> {
    sections.iter().map(|s| s.label()).collect()
}

/// The repository's own changelog.
fn real() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../CHANGELOG.md");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_version_reads_and_orders() {
    assert_eq!(v("0.1.2"), Version::new(0, 1, 2));
    assert_eq!(v("v10.20.30"), Version::new(10, 20, 30));
    assert_eq!(v("0.1.2").to_string(), "0.1.2");
    assert!(v("0.2.0") > v("0.1.12"));
    assert!(v("0.1.12") > v("0.1.2"));
    assert!(v("1.0.0") > v("0.99.99"));
}

#[test]
fn what_is_not_a_version_is_refused_and_never_panics() {
    let long = "1.".repeat(4000);
    for junk in [
        "",
        " ",
        "1",
        "1.2",
        "1.2.3.4",
        "1.2.x",
        "1..3",
        ".1.2",
        "1.2.",
        "-1.2.3",
        "+1.2.3",
        "1.2.3-rc1",
        " 1.2.3",
        "1.2.3 ",
        "vv1.2.3",
        "V1.2.3",
        "1.2.99999999999999999999",
        "4294967296.0.0",
        "1000000.0.0",
        "١.٢.٣",
        "1.2.3\n",
        "\u{0}",
        "nine",
        "latest",
        "unreleased",
        long.as_str(),
    ] {
        let error = Version::parse(junk).unwrap_err();
        assert!(
            error.message().contains("is not a version"),
            "{junk:?}: {error}"
        );
        // What is said back is short, whatever was given.
        assert!(error.message().chars().count() < 160, "{error}");
    }
}

#[test]
fn the_sections_read_newest_first_with_their_dates_and_summaries() {
    let changelog = sample();
    let sections = changelog.sections();
    assert_eq!(
        sections.iter().map(Section::label).collect::<Vec<_>>(),
        ["Not yet released", "0.3.0", "0.2.1", "0.2.0", "0.1.0"]
    );
    assert_eq!(sections[0].date, None);
    assert_eq!(sections[1].date.as_deref(), Some("2026-12-20"));
    assert_eq!(sections[1].summary, "Three things in one release.");
    assert_eq!(sections[4].summary, "The first release.");
    assert_eq!(changelog.oldest(), Some(v("0.1.0")));
    assert_eq!(changelog.newest(), Some(v("0.3.0")));
}

#[test]
fn entries_stand_under_their_kind_and_what_changes_an_answer_is_told_by_its_group() {
    let changelog = sample();
    let section = changelog.version(v("0.3.0")).unwrap();
    let kinds: Vec<&str> = section.entries.iter().map(|e| e.kind.word()).collect();
    assert_eq!(
        kinds,
        ["results", "results", "added", "changed", "fixed", "fixed"]
    );
    assert_eq!(section.changing_answers(), 2);
    for entry in &section.entries {
        assert_eq!(entry.changes_answer(), entry.kind == Kind::Results);
        assert_eq!(entry.changes_answer(), entry.heading == RESULTS_HEADING);
    }
    // Only the group tells: a fix that says "wrong" in its words is a fix.
    let other = Changelog::parse("## [1.0.0]\n\n### Fixed\n\n- **A result was wrong.**\n").unwrap();
    assert!(!other.sections()[0].entries[0].changes_answer());
    // A heading of its own is kept as it is written.
    let first = changelog.version(v("0.1.0")).unwrap();
    assert_eq!(first.entries[0].kind, Kind::Other);
    assert_eq!(first.entries[0].heading, "Modelling");
    let limits = changelog.version(v("0.2.0")).unwrap();
    assert_eq!(limits.entries[1].kind, Kind::Limitations);
    // The group's name is read whatever its case.
    assert_eq!(Kind::of_heading(" results THAT changed "), Kind::Results);
    assert_eq!(Kind::of_heading("Results"), Kind::Other);
}

#[test]
fn an_entry_with_markup_reads_as_text_a_person_can_read() {
    let changelog = sample();
    let section = changelog.version(v("0.3.0")).unwrap();
    let measured = &section.entries[0];
    assert_eq!(measured.title, "A measured volume could be wrong.");
    assert_eq!(
        measured.text,
        "document.measure reported 10 mm³ too little; see the guide."
    );
    // Code keeps what it holds, brackets and quotes too.
    let fillet = &section.entries[2];
    assert_eq!(fillet.title, "A fillet takes one edge by its own name.");
    assert_eq!(fillet.text, "Give \"edges\": [\"F3:between[…]\"].");
    // A title alone.
    assert_eq!(section.entries[3].title, "Files are format version 72.");
    assert_eq!(section.entries[3].text, "");
    // An entry that goes on over an indented line.
    assert_eq!(
        section.entries[4].text,
        "It builds now, also on a second line."
    );
    // Without a bold sentence: the first sentence is the title.
    assert_eq!(
        section.entries[5].title,
        "An entry without a bold sentence."
    );
    assert_eq!(section.entries[5].text, "It has two sentences.");
    // A link keeps its words.
    let limits = changelog.version(v("0.2.0")).unwrap();
    assert_eq!(
        limits.entries[1].title,
        "The known limitations of 0.1.0 still hold."
    );
    for section in changelog.sections() {
        for entry in &section.entries {
            for text in [&entry.title, &entry.text] {
                assert!(!text.contains("**"), "{text}");
                assert!(!text.contains('`'), "{text}");
                assert!(!text.contains("]("), "{text}");
                assert!(!text.contains("https://"), "{text}");
            }
        }
    }
}

#[test]
fn markup_is_taken_off_and_what_is_not_markup_stays() {
    for (markup, text) in [
        ("**bold** and *soft*", "bold and soft"),
        ("***both***", "both"),
        ("a * b * c", "a * b * c"),
        ("2 ** 3", "2 ** 3"),
        ("`a*b*c` and `x_y`", "a*b*c and x_y"),
        ("``a ` b``", "a ` b"),
        ("an open ` tick", "an open ` tick"),
        ("[words](https://example.org/a_(b))", "words"),
        ("[words][ref] after", "words after"),
        ("[**bold** link](x)", "bold link"),
        (
            "[edge name, face name] stays",
            "[edge name, face name] stays",
        ),
        ("a [b", "a [b"),
        ("<https://example.org/x>", "https://example.org/x"),
        ("a < b > c", "a < b > c"),
        ("<b>", "<b>"),
        ("\\*not soft\\*", "*not soft*"),
        ("back\\slash", "back\\slash"),
        (
            "R&amp;D &lt;1 &gt; 0 &quot;q&quot; &#39;s&#39;",
            "R&D <1 > 0 \"q\" 's'",
        ),
        ("a &b; c", "a &b; c"),
        ("  many   spaces\tand\nlines  ", "many spaces and lines"),
        ("*\"quoted error\"*", "\"quoted error\""),
        ("snake_case_name stays", "snake_case_name stays"),
        ("", ""),
        ("***", "***"),
        ("[", "["),
        ("[]()", ""),
        ("\\", "\\"),
        ("&", "&"),
        ("`", "`"),
    ] {
        assert_eq!(plain(markup), text, "{markup:?}");
    }
    // Nesting deeper than is followed is kept as it is, never recursed
    // into without end.
    let deep = format!("{}x{}", "[".repeat(5000), "](y)".repeat(5000));
    let _ = plain(&deep);
    let odd = "[".repeat(20_000);
    assert_eq!(plain(&odd).len(), odd.len());
    // A link longer than a link is not one.
    let long = format!("[{}](x)", "w".repeat(2000));
    assert_eq!(plain(&long), long);
}

#[test]
fn a_version_newer_than_the_one_seen_is_offered() {
    let changelog = sample();
    let offered = changelog
        .offer(v("0.3.0"), Seen::Version(v("0.2.1")))
        .unwrap();
    // What is not released yet comes before the versions.
    assert_eq!(labels(&offered), ["Not yet released", "0.3.0"]);
}

#[test]
fn the_same_version_is_not_offered() {
    let changelog = sample();
    assert_eq!(changelog.offer(v("0.3.0"), Seen::Version(v("0.3.0"))), None);
    // Also when the build has changes that are not released: the version
    // did not change.
    assert_eq!(changelog.offer(v("0.2.1"), Seen::Version(v("0.2.1"))), None);
}

#[test]
fn a_first_start_is_offered_nothing() {
    let changelog = sample();
    for running in ["0.1.0", "0.2.0", "0.3.0"] {
        assert_eq!(changelog.offer(v(running), Seen::Never), None, "{running}");
    }
}

#[test]
fn a_downgrade_is_offered_nothing() {
    let changelog = sample();
    assert_eq!(changelog.offer(v("0.2.0"), Seen::Version(v("0.3.0"))), None);
    assert_eq!(changelog.offer(v("0.2.1"), Seen::Version(v("9.0.0"))), None);
}

#[test]
fn several_skipped_versions_are_all_listed_newest_first() {
    let changelog = sample();
    let offered = changelog
        .offer(v("0.3.0"), Seen::Version(v("0.1.0")))
        .unwrap();
    assert_eq!(
        labels(&offered),
        ["Not yet released", "0.3.0", "0.2.1", "0.2.0"]
    );
    // A program older than its changelog's newest shows nothing of the
    // versions after it.
    let older = changelog
        .offer(v("0.2.1"), Seen::Version(v("0.1.0")))
        .unwrap();
    assert_eq!(labels(&older), ["Not yet released", "0.2.1", "0.2.0"]);
    // A version seen that the file does not name (a release that was
    // withdrawn) is a place in the order like any other.
    let between = changelog
        .offer(v("0.3.0"), Seen::Version(v("0.2.5")))
        .unwrap();
    assert_eq!(labels(&between), ["Not yet released", "0.3.0"]);
}

#[test]
fn after_an_earlier_version_that_kept_no_note_everything_after_the_first_release_is_offered() {
    let changelog = sample();
    let offered = changelog.offer(v("0.3.0"), Seen::Earlier).unwrap();
    assert_eq!(
        labels(&offered),
        ["Not yet released", "0.3.0", "0.2.1", "0.2.0"]
    );
    // The first release itself has no earlier one.
    assert_eq!(changelog.offer(v("0.1.0"), Seen::Earlier), None);
    assert_eq!(Changelog::default().offer(v("0.1.0"), Seen::Earlier), None);
}

#[test]
fn what_is_not_released_alone_is_not_an_offer() {
    let text = "## [Unreleased]\n\n### Added\n\n- **New.**\n\n## [0.2.0]\n\n## [0.1.0]\n\n- x\n";
    let changelog = Changelog::parse(text).unwrap();
    // 0.2.0 says nothing: only the unreleased notes would be left.
    assert_eq!(changelog.offer(v("0.2.0"), Seen::Version(v("0.1.0"))), None);
    assert_eq!(labels(&changelog.of(v("0.2.0"))), ["Not yet released"]);
}

#[test]
fn the_notes_of_a_version_itself() {
    let changelog = sample();
    assert_eq!(
        labels(&changelog.of(v("0.2.1"))),
        ["Not yet released", "0.2.1"]
    );
    assert!(
        changelog.of(v("7.0.0")).iter().all(|s| s.version.is_none()),
        "a version the file does not name has no notes of its own"
    );
    assert_eq!(
        labels(&changelog.after(v("0.2.0"), v("0.3.0"))),
        ["Not yet released", "0.3.0", "0.2.1"]
    );
    assert_eq!(
        labels(&changelog.after(v("0.0.1"), v("0.3.0"))),
        ["Not yet released", "0.3.0", "0.2.1", "0.2.0", "0.1.0"]
    );
}

#[test]
fn the_release_section_is_the_lines_under_its_heading() {
    assert_eq!(
        release_section(SAMPLE, "0.2.1").unwrap(),
        "\n### Fixed\n\n- **Windows: no console window.**\n\n"
    );
    // The last section ends at the link definitions.
    assert_eq!(
        release_section(SAMPLE, "0.1.0").unwrap(),
        "\nThe first release.\n\n### Modelling\n\n- A history of features.\n\n"
    );
    assert_eq!(release_section(SAMPLE, "0.9.0"), None);
    assert_eq!(release_section("", "0.1.0"), None);
    // The heading is matched from its start, as the workflow matches it:
    // 0.2 is not 0.2.1's, and a heading in the middle of a line is none.
    assert_eq!(release_section(SAMPLE, "0.2"), None);
    assert_eq!(
        release_section("x ## [1.0.0]\ny\n## [1.0.0]\nz\n", "1.0.0").unwrap(),
        "z\n"
    );
    // Without a line feed at the end the last line still counts.
    assert_eq!(release_section("## [1.0.0]\nz", "1.0.0").unwrap(), "z\n");
}

#[test]
fn a_changelog_that_is_not_in_the_form_is_an_error_that_names_the_line() {
    for (text, says) in [
        ("## [banana]\n", "line 1: `banana` is not a version"),
        ("## [1.0\n", "line 1: a section's heading"),
        (
            "x\n\n## [1.0.0] 2026-01-01\n",
            "line 3: after `## [<version>]`",
        ),
        ("## [1.0.0] -\n", "line 1: after ` - ` comes the day"),
        (
            "## [1.0.0]\n\n## [Unreleased]\n",
            "line 3: `## [Unreleased]` comes first",
        ),
        (
            "## [1.0.0]\n## [1.1.0]\n",
            "line 2: version 1.1.0 stands below 1.0.0",
        ),
        (
            "## [1.0.0]\n## [1.0.0]\n",
            "line 2: version 1.0.0 stands below 1.0.0",
        ),
        ("## [1.2.3-rc.1] - 2026-01-01\n", "is not a version"),
    ] {
        let error = Changelog::parse(text).unwrap_err();
        assert!(error.message().contains(says), "{text:?}: {error}");
    }
    // A file without sections is an empty changelog, not an error; what
    // a program must carry is asked by `embedded`.
    assert_eq!(Changelog::parse("").unwrap(), Changelog::default());
    assert_eq!(
        Changelog::parse("# Changelog\n\nNothing yet.\n").unwrap(),
        Changelog::default()
    );
}

#[test]
fn hostile_text_is_read_or_refused_and_never_panics() {
    let mut texts: Vec<String> = [
        "## [",
        "## []",
        "## [1.0.0] - ",
        "## [1.0.0]\n### \n- \n-\n* \n  \n\t- x\n### Fixed\n-  **\n- ****\n- ** **\n",
        "## [1.0.0]\n- **unclosed bold\n- `unclosed code\n- [unclosed](link\n",
        "## [1.0.0]\n### Fixed\n- a\n\n  second paragraph\n\nloose paragraph\n- b\n",
        "## [1.0.0]\r\n\r\n### Fixed\r\n\r\n- **Carriage returns.** Too.\r\n",
        "[x]: y\n## [1.0.0]\n- a\n[1.0.0]: z\n- lost\n## [0.9.0]\n- b\n",
        "\u{feff}## [1.0.0]\n",
        "## [1.0.0]\n### Fixed\n- \u{0}\u{1}\u{7f}\u{202e}\n",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    texts.push("## [1.0.0]\n".repeat(3));
    texts.push(format!(
        "## [1.0.0]\n### Fixed\n{}",
        "- **x.** y\n".repeat(2000)
    ));
    texts.push("#".repeat(100_000));
    texts.push(format!(
        "## [1.0.0]\n### Fixed\n- {}\n",
        "**".repeat(50_000)
    ));
    for text in &texts {
        // Either answer is fine; neither may panic.
        if let Ok(changelog) = Changelog::parse(text) {
            for section in changelog.sections() {
                let _ = section.label();
                for entry in &section.entries {
                    assert!(!entry.title.is_empty());
                }
            }
        }
        let _ = release_section(text, "1.0.0");
        let _ = embedded(text, "1.0.0");
        let _ = embedded(text, text);
    }
    // Carriage returns are not part of the text.
    let crlf = Changelog::parse(&texts[6]).unwrap();
    assert_eq!(crlf.sections()[0].entries[0].title, "Carriage returns.");
    assert_eq!(crlf.sections()[0].entries[0].text, "Too.");
    // What follows the link definitions belongs to no section.
    let after = Changelog::parse(&texts[7]).unwrap();
    assert_eq!(after.sections().len(), 2);
    // Too large a file is refused before it is read.
    let large = "x".repeat(MAX_BYTES + 1);
    assert!(Changelog::parse(&large).is_err());
}

#[test]
fn what_a_program_carries_reads_back_and_leaves_out_what_is_new_to_nobody() {
    let carried = embedded(SAMPLE, "0.3.0").unwrap();
    let read = Changelog::parse(&carried).unwrap();
    let whole = sample();
    // Every section that can be shown as new is there, word for word.
    for version in ["0.3.0", "0.2.1", "0.2.0"] {
        assert_eq!(read.version(v(version)), whole.version(v(version)));
    }
    assert_eq!(read.sections()[0], whole.sections()[0]);
    // The first release is still known, without its notes; nothing can
    // be older, so nothing is offered that was not before.
    assert_eq!(read.oldest(), Some(v("0.1.0")));
    assert!(read.version(v("0.1.0")).unwrap().is_blank());
    assert!(!carried.contains("A history of features"));
    for seen in [
        Seen::Never,
        Seen::Earlier,
        Seen::Version(v("0.1.0")),
        Seen::Version(v("0.2.0")),
        Seen::Version(v("0.3.0")),
    ] {
        assert_eq!(
            read.offer(v("0.3.0"), seen),
            whole.offer(v("0.3.0"), seen),
            "{seen:?}"
        );
    }
    // The preamble and the link definitions are not carried.
    assert!(carried.starts_with("## [Unreleased]\n"));
    assert!(!carried.contains("All notable changes"));
    assert!(!carried.contains("example.org/compare"));
    // A program that is the first release carries its own notes.
    let first = "## [0.1.0] - 2026-11-11\n\n### Modelling\n\n- A history.\n";
    let carried = embedded(first, "0.1.0").unwrap();
    assert!(carried.contains("A history."));
    // An empty Unreleased section is not carried.
    let released = "## [Unreleased]\n\n## [0.2.0]\n\n- x\n\n## [0.1.0]\n\n- y\n";
    assert_eq!(
        embedded(released, "0.2.0").unwrap(),
        "## [0.2.0]\n\n- x\n\n## [0.1.0]\n"
    );
}

#[test]
fn a_changelog_a_program_cannot_carry_is_an_error() {
    // No notes for the version being built.
    let error = embedded(SAMPLE, "0.4.0").unwrap_err();
    assert!(
        error.message().contains("no notes under `## [0.4.0]`"),
        "{error}"
    );
    let blank = "## [0.2.0] - 2026-12-01\n\n## [0.1.0]\n\n- x\n";
    assert!(embedded(blank, "0.2.0").is_err());
    // A version in the file newer than the program's.
    let error = embedded(SAMPLE, "0.2.1").unwrap_err();
    assert!(
        error
            .message()
            .contains("names version 0.3.0, which is newer than this one, 0.2.1"),
        "{error}"
    );
    // A version that is not one, and a file that does not read.
    assert!(embedded(SAMPLE, "next").is_err());
    assert!(embedded("## [banana]\n", "0.1.0").is_err());
    assert!(embedded("", "0.1.0").is_err());
}

/// The repository's changelog is in the form, every released version says
/// something, and what a program of the workspace's version carries of it
/// reads back.
#[test]
fn the_repositorys_changelog_reads() {
    let text = real();
    let changelog = Changelog::parse(&text).unwrap();
    let running = v(env!("CARGO_PKG_VERSION"));
    assert!(changelog.version(running).is_some());
    for section in changelog.sections() {
        if section.version.is_some() {
            assert!(!section.is_blank(), "{} says nothing", section.label());
            assert!(section.date.is_some(), "{} has no date", section.label());
        }
        for entry in &section.entries {
            assert!(!entry.title.is_empty());
            for text in [&entry.title, &entry.text] {
                assert!(!text.contains("**"), "{text}");
                assert!(!text.contains('`'), "{text}");
                assert!(!text.contains("](http"), "{text}");
            }
        }
    }
    let carried = embedded(&text, env!("CARGO_PKG_VERSION")).unwrap();
    let read = Changelog::parse(&carried).unwrap();
    let first = changelog.oldest().unwrap();
    for section in changelog.sections() {
        match section.version {
            Some(version) if version == first && version != running => {
                assert!(read.version(version).unwrap().is_blank());
            }
            Some(version) => assert_eq!(read.version(version), Some(section)),
            None if section.is_blank() => {}
            None => assert_eq!(read.sections().first(), Some(section)),
        }
    }
}

/// What changes an answer is told by its group alone, so a group whose
/// heading is nearly that one (a typing mistake, another wording) would
/// file its entries with the rest without a word: a heading that speaks of
/// results is the heading.
#[test]
fn a_group_that_speaks_of_results_has_the_heading_the_programs_know() {
    let changelog = Changelog::parse(&real()).unwrap();
    for section in changelog.sections() {
        for entry in &section.entries {
            if entry.heading.to_lowercase().contains("result") {
                assert_eq!(
                    entry.heading,
                    RESULTS_HEADING,
                    "{}: the group `{}` is not `{RESULTS_HEADING}`",
                    section.label(),
                    entry.heading
                );
                assert!(entry.changes_answer());
            }
        }
    }
}

/// The reading is the Release workflow's: for every version, the lines a
/// section is parsed from are the lines the draft release gets.
#[test]
fn every_version_of_the_repositorys_changelog_has_a_release_section() {
    let text = real();
    let changelog = Changelog::parse(&text).unwrap();
    for section in changelog.sections() {
        let name = match section.version {
            Some(version) => version.to_string(),
            None => "Unreleased".to_string(),
        };
        let notes = release_section(&text, &name).unwrap();
        assert!(!notes.contains("\n## ["), "{name}");
        // Parsed alone under its heading, the notes are the section.
        let alone = Changelog::parse(&format!("## [{name}]\n{notes}")).unwrap();
        assert_eq!(alone.sections()[0].entries, section.entries, "{name}");
        assert_eq!(alone.sections()[0].summary, section.summary, "{name}");
        assert_eq!(alone.sections()[0].notes, section.notes, "{name}");
    }
}

/// Nothing under a group is dropped without a word: what is not an entry
/// (a paragraph before or after the entries, a numbered list, a table) is
/// the group's own note, in the file's order, without markup; an entry
/// still goes on only with an indented line.
#[test]
fn what_stands_under_a_group_and_is_no_entry_is_the_groups_note() {
    let text = "\
## [1.0.0] - 2026-01-01

The release in a sentence.

### Results that changed

What was answered before can *differ* now.
Check [it](https://example.org) again.

- **A measure was wrong.** By a little,
  on a second line.

A paragraph after the entries.

### Fixed

- **A fix.**

1. A numbered list
2. is no entry

| a | table |
|---|-------|

### Added

- **Only entries here.**
";
    let changelog = Changelog::parse(text).unwrap();
    let section = &changelog.sections()[0];
    assert_eq!(section.summary, "The release in a sentence.");
    let said: Vec<(Kind, &str, &str)> = section
        .notes
        .iter()
        .map(|note| (note.kind, note.heading.as_str(), note.text.as_str()))
        .collect();
    assert_eq!(
        said,
        [
            (
                Kind::Results,
                "Results that changed",
                "What was answered before can differ now. Check it again.\nA paragraph after the entries."
            ),
            (
                Kind::Fixed,
                "Fixed",
                "1. A numbered list 2. is no entry\n| a | table | |---|-------|"
            ),
        ]
    );
    assert_eq!(
        section.notes_of(Kind::Results).count(),
        1,
        "by kind, as the entries"
    );
    assert_eq!(section.notes_of(Kind::Added).count(), 0);
    // The entries are what they were.
    let titles: Vec<&str> = section.entries.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(
        titles,
        ["A measure was wrong.", "A fix.", "Only entries here."]
    );
    assert_eq!(section.entries[0].text, "By a little, on a second line.");
    // A group that only says something makes its section say something.
    let only = Changelog::parse("## [1.0.0]\n\n### Fixed\n\nNothing to list.\n").unwrap();
    assert!(!only.sections()[0].is_blank());
    assert_eq!(only.sections()[0].notes[0].text, "Nothing to list.");
    // The repository's own: the sentence under the results of what is not
    // released yet is carried, word for word.
    let real = Changelog::parse(&real()).unwrap();
    for section in real.sections() {
        for note in &section.notes {
            assert!(!note.text.is_empty());
            assert!(!note.text.contains("**"), "{}", note.text);
        }
    }
}

/// The reader's time grows with the text's length, not with its square:
/// text that opens a link, an address or a code span over and over
/// without closing it is not looked through once per opening. Before
/// 2026-11-15 an entry of 800 000 `<` took 18 s in a release build (four
/// times as long per doubling); every pattern here now reads in well
/// under a second, and the bound is generous for a loaded machine.
#[test]
fn hostile_markup_reads_in_time_in_proportion_to_its_length() {
    let falling: String = (1..1200usize)
        .rev()
        .map(|n| format!("{}x", "`".repeat(n)))
        .collect();
    let patterns: Vec<(&str, String)> = vec![
        ("<", "<".repeat(800_000)),
        ("<http://", "<http://".repeat(100_000)),
        ("<a<>", "<a<>".repeat(200_000)),
        ("<https://a b>", "<https://a b>".repeat(60_000)),
        ("[", "[".repeat(800_000)),
        ("[ then ] far off", format!("{}]", "[".repeat(800_000))),
        ("[a][", "[a][".repeat(200_000)),
        ("[a](", "[a](".repeat(200_000)),
        ("falling runs of backticks", falling),
        ("` alone", "` ".repeat(400_000)),
        ("&", "&".repeat(800_000)),
        ("\\", "\\".repeat(800_000)),
        ("*", "* ".repeat(400_000)),
    ];
    for (name, text) in &patterns {
        let start = std::time::Instant::now();
        let read = plain(text);
        let took = start.elapsed();
        assert!(
            took < std::time::Duration::from_secs(5),
            "{name}: {} bytes took {took:?}",
            text.len()
        );
        // Nothing of it was markup: it is all still there, but for the
        // white space.
        let kept = |text: &str| text.chars().filter(|c| !c.is_whitespace()).count();
        if !matches!(*name, "<a<>" | "[a][" | "` alone" | "\\" | "*") {
            assert_eq!(kept(&read), kept(text), "{name}");
        }
        // And as an entry of a changelog, read whole.
        let start = std::time::Instant::now();
        let _ = Changelog::parse(&format!("## [1.0.0]\n\n### Fixed\n\n- {text}\n"));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "{name}: as an entry, {:?}",
            start.elapsed()
        );
    }
    // What is found once is still found: an address far behind many `<`,
    // and a link behind many `[`.
    let text = format!("{}<https://example.org/x> [w](y)", "< [ ".repeat(50_000));
    assert!(plain(&text).ends_with("https://example.org/x w"));
    // An address longer than an address is, is not one.
    let long = format!("<https://{}>", "a".repeat(5000));
    assert_eq!(plain(&long), long);
    // A code span longer than a code span is, is not one: its ticks stay.
    let span = format!("`{}`", "c".repeat(5000));
    assert_eq!(plain(&span), span);
    let short = format!("`{}`", "c".repeat(4000));
    assert_eq!(plain(&short), "c".repeat(4000));
}
