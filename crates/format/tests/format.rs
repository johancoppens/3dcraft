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
    AngleUnit, Block, ConstraintKind, Cylinder, DistanceDirection, Document, EntityId, Expr,
    Extrude, FeatureId, FeatureKind, LengthUnit, Sketch, SketchPlane, Sphere, Units, WorldPlane,
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
            orientation: Default::default(),
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
                orientation: Default::default(),
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
    let profile = doc
        .add_feature(Some("Profile"), FeatureKind::Sketch(sketch()))
        .unwrap();
    doc.add_feature(
        Some("Body"),
        FeatureKind::Extrude(Extrude {
            sketch: profile,
            distance: Expr::new("-half"),
            symmetric: true,
            end: Default::default(),
            reversed: false,
        }),
    )
    .unwrap();
    doc.validate().unwrap();
    doc
}

/// A `.3dcraft` file around a manifest written by hand.
fn file_with(manifest: &str) -> Vec<u8> {
    write_stored(&[(MANIFEST_NAME, manifest.as_bytes())]).unwrap()
}

/// The version line of a manifest this build writes.
fn version_field() -> String {
    format!("\"format_version\": {FORMAT_VERSION}")
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
    assert_eq!(kinds, ["block", "cylinder", "sphere", "sketch", "extrude"]);
    assert!(doc.features[1].suppressed && !doc.features[0].suppressed);
    assert_eq!(doc.rollback, Some(FeatureId(2)));
    assert_eq!(doc.features[2].id, FeatureId(4), "ids are not reused");
    assert_eq!(doc.next_feature_id, 7);
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
  "format_version": 21,
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
          ],
          "orientation": {
            "axis": [
              "0",
              "0",
              "1"
            ],
            "angle": "0 deg"
          }
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
                "radius": "width / 8", "height": "2 * half",
                "orientation": {"axis": ["0", "0", "1"], "angle": "0 deg"}
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
    let error = load_error(&edited(&version_field(), "\"format_version\": 22"));
    assert!(matches!(
        error,
        FormatError::NewerVersion {
            file: 22,
            supported: FORMAT_VERSION
        }
    ));
    let text = error.to_string();
    assert!(
        text.contains("version 22")
            && text.contains(&format!("version {FORMAT_VERSION}"))
            && text.contains("newer 3DCraft is needed"),
        "{text}"
    );
    // A newer file is likely to have fields this build does not know; the
    // version is what gets reported, not the fields.
    let newer = edited(
        &version_field(),
        &format!(
            "\"format_version\": {}, \"holograms\": []",
            FORMAT_VERSION + 1
        ),
    );
    assert!(matches!(
        load_error(&newer),
        FormatError::NewerVersion { file, .. } if file == u64::from(FORMAT_VERSION) + 1
    ));
    // Far beyond 32 bits.
    let huge = edited(&version_field(), "\"format_version\": 18446744073709551615");
    assert!(matches!(
        load_error(&huge),
        FormatError::NewerVersion { file: u64::MAX, .. }
    ));
}

#[test]
fn a_missing_or_unusable_format_version_is_an_error() {
    let missing = load_error(&edited(&format!("  {},\n", version_field()), ""));
    assert!(matches!(&missing, FormatError::BadVersion(_)), "{missing}");
    assert!(missing.to_string().contains("format_version"), "{missing}");
    assert!(missing.to_string().contains("missing"), "{missing}");
    for bad in ["0", "-1", "1.5", "\"1\"", "null", "[1]", "1e400"] {
        let json = edited(&version_field(), &format!("\"format_version\": {bad}"));
        let error = from_manifest_json(json.as_bytes()).unwrap_err();
        assert!(
            matches!(error, FormatError::BadVersion(_) | FormatError::Json(_)),
            "{bad}: {error}"
        );
    }
    let zero = load_error(&edited(&version_field(), "\"format_version\": 0"));
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
            "\"type\": \"wedge\"",
            "document.features[0].kind",
            "unknown variant `wedge`",
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

/// A file saved before 2026-10-15 may hold any positive linear tolerance
/// (`file.new` took 1e-300): it is read as the nearer end of the range a
/// document may have, and one within the range as it is. A document with
/// one outside the range is not saved.
#[test]
fn a_tolerance_outside_the_range_is_read_as_its_nearer_end() {
    let doc = small();
    let manifest = to_manifest_json(&doc).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    for (written, read) in [
        (1e-300, 1e-12),
        (5e-13, 1e-12),
        (1e-12, 1e-12),
        (3.5e-7, 3.5e-7),
        (1e3, 1e3),
        (2.5e4, 1e3),
        (1e300, 1e3),
    ] {
        value["document"]["tolerance"]["linear"] = written.into();
        let bytes = serde_json::to_vec(&value).unwrap();
        let loaded = from_manifest_json(&bytes).unwrap();
        assert_eq!(loaded.tolerance.linear, read, "{written}");
        assert_eq!(loaded.tolerance.angular, doc.tolerance.angular);
        // Said so where it changed, and only there.
        let (noted, warnings) = craft3d_format::from_manifest_json_noted(&bytes).unwrap();
        assert_eq!(noted, loaded);
        if written == read {
            assert_eq!(warnings, Vec::<String>::new(), "{written}");
        } else {
            assert_eq!(
                warnings,
                [format!(
                    "tolerance {written:e} mm read as {read:e} mm (outside 1e-12..1e3)"
                )],
                "{written}"
            );
        }
    }
    for bad in [0.0, -1.0] {
        value["document"]["tolerance"]["linear"] = bad.into();
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(from_manifest_json(&bytes).is_err(), "{bad}");
    }
    let mut far = small();
    far.tolerance.linear = 1e-300;
    assert!(matches!(to_bytes(&far), Err(FormatError::CannotSave(_))));
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

/// Version 2: cones, tori and orientations (written by hand, as a user
/// or an agent would see them in the file).
#[test]
fn cones_tori_and_orientations_round_trip() {
    use craft3d_doc::{Cone, Expr, Orientation, Torus};
    let mut doc = Document::new(Units::default());
    let turned = Orientation {
        axis: ["1", "0", "tilt"].map(Expr::new),
        angle: Expr::new("30 deg"),
    };
    doc.set_variable("tilt", "0.5").unwrap();
    doc.add_feature(
        None,
        FeatureKind::Cone(Cone {
            position: ["0", "0", "0"].map(Expr::new),
            radius: Expr::new("10 mm"),
            height: Expr::new("25 mm"),
            orientation: turned.clone(),
        }),
    )
    .unwrap();
    doc.add_feature(
        None,
        FeatureKind::Torus(Torus {
            position: ["0", "0", "40 mm"].map(Expr::new),
            major: Expr::new("20 mm"),
            minor: Expr::new("4 mm"),
            orientation: Orientation::default(),
        }),
    )
    .unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    assert_eq!(
        value["document"]["features"][0]["kind"],
        serde_json::json!({
            "type": "cone", "position": ["0", "0", "0"], "radius": "10 mm", "height": "25 mm",
            "orientation": {"axis": ["1", "0", "tilt"], "angle": "30 deg"}
        })
    );
    assert_eq!(
        value["document"]["features"][1]["kind"],
        serde_json::json!({
            "type": "torus", "position": ["0", "0", "40 mm"], "major": "20 mm", "minor": "4 mm",
            "orientation": {"axis": ["0", "0", "1"], "angle": "0 deg"}
        })
    );
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
}

/// A version-1 file has no orientations and no cones or tori; it opens as
/// the same upright part, and is written back as the current version.
#[test]
fn a_version_1_file_opens_upright_and_is_saved_as_the_current_version() {
    let v1 = r#"{"format": "3dcraft", "format_version": 1, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [],
        "features": [{"id": 1, "name": "Block1", "suppressed": false, "kind":
            {"type": "block", "position": ["0", "0", "0"], "size": ["1", "2", "3"]}}],
        "rollback": null, "next_feature_id": 2}}"#;
    let doc = craft3d_format::from_manifest_json(v1.as_bytes()).unwrap();
    let FeatureKind::Block(block) = &doc.features[0].kind else {
        panic!()
    };
    assert_eq!(block.orientation, craft3d_doc::Orientation::default());
    assert!(
        to_manifest_json(&doc)
            .unwrap()
            .contains(&format!("\"format_version\": {FORMAT_VERSION}"))
    );
}

/// A sketch with one entity of every kind and one constraint of every
/// type.
fn sketch() -> Sketch {
    let mut s = Sketch::new(SketchPlane::World {
        plane: WorldPlane::Zx,
        offset: Expr::new("half + 1 mm"),
    });
    let p: Vec<EntityId> = [
        (0.0, 0.0),
        (40.0, 0.0),
        (40.0, 12.5),
        (0.0, 12.5),
        (20.0, 6.0),
    ]
    .into_iter()
    .map(|(x, y)| s.add_point(x, y).unwrap())
    .collect();
    let bottom = s.add_line(p[0], p[1], false).unwrap();
    let right = s.add_line(p[1], p[2], false).unwrap();
    let top = s.add_line(p[2], p[3], true).unwrap();
    let hole = s.add_circle(p[4], 2.5, false).unwrap();
    let arc = s.add_arc(p[4], p[1], p[3], false).unwrap();
    let direct = DistanceDirection::Direct;
    let parts: [(&str, &[EntityId], Option<&str>, DistanceDirection); 17] = [
        ("coincident", &[p[0], p[4]], None, direct),
        ("pointOn", &[p[4], top], None, direct),
        ("horizontal", &[bottom], None, direct),
        ("vertical", &[p[1], p[2]], None, direct),
        ("parallel", &[bottom, top], None, direct),
        ("perpendicular", &[bottom, right], None, direct),
        ("tangent", &[top, hole], None, direct),
        ("equal", &[hole, arc], None, direct),
        ("concentric", &[hole, arc], None, direct),
        ("symmetric", &[p[0], p[3], top], None, direct),
        ("midpoint", &[p[4], bottom], None, direct),
        ("fixed", &[p[0]], None, direct),
        ("distance", &[bottom], Some("width"), direct),
        (
            "distance",
            &[p[0], p[2]],
            Some("half / 2"),
            DistanceDirection::Vertical,
        ),
        ("angle", &[bottom, right], Some("90 deg"), direct),
        ("radius", &[arc], Some("width / 16"), direct),
        ("diameter", &[hole], Some("5 mm"), direct),
    ];
    for (type_name, entities, value, direction) in parts {
        let kind = ConstraintKind::from_parts(type_name, entities, value.map(Expr::new), direction)
            .unwrap();
        s.add_constraint(kind).unwrap();
    }
    // Gaps in both id sequences, as after deleting something.
    let spare = s.add_point(1.0, 1.0).unwrap();
    s.add_constraint(ConstraintKind::Fixed { point: spare })
        .unwrap();
    s.remove_entity(spare).unwrap();
    s.add_point(-1.5e-7, 1e6).unwrap();
    s
}

/// Version 3: sketches, as they are written.
#[test]
fn a_sketch_is_written_like_this_and_round_trips() {
    let mut doc = small();
    doc.set_variable("half", "width / 2").unwrap();
    doc.add_feature(Some("Profile"), FeatureKind::Sketch(sketch()))
        .unwrap();
    doc.validate().unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let kind = &value["document"]["features"][1]["kind"];
    assert_eq!(kind["type"], "sketch");
    assert_eq!(
        kind["plane"],
        serde_json::json!({"type": "world", "plane": "zx", "offset": "half + 1 mm"})
    );
    assert_eq!(
        kind["entities"],
        serde_json::json!([
            {"id": 1, "construction": false, "geometry": {"type": "point", "x": 0.0, "y": 0.0}},
            {"id": 2, "construction": false, "geometry": {"type": "point", "x": 40.0, "y": 0.0}},
            {"id": 3, "construction": false, "geometry": {"type": "point", "x": 40.0, "y": 12.5}},
            {"id": 4, "construction": false, "geometry": {"type": "point", "x": 0.0, "y": 12.5}},
            {"id": 5, "construction": false, "geometry": {"type": "point", "x": 20.0, "y": 6.0}},
            {"id": 6, "construction": false, "geometry": {"type": "line", "start": 1, "end": 2}},
            {"id": 7, "construction": false, "geometry": {"type": "line", "start": 2, "end": 3}},
            {"id": 8, "construction": true, "geometry": {"type": "line", "start": 3, "end": 4}},
            {"id": 9, "construction": false, "geometry": {"type": "circle", "center": 5, "radius": 2.5}},
            {"id": 10, "construction": false, "geometry": {"type": "arc", "center": 5, "start": 2, "end": 4}},
            {"id": 12, "construction": false, "geometry": {"type": "point", "x": -1.5e-7, "y": 1e6}},
        ])
    );
    assert_eq!(
        kind["constraints"],
        serde_json::json!([
            {"id": 1, "kind": {"type": "coincident", "a": 1, "b": 5}},
            {"id": 2, "kind": {"type": "pointOn", "point": 5, "curve": 8}},
            {"id": 3, "kind": {"type": "horizontal", "a": 6, "b": null}},
            {"id": 4, "kind": {"type": "vertical", "a": 2, "b": 3}},
            {"id": 5, "kind": {"type": "parallel", "a": 6, "b": 8}},
            {"id": 6, "kind": {"type": "perpendicular", "a": 6, "b": 7}},
            {"id": 7, "kind": {"type": "tangent", "a": 8, "b": 9}},
            {"id": 8, "kind": {"type": "equal", "a": 9, "b": 10}},
            {"id": 9, "kind": {"type": "concentric", "a": 9, "b": 10}},
            {"id": 10, "kind": {"type": "symmetric", "a": 1, "b": 4, "about": 8}},
            {"id": 11, "kind": {"type": "midpoint", "point": 5, "line": 6}},
            {"id": 12, "kind": {"type": "fixed", "point": 1}},
            {"id": 13, "kind": {"type": "distance", "a": 6, "b": null, "direction": "direct", "value": "width"}},
            {"id": 14, "kind": {"type": "distance", "a": 1, "b": 3, "direction": "vertical", "value": "half / 2"}},
            {"id": 15, "kind": {"type": "angle", "a": 6, "b": 7, "value": "90 deg"}},
            {"id": 16, "kind": {"type": "radius", "curve": 10, "value": "width / 16"}},
            {"id": 17, "kind": {"type": "diameter", "curve": 9, "value": "5 mm"}},
        ])
    );
    assert_eq!(kind["next_entity_id"], 13);
    assert_eq!(kind["next_constraint_id"], 19);
    // Coordinates survive bit for bit: they are the stored solution.
    let back = from_bytes(&to_bytes(&doc).unwrap()).unwrap();
    assert_eq!(back, doc);
    assert_eq!(to_manifest_json(&back).unwrap(), json);
}

