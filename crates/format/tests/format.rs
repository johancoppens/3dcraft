//! The `.3dcraft` format in memory: round trips, the exact text on disk,
//! and every way a file can be wrong.

// Helpers outside `#[test]` functions are test code too (clippy.toml only
// exempts the test functions themselves).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{
    AngleUnit, Block, Cylinder, Document, Expr, FeatureId, FeatureKind, LengthUnit, Sphere, Units,
};
use craft3d_format::{
    FORMAT_VERSION, FormatError, MANIFEST_NAME, MAX_FILE_BYTES, MAX_MANIFEST_BYTES, from_bytes,
    from_manifest_json, to_bytes, to_manifest_json,
};
use craft3d_zipstore::{read_stored, write_stored};

fn exprs(values: [&str; 3]) -> [Expr; 3] {
    values.map(Expr::new)
}

/// One variable and one block: small enough to write its file out in full.
fn small() -> Document {
    let mut doc = Document::new(Units::default());
    doc.set_variable("width", "40 mm").unwrap();
    doc.add_feature(
        None,
        FeatureKind::Block(Block {
            position: exprs(["0 mm", "0 mm", "0 mm"]),
            size: exprs(["width", "width / 2", "10 mm"]),
        }),
    )
    .unwrap();
    doc
}

/// Every feature kind, variables (one of them a formula that does not
/// parse: it is stored all the same), a suppressed feature, a gap in the
/// ids and a rollback marker.
fn full(units: Units) -> Document {
    let mut doc = small();
    doc.units = units;
    doc.tolerance.linear = 2.5e-7;
    doc.tolerance.angular = 1e-10;
    doc.set_variable("half", "width / 2").unwrap();
    doc.set_variable("broken", "width +").unwrap();
    let pin = doc
        .add_feature(
            Some("Pin \u{2300}5"),
            FeatureKind::Cylinder(Cylinder {
                position: exprs(["half", "0 mm", "10 mm"]),
                radius: Expr::new("width / 8"),
                height: Expr::new("2 * half"),
            }),
        )
        .unwrap();
    let gone = doc
        .add_feature(
            None,
            FeatureKind::Sphere(Sphere {
                position: exprs(["0", "0", "0"]),
                radius: Expr::new("1 mm"),
            }),
        )
        .unwrap();
    doc.remove_feature(gone).unwrap();
    doc.add_feature(
        Some("Ball"),
        FeatureKind::Sphere(Sphere {
            position: exprs(["0 mm", "-half", "1e3 um"]),
            radius: Expr::new("\"quoted\" \\ and \n newline"),
        }),
    )
    .unwrap();
    doc.feature_mut(pin).unwrap().suppressed = true;
    doc.set_rollback(Some(pin)).unwrap();
    doc.validate().unwrap();
    doc
}

/// A `.3dcraft` file around a manifest written by hand.
fn file_with(manifest: &str) -> Vec<u8> {
    write_stored(&[(MANIFEST_NAME, manifest.as_bytes())]).unwrap()
}

/// The manifest of [`small`] with one piece of text replaced.
fn edited(from: &str, to: &str) -> String {
    let json = to_manifest_json(&small()).unwrap();
    assert!(json.contains(from), "`{from}` is not in the manifest");
    json.replacen(from, to, 1)
}

fn load_error(manifest: &str) -> FormatError {
    let from_file = from_bytes(&file_with(manifest)).unwrap_err();
    let direct = from_manifest_json(manifest.as_bytes()).unwrap_err();
    assert_eq!(from_file.to_string(), direct.to_string());
    from_file
}

#[test]
fn an_empty_document_round_trips() {
    for doc in [Document::new(Units::default()), Document::default()] {
        // `Document::default()` has next_feature_id 0; both are valid.
        assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    }
}

