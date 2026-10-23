//! Public parameters, document info and the thumbnail in the file
//! (format 32): they round-trip, they are absent from the manifest of a
//! document without them, and the thumbnail is an entry of its own that
//! loading ignores.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{DocInfo, Document, Expr, ParamKind, PublicParam, Units};
use craft3d_format::{
    MANIFEST_NAME, MAX_THUMBNAIL_BYTES, THUMBNAIL_NAME, from_bytes, from_manifest_json, thumbnail,
    to_bytes, to_bytes_with_thumbnail, to_manifest_json,
};

fn jig() -> Document {
    let mut doc = Document::new(Units::default());
    doc.set_variable("teeth", "20").unwrap();
    doc.set_variable("bore", "5 mm").unwrap();
    doc.set_variable("profile", "1").unwrap();
    doc.publish_variable(
        "teeth",
        PublicParam {
            label: "Teeth".into(),
            description: "Number of teeth".into(),
            group: "Belt".into(),
            order: 2,
            kind: ParamKind::Integer,
            min: Some(Expr::new("10")),
            max: Some(Expr::new("60")),
            step: Some(Expr::new("1")),
            default: Expr::new("20"),
            ..PublicParam::default()
        },
    )
    .unwrap();
    doc.publish_variable(
        "bore",
        PublicParam {
            max: Some(Expr::new("teeth * 0.4 mm")),
            unit: Some("in".into()),
            default: Expr::new("5 mm"),
            ..PublicParam::default()
        },
    )
    .unwrap();
    doc.publish_variable(
        "profile",
        PublicParam {
            kind: ParamKind::Choice,
            choices: vec!["GT2".into(), "HTD 3M".into()],
            default: Expr::new("1"),
            ..PublicParam::default()
        },
    )
    .unwrap();
    doc.set_info(DocInfo {
        title: "Pulley".into(),
        description: "A toothed pulley".into(),
        author: "Someone".into(),
        licence: "CC-BY-4.0".into(),
        url: "https://example.org/pulley".into(),
        tags: vec!["gt2".into(), "pulley".into()],
    })
    .unwrap();
    doc
}

#[test]
fn parameters_and_info_round_trip() {
    let doc = jig();
    let bytes = to_bytes(&doc).unwrap();
    assert_eq!(from_bytes(&bytes).unwrap(), doc);
    let json = to_manifest_json(&doc).unwrap();
    // As the file writes them: empty and default fields left out.
    assert!(json.contains("\"kind\": \"choice\""), "{json}");
    assert!(json.contains("\"choices\": ["), "{json}");
    assert!(json.contains("\"max\": \"teeth * 0.4 mm\""), "{json}");
    assert!(json.contains("\"licence\": \"CC-BY-4.0\""), "{json}");
    let manifest: serde_json::Value = serde_json::from_str(&json).unwrap();
    let bore = &manifest["document"]["variables"][1]["public"];
    assert_eq!(
        bore,
        &serde_json::json!({"kind": "number", "max": "teeth * 0.4 mm", "unit": "in", "default": "5 mm"})
    );
}

#[test]
fn a_document_without_them_writes_neither() {
    let mut doc = Document::new(Units::default());
    doc.set_variable("w", "4 mm").unwrap();
    let json = to_manifest_json(&doc).unwrap();
    assert!(!json.contains("public"), "{json}");
    assert!(!json.contains("info"), "{json}");
    // An empty info reads as none.
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["document"]["info"] = serde_json::json!({});
    let read = from_manifest_json(value.to_string().as_bytes()).unwrap();
    assert!(read.info.is_empty());
}

#[test]
fn what_the_file_breaks_is_refused_with_where() {
    let json = to_manifest_json(&jig()).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["document"]["variables"][0]["public"]["colour"] = "red".into();
    let e = from_manifest_json(value.to_string().as_bytes())
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("document.variables[0]") && e.contains("colour"),
        "{e}"
    );
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["document"]["variables"][2]["public"]["choices"] = serde_json::json!([]);
    let e = from_manifest_json(value.to_string().as_bytes())
        .unwrap_err()
        .to_string();
    assert!(e.contains("at least one label"), "{e}");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["document"]["variables"][1]["public"]["unit"] = "parsec".into();
    let e = from_manifest_json(value.to_string().as_bytes())
        .unwrap_err()
        .to_string();
    assert!(e.contains("is not a unit"), "{e}");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["document"]["info"]["tags"] = serde_json::json!(["a\u{7}"]);
    assert!(from_manifest_json(value.to_string().as_bytes()).is_err());
}

#[test]
fn the_thumbnail_is_an_entry_that_loading_ignores() {
    let doc = jig();
    let png = b"\x89PNG\r\n\x1a\nnot really a picture".to_vec();
    let bytes = to_bytes_with_thumbnail(&doc, Some(&png)).unwrap();
    let names: Vec<&str> = craft3d_zipstore::read_stored(&bytes)
        .unwrap()
        .iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, [MANIFEST_NAME, THUMBNAIL_NAME]);
    assert_eq!(thumbnail(&bytes).unwrap(), Some(png.clone()));
    assert_eq!(from_bytes(&bytes).unwrap(), doc);
    // Deterministic, and without one there is none.
    assert_eq!(to_bytes_with_thumbnail(&doc, Some(&png)).unwrap(), bytes);
    assert_eq!(thumbnail(&to_bytes(&doc).unwrap()).unwrap(), None);
    // Too large to write; not a .3dcraft file to read.
    let huge = vec![0_u8; MAX_THUMBNAIL_BYTES + 1];
    assert!(to_bytes_with_thumbnail(&doc, Some(&huge)).is_err());
    assert!(thumbnail(b"ISO-10303-21;").is_err());
}
