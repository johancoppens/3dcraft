//! Saving and loading on disk: round trips and atomicity.
#![cfg(not(target_arch = "wasm32"))]
// Helpers outside `#[test]` functions are test code too (clippy.toml only
// exempts the test functions themselves).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{Block, Document, Expr, FeatureKind, Units};
use craft3d_format::{FormatError, load, save, to_bytes};
use std::fs;
use std::path::{Path, PathBuf};

fn document(width: &str) -> Document {
    let mut doc = Document::new(Units::default());
    doc.set_variable("width", width).unwrap();
    doc.add_feature(
        None,
        FeatureKind::Block(Block {
            position: ["0 mm", "0 mm", "0 mm"].map(Expr::new),
            size: ["width", "width", "width"].map(Expr::new),
            orientation: Default::default(),
        }),
    )
    .unwrap();
    doc
}

/// A fresh, empty directory for one test, removed when the test passes
/// (and left for inspection when it fails).
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("format-disk-{test}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Names of everything in the directory, sorted.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn done(self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn save_then_load_round_trips_and_leaves_no_temporary_file() {
    let scratch = Scratch::new("round-trip");
    let path = scratch.path("bracket.3dcraft");
    let doc = document("40 mm");
    save(&path, &doc).unwrap();
    assert_eq!(scratch.names(), ["bracket.3dcraft"]);
    assert_eq!(load(&path).unwrap(), doc);
    // What is on disk is exactly what `to_bytes` gives.
    assert_eq!(fs::read(&path).unwrap(), to_bytes(&doc).unwrap());
    scratch.done();
}

#[test]
fn save_over_an_existing_file_replaces_it() {
    let scratch = Scratch::new("replace");
    let path = scratch.path("part.3dcraft");
    save(&path, &document("40 mm")).unwrap();
    let newer = document("2 in");
    save(&path, &newer).unwrap();
    assert_eq!(load(&path).unwrap(), newer);
    // Also over something that is not a document at all, and longer than
    // the new file (no stale tail).
    fs::write(&path, vec![b'x'; 100_000]).unwrap();
    save(&path, &newer).unwrap();
    assert_eq!(fs::read(&path).unwrap(), to_bytes(&newer).unwrap());
    assert_eq!(scratch.names(), ["part.3dcraft"]);
    scratch.done();
}

#[test]
fn many_saves_in_a_row_leave_one_file() {
    let scratch = Scratch::new("repeat");
    let path = scratch.path("part.3dcraft");
    for width in 1..=20 {
        save(&path, &document(&format!("{width} mm"))).unwrap();
    }
    assert_eq!(load(&path).unwrap(), document("20 mm"));
    assert_eq!(scratch.names(), ["part.3dcraft"]);
    scratch.done();
}

#[test]
fn save_into_a_missing_directory_fails_and_leaves_nothing_behind() {
    let scratch = Scratch::new("missing-dir");
    let path = scratch.path("no-such-dir").join("part.3dcraft");
    let error = save(&path, &document("40 mm")).unwrap_err();
    assert!(
        matches!(&error, FormatError::Io { action: "save", path: p, .. } if p == &path),
        "{error}"
    );
    let text = error.to_string();
    assert!(
        text.starts_with("cannot save") && text.contains("no-such-dir"),
        "{text}"
    );
    assert_eq!(scratch.names(), [] as [&str; 0], "nothing was created");
    scratch.done();
}

/// The rename is the last step, so making it fail (the target is a
/// directory) exercises the clean-up after the temporary file was written.
#[test]
fn a_save_that_fails_at_the_rename_removes_its_temporary_file() {
    let scratch = Scratch::new("rename-fails");
    let path = scratch.path("taken.3dcraft");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("inside.txt"), "kept").unwrap();
    let error = save(&path, &document("40 mm")).unwrap_err();
    assert!(matches!(error, FormatError::Io { .. }), "{error}");
    assert_eq!(scratch.names(), ["taken.3dcraft"], "no temporary file");
    assert_eq!(fs::read_to_string(path.join("inside.txt")).unwrap(), "kept");
    scratch.done();
}

#[test]
fn a_failed_save_leaves_the_existing_file_untouched() {
    let scratch = Scratch::new("untouched");
    let path = scratch.path("part.3dcraft");
    save(&path, &document("40 mm")).unwrap();
    let before = fs::read(&path).unwrap();
    // A document that breaks the model's rules is not written.
    let mut invalid = document("50 mm");
    invalid.tolerance.linear = -1.0;
    let error = save(&path, &invalid).unwrap_err();
    assert!(matches!(error, FormatError::CannotSave(_)), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(scratch.names(), ["part.3dcraft"]);
    assert_eq!(load(&path).unwrap(), document("40 mm"));
    scratch.done();
}

#[test]
fn a_path_without_a_file_name_is_an_error() {
    let scratch = Scratch::new("no-name");
    for path in [scratch.path(".."), PathBuf::from("/"), PathBuf::new()] {
        let error = save(&path, &document("40 mm")).unwrap_err();
        assert!(matches!(error, FormatError::Io { .. }), "{error}");
    }
    assert_eq!(scratch.names(), [] as [&str; 0]);
    scratch.done();
}

#[test]
fn loading_what_is_not_there_or_not_a_document_is_an_error() {
    let scratch = Scratch::new("load-errors");
    let missing = scratch.path("missing.3dcraft");
    let error = load(&missing).unwrap_err();
    assert!(
        matches!(&error, FormatError::Io { action: "open", .. }),
        "{error}"
    );
    assert!(error.to_string().contains("missing.3dcraft"), "{error}");
    // A directory.
    assert!(matches!(load(&scratch.0), Err(FormatError::Io { .. })));
    // An empty file, and one with other content.
    let path = scratch.path("empty.3dcraft");
    fs::write(&path, b"").unwrap();
    assert!(matches!(load(&path), Err(FormatError::Zip(_))));
    fs::write(&path, b"solid ascii\nendsolid\n").unwrap();
    assert!(matches!(load(&path), Err(FormatError::Zip(_))));
    // A file cut in half, as an interrupted copy leaves it.
    let bytes = to_bytes(&document("40 mm")).unwrap();
    fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    assert!(load(&path).is_err());
    scratch.done();
}

#[test]
fn a_file_over_the_size_cap_is_refused_without_reading_it() {
    let scratch = Scratch::new("too-large");
    let path = scratch.path("huge.3dcraft");
    // A sparse file: it has the size but takes no disk space.
    let file = fs::File::create(&path).unwrap();
    file.set_len(craft3d_format::MAX_FILE_BYTES + 1).unwrap();
    drop(file);
    let error = load(&path).unwrap_err();
    assert!(matches!(error, FormatError::TooLarge { .. }), "{error}");
    scratch.done();
}