#[test]
fn a_document_with_everything_round_trips_in_every_unit() {
    for length in LengthUnit::ALL {
        for angle in [AngleUnit::Degree, AngleUnit::Radian] {
            let doc = full(Units { length, angle });
            let back = from_bytes(&to_bytes(&doc).unwrap()).unwrap();
            assert_eq!(back, doc, "{length:?} {angle:?}");
        }
    }
    // What the fixture is meant to contain.
    let doc = full(Units::default());
    let kinds: Vec<_> = doc.features.iter().map(|f| f.kind.type_name()).collect();
    assert_eq!(kinds, ["block", "cylinder", "sphere"]);
    assert!(doc.features[1].suppressed && !doc.features[0].suppressed);
    assert_eq!(doc.rollback, Some(FeatureId(2)));
    assert_eq!(doc.features[2].id, FeatureId(4), "ids are not reused");
    assert_eq!(doc.next_feature_id, 5);
    assert_eq!(doc.variables.len(), 3);
}

#[test]
fn saving_twice_gives_the_same_bytes() {
    let doc = full(Units::default());
    let bytes = to_bytes(&doc).unwrap();
    assert_eq!(bytes, to_bytes(&doc).unwrap());
    assert_eq!(bytes, to_bytes(&doc.clone()).unwrap());
    // And saving what was loaded changes nothing.
    assert_eq!(bytes, to_bytes(&from_bytes(&bytes).unwrap()).unwrap());
}

/// The on-disk shape, in full. A change here is a change of the file
/// format: is it a new optional field, or does it need a version bump and
/// a migration (AGENTS.md §8)?
#[test]
fn the_manifest_of_a_small_document_is_exactly_this() {
    let expected = r#"{
  "format": "3dcraft",
  "format_version": 1,
  "document": {
    "units": {
      "length": "mm",
      "angle": "deg"
    },
    "tolerance": {
      "linear": 1e-6,
      "angular": 1e-9
    },
    "variables": [
      {
        "name": "width",
        "expr": "40 mm"
      }
    ],
    "features": [
      {
        "id": 1,
        "name": "Block1",
        "suppressed": false,
        "kind": {
          "type": "block",
          "position": [
            "0 mm",
            "0 mm",
            "0 mm"
          ],
          "size": [
            "width",
            "width / 2",
            "10 mm"
          ]
        }
      }
    ],
    "rollback": null,
    "next_feature_id": 2
  }
}
"#;
    assert_eq!(to_manifest_json(&small()).unwrap(), expected);
    assert_eq!(from_manifest_json(expected.as_bytes()).unwrap(), small());
}

#[test]
fn the_file_is_a_zip_with_one_stored_manifest() {
    let bytes = to_bytes(&small()).unwrap();
    let entries = read_stored(&bytes).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "manifest.json");
    assert_eq!(
        entries[0].data,
        to_manifest_json(&small()).unwrap().as_bytes()
    );
}

#[test]
fn the_other_feature_kinds_are_written_like_this() {
    let json = to_manifest_json(&full(Units {
        length: LengthUnit::Inch,
        angle: AngleUnit::Radian,
    }))
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let document = &value["document"];
    assert_eq!(
        document["units"],
        serde_json::json!({"length": "in", "angle": "rad"})
    );
    assert_eq!(
        document["features"][1],
        serde_json::json!({
            "id": 2, "name": "Pin \u{2300}5", "suppressed": true,
            "kind": {
                "type": "cylinder", "position": ["half", "0 mm", "10 mm"],
                "radius": "width / 8", "height": "2 * half"
            }
        })
    );
    assert_eq!(
        document["features"][2]["kind"],
        serde_json::json!({
            "type": "sphere", "position": ["0 mm", "-half", "1e3 um"],
            "radius": "\"quoted\" \\ and \n newline"
        })
    );
    assert_eq!(document["rollback"], serde_json::json!(2));
    // Expressions are the text the user wrote, even when it does not parse.
    assert_eq!(
        document["variables"][2],
        serde_json::json!({"name": "broken", "expr": "width +"})
    );
    for unit in LengthUnit::ALL {
        let doc = Document::new(Units {
            length: unit,
            angle: AngleUnit::Degree,
        });
        let json = to_manifest_json(&doc).unwrap();
        assert!(
            json.contains(&format!("\"length\": \"{}\"", unit.symbol())),
            "{json}"
        );
    }
}

