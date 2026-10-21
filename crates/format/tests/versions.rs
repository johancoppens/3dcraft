//! The format promise (docs/file-formats.md › *The promise*): a `.3dcraft`
//! file written by any format version opens in every later build.
//!
//! `tests/fixtures/versions/` holds one file per format version, `vN.3dcraft`,
//! written by the build of 3DCraft that wrote version N, never by hand:
//! `craft3d-cli run --new '{"units": "mm"}' --script vN.steps.json --out
//! vN.3dcraft`, with the CLI built from the commit named in
//! `vN.expected.json` (`written_by`). The same file holds what that build's
//! `craft3d-cli info` said about the file (features, bodies, volumes):
//! `crates/engine/tests/format_versions.rs` holds today's regeneration to
//! it. Each script exercises what its version added.
//!
//! This file holds the files themselves: every version has one, none is
//! ever edited (a hash per file), and each opens and saves as the current
//! version. Raising `FORMAT_VERSION` fails here until the new version's
//! fixture is written by the new build and its hash added below; the
//! migration step is held by the crate itself (`migrate::STEPS` does not
//! compile without it).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_format::{FORMAT_VERSION, MANIFEST_NAME, from_bytes, from_bytes_noted, to_bytes};
use serde_json::Value;
use std::path::PathBuf;

/// Per format version: the size and the FNV-1a hash of its fixture. A
/// fixture stands for the files people have on disk: it is never edited
/// or written again, so a line here never changes. A new format version
/// adds its line, with the fixture its own build wrote.
const FROZEN: &[(u32, usize, u64)] = &[
    (1, 1782, 0xa87da3fa4f701616),
    (2, 2328, 0x5127ab740203a1e8),
    (3, 6024, 0x8cc849deb528f771),
    (4, 6599, 0xb3266b3fa32508c7),
    (5, 6310, 0x36a7c0eefd9ee056),
    (6, 6796, 0xa34f24430e80ed65),
    (7, 4321, 0x9ba1a58e68c9d42b),
    (8, 11803, 0xd066a8bdefc861f3),
    (9, 4394, 0x693f7b867216d50e),
    (10, 7285, 0x4ecb49d0f2afbdd7),
    (11, 2344, 0x838a43eeb9737208),
    (12, 3579, 0xa405f45f1320f4e4),
    (13, 4800, 0x1b85a757bafde25e),
    (14, 3363, 0x74905f76a2a68089),
    (15, 3956, 0xa20b672f2a2b150e),
    (16, 4049, 0xf11d08097c891a21),
    (17, 2146, 0x4add1764866df9fe),
    (18, 5784, 0x754c0511ec8226b2),
    (19, 6098, 0x4012ebc780bfcba8),
    (20, 8445, 0x4993f05a148f855f),
    (21, 4243, 0x1120e54a06027084),
    (22, 3661, 0x3692c968f970ed7f),
    (23, 1946, 0x55e82651e2ee5a21),
    (24, 3024, 0xaf53333f2e2de6f5),
];

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/versions")
}

fn fixture(version: u32) -> Vec<u8> {
    let path = dir().join(format!("v{version}.3dcraft"));
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "format version {version} has no fixture `{}` ({e}): write it with the build that \
             raised the version (see the top of this file)",
            path.display()
        )
    })
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The `format_version` the file's manifest says it has.
fn written_version(bytes: &[u8]) -> u64 {
    let entries = craft3d_zipstore::read_stored(bytes).unwrap();
    let manifest = entries.iter().find(|e| e.name == MANIFEST_NAME).unwrap();
    let value: Value = serde_json::from_slice(manifest.data).unwrap();
    value["format_version"].as_u64().unwrap()
}

#[test]
fn every_format_version_has_a_frozen_fixture() {
    let listed: Vec<u32> = FROZEN.iter().map(|(v, _, _)| *v).collect();
    let expected: Vec<u32> = (1..=FORMAT_VERSION).collect();
    assert_eq!(
        listed, expected,
        "FORMAT_VERSION is {FORMAT_VERSION}: every version from 1 to it needs a fixture written \
         by its own build and a line in FROZEN, and no other"
    );
    for &(version, size, hash) in FROZEN {
        let bytes = fixture(version);
        assert_eq!(
            (bytes.len(), fnv1a(&bytes)),
            (size, hash),
            "v{version}.3dcraft changed: a fixture stands for files on disk and is never edited"
        );
        assert_eq!(written_version(&bytes), u64::from(version), "v{version}");
        for part in ["steps", "expected"] {
            let path = dir().join(format!("v{version}.{part}.json"));
            assert!(path.is_file(), "{} is missing", path.display());
        }
    }
    // Nothing for a version this build does not know.
    let next = FORMAT_VERSION + 1;
    assert!(!dir().join(format!("v{next}.3dcraft")).exists());
}

#[test]
fn every_fixture_opens_and_saves_as_the_current_version() {
    for version in 1..=FORMAT_VERSION {
        let bytes = fixture(version);
        let (document, warnings) = from_bytes_noted(&bytes)
            .unwrap_or_else(|e| panic!("v{version}.3dcraft does not open: {e}"));
        assert!(!document.features.is_empty(), "v{version}");
        // Only version 16 has something a later version reads differently:
        // its variable `floor`, a function since version 17.
        if version == 16 {
            assert_eq!(
                warnings,
                [
                    "variable `floor` renamed `floor_1`: `floor` is a function in expressions \
                  since format version 17"
                ],
            );
        } else {
            assert!(warnings.is_empty(), "v{version}: {warnings:?}");
        }
        let saved = to_bytes(&document).unwrap();
        assert_eq!(written_version(&saved), u64::from(FORMAT_VERSION));
        let reopened = from_bytes(&saved).unwrap();
        assert_eq!(reopened, document, "v{version} saved and reopened");
        // And saving that again gives the same bytes: nothing drifts.
        assert_eq!(to_bytes(&reopened).unwrap(), saved, "v{version}");
    }
}

#[test]
fn a_fixture_is_refused_by_a_build_that_knows_only_older_versions() {
    // What an older build says about a newer file, pinned on the newest
    // fixture with its version raised past what this build knows: the
    // promise does not run backwards, and the refusal says why.
    let bytes = fixture(FORMAT_VERSION);
    let entries = craft3d_zipstore::read_stored(&bytes).unwrap();
    let manifest = entries.iter().find(|e| e.name == MANIFEST_NAME).unwrap();
    let mut value: Value = serde_json::from_slice(manifest.data).unwrap();
    value["format_version"] = Value::from(FORMAT_VERSION + 1);
    let json = serde_json::to_vec(&value).unwrap();
    let error = craft3d_format::from_manifest_json(&json)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(&format!("format version {}", FORMAT_VERSION + 1))
            && error.contains("a newer 3DCraft is needed"),
        "{error}"
    );
}