#[test]
fn a_sketch_that_breaks_the_rules_is_not_loaded() {
    let mut doc = small();
    doc.add_feature(None, FeatureKind::Sketch(sketch()))
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&to_manifest_json(&doc).unwrap()).unwrap();
    type Damage = fn(&mut serde_json::Value);
    let broken: [(&str, Damage); 9] = [
        ("a line on a point that is not there", |k| {
            k["entities"][5]["geometry"]["start"] = 99.into()
        }),
        ("a constraint on the wrong kind of entity", |k| {
            k["constraints"][11]["kind"]["point"] = 6.into()
        }),
        ("an entity id that is not below the next", |k| {
            k["next_entity_id"] = 3.into()
        }),
        ("a negative radius", |k| {
            k["entities"][8]["geometry"]["radius"] = (-2.5).into()
        }),
        ("an unknown plane", |k| k["plane"]["plane"] = "xz".into()),
        ("an unknown constraint type", |k| {
            k["constraints"][1]["kind"]["type"] = "pointUnder".into()
        }),
        ("an unknown direction", |k| {
            k["constraints"][13]["kind"]["direction"] = "diagonal".into()
        }),
        ("a coordinate that is text", |k| {
            k["entities"][1]["geometry"]["x"] = "40".into()
        }),
        ("an unknown field", |k| {
            k["entities"][0]["colour"] = "red".into()
        }),
    ];
    for (what, damage) in broken {
        let mut value = value.clone();
        damage(&mut value["document"]["features"][1]["kind"]);
        let json = serde_json::to_string(&value).unwrap();
        assert!(from_manifest_json(json.as_bytes()).is_err(), "{what}");
    }
    // Undamaged, the same route loads.
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
}

/// A version-2 file, as written before sketches existed. Never edit this.
#[test]
fn a_version_2_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v2 = r#"{"format": "3dcraft", "format_version": 2, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "r", "expr": "4 mm"}],
        "features": [
          {"id": 1, "name": "Ring", "suppressed": false, "kind":
            {"type": "torus", "position": ["0", "0", "0"], "major": "20 mm", "minor": "r",
             "orientation": {"axis": ["1", "0", "0"], "angle": "15 deg"}}},
          {"id": 2, "name": "Tip", "kind":
            {"type": "cone", "position": ["0", "0", "0"], "radius": "r", "height": "9 mm"}}],
        "rollback": null, "next_feature_id": 3}}"#;
    let doc = from_manifest_json(v2.as_bytes()).unwrap();
    let FeatureKind::Torus(ring) = &doc.features[0].kind else {
        panic!()
    };
    assert_eq!(ring.minor.source(), "r");
    assert_eq!(ring.orientation.angle.source(), "15 deg");
    assert_eq!(doc.features[1].kind.type_name(), "cone");
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
}