#[test]
fn fields_that_are_optional_by_design_may_be_absent() {
    let without_rollback = edited("    \"rollback\": null,\n", "");
    assert_eq!(
        from_manifest_json(without_rollback.as_bytes()).unwrap(),
        small()
    );
    let without_suppressed = edited("        \"suppressed\": false,\n", "");
    assert_eq!(
        from_manifest_json(without_suppressed.as_bytes()).unwrap(),
        small()
    );
}

#[test]
fn every_other_field_is_required() {
    for (line, field) in [
        ("    \"next_feature_id\": 2\n", "next_feature_id"),
        ("        \"id\": 1,\n", "id"),
        ("        \"name\": \"Block1\",\n", "name"),
        ("          \"type\": \"block\",\n", "type"),
        ("      \"length\": \"mm\",\n", "length"),
        ("      \"linear\": 1e-6,\n", "linear"),
        ("        \"expr\": \"40 mm\"\n", "expr"),
    ] {
        // Removing a last line leaves a trailing comma on the one before.
        let mut json = edited(line, "");
        if !line.ends_with(",\n") {
            json = fix_trailing_commas(&json);
        }
        let error = load_error(&json);
        assert!(
            matches!(&error, FormatError::Shape { message, .. } if message.contains(field)),
            "{field}: {error}"
        );
    }
}

/// Removes a comma that is followed only by whitespace and a closing
/// bracket, which removing the last field of an object leaves behind.
fn fix_trailing_commas(json: &str) -> String {
    let mut out = String::new();
    let mut rest = json;
    while let Some(at) = rest.find(',') {
        let after = rest[at + 1..].trim_start();
        out.push_str(&rest[..at]);
        if !(after.starts_with('}') || after.starts_with(']')) {
            out.push(',');
        }
        rest = &rest[at + 1..];
    }
    out.push_str(rest);
    out
}

#[test]
fn a_newer_format_version_says_a_newer_3dcraft_is_needed() {
    let error = load_error(&edited("\"format_version\": 1", "\"format_version\": 7"));
    assert!(matches!(
        error,
        FormatError::NewerVersion {
            file: 7,
            supported: FORMAT_VERSION
        }
    ));
    let text = error.to_string();
    assert!(
        text.contains("version 7")
            && text.contains(&format!("version {FORMAT_VERSION}"))
            && text.contains("newer 3DCraft is needed"),
        "{text}"
    );
    // A newer file is likely to have fields this build does not know; the
    // version is what gets reported, not the fields.
    let newer = edited(
        "\"format_version\": 1",
        "\"format_version\": 2, \"sketches\": []",
    );
    assert!(matches!(
        load_error(&newer),
        FormatError::NewerVersion { file: 2, .. }
    ));
    // Far beyond 32 bits.
    let huge = edited(
        "\"format_version\": 1",
        "\"format_version\": 18446744073709551615",
    );
    assert!(matches!(
        load_error(&huge),
        FormatError::NewerVersion { file: u64::MAX, .. }
    ));
}

#[test]
fn a_missing_or_unusable_format_version_is_an_error() {
    let missing = load_error(&edited("  \"format_version\": 1,\n", ""));
    assert!(matches!(&missing, FormatError::BadVersion(_)), "{missing}");
    assert!(missing.to_string().contains("format_version"), "{missing}");
    assert!(missing.to_string().contains("missing"), "{missing}");
    for bad in ["0", "-1", "1.5", "\"1\"", "null", "[1]", "1e400"] {
        let json = edited(
            "\"format_version\": 1",
            &format!("\"format_version\": {bad}"),
        );
        let error = from_manifest_json(json.as_bytes()).unwrap_err();
        assert!(
            matches!(error, FormatError::BadVersion(_) | FormatError::Json(_)),
            "{bad}: {error}"
        );
    }
    let zero = load_error(&edited("\"format_version\": 1", "\"format_version\": 0"));
    assert!(matches!(zero, FormatError::BadVersion(_)), "{zero}");
}

#[test]
fn a_missing_or_wrong_format_name_is_an_error() {
    let missing = load_error(&edited("  \"format\": \"3dcraft\",\n", ""));
    assert!(matches!(&missing, FormatError::WrongFormat { found: None }));
    assert!(
        missing.to_string().contains("not a .3dcraft file")
            && missing.to_string().contains("no `format` field"),
        "{missing}"
    );
    let other = load_error(&edited("\"3dcraft\"", "\"photocraft\""));
    assert!(
        matches!(&other, FormatError::WrongFormat { found: Some(f) } if f == "\"photocraft\""),
        "{other}"
    );
    assert!(other.to_string().contains("\"photocraft\""), "{other}");
    assert!(other.to_string().contains("\"3dcraft\""), "{other}");
    for wrong in ["7", "null", "[\"3dcraft\"]", "\"3DCraft\"", "\"\""] {
        let error = load_error(&edited("\"3dcraft\"", wrong));
        assert!(
            matches!(error, FormatError::WrongFormat { found: Some(_) }),
            "{wrong}: {error}"
        );
    }
    // A hostile value does not end up in the message at full length.
    let long = format!("\"{}\"", "x".repeat(100_000));
    let error = load_error(&edited("\"3dcraft\"", &long));
    assert!(error.to_string().len() < 400, "{}", error.to_string().len());
    // Another program's manifest.json.
    assert!(matches!(
        load_error(r#"{"name": "my-extension", "version": "1.0"}"#),
        FormatError::WrongFormat { found: None }
    ));
}

#[test]
fn a_file_without_a_manifest_is_an_error() {
    let bytes = write_stored(&[("thumb.png", b"\x89PNG"), ("Manifest.json", b"{}")]).unwrap();
    let error = from_bytes(&bytes).unwrap_err();
    assert!(matches!(error, FormatError::MissingManifest), "{error}");
    assert!(error.to_string().contains("manifest.json"), "{error}");
    let empty = write_stored(&[]).unwrap();
    assert!(matches!(
        from_bytes(&empty),
        Err(FormatError::MissingManifest)
    ));
}

#[test]
fn invalid_json_is_an_error_with_a_position() {
    let error = load_error(&edited("\"variables\": [", "\"variables\": "));
    assert!(matches!(error, FormatError::Json(_)), "{error}");
    let error = load_error("{\n  \"format\": \"3dcraft\",\n  oops\n}");
    assert!(matches!(&error, FormatError::Json(_)), "{error}");
    let text = error.to_string();
    assert!(
        text.contains("not valid JSON") && text.contains("line 3"),
        "{text}"
    );
    for bad in [
        "",
        " ",
        "{",
        "nul",
        "{\"format\": \"3dcraft\",}",
        "\u{feff}{}",
    ] {
        assert!(matches!(load_error(bad), FormatError::Json(_)), "`{bad}`");
    }
    // Not UTF-8.
    assert!(matches!(
        from_manifest_json(b"{\"format\": \"\xFF\"}"),
        Err(FormatError::Json(_))
    ));
}

#[test]
fn json_that_is_not_an_object_is_an_error() {
    for other in ["[]", "7", "\"3dcraft\"", "null", "true"] {
        let error = load_error(other);
        assert!(
            matches!(error, FormatError::NotAnObject),
            "{other}: {error}"
        );
    }
}

#[test]
fn deeply_nested_json_is_an_error_not_a_stack_overflow() {
    let deep = "[".repeat(200_000);
    assert!(matches!(load_error(&deep), FormatError::Json(_)));
    let deep = format!(
        "{{\"format\": \"3dcraft\", \"format_version\": 1, \"document\": {}",
        "{\"a\":".repeat(100_000)
    );
    assert!(matches!(load_error(&deep), FormatError::Json(_)));
}

#[test]
fn wrong_types_are_errors_that_say_where() {
    let cases: [(&str, &str, &str, &str); 12] = [
        (
            "\"id\": 1",
            "\"id\": \"one\"",
            "document.features[0]",
            "u64",
        ),
        ("\"id\": 1", "\"id\": -1", "document.features[0]", "u64"),
        ("\"id\": 1", "\"id\": 1.5", "document.features[0]", "u64"),
        (
            "\"suppressed\": false",
            "\"suppressed\": 0",
            "document.features[0]",
            "boolean",
        ),
        (
            "\"type\": \"block\"",
            "\"type\": \"cone\"",
            "document.features[0].kind",
            "unknown variant `cone`",
        ),
        (
            "\"width / 2\",\n",
            "",
            "document.features[0].kind",
            "length 2",
        ),
        (
            "\"width / 2\"",
            "20",
            "document.features[0].kind",
            "expected a string",
        ),
        (
            "\"length\": \"mm\"",
            "\"length\": \"furlong\"",
            "document.units",
            "unknown variant `furlong`",
        ),
        (
            "\"linear\": 1e-6",
            "\"linear\": \"tight\"",
            "document.tolerance",
            "f64",
        ),
        (
            "\"expr\": \"40 mm\"",
            "\"expr\": 40",
            "document.variables[0]",
            "expected a string",
        ),
        (
            "\"rollback\": null",
            "\"rollback\": \"Block1\"",
            "document",
            "u64",
        ),
        (
            "\"next_feature_id\": 2",
            "\"next_feature_id\": []",
            "document",
            "u64",
        ),
    ];
    for (from, to, path, message) in cases {
        let error = load_error(&edited(from, to));
        assert!(
            matches!(&error, FormatError::Shape { path: p, message: m } if p == path && m.contains(message)),
            "{from} -> {to}: {error:?}"
        );
        assert!(error.to_string().contains(path), "{error}");
    }
    // The whole document, or a list, of the wrong type.
    let json = r#"{"format": "3dcraft", "format_version": 1, "document": []}"#;
    assert!(matches!(
        load_error(json),
        FormatError::Shape { path, .. } if path == "document"
    ));
    let json = r#"{"format": "3dcraft", "format_version": 1}"#;
    assert!(matches!(
        load_error(json),
        FormatError::Shape { path, message } if path.is_empty() && message.contains("document")
    ));
    let error = load_error(&edited("\"variables\": [", "\"variables\": 3, \"x\": ["));
    assert!(matches!(error, FormatError::Shape { .. }), "{error}");
}

#[test]
fn an_unknown_field_is_an_error_at_every_level() {
    let cases = [
        ("\"format\":", "\"author\": \"me\", \"format\":", ""),
        ("\"units\":", "\"sketches\": [], \"units\":", "document"),
        (
            "\"length\":",
            "\"mass\": \"kg\", \"length\":",
            "document.units",
        ),
        (
            "\"linear\":",
            "\"chordal\": 0.1, \"linear\":",
            "document.tolerance",
        ),
        (
            "\"expr\":",
            "\"comment\": \"\", \"expr\":",
            "document.variables[0]",
        ),
        (
            "\"id\":",
            "\"colour\": \"red\", \"id\":",
            "document.features[0]",
        ),
        (
            "\"size\":",
            "\"chamfer\": \"1 mm\", \"size\":",
            "document.features[0].kind",
        ),
    ];
    for (from, to, path) in cases {
        let error = load_error(&edited(from, to));
        assert!(
            matches!(&error, FormatError::Shape { path: p, message } if p == path && message.contains("unknown field")),
            "{to}: {error:?}"
        );
    }
    // A parameter of another feature kind is unknown too.
    let error = load_error(&edited("\"size\":", "\"radius\": \"1 mm\", \"size\":"));
    assert!(matches!(error, FormatError::Shape { .. }), "{error}");
}

#[test]
fn extra_zip_entries_are_ignored() {
    let manifest = to_manifest_json(&small()).unwrap();
    let bytes = write_stored(&[
        ("thumb.png", b"\x89PNG not really"),
        (MANIFEST_NAME, manifest.as_bytes()),
        ("blobs/0123abcd.zst", &[0; 64]),
        ("notes/manifest.json", b"not the manifest"),
    ])
    .unwrap();
    assert_eq!(from_bytes(&bytes).unwrap(), small());
}

#[test]
fn a_document_that_breaks_the_models_rules_is_rejected() {
    // Two features with the same id, written by hand.
    let duplicate_ids = r#"{
      "format": "3dcraft",
      "format_version": 1,
      "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [],
        "features": [
          {"id": 1, "name": "A", "kind": {"type": "sphere", "position": ["0", "0", "0"], "radius": "1 mm"}},
          {"id": 1, "name": "B", "kind": {"type": "sphere", "position": ["0", "0", "0"], "radius": "2 mm"}}
        ],
        "next_feature_id": 2
      }
    }"#;
    let error = load_error(duplicate_ids);
    assert!(matches!(error, FormatError::InvalidDocument(_)), "{error}");
    let text = error.to_string();
    assert!(
        text.contains("two features have the id F1") && text.contains("manifest.json"),
        "{text}"
    );
    // The same file with distinct ids is fine: the test tests the ids.
    let distinct = duplicate_ids
        .replacen(
            "{\"id\": 1, \"name\": \"B\"",
            "{\"id\": 2, \"name\": \"B\"",
            1,
        )
        .replace("\"next_feature_id\": 2", "\"next_feature_id\": 3");
    assert_eq!(
        from_manifest_json(distinct.as_bytes())
            .unwrap()
            .features
            .len(),
        2
    );

    for (from, to) in [
        ("\"next_feature_id\": 2", "\"next_feature_id\": 1"),
        ("\"rollback\": null", "\"rollback\": 9"),
        ("\"linear\": 1e-6", "\"linear\": 0"),
        ("\"linear\": 1e-6", "\"linear\": -1e-6"),
        ("\"name\": \"Block1\"", "\"name\": \"\""),
        ("\"name\": \"width\"", "\"name\": \"mm\""),
        ("\"id\": 1", "\"id\": 0"),
    ] {
        let error = load_error(&edited(from, to));
        assert!(
            matches!(error, FormatError::InvalidDocument(_)),
            "{to}: {error}"
        );
    }
}