/// Version 4: extrusions, as they are written.
#[test]
fn an_extrusion_is_written_like_this_and_round_trips() {
    let doc = full(Units::default());
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    assert_eq!(
        value["document"]["features"][4],
        serde_json::json!({
            "id": 6, "name": "Body", "suppressed": false,
            "kind": {"type": "extrude", "sketch": 5, "distance": "-half", "symmetric": true}
        })
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    // `symmetric` is optional; `sketch` and `distance` are not.
    let kind = |text: &str| {
        let mut value = value.clone();
        value["document"]["features"][4]["kind"] = serde_json::from_str(text).unwrap();
        from_manifest_json(serde_json::to_string(&value).unwrap().as_bytes())
    };
    let one_sided = kind(r#"{"type": "extrude", "sketch": 5, "distance": "1"}"#).unwrap();
    assert!(matches!(
        &one_sided.features[4].kind,
        FeatureKind::Extrude(Extrude {
            symmetric: false,
            ..
        })
    ));
    assert!(kind(r#"{"type": "extrude", "distance": "1"}"#).is_err());
    assert!(kind(r#"{"type": "extrude", "sketch": 5}"#).is_err());
    assert!(kind(r#"{"type": "extrude", "sketch": "Profile", "distance": "1"}"#).is_err());
    assert!(
        kind(r#"{"type": "extrude", "sketch": 5, "distance": "1", "taper": "2 deg"}"#).is_err()
    );
}

/// The ends of an extrusion (2026-10-13): an extrusion that ends at its
/// distance is written as it always was (the test above pins the bytes),
/// so the fields are new optional ones and the version stays; the other
/// ends are written like this.
#[test]
fn the_ends_of_an_extrusion_are_written_like_this_and_round_trip() {
    use craft3d_doc::{ExtrudeEnd, NameRole, PersistentName};
    let mut doc = full(Units::default());
    let face = PersistentName::copy(
        FeatureId(4),
        1,
        PersistentName::new(FeatureId(2), NameRole::Face, vec![1]),
    );
    let ends = [
        (
            ExtrudeEnd::ThroughAll,
            false,
            serde_json::json!({"type": "extrude", "sketch": 5, "distance": "-half",
                "symmetric": true, "end": {"type": "throughAll"}}),
        ),
        (
            ExtrudeEnd::UpToFace { face },
            true,
            serde_json::json!({"type": "extrude", "sketch": 5, "distance": "-half",
                "symmetric": true,
                "end": {"type": "upToFace", "face": {"feature": 4, "role": "copy", "sources": [1],
                    "of": {"feature": 2, "role": "face", "sources": [1]}}},
                "reversed": true}),
        ),
    ];
    for (end, reversed, written) in ends {
        let FeatureKind::Extrude(extrusion) = &mut doc.features[4].kind else {
            panic!("the body is an extrusion")
        };
        (extrusion.end, extrusion.reversed) = (end, reversed);
        let json = to_manifest_json(&doc).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["format_version"], FORMAT_VERSION);
        assert_eq!(value["document"]["features"][4]["kind"], written);
        assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    }
    // What is not an end, or not a name, is refused.
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let kind = |end: &str| {
        let mut value = value.clone();
        value["document"]["features"][4]["kind"]["end"] = serde_json::from_str(end).unwrap();
        from_manifest_json(serde_json::to_string(&value).unwrap().as_bytes())
    };
    assert!(kind(r#"{"type": "distance"}"#).is_ok());
    assert!(kind(r#"{"type": "upToNext"}"#).is_err());
    assert!(kind(r#"{"type": "upToFace"}"#).is_err());
    assert!(kind(r#"{"type": "throughAll", "margin": 1}"#).is_err());
    assert!(kind(r#""throughAll""#).is_err());
    // A well-formed name that is not a face is a file that loads; the
    // feature fails at regeneration, with the reason.
    assert!(
        kind(r#"{"type": "upToFace", "face": {"feature": 2, "role": "edge", "sources": [1]}}"#)
            .is_ok_and(|doc| doc.validate().is_ok())
    );
}

/// A version-3 file, as written before extrusions existed. Never edit this.
#[test]
fn a_version_3_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v3 = r#"{"format": "3dcraft", "format_version": 3, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "w", "expr": "4 mm"}],
        "features": [
          {"id": 1, "name": "Profile", "suppressed": false, "kind": {"type": "sketch",
            "plane": {"type": "world", "plane": "yz", "offset": "w"},
            "entities": [
              {"id": 1, "construction": false, "geometry": {"type": "point", "x": 0.0, "y": 0.0}},
              {"id": 2, "geometry": {"type": "point", "x": 4.0, "y": 0.0}},
              {"id": 3, "construction": true, "geometry": {"type": "line", "start": 1, "end": 2}},
              {"id": 4, "construction": false, "geometry": {"type": "circle", "center": 1, "radius": 1.5}}],
            "constraints": [
              {"id": 1, "kind": {"type": "fixed", "point": 1}},
              {"id": 2, "kind": {"type": "distance", "a": 3, "b": null, "value": "w"}}],
            "next_entity_id": 5, "next_constraint_id": 3}}],
        "rollback": null, "next_feature_id": 2}}"#;
    let doc = from_manifest_json(v3.as_bytes()).unwrap();
    let FeatureKind::Sketch(sketch) = &doc.features[0].kind else {
        panic!()
    };
    assert_eq!(sketch.entities.len(), 4);
    assert!(sketch.entities[2].construction && !sketch.entities[1].construction);
    assert_eq!(sketch.point(EntityId(2)).unwrap(), (4.0, 0.0));
    assert_eq!(
        sketch.constraints[1].kind,
        ConstraintKind::Distance {
            a: EntityId(3),
            b: None,
            direction: DistanceDirection::Direct,
            value: Expr::new("w"),
        }
    );
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
}

/// Version 4 also: a sketch on a face of a body, by the face's name.
#[test]
fn a_sketch_on_a_face_is_written_like_this_and_round_trips() {
    use craft3d_doc::{NameRole, PersistentName};
    let mut doc = full(Units::default());
    let face = PersistentName::new(FeatureId(6), NameRole::EndCap, vec![6, 7, 8]);
    doc.add_feature(
        Some("Seat"),
        FeatureKind::Sketch(Sketch::new(SketchPlane::Face {
            face: face.clone(),
            offset: Expr::new("half / 4"),
        })),
    )
    .unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        value["document"]["features"][5]["kind"]["plane"],
        serde_json::json!({
            "type": "face",
            "face": {"feature": 6, "role": "endCap", "sources": [6, 7, 8]},
            "offset": "half / 4"
        })
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    // Every role is written by its name and read back.
    for role in NameRole::ALL {
        let mut doc = doc.clone();
        let FeatureKind::Sketch(sketch) = &mut doc.features[5].kind else {
            panic!()
        };
        sketch.plane = SketchPlane::Face {
            face: if role == NameRole::Piece {
                // The fourth face that the name after it would name.
                PersistentName::piece(3, face.clone())
            } else if role.is_derived() {
                PersistentName::copy(FeatureId(6), 3, face.clone())
            } else if role.is_numbered() {
                PersistentName::new(FeatureId(6), role, vec![3])
            } else if role.is_by_faces() {
                // An edge or a vertex by its faces: the faces it lies on,
                // those near it, and a number.
                PersistentName::new(FeatureId(6), role, vec![2]).with_faces(
                    vec![face.clone(), face.clone()],
                    vec![PersistentName::copy(FeatureId(6), 3, face.clone())],
                )
            } else {
                PersistentName {
                    role,
                    ..face.clone()
                }
            },
            offset: Expr::new("0"),
        };
        let json = to_manifest_json(&doc).unwrap();
        assert!(
            json.contains(&format!("\"role\": \"{}\"", role.name())),
            "{json}"
        );
        assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    }
    // A name that is damaged in the file is an error, not another face.
    let damaged = |from: &str, to: &str| {
        assert!(json.contains(from), "{from}");
        from_manifest_json(json.replacen(from, to, 1).as_bytes())
    };
    assert!(damaged("\"role\": \"endCap\"", "\"role\": \"roof\"").is_err());
    assert!(damaged("\"feature\": 6,", "\"feature\": \"F6\",").is_err());
    assert!(damaged("\"feature\": 6,", "\"feature\": 6, \"index\": 0,").is_err());
}

/// Version 5: the features that make a body out of another body, as they
/// are written, and a name that is a copy of a name.
#[test]
fn moved_mirrored_and_repeated_bodies_are_written_like_this_and_round_trip() {
    use craft3d_doc::{Mirror, NameRole, Pattern, PatternLayout, PersistentName, Transform};
    let mut doc = full(Units::default());
    let body = doc.feature_by_name("Body").unwrap().id;
    let moved = doc
        .add_feature(
            Some("Moved"),
            FeatureKind::Transform(Transform {
                source: body,
                translate: exprs(["width", "0", "-half"]),
                pivot: exprs(["1 mm", "2 mm", "3 mm"]),
                axis: exprs(["0", "1", "0"]),
                angle: Expr::new("30 deg"),
                keep: true,
            }),
        )
        .unwrap();
    let mirrored = doc
        .add_feature(
            Some("Mirrored"),
            FeatureKind::Mirror(Mirror {
                source: moved,
                point: exprs(["half", "0", "0"]),
                normal: exprs(["1", "0", "0"]),
                plane: None,
                keep: false,
            }),
        )
        .unwrap();
    doc.add_feature(
        Some("Row"),
        FeatureKind::Pattern(Pattern {
            source: mirrored,
            count: Expr::new("4"),
            layout: PatternLayout::Linear {
                step: exprs(["2 * width", "0", "0"]),
            },
        }),
    )
    .unwrap();
    let ring = doc
        .add_feature(
            Some("Ring"),
            FeatureKind::Pattern(Pattern {
                source: body,
                count: Expr::new("teeth"),
                layout: PatternLayout::Circular {
                    about: None,
                    point: exprs(["0", "0", "0"]),
                    axis: exprs(["0", "0", "1"]),
                    angle: Expr::new("360 deg / teeth"),
                },
            }),
        )
        .unwrap();
    let face = PersistentName::copy(
        ring,
        3,
        PersistentName::new(body, NameRole::EndCap, vec![6, 7, 8]),
    );
    doc.add_feature(
        Some("Seat"),
        FeatureKind::Sketch(Sketch::new(SketchPlane::Face {
            face,
            offset: Expr::new("0"),
        })),
    )
    .unwrap();
    doc.validate().unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let kinds: Vec<&serde_json::Value> = (5..10)
        .map(|i| &value["document"]["features"][i]["kind"])
        .collect();
    assert_eq!(
        kinds[0],
        &serde_json::json!({"type": "transform", "source": 6,
            "translate": ["width", "0", "-half"], "pivot": ["1 mm", "2 mm", "3 mm"],
            "axis": ["0", "1", "0"], "angle": "30 deg", "keep": true})
    );
    assert_eq!(
        kinds[1],
        &serde_json::json!({"type": "mirror", "source": 7,
            "point": ["half", "0", "0"], "normal": ["1", "0", "0"], "keep": false})
    );
    assert_eq!(
        kinds[2],
        &serde_json::json!({"type": "pattern", "source": 8, "count": "4",
            "layout": {"type": "linear", "step": ["2 * width", "0", "0"]}})
    );
    assert_eq!(
        kinds[3],
        &serde_json::json!({"type": "pattern", "source": 6, "count": "teeth",
            "layout": {"type": "circular", "point": ["0", "0", "0"], "axis": ["0", "0", "1"],
                       "angle": "360 deg / teeth"}})
    );
    assert_eq!(
        kinds[4]["plane"]["face"],
        serde_json::json!({"feature": 10, "role": "copy", "sources": [3],
            "of": {"feature": 6, "role": "endCap", "sources": [6, 7, 8]}})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);

    // Damaged: unknown fields, missing ones, and names that cannot be.
    let with = |index: usize, pointer: &str, replacement: serde_json::Value| {
        let mut value = value.clone();
        *value["document"]["features"][index]
            .pointer_mut(pointer)
            .unwrap() = replacement;
        from_manifest_json(serde_json::to_string(&value).unwrap().as_bytes())
    };
    assert!(with(5, "/kind", serde_json::json!({"type": "transform"})).is_err());
    assert!(with(5, "/kind/source", serde_json::json!("Body")).is_err());
    assert!(with(5, "/kind/axis", serde_json::json!(["0", "1"])).is_err());
    assert!(
        with(
            6,
            "/kind",
            serde_json::json!({"type": "mirror", "source": 7, "point": ["0", "0", "0"],
                "normal": ["1", "0", "0"], "scale": 2})
        )
        .is_err()
    );
    assert!(with(7, "/kind/layout/type", serde_json::json!("spiral")).is_err());
    assert!(
        with(
            8,
            "/kind/layout",
            serde_json::json!({"type": "circular", "point": ["0", "0", "0"],
                "axis": ["0", "0", "1"], "angle": "1 deg", "radius": "1"})
        )
        .is_err()
    );
    // A copy without what it is a copy of, a copy of nothing, a side that
    // claims to be a copy, and a chain deeper than a name may be.
    assert!(with(9, "/kind/plane/face/of", serde_json::Value::Null).is_err());
    assert!(with(9, "/kind/plane/face/sources", serde_json::json!([])).is_err());
    assert!(with(9, "/kind/plane/face/role", serde_json::json!("side")).is_err());
    let mut deep = serde_json::json!({"feature": 6, "role": "endCap", "sources": [1]});
    for _ in 0..40 {
        deep = serde_json::json!({"feature": 6, "role": "piece", "sources": [1], "of": deep});
    }
    assert!(with(9, "/kind/plane/face", deep).is_err());
    // A copy of a copy in the long form, as a version-14 file has it, is
    // read in the short form, however long the row of copies: the run of
    // instance 0 is one link, the tool one more.
    let mut long = serde_json::json!({"feature": 6, "role": "endCap", "sources": [1]});
    for feature in 0..40 {
        let instance = u64::from(feature == 0);
        long = serde_json::json!({"feature": 50 - feature, "role": "copy",
            "sources": [instance], "of": long});
    }
    let read = with(9, "/kind/plane/face", long).unwrap();
    let FeatureKind::Sketch(sketch) = &read.features[9].kind else {
        panic!()
    };
    let SketchPlane::Face { face, .. } = &sketch.plane else {
        panic!()
    };
    let short = "F11:copy(0)/F50:copy(1)/F6:endCap(e1)";
    assert_eq!(face.to_string(), short);
}

/// A version-4 file, as written before bodies could be moved. Never edit
/// this.
#[test]
fn a_version_4_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v4 = r#"{"format": "3dcraft", "format_version": 4, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "d", "expr": "8 mm"}],
        "features": [
          {"id": 1, "name": "Profile", "suppressed": false, "kind": {"type": "sketch",
            "plane": {"type": "world", "plane": "xy", "offset": "0"},
            "entities": [
              {"id": 1, "construction": false, "geometry": {"type": "point", "x": 0.0, "y": 0.0}},
              {"id": 2, "construction": false, "geometry": {"type": "circle", "center": 1, "radius": 1.5}}],
            "constraints": [],
            "next_entity_id": 3, "next_constraint_id": 1}},
          {"id": 2, "name": "Body", "suppressed": false,
            "kind": {"type": "extrude", "sketch": 1, "distance": "d", "symmetric": false}},
          {"id": 3, "name": "Seat", "suppressed": false, "kind": {"type": "sketch",
            "plane": {"type": "face", "face": {"feature": 2, "role": "endCap", "sources": [2]},
                      "offset": "1 mm"},
            "entities": [], "constraints": [],
            "next_entity_id": 1, "next_constraint_id": 1}}],
        "rollback": null, "next_feature_id": 4}}"#;
    let doc = from_manifest_json(v4.as_bytes()).unwrap();
    let FeatureKind::Sketch(seat) = &doc.features[2].kind else {
        panic!()
    };
    let SketchPlane::Face { face, offset } = &seat.plane else {
        panic!()
    };
    assert_eq!(face.to_string(), "F2:endCap(e2)");
    assert_eq!(offset, &Expr::new("1 mm"));
    assert!(matches!(&doc.features[1].kind, FeatureKind::Extrude(e) if e.sketch == FeatureId(1)));
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    // A name that is not a copy is written as it was: no `of`.
    assert!(!saved.contains("\"of\""), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
}

/// Version 6: a spline in a sketch, as it is written.
/// Patterns inside a sketch (format 18): a row, a grid, a chained run and
/// a ring about fixed coordinates, and image points of each kind of index.
#[test]
fn a_sketch_pattern_is_written_like_this_and_round_trips() {
    use craft3d_doc::{CenterRef, InstanceIndex, SecondDirection, SketchPatternLayout};
    let mut doc = small();
    let mut sketch = Sketch::new(SketchPlane::default());
    let p: Vec<EntityId> = [(0.0, 0.0), (1.0, 0.0), (2.0, 1.0), (5.0, 5.0)]
        .into_iter()
        .map(|(x, y)| sketch.add_point(x, y).unwrap())
        .collect();
    let run = sketch.add_line(p[0], p[1], false).unwrap();
    let disc = sketch.add_circle(p[2], 0.25, false).unwrap();
    let ring = sketch.add_circle(p[3], 0.5, true).unwrap();
    let pair = |a: &str, b: &str| [Expr::new(a), Expr::new(b)];
    sketch
        .add_pattern(
            vec![run],
            Expr::new("n"),
            SketchPatternLayout::Chain {
                from: p[0],
                to: p[1],
            },
        )
        .unwrap();
    let grid = sketch
        .add_pattern(
            vec![disc],
            Expr::new("3"),
            SketchPatternLayout::Linear {
                step: pair("2 mm", "0"),
                second: Some(SecondDirection {
                    step: pair("0", "pitch"),
                    count: Expr::new("rows"),
                }),
            },
        )
        .unwrap();
    sketch
        .add_pattern(
            vec![ring],
            Expr::new("6"),
            SketchPatternLayout::Circular {
                center: CenterRef::At {
                    at: pair("0", "1 mm"),
                },
                angle: Expr::new("60 deg"),
                chain: None,
            },
        )
        .unwrap();
    let last = sketch.image_point(p[1], InstanceIndex::Last).unwrap();
    sketch
        .image_point(p[2], InstanceIndex::Grid([2, 1]))
        .unwrap();
    sketch.image_point(p[3], InstanceIndex::At(4)).unwrap();
    sketch.add_line(last, p[3], false).unwrap();
    doc.add_feature(Some("Rows"), FeatureKind::Sketch(sketch))
        .unwrap();
    doc.validate().unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], 21);
    let kind = &value["document"]["features"][1]["kind"];
    assert_eq!(kind["next_pattern_id"], 4);
    assert_eq!(
        kind["patterns"][0],
        serde_json::json!({"id": 1, "seed": [5], "count": "n",
            "layout": {"type": "chain", "from": 1, "to": 2}})
    );
    assert_eq!(
        kind["patterns"][1],
        serde_json::json!({"id": grid.0, "seed": [6], "count": "3",
            "layout": {"type": "linear", "step": ["2 mm", "0"],
                "second": {"step": ["0", "pitch"], "count": "rows"}}})
    );
    assert_eq!(
        kind["patterns"][2]["layout"],
        serde_json::json!({"type": "circular", "center": {"type": "at", "at": ["0", "1 mm"]},
            "angle": "60 deg"})
    );
    let entities = &kind["entities"];
    assert_eq!(
        entities[7]["geometry"],
        serde_json::json!({"type": "image", "of": 2, "pattern": 1, "index": "last"})
    );
    assert_eq!(
        entities[8]["geometry"],
        serde_json::json!({"type": "image", "of": 3, "pattern": 2, "index": {"grid": [2, 1]}})
    );
    assert_eq!(
        entities[9]["geometry"],
        serde_json::json!({"type": "image", "of": 4, "pattern": 3, "index": {"at": 4}})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);

    // What a file may not hold is an error, not a panic or a document.
    let with = |from: &str, to: &str| {
        assert!(json.contains(from), "`{from}` is not in the manifest");
        from_manifest_json(json.replacen(from, to, 1).as_bytes())
    };
    for (from, to) in [
        ("\"pattern\": 1", "\"pattern\": 9"),
        ("\"of\": 2", "\"of\": 5"),
        ("\"index\": \"last\"", "\"index\": \"first\""),
        ("\"next_pattern_id\": 4", "\"next_pattern_id\": 2"),
        ("\"type\": \"chain\"", "\"type\": \"spiral\""),
        ("\"from\": 1", "\"from\": 4"),
    ] {
        assert!(with(from, to).is_err(), "{from} -> {to}");
    }
    // A version-17 file has no patterns; it reads as it is.
    let older = edited(&version_field(), "\"format_version\": 17");
    assert_eq!(from_manifest_json(older.as_bytes()).unwrap(), small());
}

/// Places of image points in a file that are not in the form a document
/// keeps them (of an entity that is no image point, of no entity, twice
/// for one image, out of order) are dropped on reading with a
/// warning, not a reason to refuse the file: a place is where the next
/// solve starts carrying an image from, not part of the design. A million
/// of them read in about a second, not quadratic: four times as many
/// cost less than eight times the work (`craft3d_testkit::cost`, which
/// a busy machine does not stretch), checked first so that a quadratic
/// reader fails there and not after hours at a million.
#[test]
fn stale_image_places_are_dropped_on_reading() {
    use craft3d_doc::{ImagePlace, InstanceIndex, PlaceMap, SketchPatternLayout};
    let mut doc = small();
    let mut sketch = Sketch::new(SketchPlane::default());
    let a = sketch.add_point(0.0, 0.0).unwrap();
    let b = sketch.add_point(1.0, 0.0).unwrap();
    let run = sketch.add_line(a, b, false).unwrap();
    sketch
        .add_pattern(
            vec![run],
            Expr::new("3"),
            SketchPatternLayout::Chain { from: a, to: b },
        )
        .unwrap();
    let last = sketch.image_point(b, InstanceIndex::Last).unwrap();
    let second = sketch.image_point(b, InstanceIndex::At(1)).unwrap();
    let good = |entity, x: f64| ImagePlace {
        entity,
        at: [x, 0.0],
        map: Some(PlaceMap::Chain { k: x }),
    };
    // `last` was made first: the lower id.
    sketch.image_places = vec![good(last, 3.0), good(second, 2.0)];
    doc.add_feature(Some("Run"), FeatureKind::Sketch(sketch.clone()))
        .unwrap();
    doc.validate().unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let hostile = vec![
        // Out of order, and the second one again.
        serde_json::json!({"entity": second.0, "at": [2.0, 0.0], "map": {"type": "chain", "k": 2.0}}),
        serde_json::json!({"entity": last.0, "at": [3.0, 0.0], "map": {"type": "chain", "k": 3.0}}),
        serde_json::json!({"entity": second.0, "at": [9.0, 9.0]}),
        // A point, a line, no entity, one beyond every id.
        serde_json::json!({"entity": a.0, "at": [0.0, 0.0]}),
        serde_json::json!({"entity": run.0, "at": [0.0, 0.0]}),
        serde_json::json!({"entity": 0, "at": [0.0, 0.0]}),
        serde_json::json!({"entity": u64::MAX, "at": [0.0, 0.0]}),
    ];
    let with_stale = |count: u64| {
        let mut value = value.clone();
        let mut hostile = hostile.clone();
        for i in 0..count {
            hostile.push(serde_json::json!({"entity": 1000 + i % 977, "at": [0.0, 0.0]}));
        }
        value["document"]["features"][1]["kind"]["image_places"] =
            serde_json::Value::Array(hostile);
        value.to_string()
    };
    let texts = [with_stale(12_500), with_stale(50_000)];
    craft3d_testkit::cost::assert_not_quadratic(
        "reading stale image places",
        (12_500, 50_000),
        |n| {
            let text = &texts[usize::from(n == 50_000)];
            craft3d_format::from_manifest_json_noted(text.as_bytes()).unwrap();
        },
    );
    let text = with_stale(1_000_000);
    let (read, notes) = craft3d_format::from_manifest_json_noted(text.as_bytes()).unwrap();
    let Some(FeatureKind::Sketch(read)) = read.features.get(1).map(|f| &f.kind) else {
        panic!("no sketch");
    };
    assert_eq!(read.image_places, sketch.image_places);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("1000005 image places dropped"),
        "{notes:?}"
    );
    // A document keeps them in that form: validate holds it to it.
    let mut bad = sketch.clone();
    bad.image_places.reverse();
    assert!(bad.validate().is_err());
    let mut bad = sketch.clone();
    bad.image_places.push(good(a, 0.0));
    assert!(bad.validate().is_err());
    let mut bad = sketch.clone();
    bad.image_places[0].at[0] = f64::NAN;
    assert!(bad.validate().is_err());
    let mut bad = sketch;
    bad.image_places[1].map = Some(PlaceMap::Turn {
        angle: f64::INFINITY,
        at: [0.0, 0.0],
    });
    assert!(bad.validate().is_err());
}

/// Where image points lay in the stored solution (version 20): a list of
/// entity and coordinates on the sketch, written only when there is one.
#[test]
fn image_places_are_written_like_this_and_round_trip() {
    use craft3d_doc::{ImagePlace, InstanceIndex, PlaceMap, SketchPatternLayout};
    let mut doc = small();
    let mut sketch = Sketch::new(SketchPlane::default());
    let a = sketch.add_point(0.0, 0.0).unwrap();
    let b = sketch.add_point(1.0, 0.0).unwrap();
    let run = sketch.add_line(a, b, false).unwrap();
    sketch
        .add_pattern(
            vec![run],
            Expr::new("3"),
            SketchPatternLayout::Chain { from: a, to: b },
        )
        .unwrap();
    let last = sketch.image_point(b, InstanceIndex::Last).unwrap();
    sketch.image_places = vec![ImagePlace {
        entity: last,
        at: [3.0, -0.25],
        map: Some(PlaceMap::Chain { k: 2.0 }),
    }];
    doc.add_feature(Some("Run"), FeatureKind::Sketch(sketch))
        .unwrap();
    doc.validate().unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], 21);
    assert_eq!(
        value["document"]["features"][1]["kind"]["image_places"],
        serde_json::json!([{"entity": last.0, "at": [3.0, -0.25], "map": {"type": "chain", "k": 2.0}}])
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // Not written when there are none.
    assert!(!to_manifest_json(&small()).unwrap().contains("image_places"));
    // A field it does not know is an error, not a place.
    let odd = json.replacen("\"image_places\"", "\"image_spots\"", 1);
    assert!(from_manifest_json(odd.as_bytes()).is_err());
    // A version-19 file has none; it reads as it is.
    let older = edited(&version_field(), "\"format_version\": 19");
    assert_eq!(from_manifest_json(older.as_bytes()).unwrap(), small());
}

#[test]
fn a_spline_is_written_like_this_and_round_trips() {
    let mut doc = small();
    let mut sketch = Sketch::new(SketchPlane::default());
    let p: Vec<EntityId> = [(0.0, 0.0), (1.0, 2.5), (3.0, -1e-7), (4e6, 0.0)]
        .into_iter()
        .map(|(x, y)| sketch.add_point(x, y).unwrap())
        .collect();
    let open = sketch.add_spline(3, p.clone(), false).unwrap();
    let closed = sketch
        .add_spline(2, vec![p[0], p[2], p[1], p[0]], true)
        .unwrap();
    sketch.add_line(p[3], p[0], false).unwrap();
    sketch
        .add_constraint(ConstraintKind::Tangent { a: open, b: closed })
        .unwrap();
    doc.add_feature(Some("Hull"), FeatureKind::Sketch(sketch))
        .unwrap();
    doc.validate().unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let entities = &value["document"]["features"][1]["kind"]["entities"];
    assert_eq!(
        entities[4],
        serde_json::json!({"id": 5, "construction": false,
            "geometry": {"type": "spline", "degree": 3, "control": [1, 2, 3, 4]}})
    );
    assert_eq!(
        entities[5],
        serde_json::json!({"id": 6, "construction": true,
            "geometry": {"type": "spline", "degree": 2, "control": [1, 3, 2, 1]}})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);

    // A spline a file may not hold: each is an error, not a panic and not
    // a document.
    let with = |from: &str, to: &str| {
        assert!(json.contains(from), "`{from}` is not in the manifest");
        from_manifest_json(json.replacen(from, to, 1).as_bytes())
    };
    let degree = "\"degree\": 3";
    for bad in ["0", "8", "-1", "4294967296", "2.5", "\"3\"", "null"] {
        assert!(
            with(degree, &format!("\"degree\": {bad}")).is_err(),
            "degree {bad}"
        );
    }
    let control = "\"control\": [\n                  1,\n                  2,\n                  3,\n                  4\n                ]";
    for bad in [
        "[1, 2, 3]",
        "[]",
        "[1, 2, 2, 4]",
        "[1, 2, 3, 99]",
        "[1, 2, 3, 5]",
        "7",
    ] {
        assert!(
            with(control, &format!("\"control\": {bad}")).is_err(),
            "control {bad}"
        );
    }
    assert!(with(degree, "\"degree\": 3, \"weights\": [1, 1, 1, 1]").is_err());
}

/// A version-5 file, as written before sketches had splines. Never edit
/// this.
#[test]
fn a_version_5_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v5 = r#"{"format": "3dcraft", "format_version": 5, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "d", "expr": "8 mm"}],
        "features": [
          {"id": 1, "name": "Profile", "suppressed": false, "kind": {"type": "sketch",
            "plane": {"type": "world", "plane": "xy", "offset": "0"},
            "entities": [
              {"id": 1, "construction": false, "geometry": {"type": "point", "x": 0.0, "y": 0.0}},
              {"id": 2, "construction": false, "geometry": {"type": "point", "x": 4.0, "y": 0.0}},
              {"id": 3, "construction": false, "geometry": {"type": "point", "x": 4.0, "y": 3.0}},
              {"id": 4, "construction": false, "geometry": {"type": "line", "start": 1, "end": 2}},
              {"id": 5, "construction": false, "geometry": {"type": "line", "start": 2, "end": 3}},
              {"id": 6, "construction": false, "geometry": {"type": "arc", "center": 2, "start": 3, "end": 1}},
              {"id": 7, "construction": true, "geometry": {"type": "circle", "center": 1, "radius": 1.5}}],
            "constraints": [{"id": 1, "kind": {"type": "tangent", "a": 4, "b": 7}}],
            "next_entity_id": 8, "next_constraint_id": 2}},
          {"id": 2, "name": "Body", "suppressed": false,
            "kind": {"type": "revolve", "sketch": 1, "axis": {"type": "x"}, "angle": "90 deg", "symmetric": false}},
          {"id": 3, "name": "Moved", "suppressed": false,
            "kind": {"type": "transform", "source": 2, "translate": ["d", "0", "0"],
                     "pivot": ["0", "0", "0"], "axis": ["0", "0", "1"], "angle": "0 deg",
                     "keep": false}}],
        "rollback": null, "next_feature_id": 4}}"#;
    let doc = match from_manifest_json(v5.as_bytes()) {
        Ok(doc) => doc,
        Err(error) => panic!("{error}"),
    };
    let FeatureKind::Sketch(profile) = &doc.features[0].kind else {
        panic!()
    };
    assert_eq!(profile.entities.len(), 7);
    assert_eq!(profile.entities[5].geometry.type_name(), "arc");
    assert_eq!(doc.features[1].kind.type_name(), "revolve");
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert!(!saved.contains("spline"), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
}

#[test]
fn datum_planes_are_written_like_this_and_round_trip() {
    use craft3d_doc::{
        AxisRef, DatumPlane, Mirror, NameRole, PersistentName, PlaneBy, PlaneRef, PointRef,
        WorldAxis,
    };
    let mut doc = small();
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let plane = |doc: &mut Document, by: PlaneBy| {
        doc.add_feature(None, FeatureKind::DatumPlane(DatumPlane { by }))
            .unwrap()
    };
    let shelf = plane(
        &mut doc,
        PlaneBy::Offset {
            from: PlaneRef::Face { face: face(1) },
            distance: Expr::new("width / 4"),
        },
    );
    plane(
        &mut doc,
        PlaneBy::Offset {
            from: PlaneRef::World {
                plane: WorldPlane::Yz,
            },
            distance: Expr::new("-3 mm"),
        },
    );
    plane(
        &mut doc,
        PlaneBy::Angle {
            from: PlaneRef::Datum { feature: shelf },
            about: AxisRef::Edge {
                between: [face(1), face(5)],
            },
            angle: Expr::new("30 deg"),
        },
    );
    plane(
        &mut doc,
        PlaneBy::Angle {
            from: PlaneRef::World {
                plane: WorldPlane::Xy,
            },
            about: AxisRef::World { axis: WorldAxis::X },
            angle: Expr::new("width * 1 deg / 1 mm"),
        },
    );
    plane(
        &mut doc,
        PlaneBy::ThreePoints {
            points: [
                PointRef::At {
                    position: exprs(["width", "0", "0"]),
                },
                PointRef::Vertex {
                    vertex: PersistentName::new(FeatureId(1), NameRole::Vertex, vec![6]),
                },
                PointRef::At {
                    position: exprs(["0", "0", "1 in"]),
                },
            ],
        },
    );
    doc.add_feature(
        Some("On the shelf"),
        FeatureKind::Sketch(Sketch::new(SketchPlane::Datum {
            plane: shelf,
            offset: Expr::new("1 mm"),
        })),
    )
    .unwrap();
    doc.add_feature(
        None,
        FeatureKind::Mirror(Mirror {
            source: FeatureId(1),
            point: exprs(["0", "0", "0"]),
            normal: exprs(["0", "0", "1"]),
            plane: Some(shelf),
            keep: true,
        }),
    )
    .unwrap();
    assert_eq!(doc.validate(), Ok(()));
    assert_eq!(doc.features[1].name, "Plane1");

    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Datums raised no version: nothing an earlier file holds changed
    // meaning. (7 is the chamfer's, 8 the loft's, 9 the fillet's, 10 the
    // sweep's.)
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let face_json = |n: u64| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    let kinds: Vec<&serde_json::Value> = value["document"]["features"]
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
        .map(|f| &f["kind"])
        .collect();
    let written = [
        serde_json::json!({"type": "datumPlane", "by": {"type": "offset",
            "from": {"type": "face", "face": face_json(1)}, "distance": "width / 4"}}),
        serde_json::json!({"type": "datumPlane", "by": {"type": "offset",
            "from": {"type": "world", "plane": "yz"}, "distance": "-3 mm"}}),
        serde_json::json!({"type": "datumPlane", "by": {"type": "angle",
            "from": {"type": "datum", "feature": 2},
            "about": {"type": "edge", "between": [face_json(1), face_json(5)]},
            "angle": "30 deg"}}),
        serde_json::json!({"type": "datumPlane", "by": {"type": "angle",
            "from": {"type": "world", "plane": "xy"},
            "about": {"type": "world", "axis": "x"},
            "angle": "width * 1 deg / 1 mm"}}),
        serde_json::json!({"type": "datumPlane", "by": {"type": "threePoints", "points": [
            {"type": "at", "position": ["width", "0", "0"]},
            {"type": "vertex", "vertex": {"feature": 1, "role": "vertex", "sources": [6]}},
            {"type": "at", "position": ["0", "0", "1 in"]}]}}),
        serde_json::json!({"type": "sketch",
            "plane": {"type": "datum", "plane": 2, "offset": "1 mm"},
            "entities": [], "constraints": [], "next_entity_id": 1, "next_constraint_id": 1}),
        serde_json::json!({"type": "mirror", "source": 1, "point": ["0", "0", "0"],
            "normal": ["0", "0", "1"], "plane": 2, "keep": true}),
    ];
    assert_eq!(kinds.len(), written.len());
    for (kind, expected) in kinds.iter().zip(&written) {
        assert_eq!(*kind, expected);
    }
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);

    // A mirror without a datum plane is written without the field: a file
    // that uses no datum is the file an earlier build wrote.
    let FeatureKind::Mirror(mirror) = &mut doc.features[7].kind else {
        panic!("the last feature is the mirror")
    };
    mirror.plane = None;
    let plain: serde_json::Value = serde_json::from_str(&to_manifest_json(&doc).unwrap()).unwrap();
    assert!(
        plain["document"]["features"][7]["kind"]
            .get("plane")
            .is_none()
    );

    // What is not a way to place a plane is refused; a name that is not
    // one makes the document invalid.
    let with = |by: &str| {
        let mut value = value.clone();
        value["document"]["features"][1]["kind"]["by"] = serde_json::from_str(by).unwrap();
        from_manifest_json(serde_json::to_string(&value).unwrap().as_bytes())
    };
    assert!(
        with(r#"{"type": "offset", "from": {"type": "world", "plane": "xy"}, "distance": "1"}"#)
            .is_ok()
    );
    assert!(with(r#"{"type": "offset", "from": {"type": "world", "plane": "xy"}}"#).is_err());
    assert!(
        with(r#"{"type": "offset", "from": {"type": "world", "plane": "xz"}, "distance": "1"}"#)
            .is_err()
    );
    assert!(with(r#"{"type": "offset", "from": {"type": "datum", "feature": 2}, "distance": "1", "angle": "1"}"#).is_err());
    assert!(with(r#"{"type": "tangent", "from": {"type": "world", "plane": "xy"}}"#).is_err());
    assert!(
        with(r#"{"type": "threePoints", "points": [{"type": "at", "position": ["0", "0", "0"]}]}"#)
            .is_err()
    );
    assert!(
        with(r#"{"type": "offset", "from": {"type": "face", "face": {"feature": 1, "role": "copy", "sources": [0]}}, "distance": "1"}"#)
            .is_err(),
        "a copy of nothing is not a name"
    );
    // A reference to a feature that is not there, or to itself, is a file
    // that loads; the datum fails at regeneration, with the reason.
    for feature in [2, 99] {
        let by = format!(
            r#"{{"type": "offset", "from": {{"type": "datum", "feature": {feature}}}, "distance": "1"}}"#
        );
        assert!(with(&by).is_ok_and(|doc| doc.validate().is_ok()));
    }
}

#[test]
fn datum_axes_are_written_like_this_and_round_trip() {
    use craft3d_doc::{
        AxisBy, AxisRef, DatumAxis, DatumPlane, NameRole, Pattern, PatternLayout, PersistentName,
        PlaneBy, PlaneRef, PointRef, Revolve, RevolveAxis,
    };
    let mut doc = small();
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let axis = |doc: &mut Document, by: AxisBy| {
        doc.add_feature(None, FeatureKind::DatumAxis(DatumAxis { by }))
            .unwrap()
    };
    let through = axis(
        &mut doc,
        AxisBy::TwoPoints {
            points: [
                PointRef::At {
                    position: exprs(["width", "0", "0"]),
                },
                PointRef::Vertex {
                    vertex: PersistentName::new(FeatureId(1), NameRole::Vertex, vec![6]),
                },
            ],
        },
    );
    axis(
        &mut doc,
        AxisBy::Edge {
            between: [face(1), face(5)],
        },
    );
    axis(&mut doc, AxisBy::Face { face: face(0) });
    axis(
        &mut doc,
        AxisBy::TwoPlanes {
            planes: [
                PlaneRef::World {
                    plane: WorldPlane::Zx,
                },
                PlaneRef::Face { face: face(1) },
            ],
        },
    );
    doc.add_feature(
        None,
        FeatureKind::DatumPlane(DatumPlane {
            by: PlaneBy::Angle {
                from: PlaneRef::World {
                    plane: WorldPlane::Xy,
                },
                about: AxisRef::Datum { feature: through },
                angle: Expr::new("12 deg"),
            },
        }),
    )
    .unwrap();
    let sketch = doc
        .add_feature(None, FeatureKind::Sketch(Sketch::default()))
        .unwrap();
    doc.add_feature(
        None,
        FeatureKind::Revolve(Revolve {
            sketch,
            axis: RevolveAxis::Datum { axis: through },
            angle: Expr::new("90 deg"),
            symmetric: false,
        }),
    )
    .unwrap();
    let circular = |about| {
        FeatureKind::Pattern(Pattern {
            source: FeatureId(1),
            count: Expr::new("3"),
            layout: PatternLayout::Circular {
                point: exprs(["0", "0", "0"]),
                axis: exprs(["0", "0", "1"]),
                angle: Expr::new("120 deg"),
                about,
            },
        })
    };
    doc.add_feature(None, circular(Some(through))).unwrap();
    doc.add_feature(None, circular(None)).unwrap();
    assert_eq!(doc.validate(), Ok(()));
    assert_eq!(doc.features[1].name, "Axis1");

    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let face_json = |n: u64| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    let kind = |index: usize| &value["document"]["features"][index]["kind"];
    let written = [
        serde_json::json!({"type": "datumAxis", "by": {"type": "twoPoints", "points": [
            {"type": "at", "position": ["width", "0", "0"]},
            {"type": "vertex", "vertex": {"feature": 1, "role": "vertex", "sources": [6]}}]}}),
        serde_json::json!({"type": "datumAxis", "by": {"type": "edge",
            "between": [face_json(1), face_json(5)]}}),
        serde_json::json!({"type": "datumAxis", "by": {"type": "face", "face": face_json(0)}}),
        serde_json::json!({"type": "datumAxis", "by": {"type": "twoPlanes", "planes": [
            {"type": "world", "plane": "zx"}, {"type": "face", "face": face_json(1)}]}}),
        serde_json::json!({"type": "datumPlane", "by": {"type": "angle",
            "from": {"type": "world", "plane": "xy"},
            "about": {"type": "datum", "feature": 2}, "angle": "12 deg"}}),
    ];
    for (index, expected) in written.iter().enumerate() {
        assert_eq!(kind(index + 1), expected);
    }
    assert_eq!(
        *kind(7),
        serde_json::json!({"type": "revolve", "sketch": 7,
            "axis": {"type": "datum", "axis": 2}, "angle": "90 deg", "symmetric": false})
    );
    assert_eq!(
        kind(8)["layout"],
        serde_json::json!({"type": "circular", "point": ["0", "0", "0"],
            "axis": ["0", "0", "1"], "angle": "120 deg", "about": 2})
    );
    // A pattern about a line of its own is written as before datums.
    assert_eq!(
        kind(9)["layout"],
        serde_json::json!({"type": "circular", "point": ["0", "0", "0"],
            "axis": ["0", "0", "1"], "angle": "120 deg"})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);

    let with = |by: &str| {
        let mut value = value.clone();
        value["document"]["features"][1]["kind"]["by"] = serde_json::from_str(by).unwrap();
        from_manifest_json(serde_json::to_string(&value).unwrap().as_bytes())
    };
    assert!(
        with(r#"{"type": "face", "face": {"feature": 1, "role": "face", "sources": [0]}}"#).is_ok()
    );
    assert!(with(r#"{"type": "face"}"#).is_err());
    assert!(
        with(r#"{"type": "twoPlanes", "planes": [{"type": "world", "plane": "xy"}]}"#).is_err()
    );
    assert!(with(r#"{"type": "edge", "between": []}"#).is_err());
    assert!(
        with(r#"{"type": "normal", "face": {"feature": 1, "role": "face", "sources": [0]}}"#)
            .is_err()
    );
    assert!(
        with(r#"{"type": "face", "face": {"feature": 1, "role": "face", "sources": [0, 1]}}"#)
            .is_err(),
        "a face of a primitive has one number"
    );
}

#[test]
fn datum_points_are_written_like_this_and_round_trip() {
    use craft3d_doc::{AxisBy, DatumAxis, DatumPoint, NameRole, PersistentName, PointRef};
    let mut doc = small();
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let point = |doc: &mut Document, at: PointRef| {
        doc.add_feature(None, FeatureKind::DatumPoint(DatumPoint { at }))
            .unwrap()
    };
    let free = point(
        &mut doc,
        PointRef::At {
            position: exprs(["width", "0", "1 in"]),
        },
    );
    point(
        &mut doc,
        PointRef::Vertex {
            vertex: PersistentName::new(FeatureId(1), NameRole::Vertex, vec![6]),
        },
    );
    point(
        &mut doc,
        PointRef::Center {
            between: [face(0), face(2)],
        },
    );
    doc.add_feature(
        None,
        FeatureKind::DatumAxis(DatumAxis {
            by: AxisBy::TwoPoints {
                points: [
                    PointRef::Datum { feature: free },
                    PointRef::Center {
                        between: [face(0), face(1)],
                    },
                ],
            },
        }),
    )
    .unwrap();
    assert_eq!(doc.validate(), Ok(()));
    assert_eq!(doc.features[1].name, "Point1");
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let face_json = |n: u64| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    let written = [
        serde_json::json!({"type": "datumPoint",
            "at": {"type": "at", "position": ["width", "0", "1 in"]}}),
        serde_json::json!({"type": "datumPoint", "at": {"type": "vertex",
            "vertex": {"feature": 1, "role": "vertex", "sources": [6]}}}),
        serde_json::json!({"type": "datumPoint", "at": {"type": "center",
            "between": [face_json(0), face_json(2)]}}),
        serde_json::json!({"type": "datumAxis", "by": {"type": "twoPoints", "points": [
            {"type": "datum", "feature": 2},
            {"type": "center", "between": [face_json(0), face_json(1)]}]}}),
    ];
    for (index, expected) in written.iter().enumerate() {
        assert_eq!(&value["document"]["features"][index + 1]["kind"], expected);
    }
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);

    let with = |at: &str| {
        let mut value = value.clone();
        value["document"]["features"][1]["kind"]["at"] = serde_json::from_str(at).unwrap();
        from_manifest_json(serde_json::to_string(&value).unwrap().as_bytes())
    };
    assert!(with(r#"{"type": "at", "position": ["0", "0", "0"]}"#).is_ok());
    assert!(with(r#"{"type": "at", "position": ["0", "0"]}"#).is_err());
    assert!(with(r#"{"type": "at", "position": [0, 0, 0]}"#).is_err());
    assert!(with(r#"{"type": "midpoint", "between": []}"#).is_err());
    assert!(with(r#"{"type": "datum", "feature": 2, "offset": "1"}"#).is_err());
    assert!(with(r#"["0", "0", "0"]"#).is_err());
}

/// A vertex and an edge of a boolean are named by their faces: the name
/// lists them, in the file as in the document.
#[test]
fn a_name_by_faces_is_written_like_this_and_round_trips() {
    use craft3d_doc::{DatumPoint, PersistentName, PointRef};
    let mut doc = small();
    let name = |text: &str| text.parse::<PersistentName>().unwrap();
    let corner = name(
        "F1:corner[F1:copy(0)/F1:face(1),F1:copy(0)/F1:face(2)|F1:copy(1)/F1:endCap(e3,e4)](1)",
    );
    doc.add_feature(
        None,
        FeatureKind::DatumPoint(DatumPoint {
            at: PointRef::Vertex {
                vertex: PersistentName::copy(FeatureId(1), 2, corner.clone()),
            },
        }),
    )
    .unwrap();
    assert_eq!(doc.validate(), Ok(()));
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let face = |n: u64| {
        serde_json::json!({"feature": 1, "role": "copy", "sources": [0],
            "of": {"feature": 1, "role": "face", "sources": [n]}})
    };
    let cap = serde_json::json!({"feature": 1, "role": "copy", "sources": [1],
        "of": {"feature": 1, "role": "endCap", "sources": [3, 4]}});
    let written = serde_json::json!({"feature": 1, "role": "copy", "sources": [2],
        "of": {"feature": 1, "role": "corner", "sources": [1],
               "faces": {"on": [face(1), face(2)], "near": [cap.clone()]}}});
    let stored = &value["document"]["features"][1]["kind"]["at"]["vertex"];
    assert_eq!(stored, &written);
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);

    let with = |vertex: serde_json::Value| {
        let mut value = value.clone();
        value["document"]["features"][1]["kind"]["at"]["vertex"] = vertex;
        from_manifest_json(serde_json::to_string(&value).unwrap().as_bytes())
    };
    let by = |faces: serde_json::Value| serde_json::json!({"feature": 1, "role": "corner", "sources": [], "faces": faces});
    // The faces in another order are the same name.
    let turned = with(by(serde_json::json!({"on": [face(2), face(1)]}))).unwrap();
    let FeatureKind::DatumPoint(point) = &turned.features[1].kind else {
        panic!()
    };
    assert_eq!(
        point.at,
        PointRef::Vertex {
            vertex: name("F1:corner[F1:copy(0)/F1:face(1),F1:copy(0)/F1:face(2)]")
        }
    );
    // A vertex on no face at all, and one with nothing near it, are names.
    assert!(with(by(serde_json::json!({}))).is_ok());
    assert!(with(serde_json::json!({"feature": 1, "role": "corner", "sources": []})).is_ok());
    // What is not a name is refused, never read as another.
    for (damaged, why) in [
        (
            by(serde_json::json!({"on": [by(serde_json::json!({"on": [face(1)]}))]})),
            "nested",
        ),
        (
            by(serde_json::json!({"on": [{"feature": 1, "role": "vertex", "sources": [1]}]})),
            "not a face",
        ),
        (
            by(serde_json::json!({"on": [face(1)], "far": []})),
            "an unknown field",
        ),
        (by(serde_json::json!([face(1)])), "a list"),
        (
            serde_json::json!({"feature": 1, "role": "face", "sources": [1],
                               "faces": {"on": [face(1)]}}),
            "a face that lists faces",
        ),
        (
            serde_json::json!({"feature": 1, "role": "corner", "sources": [1, 2],
                               "faces": {"on": [face(1)]}}),
            "two numbers",
        ),
        (
            serde_json::json!({"feature": 1, "role": "copy", "sources": [0],
                "of": face(1), "faces": {"on": [face(1)]}}),
            "a copy that lists faces",
        ),
    ] {
        assert!(with(damaged).is_err(), "{why}");
    }
}

/// A version-6 file, as written before chamfers. Never edit this.
#[test]
fn a_version_6_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v6 = r#"{"format": "3dcraft", "format_version": 6, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "d", "expr": "8 mm"}],
        "features": [
          {"id": 1, "name": "Profile", "suppressed": false, "kind": {"type": "sketch",
            "plane": {"type": "world", "plane": "xy", "offset": "0"},
            "entities": [
              {"id": 1, "construction": false, "geometry": {"type": "point", "x": 0.0, "y": 0.0}},
              {"id": 2, "construction": false, "geometry": {"type": "point", "x": 4.0, "y": 0.0}},
              {"id": 3, "construction": false, "geometry": {"type": "point", "x": 4.0, "y": 3.0}},
              {"id": 4, "construction": false, "geometry": {"type": "point", "x": 0.0, "y": 3.0}},
              {"id": 5, "construction": false, "geometry": {"type": "spline", "degree": 3, "control": [1, 2, 3, 4]}},
              {"id": 6, "construction": false, "geometry": {"type": "line", "start": 4, "end": 1}}],
            "constraints": [],
            "next_entity_id": 7, "next_constraint_id": 1}},
          {"id": 2, "name": "Body", "suppressed": false,
            "kind": {"type": "extrude", "sketch": 1, "distance": "d", "symmetric": false}},
          {"id": 3, "name": "Top", "suppressed": false,
            "kind": {"type": "datumPlane", "by": {"type": "offset",
              "from": {"type": "face", "face": {"feature": 2, "role": "endCap", "sources": [5, 6]}},
              "distance": "1 mm"}}}],
        "rollback": null, "next_feature_id": 4}}"#;
    let doc = match from_manifest_json(v6.as_bytes()) {
        Ok(doc) => doc,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(doc.features[1].kind.type_name(), "extrude");
    assert_eq!(doc.features[2].kind.type_name(), "datumPlane");
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert!(!saved.contains("chamfer"), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
}

#[test]
fn a_chamfer_is_written_like_this_and_round_trips() {
    use craft3d_doc::{Chamfer, ChamferSize, NameRole, PersistentName};
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let face_json = |n: u64| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    let sizes = [
        (
            ChamferSize::Equal {
                distance: Expr::new("width / 20"),
            },
            serde_json::json!({"type": "equal", "distance": "width / 20"}),
        ),
        (
            ChamferSize::TwoDistances {
                distance: Expr::new("2 mm"),
                second: Expr::new("1 mm"),
            },
            serde_json::json!({"type": "twoDistances", "distance": "2 mm", "second": "1 mm"}),
        ),
        (
            ChamferSize::DistanceAngle {
                distance: Expr::new("2 mm"),
                angle: Expr::new("30 deg"),
            },
            serde_json::json!({"type": "distanceAngle", "distance": "2 mm", "angle": "30 deg"}),
        ),
    ];
    for (size, size_json) in sizes {
        let mut doc = small();
        doc.add_feature(
            Some("Bevel"),
            FeatureKind::Chamfer(Chamfer {
                edges: vec![[face(1), face(2)], [face(5), face(1)]],
                size,
                chain: false,
            }),
        )
        .unwrap();
        // A sketch on the chamfer face and one on the corner: the roles a
        // chamfer adds.
        for (role, sources) in [
            (NameRole::Chamfer, vec![0]),
            (NameRole::Chamfer, vec![1, 2]),
            (NameRole::ChamferCorner, vec![0, 1, 2]),
        ] {
            let name = PersistentName::new(FeatureId(2), role, sources);
            assert_eq!(name.to_string().parse::<PersistentName>().unwrap(), name);
            assert_eq!(name.check(), Ok(()));
        }
        assert_eq!(
            PersistentName::new(FeatureId(2), NameRole::Chamfer, vec![1, 2]).to_string(),
            "F2:chamfer(1,2)"
        );
        assert!("F2:chamfer()".parse::<PersistentName>().is_err());
        assert!("F2:chamfer(e1)".parse::<PersistentName>().is_err());
        assert_eq!(doc.validate(), Ok(()));
        let json = to_manifest_json(&doc).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["format_version"], FORMAT_VERSION);
        assert_eq!(
            value["document"]["features"][1]["kind"],
            serde_json::json!({
                "type": "chamfer",
                "edges": [[face_json(1), face_json(2)], [face_json(5), face_json(1)]],
                "size": size_json,
                "chain": false
            })
        );
        assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
        assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
        // `chain` may be left out; an unknown field is refused.
        let without = json.replacen(",\n          \"chain\": false", "", 1);
        assert_ne!(without, json);
        assert_eq!(from_manifest_json(without.as_bytes()).unwrap(), doc);
        let unknown = json.replacen("\"chain\": false", "\"chain\": false, \"radius\": 1", 1);
        assert!(from_manifest_json(unknown.as_bytes()).is_err());
    }
}

/// Version 8: lofts, as they are written.
#[test]
fn a_loft_is_written_like_this_and_round_trips() {
    use craft3d_doc::{EntityId, FeatureId, Loft, LoftSection, MAX_LOFT_SECTIONS};
    let mut doc = small();
    let section = |sketch: u64, start: Option<u64>| LoftSection {
        sketch: FeatureId(sketch),
        start: start.map(EntityId),
    };
    doc.add_feature(
        Some("Hull"),
        FeatureKind::Loft(Loft {
            sections: vec![section(7, None), section(9, Some(3)), section(8, None)],
            ruled: false,
            closed: false,
        }),
    )
    .unwrap();
    doc.add_feature(
        Some("Ring"),
        FeatureKind::Loft(Loft {
            sections: vec![section(7, None), section(9, None), section(8, None)],
            ruled: true,
            closed: true,
        }),
    )
    .unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Lofts came with version 8; the current one is written.
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let features = value["document"]["features"].as_array().unwrap();
    assert_eq!(
        features[features.len() - 2]["kind"],
        serde_json::json!({"type": "loft", "sections": [
            {"sketch": 7}, {"sketch": 9, "start": 3}, {"sketch": 8}]})
    );
    assert_eq!(
        features[features.len() - 1]["kind"],
        serde_json::json!({"type": "loft", "sections": [
            {"sketch": 7}, {"sketch": 9}, {"sketch": 8}], "ruled": true, "closed": true})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // `sections` is required; a field a section does not have is refused,
    // and so are more sections than a loft can have.
    let without = json.replace(r#""sections": ["#, r#""profiles": ["#);
    assert!(from_manifest_json(without.as_bytes()).is_err());
    let extra = json.replace(r#""start": 3"#, r#""start": 3, "twist": 1"#);
    assert_ne!(extra, json);
    assert!(from_manifest_json(extra.as_bytes()).is_err());
    let mut long = small();
    long.add_feature(
        None,
        FeatureKind::Loft(Loft {
            sections: vec![section(1, None); MAX_LOFT_SECTIONS + 1],
            ruled: false,
            closed: false,
        }),
    )
    .unwrap();
    assert!(
        to_manifest_json(&long).is_err() || {
            let json = to_manifest_json(&long).unwrap();
            from_manifest_json(json.as_bytes()).is_err()
        }
    );
}

/// A version-7 file, as written before lofts existed. Never edit this.
#[test]
fn a_version_7_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v7 = r#"{"format": "3dcraft", "format_version": 7, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "c", "expr": "1 mm"}],
        "features": [
          {"id": 1, "name": "Block", "suppressed": false, "kind":
            {"type": "block", "position": ["0", "0", "0"], "size": ["10", "8", "6"]}},
          {"id": 2, "name": "Edge", "suppressed": false, "kind":
            {"type": "chamfer",
             "edges": [[{"feature": 1, "role": "face", "sources": [1]},
                        {"feature": 1, "role": "face", "sources": [2]}]],
             "size": {"type": "equal", "distance": "c"}, "chain": false}}],
        "rollback": null, "next_feature_id": 3}}"#;
    let doc = match from_manifest_json(v7.as_bytes()) {
        Ok(doc) => doc,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(doc.features[1].kind.type_name(), "chamfer");
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert!(!saved.contains("loft"), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
}

/// A version-8 file, as written before fillets: a loft, and a chamfer
/// with a sketch's plane on its face. Never edit this.
#[test]
fn a_version_8_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v8 = r#"{"format": "3dcraft", "format_version": 8, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "d", "expr": "8 mm"}],
        "features": [
          {"id": 1, "name": "Base", "suppressed": false, "kind": {"type": "block",
            "position": ["0", "0", "0"], "size": ["40 mm", "30 mm", "d"]}},
          {"id": 2, "name": "Bevel", "suppressed": false, "kind": {"type": "chamfer",
            "edges": [[{"feature": 1, "role": "face", "sources": [1]},
                       {"feature": 1, "role": "face", "sources": [2]}]],
            "size": {"type": "twoDistances", "distance": "2 mm", "second": "d / 8"},
            "chain": true}},
          {"id": 3, "name": "On the bevel", "suppressed": false, "kind": {"type": "datumPlane",
            "by": {"type": "offset",
              "from": {"type": "face", "face": {"feature": 2, "role": "chamfer", "sources": [0]}},
              "distance": "1 mm"}}},
          {"id": 4, "name": "Hull", "suppressed": false, "kind": {"type": "loft",
            "sections": [{"sketch": 7}, {"sketch": 9, "start": 3}, {"sketch": 8}]}}],
        "rollback": null, "next_feature_id": 5}}"#;
    let doc = match from_manifest_json(v8.as_bytes()) {
        Ok(doc) => doc,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(doc.features[1].kind.type_name(), "chamfer");
    assert_eq!(doc.features[3].kind.type_name(), "loft");
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert!(!saved.contains("fillet"), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
}

/// Version 9: fillets, as they are written.
#[test]
fn a_fillet_is_written_like_this_and_round_trips() {
    use craft3d_doc::{Fillet, NameRole, PersistentName};
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let face_json = |n: u64| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    let mut doc = small();
    doc.add_feature(
        Some("Round"),
        FeatureKind::Fillet(Fillet {
            edges: vec![[face(1), face(2)], [face(5), face(1)]],
            radius: Expr::new("width / 20"),
            chain: false,
        }),
    )
    .unwrap();
    // The roles a fillet adds: its faces and the ball in a corner.
    for (role, sources) in [
        (NameRole::Fillet, vec![0]),
        (NameRole::Fillet, vec![1, 2]),
        (NameRole::FilletCorner, vec![0, 1, 2]),
    ] {
        let name = PersistentName::new(FeatureId(2), role, sources);
        assert_eq!(name.to_string().parse::<PersistentName>().unwrap(), name);
        assert_eq!(name.check(), Ok(()));
        assert!(name.is_face());
    }
    assert_eq!(
        PersistentName::new(FeatureId(2), NameRole::Fillet, vec![1, 2]).to_string(),
        "F2:fillet(1,2)"
    );
    assert_eq!(
        PersistentName::new(FeatureId(2), NameRole::FilletCorner, vec![0, 1, 2]).to_string(),
        "F2:filletCorner(0,1,2)"
    );
    assert!("F2:fillet()".parse::<PersistentName>().is_err());
    assert!("F2:fillet(e1)".parse::<PersistentName>().is_err());
    assert_eq!(doc.validate(), Ok(()));
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Fillets came with version 9; the current one is written.
    assert_eq!(value["format_version"], FORMAT_VERSION);
    assert_eq!(
        value["document"]["features"][1]["kind"],
        serde_json::json!({
            "type": "fillet",
            "edges": [[face_json(1), face_json(2)], [face_json(5), face_json(1)]],
            "radius": "width / 20",
            "chain": false
        })
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // `chain` may be left out; an unknown field is refused.
    let without = json.replacen(",\n          \"chain\": false", "", 1);
    assert_ne!(without, json);
    assert_eq!(from_manifest_json(without.as_bytes()).unwrap(), doc);
    let unknown = json.replacen("\"chain\": false", "\"chain\": false, \"size\": 1", 1);
    assert!(from_manifest_json(unknown.as_bytes()).is_err());
    // A fillet without a radius is not a fillet.
    let bare = json.replacen("\"radius\": \"width / 20\",", "", 1);
    assert_ne!(bare, json);
    assert!(from_manifest_json(bare.as_bytes()).is_err());
}

/// Version 10: sweeps, as they are written.
#[test]
fn a_sweep_is_written_like_this_and_round_trips() {
    use craft3d_doc::{FeatureId, Sweep};
    let mut doc = small();
    doc.add_feature(
        Some("Pipe"),
        FeatureKind::Sweep(Sweep {
            sketch: FeatureId(7),
            path: FeatureId(9),
        }),
    )
    .unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Sweeps came with version 10; the current one is written.
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let features = value["document"]["features"].as_array().unwrap();
    assert_eq!(
        features[features.len() - 1]["kind"],
        serde_json::json!({"type": "sweep", "sketch": 7, "path": 9})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // Both sketches are required, and a field a sweep does not have is
    // refused.
    let without = json.replace(r#""path": 9"#, r#""sketch": 7"#);
    assert_ne!(without, json);
    assert!(from_manifest_json(without.as_bytes()).is_err());
    let extra = json.replace(r#""path": 9"#, r#""path": 9, "twist": "30 deg""#);
    assert_ne!(extra, json);
    assert!(from_manifest_json(extra.as_bytes()).is_err());
}

/// An offset of the profiles of a sketch (format 19), and a name of what
/// a sweep made of it: the offset's curves are numbers in `sources` (a
/// head with the feature, then the curves), and read back to the same
/// name.
#[test]
fn an_offset_profile_is_written_like_this_and_round_trips() {
    use craft3d_doc::{Expr, FeatureId, Fillet, OffsetCorners, OffsetProfile, PersistentName};
    let mut doc = small();
    doc.add_feature(
        Some("Kerf"),
        FeatureKind::OffsetProfile(OffsetProfile {
            sketch: FeatureId(7),
            distance: Expr::new("kerf / 2"),
            corners: OffsetCorners::Round,
        }),
    )
    .unwrap();
    doc.add_feature(
        Some("Sharp"),
        FeatureKind::OffsetProfile(OffsetProfile {
            sketch: FeatureId(7),
            distance: Expr::new("-1 mm"),
            corners: OffsetCorners::Sharp,
        }),
    )
    .unwrap();
    let side: PersistentName = "F10:side(F9:loop(e1,e7[3])/F9:offset(e7[3]))"
        .parse()
        .unwrap();
    let corner: PersistentName = "F10:piece(1)/F10:side(F9:offsetCorner(e7,e8))"
        .parse()
        .unwrap();
    doc.add_feature(
        Some("Round"),
        FeatureKind::Fillet(Fillet {
            edges: vec![[side.clone(), corner.clone()]],
            radius: Expr::new("1 mm"),
            chain: false,
        }),
    )
    .unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], 21);
    let features = value["document"]["features"].as_array().unwrap();
    let n = features.len();
    assert_eq!(
        features[n - 3]["kind"],
        serde_json::json!({"type": "offsetProfile", "sketch": 7, "distance": "kerf / 2"})
    );
    assert_eq!(
        features[n - 2]["kind"],
        serde_json::json!({"type": "offsetProfile", "sketch": 7, "distance": "-1 mm", "corners": "sharp"})
    );
    let read = from_manifest_json(json.as_bytes()).unwrap();
    assert_eq!(read, doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    let FeatureKind::Fillet(fillet) = &read.features[n - 1].kind else {
        panic!("not a fillet");
    };
    assert_eq!(fillet.edges[0][0].to_string(), side.to_string());
    assert_eq!(fillet.edges[0][1].to_string(), corner.to_string());
    // An unknown corner rule is refused.
    let bad = json.replace(r#""corners": "sharp""#, r#""corners": "square""#);
    assert_ne!(bad, json);
    assert!(from_manifest_json(bad.as_bytes()).is_err());
}

/// A version-9 file, as written before sweeps existed: a fillet and a
/// loft. Never edit this.
#[test]
fn a_version_9_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v9 = r#"{"format": "3dcraft", "format_version": 9, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "r", "expr": "2 mm"}],
        "features": [
          {"id": 1, "name": "Block", "suppressed": false, "kind":
            {"type": "block", "position": ["0", "0", "0"], "size": ["10", "8", "6"]}},
          {"id": 2, "name": "Round", "suppressed": false, "kind": {"type": "fillet",
            "edges": [[{"feature": 1, "role": "face", "sources": [1]},
                       {"feature": 1, "role": "face", "sources": [2]}]],
            "radius": "r", "chain": false}},
          {"id": 3, "name": "Hull", "suppressed": false, "kind":
            {"type": "loft", "sections": [{"sketch": 7}, {"sketch": 9, "start": 3}],
             "ruled": true}}],
        "rollback": null, "next_feature_id": 4}}"#;
    let doc = match from_manifest_json(v9.as_bytes()) {
        Ok(doc) => doc,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(doc.features[1].kind.type_name(), "fillet");
    assert_eq!(doc.features[2].kind.type_name(), "loft");
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert!(!saved.contains("sweep"), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
    // The same file from a version this build does not know is refused.
    let newer = v9.replace(
        "\"format_version\": 9",
        &format!("\"format_version\": {}", FORMAT_VERSION + 1),
    );
    assert!(from_manifest_json(newer.as_bytes()).is_err());
}

/// A version-10 file, as written before the face operations: a fillet, a
/// datum on its face and a sweep. Never edit this.
#[test]
fn a_version_10_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v10 = r#"{"format": "3dcraft", "format_version": 10, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "r", "expr": "3 mm"}],
        "features": [
          {"id": 1, "name": "Base", "suppressed": false, "kind": {"type": "block",
            "position": ["0", "0", "0"], "size": ["40 mm", "30 mm", "10 mm"]}},
          {"id": 2, "name": "Round", "suppressed": false, "kind": {"type": "fillet",
            "edges": [[{"feature": 1, "role": "face", "sources": [1]},
                       {"feature": 1, "role": "face", "sources": [2]}]],
            "radius": "r", "chain": false}},
          {"id": 3, "name": "On the round", "suppressed": false, "kind": {"type": "datumAxis",
            "by": {"type": "face", "face": {"feature": 2, "role": "fillet", "sources": [0]}}}},
          {"id": 4, "name": "Pipe", "suppressed": false, "kind": {"type": "sweep",
            "sketch": 7, "path": 9}}],
        "rollback": null, "next_feature_id": 5}}"#;
    let doc = match from_manifest_json(v10.as_bytes()) {
        Ok(doc) => doc,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(doc.features[1].kind.type_name(), "fillet");
    assert_eq!(doc.features[3].kind.type_name(), "sweep");
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert!(
        !saved.contains("offsetFace") && !saved.contains("shell"),
        "{saved}"
    );
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
}

/// Version 11: the face operations, as they are written.
#[test]
fn the_face_operations_are_written_like_this_and_round_trip() {
    use craft3d_doc::{NameRole, OffsetFace, PersistentName, Shell};
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let face_json = |n: u64| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    let mut doc = small();
    doc.add_feature(
        Some("Taller"),
        FeatureKind::OffsetFace(OffsetFace {
            faces: vec![face(1), face(3)],
            distance: Expr::new("-width / 20"),
        }),
    )
    .unwrap();
    doc.add_feature(
        Some("Hollow"),
        FeatureKind::Shell(Shell {
            source: FeatureId(1),
            thickness: Expr::new("2 mm"),
            open: vec![face(1)],
        }),
    )
    .unwrap();
    assert_eq!(doc.validate(), Ok(()));
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    // The face operations came with version 11; the current one is
    // written.
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let kinds: Vec<&serde_json::Value> = value["document"]["features"]
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
        .map(|f| &f["kind"])
        .collect();
    assert_eq!(
        kinds,
        [
            &serde_json::json!({"type": "offsetFace",
                "faces": [face_json(1), face_json(3)], "distance": "-width / 20"}),
            &serde_json::json!({"type": "shell", "source": 1, "thickness": "2 mm",
                "open": [face_json(1)]}),
        ]
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // An offset without a distance and a shell without a thickness are
    // neither; an unknown field is refused.
    let bare = json.replacen("\"distance\": \"-width / 20\"", "\"far\": 1", 1);
    assert_ne!(bare, json);
    assert!(from_manifest_json(bare.as_bytes()).is_err());
    let bare = json.replacen("\"thickness\": \"2 mm\",", "", 1);
    assert_ne!(bare, json);
    assert!(from_manifest_json(bare.as_bytes()).is_err());
}

/// A version-11 file, as written before delete face: an offset and a
/// shell. Never edit this.
#[test]
fn a_version_11_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v11 = r#"{"format": "3dcraft", "format_version": 11, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "t", "expr": "1 mm"}],
        "features": [
          {"id": 1, "name": "Base", "suppressed": false, "kind": {"type": "block",
            "position": ["0", "0", "0"], "size": ["40 mm", "30 mm", "10 mm"]}},
          {"id": 2, "name": "Taller", "suppressed": false, "kind": {"type": "offsetFace",
            "faces": [{"feature": 1, "role": "face", "sources": [1]}], "distance": "2 mm"}},
          {"id": 3, "name": "Hollow", "suppressed": false, "kind": {"type": "shell",
            "source": 2, "thickness": "t",
            "open": [{"feature": 2, "role": "copy", "sources": [0],
                      "of": {"feature": 1, "role": "face", "sources": [1]}}]}}],
        "rollback": null, "next_feature_id": 4}}"#;
    let doc = match from_manifest_json(v11.as_bytes()) {
        Ok(doc) => doc,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(doc.features[1].kind.type_name(), "offsetFace");
    assert_eq!(doc.features[2].kind.type_name(), "shell");
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert!(!saved.contains("deleteFace"), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
    // The same file from a version this build does not know is refused.
    let newer = v11.replace(
        "\"format_version\": 11",
        &format!("\"format_version\": {}", FORMAT_VERSION + 1),
    );
    assert!(from_manifest_json(newer.as_bytes()).is_err());
}

/// Version 12: delete face, as it is written.
#[test]
fn delete_face_is_written_like_this_and_round_trips() {
    use craft3d_doc::{DeleteFace, NameRole, PersistentName};
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let face_json = |n: u64| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    let mut doc = small();
    doc.add_feature(
        Some("Plain"),
        FeatureKind::DeleteFace(DeleteFace {
            faces: vec![face(2), face(0)],
        }),
    )
    .unwrap();
    assert_eq!(doc.validate(), Ok(()));
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Delete face came with version 12; the current one is written.
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let features = value["document"]["features"].as_array().unwrap();
    assert_eq!(
        features[features.len() - 1]["kind"],
        serde_json::json!({"type": "deleteFace", "faces": [face_json(2), face_json(0)]})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // Without its faces it is no deletion; an unknown field is refused.
    let bare = json.replacen(
        "\"type\": \"deleteFace\",",
        "\"type\": \"deleteFace\", \"also\": 1,",
        1,
    );
    assert_ne!(bare, json);
    assert!(from_manifest_json(bare.as_bytes()).is_err());
}

/// A version-12 file, as written before the helix: a block with a face
/// deleted. Never edit this.
#[test]
fn a_version_12_file_opens_unchanged_and_is_saved_as_the_current_version() {
    let v12 = r#"{"format": "3dcraft", "format_version": 12, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [],
        "features": [
          {"id": 1, "name": "Base", "suppressed": false, "kind": {"type": "block",
            "position": ["0", "0", "0"], "size": ["40 mm", "30 mm", "10 mm"]}},
          {"id": 2, "name": "Plain", "suppressed": false, "kind": {"type": "deleteFace",
            "faces": [{"feature": 1, "role": "face", "sources": [2]}]}}],
        "rollback": null, "next_feature_id": 3}}"#;
    let doc = match from_manifest_json(v12.as_bytes()) {
        Ok(doc) => doc,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(doc.features[1].kind.type_name(), "deleteFace");
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert!(!saved.contains("datumHelix"), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
    let newer = v12.replace(
        "\"format_version\": 12",
        &format!("\"format_version\": {}", FORMAT_VERSION + 1),
    );
    assert!(from_manifest_json(newer.as_bytes()).is_err());
}

/// Version 13: a helix, and a sweep along it, as they are written.
#[test]
fn a_helix_is_written_like_this_and_round_trips() {
    use craft3d_doc::{
        AxisRef, DatumHelix, Expr, HelixExtent, HelixStart, PointRef, Sweep, WorldAxis,
    };
    let mut doc = small();
    let coil = doc
        .add_feature(
            Some("Coil"),
            FeatureKind::DatumHelix(DatumHelix {
                axis: AxisRef::World { axis: WorldAxis::Z },
                start: HelixStart::Radius {
                    radius: Expr::new("10 mm"),
                },
                pitch: Expr::new("4 mm"),
                extent: HelixExtent::Turns {
                    turns: Expr::new("2.5"),
                },
                left_handed: false,
            }),
        )
        .unwrap();
    doc.add_feature(
        Some("Other"),
        FeatureKind::DatumHelix(DatumHelix {
            axis: AxisRef::Datum { feature: coil },
            start: HelixStart::Point {
                point: PointRef::At {
                    position: [Expr::new("5"), Expr::new("0"), Expr::new("0")],
                },
            },
            pitch: Expr::new("1.5 mm"),
            extent: HelixExtent::Height {
                height: Expr::new("12 mm"),
            },
            left_handed: true,
        }),
    )
    .unwrap();
    let sketch = doc.features[0].id;
    doc.add_feature(
        Some("Spring"),
        FeatureKind::Sweep(Sweep { sketch, path: coil }),
    )
    .unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let features = value["document"]["features"].as_array().unwrap();
    let n = features.len();
    assert_eq!(
        features[n - 3]["kind"],
        serde_json::json!({"type": "datumHelix",
            "axis": {"type": "world", "axis": "z"},
            "start": {"type": "radius", "radius": "10 mm"},
            "pitch": "4 mm",
            "extent": {"type": "turns", "turns": "2.5"}})
    );
    assert_eq!(
        features[n - 2]["kind"],
        serde_json::json!({"type": "datumHelix",
            "axis": {"type": "datum", "feature": coil.0},
            "start": {"type": "point", "point": {"type": "at", "position": ["5", "0", "0"]}},
            "pitch": "1.5 mm",
            "extent": {"type": "height", "height": "12 mm"},
            "left_handed": true})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    let bare = json.replacen(
        "\"type\": \"datumHelix\",",
        "\"type\": \"datumHelix\", \"also\": 1,",
        1,
    );
    assert_ne!(bare, json);
    assert!(from_manifest_json(bare.as_bytes()).is_err());
}

/// Version 14: a thread's size given as a designation, as a diameter or
/// not at all, its pitch given or not, as they are written.
#[test]
fn a_thread_is_written_like_this_and_round_trips() {
    use craft3d_doc::{Expr, NameRole, PersistentName, Thread, ThreadSize};
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let mut doc = small();
    for (size, pitch) in [
        (None, None),
        (Some(ThreadSize::Designation("M10".into())), None),
        (
            Some(ThreadSize::Diameter {
                diameter: Expr::new("d"),
            }),
            Some(Expr::new("1.25 mm")),
        ),
    ] {
        doc.add_feature(
            None,
            FeatureKind::Thread(Thread {
                face: face(0),
                start: face(1),
                size,
                pitch,
                length: Expr::new("12 mm"),
                left_handed: false,
                modeled: false,
            }),
        )
        .unwrap();
    }
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let features = value["document"]["features"].as_array().unwrap();
    let n = features.len();
    let face = |n: u32| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    assert_eq!(
        features[n - 3]["kind"],
        serde_json::json!({"type": "thread", "face": face(0), "start": face(1),
            "length": "12 mm"})
    );
    assert_eq!(
        features[n - 2]["kind"],
        serde_json::json!({"type": "thread", "face": face(0), "start": face(1),
            "size": "M10", "length": "12 mm"})
    );
    assert_eq!(
        features[n - 1]["kind"],
        serde_json::json!({"type": "thread", "face": face(0), "start": face(1),
            "size": {"diameter": "d"}, "pitch": "1.25 mm", "length": "12 mm"})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // A diameter with anything else beside it is not read.
    let bare = json.replacen("\"diameter\": \"d\"", "\"diameter\": \"d\", \"also\": 1", 1);
    assert_ne!(bare, json);
    assert!(from_manifest_json(bare.as_bytes()).is_err());
}

/// A version-13 file, as written before a thread's pitch could follow its
/// size: the pitch the command wrote when it was given none (the coarse
/// pitch of the designation, as text) is taken out, so it follows the
/// size; every other pitch stays. And a thread that did not say
/// `cosmetic: true` was cut, so it is `modeled` now that a thread is
/// cosmetic by default. Never edit this.
#[test]
fn a_version_13_thread_loses_its_default_pitch_and_stays_cut() {
    let v13 = r#"{"format": "3dcraft", "format_version": 13, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [{"name": "d", "expr": "10 mm"}],
        "features": [
          {"id": 1, "name": "Shank", "suppressed": false, "kind": {"type": "cylinder",
            "position": ["0", "0", "0"], "radius": "d / 2", "height": "20 mm"}},
          {"id": 2, "name": "Coarse", "suppressed": false, "kind": {"type": "thread",
            "face": {"feature": 1, "role": "face", "sources": [0]},
            "start": {"feature": 1, "role": "face", "sources": [1]},
            "size": "M10", "pitch": "1.5 mm", "length": "12 mm", "cosmetic": true}},
          {"id": 3, "name": "Fine", "suppressed": false, "kind": {"type": "thread",
            "face": {"feature": 1, "role": "face", "sources": [0]},
            "start": {"feature": 1, "role": "face", "sources": [1]},
            "size": "M10", "pitch": "1.25 mm", "length": "12 mm"}},
          {"id": 4, "name": "Six", "suppressed": false, "kind": {"type": "thread",
            "face": {"feature": 1, "role": "face", "sources": [0]},
            "start": {"feature": 1, "role": "face", "sources": [1]},
            "size": "M6", "pitch": "1 mm", "length": "12 mm", "left_handed": true}},
          {"id": 5, "name": "Number", "suppressed": false, "kind": {"type": "thread",
            "face": {"feature": 1, "role": "face", "sources": [0]},
            "start": {"feature": 1, "role": "face", "sources": [1]},
            "size": "M10", "pitch": "1.5", "length": "12 mm", "cosmetic": false}},
          {"id": 6, "name": "Formula", "suppressed": false, "kind": {"type": "thread",
            "face": {"feature": 1, "role": "face", "sources": [0]},
            "start": {"feature": 1, "role": "face", "sources": [1]},
            "size": "M2.5", "pitch": "0.45 mm * 1", "length": "1 mm"}}],
        "rollback": null, "next_feature_id": 7}}"#;
    let doc = match from_manifest_json(v13.as_bytes()) {
        Ok(doc) => doc,
        Err(error) => panic!("{error}"),
    };
    let threads: Vec<_> = doc
        .features
        .iter()
        .filter_map(|f| match &f.kind {
            FeatureKind::Thread(t) => Some((
                t.size.clone(),
                t.pitch.as_ref().map(|p| p.source().to_string()),
                t.modeled,
            )),
            _ => None,
        })
        .collect();
    use craft3d_doc::ThreadSize::Designation;
    assert_eq!(
        threads,
        [
            // `cosmetic: true` stays cosmetic; every other thread of
            // version 13 was cut, and is modelled.
            (Some(Designation("M10".into())), None, false),
            (
                Some(Designation("M10".into())),
                Some("1.25 mm".into()),
                true
            ),
            (Some(Designation("M6".into())), None, true),
            (Some(Designation("M10".into())), Some("1.5".into()), true),
            (
                Some(Designation("M2.5".into())),
                Some("0.45 mm * 1".into()),
                true
            ),
        ]
    );
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert_eq!(saved.matches("\"modeled\": true").count(), 4, "{saved}");
    assert!(!saved.contains("cosmetic"), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
    let newer = v13.replace(
        "\"format_version\": 13",
        &format!("\"format_version\": {}", FORMAT_VERSION + 1),
    );
    assert!(from_manifest_json(newer.as_bytes()).is_err());
}

/// Version 16: move face, as it is written (a new kind, with the
/// bodies' appearance; nothing an earlier file holds changed meaning).
#[test]
fn move_face_is_written_like_this_and_round_trips() {
    use craft3d_doc::{MoveFace, NameRole, PersistentName};
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let face_json = |n: u64| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    let mut doc = small();
    doc.add_feature(
        Some("Tilted"),
        FeatureKind::MoveFace(MoveFace {
            faces: vec![face(1)],
            translate: [Expr::new("0"), Expr::new("0"), Expr::new("width / 10")],
            pivot: [Expr::new("5 mm"), Expr::new("0"), Expr::new("10 mm")],
            axis: [Expr::new("0"), Expr::new("1"), Expr::new("0")],
            angle: Expr::new("10 deg"),
        }),
    )
    .unwrap();
    assert_eq!(doc.validate(), Ok(()));
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    let kind = &value["document"]["features"][1]["kind"];
    assert_eq!(
        kind,
        &serde_json::json!({"type": "moveFace", "faces": [face_json(1)],
            "translate": ["0", "0", "width / 10"], "pivot": ["5 mm", "0", "10 mm"],
            "axis": ["0", "1", "0"], "angle": "10 deg"})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // A move without its angle is not one.
    let bare = json.replacen("\"angle\": \"10 deg\"", "\"far\": 1", 1);
    assert_ne!(bare, json);
    assert!(from_manifest_json(bare.as_bytes()).is_err());
}

/// Version 16: thicken, as it is written.
#[test]
fn thicken_is_written_like_this_and_round_trips() {
    use craft3d_doc::{NameRole, PersistentName, Thicken, ThickenSide};
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let face_json = |n: u64| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    let mut doc = small();
    for (name, side) in [("Plate", ThickenSide::Outside), ("Skin", ThickenSide::Both)] {
        doc.add_feature(
            Some(name),
            FeatureKind::Thicken(Thicken {
                faces: vec![face(1), face(2)],
                thickness: Expr::new("width / 20"),
                side,
            }),
        )
        .unwrap();
    }
    assert_eq!(doc.validate(), Ok(()));
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    assert_eq!(
        value["document"]["features"][1]["kind"],
        serde_json::json!({"type": "thicken", "faces": [face_json(1), face_json(2)],
            "thickness": "width / 20", "side": "outside"})
    );
    assert_eq!(value["document"]["features"][2]["kind"]["side"], "both");
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // A side that is none of the three is refused.
    let bad = json.replacen("\"side\": \"both\"", "\"side\": \"up\"", 1);
    assert_ne!(bad, json);
    assert!(from_manifest_json(bad.as_bytes()).is_err());
}

/// Version 16: draft, as it is written.
#[test]
fn draft_is_written_like_this_and_round_trips() {
    use craft3d_doc::{Draft, NameRole, PersistentName, PlaneRef, WorldPlane};
    let face = |n: u64| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
    let face_json = |n: u64| serde_json::json!({"feature": 1, "role": "face", "sources": [n]});
    let mut doc = small();
    doc.add_feature(
        Some("Tapered"),
        FeatureKind::Draft(Draft {
            faces: vec![face(2), face(3)],
            neutral: PlaneRef::Face { face: face(0) },
            angle: Expr::new("3 deg"),
            reversed: false,
        }),
    )
    .unwrap();
    doc.add_feature(
        Some("Back"),
        FeatureKind::Draft(Draft {
            faces: vec![face(4)],
            neutral: PlaneRef::World {
                plane: WorldPlane::Xy,
            },
            angle: Expr::new("-2 deg"),
            reversed: true,
        }),
    )
    .unwrap();
    assert_eq!(doc.validate(), Ok(()));
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], FORMAT_VERSION);
    assert_eq!(
        value["document"]["features"][1]["kind"],
        serde_json::json!({"type": "draft", "faces": [face_json(2), face_json(3)],
            "neutral": {"type": "face", "face": face_json(0)}, "angle": "3 deg"})
    );
    assert_eq!(value["document"]["features"][2]["kind"]["reversed"], true);
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // A draft without a neutral plane is not one.
    let bare = json.replacen("\"neutral\"", "\"far\"", 1);
    assert_ne!(bare, json);
    assert!(from_manifest_json(bare.as_bytes()).is_err());
}

/// Version 16: a colour and a material per body, as they are written.
#[test]
fn body_appearances_and_materials_are_written_like_this_and_round_trip() {
    use craft3d_doc::{Appearance, Material};
    let mut doc = small();
    doc.set_appearance(
        FeatureId(1),
        Some(Appearance {
            color: [200, 30, 4],
            metallic: Some(0.25),
            roughness: None,
        }),
    )
    .unwrap();
    doc.set_material(
        FeatureId(1),
        Some(Material {
            name: "Cast iron".into(),
            density: 7200.0,
        }),
    )
    .unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        value["document"]["bodies"],
        serde_json::json!([{
            "feature": 1,
            "appearance": {"color": [200, 30, 4], "metallic": 0.25},
            "material": {"name": "Cast iron", "density": 7200.0}
        }])
    );
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // A document without any writes no `bodies` at all.
    assert!(!to_manifest_json(&small()).unwrap().contains("bodies"));
}

#[test]
fn a_version_15_file_opens_without_appearances_and_is_saved_as_the_current_version() {
    let v15 = r#"{"format": "3dcraft", "format_version": 15, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [],
        "features": [
          {"id": 1, "name": "Base", "suppressed": false, "kind": {"type": "block",
            "position": ["0", "0", "0"], "size": ["40 mm", "30 mm", "10 mm"]}}],
        "rollback": null, "next_feature_id": 2}}"#;
    let doc = match from_manifest_json(v15.as_bytes()) {
        Ok(doc) => doc,
        Err(error) => panic!("{error}"),
    };
    assert!(doc.bodies.is_empty());
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    assert_eq!(from_manifest_json(saved.as_bytes()).unwrap(), doc);
    // `bodies` in a version-15 file is still read (the step changes
    // nothing), but a bad entry is refused like any invalid document.
    let bad = v15.replace(
        "\"next_feature_id\": 2}",
        "\"next_feature_id\": 2, \"bodies\": [{\"feature\": 5, \"appearance\": {\"color\": [1, 2, 3]}}]}",
    );
    assert!(from_manifest_json(bad.as_bytes()).is_err());
    let out_of_range = v15.replace(
        "\"next_feature_id\": 2}",
        "\"next_feature_id\": 2, \"bodies\": [{\"feature\": 1, \"appearance\": {\"color\": [1, 2, 300]}}]}",
    );
    assert!(from_manifest_json(out_of_range.as_bytes()).is_err());
    let newer = v15.replace(
        "\"format_version\": 15",
        &format!("\"format_version\": {}", FORMAT_VERSION + 1),
    );
    assert!(from_manifest_json(newer.as_bytes()).is_err());
}

/// Version 17 made `floor`, `ceil`, `round`, `trunc` and `mod` function
/// names. A version-16 file that gave a variable such a name opens with
/// the variable renamed everywhere, and says so. Never edit this
/// fixture: it stands for the files people have on disk.
#[test]
fn a_version_16_file_with_a_variable_named_like_a_new_function_opens_renamed() {
    let v16 = r#"{"format": "3dcraft", "format_version": 16, "document": {
        "units": {"length": "mm", "angle": "deg"},
        "tolerance": {"linear": 1e-6, "angular": 1e-9},
        "variables": [
          {"name": "floor", "expr": "4 mm"},
          {"name": "round", "expr": "floor * 2"},
          {"name": "width", "expr": "40 mm"}],
        "features": [
          {"id": 1, "name": "Base", "suppressed": false, "kind": {"type": "block",
            "position": ["0", "0", "-floor"], "size": ["width", "round + 1 mm", "floor"]}}],
        "rollback": null, "next_feature_id": 2}}"#;
    let (doc, warnings) = match craft3d_format::from_manifest_json_noted(v16.as_bytes()) {
        Ok(read) => read,
        Err(error) => panic!("{error}"),
    };
    let variables: Vec<(&str, &str)> = doc
        .variables
        .iter()
        .map(|v| (v.name.as_str(), v.expr.source()))
        .collect();
    assert_eq!(
        variables,
        [
            ("floor_1", "4 mm"),
            ("round_1", "floor_1 * 2"),
            ("width", "40 mm")
        ]
    );
    let FeatureKind::Block(block) = &doc.features[0].kind else {
        panic!("not a block")
    };
    assert_eq!(block.position[2].source(), "-floor_1");
    assert_eq!(block.size[1].source(), "round_1 + 1 mm");
    assert_eq!(block.size[2].source(), "floor_1");
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings[0].contains("`floor` renamed `floor_1`"),
        "{warnings:?}"
    );
    // Saved as the current version, it reads back as it is.
    let saved = to_manifest_json(&doc).unwrap();
    assert!(saved.contains(&version_field()), "{saved}");
    let (again, none) = craft3d_format::from_manifest_json_noted(saved.as_bytes()).unwrap();
    assert_eq!(again, doc);
    assert!(none.is_empty());
    // A file of the current version with such a name is invalid: the name
    // is reserved there.
    let current = v16.replace(
        "\"format_version\": 16",
        &format!("\"format_version\": {FORMAT_VERSION}"),
    );
    assert!(from_manifest_json(current.as_bytes()).is_err());
}

/// Nested sketch patterns (version 21): a pattern's `patterns` and
/// `skip`, an image point of an image point, and names with an index per
/// level.
#[test]
fn nested_sketch_patterns_are_written_like_this_and_round_trip() {
    use craft3d_doc::{InstanceIndex, SketchPatternLayout};
    let mut doc = small();
    let mut sketch = Sketch::new(SketchPlane::default());
    let a = sketch.add_point(0.0, 0.0).unwrap();
    let b = sketch.add_point(1.0, 0.0).unwrap();
    let tab = sketch.add_line(a, b, false).unwrap();
    let chain = |from, to| SketchPatternLayout::Chain { from, to };
    let p1 = sketch
        .add_pattern(vec![tab], Expr::new("k"), chain(a, b))
        .unwrap();
    let end = sketch.image_point(b, InstanceIndex::Last).unwrap();
    let far = sketch.add_point(3.0, 0.0).unwrap();
    let spacer = sketch.add_line(end, far, false).unwrap();
    let p2 = sketch
        .add_nested_pattern(
            vec![spacer],
            vec![p1],
            Expr::new("n"),
            chain(a, far),
            vec![],
        )
        .unwrap();
    let corner = sketch
        .image_path(b, &[InstanceIndex::Last, InstanceIndex::Last])
        .unwrap();
    let hole = sketch.add_point(0.0, -5.0).unwrap();
    let disc = sketch.add_circle(hole, 0.5, false).unwrap();
    sketch
        .add_nested_pattern(
            vec![disc],
            vec![],
            Expr::new("6"),
            SketchPatternLayout::Linear {
                step: [Expr::new("2"), Expr::new("0")],
                second: None,
            },
            vec![InstanceIndex::At(3)],
        )
        .unwrap();
    doc.add_feature(Some("Edge"), FeatureKind::Sketch(sketch))
        .unwrap();
    // A name with an index per level, in a sketch on a face.
    let mut on = Sketch::new(SketchPlane::Face {
        face: "F9:side(e3[2][1])".parse().unwrap(),
        offset: Expr::new("0"),
    });
    on.add_point(0.0, 0.0).unwrap();
    doc.add_feature(Some("Mark"), FeatureKind::Sketch(on))
        .unwrap();
    doc.validate().unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format_version"], 21);
    let kind = &value["document"]["features"][1]["kind"];
    assert_eq!(kind["patterns"][1]["patterns"], serde_json::json!([p1.0]));
    assert_eq!(kind["patterns"][2]["skip"], serde_json::json!([{"at": 3}]));
    let image = kind["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == corner.0)
        .unwrap();
    assert_eq!(
        image["geometry"],
        serde_json::json!({"type": "image", "of": end.0, "pattern": p2.0, "index": "last"})
    );
    // The curve's own number, then one word for the outer level: bits 32
    // to 47 zero, tag 5 from bit 48, the index plus one.
    let (curve, cell) = (3 + (3_u64 << 32), (5_u64 << 48) + 2);
    assert_eq!(
        value["document"]["features"][2]["kind"]["plane"]["face"],
        serde_json::json!({"feature": 9, "role": "side", "sources": [curve, cell]})
    );
    assert_eq!(from_manifest_json(json.as_bytes()).unwrap(), doc);
    assert_eq!(from_bytes(&to_bytes(&doc).unwrap()).unwrap(), doc);
    // What a file may not hold: a cycle, a pattern in two seeds, four
    // levels, an image of an image after it, a name whose levels are not
    // in the form names keep.
    for (from, to) in [
        (
            "\"count\": \"k\",".to_string(),
            format!("\"count\": \"k\",\n            \"patterns\": [{}],", p2.0),
        ),
        (
            "\"count\": \"6\",".to_string(),
            format!("\"count\": \"6\",\n            \"patterns\": [{}],", p1.0),
        ),
        // A trailing level at instance 0, a continuation word without a
        // curve, more levels than a tree has.
        (cell.to_string(), ((5_u64 << 48) + 1).to_string()),
        (format!("{curve},"), String::new()),
        (cell.to_string(), format!("{cell}, {cell}, {cell}")),
    ] {
        let broken = json.replacen(&from, &to, 1);
        assert_ne!(broken, json, "{from}");
        assert!(
            from_manifest_json(broken.as_bytes()).is_err(),
            "{from} -> {to}"
        );
    }
}

/// A version-20 file (patterns of one level, image places) opens
/// unchanged: version 21 adds fields, and changes the meaning of none.
#[test]
fn a_version_20_file_opens_unchanged_and_is_saved_as_the_current_version() {
    use craft3d_doc::{InstanceIndex, SketchPatternLayout};
    let mut doc = small();
    let mut sketch = Sketch::new(SketchPlane::default());
    let a = sketch.add_point(0.0, 0.0).unwrap();
    let b = sketch.add_point(1.0, 0.0).unwrap();
    let run = sketch.add_line(a, b, false).unwrap();
    sketch
        .add_pattern(
            vec![run],
            Expr::new("4"),
            SketchPatternLayout::Chain { from: a, to: b },
        )
        .unwrap();
    sketch.image_point(b, InstanceIndex::Last).unwrap();
    doc.add_feature(Some("Run"), FeatureKind::Sketch(sketch))
        .unwrap();
    let mut on = Sketch::new(SketchPlane::Face {
        face: "F3:side(e3[2])".parse().unwrap(),
        offset: Expr::new("0"),
    });
    on.add_point(0.0, 0.0).unwrap();
    doc.add_feature(Some("Mark"), FeatureKind::Sketch(on))
        .unwrap();
    let json = to_manifest_json(&doc).unwrap();
    let old = json.replacen(&version_field(), "\"format_version\": 20", 1);
    assert_ne!(old, json);
    let opened = from_manifest_json(old.as_bytes()).unwrap();
    assert_eq!(opened, doc);
    assert_eq!(to_manifest_json(&opened).unwrap(), json);
}