#[test]
fn a_document_that_breaks_the_rules_is_not_saved() {
    let mut doc = small();
    doc.tolerance.linear = f64::NAN;
    let error = to_bytes(&doc).unwrap_err();
    assert!(matches!(error, FormatError::CannotSave(_)), "{error}");
    assert!(error.to_string().contains("cannot be saved"), "{error}");
    let mut doc = small();
    doc.rollback = Some(FeatureId(99));
    assert!(matches!(
        to_manifest_json(&doc),
        Err(FormatError::CannotSave(_))
    ));
}

#[test]
fn something_that_is_not_a_zip_is_an_error_that_says_what_the_file_should_be() {
    for bytes in [&b""[..], b"hello", b"{\"format\": \"3dcraft\"}", &[0; 100]] {
        let error = from_bytes(bytes).unwrap_err();
        assert!(matches!(error, FormatError::Zip(_)), "{error}");
        assert!(error.to_string().contains("ZIP archive"), "{error}");
    }
    // A file someone repacked with compression: the message says what to do.
    let mut bytes = to_bytes(&small()).unwrap();
    let central = bytes.len() - 22 - 46 - MANIFEST_NAME.len();
    bytes[8] = 8;
    bytes[central + 10] = 8;
    let text = from_bytes(&bytes).unwrap_err().to_string();
    assert!(
        text.contains("compression is not supported") && text.contains("zip -0"),
        "{text}"
    );
}

#[test]
fn oversized_input_is_refused_before_it_is_parsed() {
    // Zeroed pages: never touched, because the size is checked first.
    let huge = vec![0_u8; usize::try_from(MAX_FILE_BYTES).unwrap() + 1];
    let error = from_bytes(&huge).unwrap_err();
    assert!(
        matches!(error, FormatError::TooLarge { size, max, .. } if size == MAX_FILE_BYTES + 1 && max == MAX_FILE_BYTES),
        "{error}"
    );
    assert!(error.to_string().contains(&MAX_FILE_BYTES.to_string()));
    drop(huge);
    // Not JSON at all, which shows the cap comes before the parser.
    let manifest = vec![b'x'; MAX_MANIFEST_BYTES + 1];
    let error = from_manifest_json(&manifest).unwrap_err();
    assert!(
        matches!(error, FormatError::TooLarge { max, .. } if max == MAX_MANIFEST_BYTES as u64),
        "{error}"
    );
    assert!(matches!(
        from_manifest_json(&manifest[..MAX_MANIFEST_BYTES]),
        Err(FormatError::Json(_))
    ));
}

#[test]
fn a_file_cut_short_anywhere_is_an_error() {
    let bytes = to_bytes(&full(Units::default())).unwrap();
    for len in 0..bytes.len() {
        assert!(from_bytes(&bytes[..len]).is_err(), "cut at {len}");
    }
    // The manifest alone: every proper prefix is an error too.
    let json = to_manifest_json(&full(Units::default())).unwrap();
    let json = json.trim_end().as_bytes();
    for len in 0..json.len() {
        assert!(from_manifest_json(&json[..len]).is_err(), "cut at {len}");
    }
}

#[test]
fn flipping_any_byte_of_a_file_never_panics_or_changes_the_document() {
    let doc = full(Units::default());
    let bytes = to_bytes(&doc).unwrap();
    let mut accepted = 0;
    for at in 0..bytes.len() {
        for mask in [0x01, 0x20, 0x80, 0xFF] {
            let mut damaged = bytes.clone();
            damaged[at] ^= mask;
            // The CRC guards the manifest, so whatever still loads is the
            // document that was saved.
            if let Ok(loaded) = from_bytes(&damaged) {
                accepted += 1;
                assert_eq!(loaded, doc, "byte {at} ^ {mask:#x}");
            }
        }
    }
    assert!(accepted > 0, "the sweep reaches the Ok branch");
}

/// Without the ZIP's CRC in the way, damage reaches the JSON parser, the
/// manifest types and validation.
#[test]
fn flipping_any_byte_of_a_manifest_never_panics() {
    let doc = full(Units::default());
    let json = to_manifest_json(&doc).unwrap().into_bytes();
    let (mut accepted, mut rejected) = (0, 0);
    for at in 0..json.len() {
        for mask in [0x01, 0x02, 0x10, 0x20, 0x80, 0xFF] {
            let mut damaged = json.clone();
            damaged[at] ^= mask;
            match from_manifest_json(&damaged) {
                Ok(loaded) => {
                    accepted += 1;
                    // Whatever is accepted is a valid document that saves.
                    loaded.validate().unwrap();
                    to_bytes(&loaded).unwrap();
                }
                Err(error) => {
                    rejected += 1;
                    assert!(!error.to_string().is_empty());
                }
            }
        }
    }
    assert!(accepted > 0 && rejected > 0, "{accepted} / {rejected}");
}

#[test]
fn every_error_reads_as_a_sentence() {
    let errors = [
        from_bytes(b"").unwrap_err(),
        from_bytes(&write_stored(&[]).unwrap()).unwrap_err(),
        load_error("{"),
        load_error("[]"),
        load_error("{}"),
        load_error(r#"{"format": "3dcraft"}"#),
        load_error(r#"{"format": "3dcraft", "format_version": 2}"#),
        load_error(r#"{"format": "3dcraft", "format_version": 1}"#),
        load_error(&edited("\"rollback\": null", "\"rollback\": 9")),
    ];
    for error in errors {
        let text = error.to_string();
        assert!(text.len() > 30, "{text}");
        assert!(
            !text.contains("FormatError") && !text.contains("Some("),
            "{text}"
        );
        assert_eq!(text, text.trim());
    }
}
