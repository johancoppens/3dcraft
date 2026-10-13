//! `file.export` to STEP through the command layer: what is written, with
//! which units, tolerance and names, and what is refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_step::{Entity, Param, StepFile};
use serde_json::{Value, json};
use std::path::PathBuf;

fn session(tag: &str) -> (Session, PathBuf) {
    let dir = std::env::temp_dir().join(format!("craft3d-step-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    (session, dir)
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn read(dir: &std::path::Path, file: &str) -> StepFile {
    let bytes = std::fs::read(dir.join(file)).unwrap();
    assert!(bytes.is_ascii(), "{file} is plain ASCII");
    let step = craft3d_step::read(&bytes).unwrap_or_else(|e| panic!("{file}: {e}"));
    assert_eq!(step.reference_cycle(), None);
    // What was read writes back to the same bytes.
    assert_eq!(craft3d_step::write(&step).unwrap().into_bytes(), bytes);
    step
}

/// The parameters of the one instance with this keyword.
fn only<'a>(step: &'a StepFile, keyword: &str) -> &'a [Param] {
    let mut found = step.data.iter().filter_map(|i| i.entity.record(keyword));
    let record = found.next().unwrap_or_else(|| panic!("no {keyword}"));
    assert!(found.next().is_none(), "more than one {keyword}");
    &record.params
}

fn primitives(s: &mut Session) {
    for (id, params) in [
        (
            "feature.block",
            json!({"size": [40, 20, 10], "name": "Plate"}),
        ),
        (
            "feature.cylinder",
            json!({"radius": 5, "height": 30, "position": [60, 0, 0], "name": "Pin"}),
        ),
        (
            "feature.sphere",
            json!({"radius": 8, "position": [0, 60, 0], "name": "Ball"}),
        ),
        (
            "feature.cone",
            json!({"radius": 10, "height": 10, "position": [60, 60, 0], "name": "Cône \u{2460}"}),
        ),
        (
            "feature.torus",
            json!({"major": 12, "minor": 3, "position": [0, 0, 60], "name": "it's a ring"}),
        ),
    ] {
        run(s, id, params);
    }
}

#[test]
fn every_body_is_exported_with_units_tolerance_and_names() {
    let (mut s, dir) = session("all");
    run(&mut s, "file.new", json!({"units": "mm"}));
    primitives(&mut s);
    let result = run(&mut s, "file.export", json!({"path": "all.step"}));
    assert_eq!(result["format"], "step");
    assert_eq!(result["bodies"], 5);
    assert_eq!(result["solids"], 5);
    assert_eq!(result["cavities"], 0);
    assert_eq!(result["faces"], 6 + 3 + 1 + 2 + 1);
    assert_eq!(result["edges"], 12 + 2 + 1);
    // The sphere, the torus and the cone's apex each add a vertex.
    assert_eq!(result["vertices"], 8 + 2 + 1 + 3);
    assert_eq!(result["unit"], "mm");
    assert_eq!(result["uncertainty"], 1e-6);
    assert_eq!(
        result["bytes"].as_u64().unwrap(),
        dir.join("all.step").metadata().unwrap().len()
    );

    let step = read(&dir, "all.step");
    assert_eq!(
        result["entities"].as_u64().unwrap() as usize,
        step.data.len()
    );
    let header = step.header_values().unwrap();
    assert_eq!(header.name, "all.step");
    assert_eq!(
        header.schemas,
        ["AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }"]
    );
    assert_eq!(header.originating_system, "3DCraft");
    assert_eq!(header.time_stamp, "", "reproducible: no time in the file");
    for (keyword, count) in [
        ("PRODUCT", 5),
        ("SHAPE_DEFINITION_REPRESENTATION", 5),
        ("ADVANCED_BREP_SHAPE_REPRESENTATION", 5),
        ("MANIFOLD_SOLID_BREP", 5),
        ("CLOSED_SHELL", 5),
        ("ADVANCED_FACE", 13),
        ("EDGE_CURVE", 15),
        ("VERTEX_LOOP", 3),
        ("PLANE", 6 + 2 + 1),
        ("CYLINDRICAL_SURFACE", 1),
        ("SPHERICAL_SURFACE", 1),
        ("CONICAL_SURFACE", 1),
        ("TOROIDAL_SURFACE", 1),
        ("UNCERTAINTY_MEASURE_WITH_UNIT", 1),
        ("GEOMETRIC_REPRESENTATION_CONTEXT", 1),
        ("APPLICATION_PROTOCOL_DEFINITION", 1),
    ] {
        assert_eq!(step.count(keyword), count, "{keyword}");
    }
    let names: Vec<&str> = step
        .data
        .iter()
        .filter_map(|i| i.entity.record("PRODUCT"))
        .map(|r| r.params[1].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["Plate", "Pin", "Ball", "Cône \u{2460}", "it's a ring"]
    );
    // Millimetres, the tolerance as uncertainty, and degrees: the
    // document's angle unit. A cone as wide as it is high is 45°.
    assert_eq!(
        only(&step, "UNCERTAINTY_MEASURE_WITH_UNIT")[0],
        Param::typed("LENGTH_MEASURE", Param::Real(1e-6))
    );
    let si: Vec<&[Param]> = step
        .data
        .iter()
        .filter_map(|i| i.entity.record("SI_UNIT"))
        .map(|r| r.params.as_slice())
        .collect();
    assert!(si.contains(&&[Param::enumeration("MILLI"), Param::enumeration("METRE")][..]));
    let degree = step
        .data
        .iter()
        .find(|i| i.entity.is("PLANE_ANGLE_UNIT") && i.entity.is("CONVERSION_BASED_UNIT"))
        .expect("the plane angle unit is the degree");
    assert_eq!(
        degree
            .entity
            .record("CONVERSION_BASED_UNIT")
            .unwrap()
            .params[0],
        Param::string("DEGREE")
    );
    let cone = only(&step, "CONICAL_SURFACE");
    assert_eq!(cone[2], Param::Real(10.0));
    assert!(
        (cone[3].as_real().unwrap() - 45.0).abs() < 1e-12,
        "{cone:?}"
    );
    let torus = only(&step, "TOROIDAL_SURFACE");
    assert_eq!(
        (&torus[2], &torus[3]),
        (&Param::Real(12.0), &Param::Real(3.0))
    );

    // The same document gives the same bytes; so does `format` with
    // another extension, apart from the file's name in the header.
    let first = std::fs::read(dir.join("all.step")).unwrap();
    run(&mut s, "file.export", json!({"path": "all.step"}));
    assert_eq!(std::fs::read(dir.join("all.step")).unwrap(), first);
    for (path, format) in [
        ("ALL.STP", None),
        ("all.dat", Some("step")),
        ("b.txt", Some("stp")),
    ] {
        let mut params = json!({"path": path});
        if let Some(format) = format {
            params["format"] = format.into();
        }
        let again = run(&mut s, "file.export", params);
        assert_eq!(again["format"], "step", "{path}");
        let text = String::from_utf8(std::fs::read(dir.join(path)).unwrap()).unwrap();
        assert_eq!(
            text.replace(path, "all.step"),
            String::from_utf8(first.clone()).unwrap(),
            "{path}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_document_in_inches_says_inch_and_keeps_its_numbers() {
    let (mut s, dir) = session("inch");
    run(&mut s, "file.new", json!({"units": "in"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": ["40 mm", "20 mm", "10 mm"], "name": "Plate"}),
    );
    let result = run(&mut s, "file.export", json!({"path": "plate.stp"}));
    assert_eq!(result["unit"], "in");
    let step = read(&dir, "plate.stp");
    let inch = step
        .data
        .iter()
        .find(|i| i.entity.is("LENGTH_UNIT") && i.entity.is("CONVERSION_BASED_UNIT"))
        .expect("the length unit is a conversion-based one");
    let unit = inch.entity.record("CONVERSION_BASED_UNIT").unwrap();
    assert_eq!(unit.params[0], Param::string("INCH"));
    // One inch is 25.4 of the millimetre it refers to.
    let measure = &step.index()[&unit.params[1].as_ref().unwrap()].entity;
    let Entity::Simple(measure) = measure else {
        panic!()
    };
    assert_eq!(measure.keyword, "LENGTH_MEASURE_WITH_UNIT");
    assert_eq!(
        measure.params[0],
        Param::typed("LENGTH_MEASURE", Param::Real(25.4))
    );
    let millimetre = &step.index()[&measure.params[1].as_ref().unwrap()].entity;
    assert_eq!(
        millimetre.record("SI_UNIT").unwrap().params,
        [Param::enumeration("MILLI"), Param::enumeration("METRE")]
    );
    // Coordinates are in inches: the far corner is at 40 mm = 1.5748 in.
    let far = step
        .data
        .iter()
        .filter_map(|i| i.entity.record("CARTESIAN_POINT"))
        .filter_map(|r| r.params[1].as_list())
        .map(|c| c.iter().map(|x| x.as_real().unwrap()).collect::<Vec<_>>())
        .fold([0.0_f64; 3], |m, c| {
            [m[0].max(c[0]), m[1].max(c[1]), m[2].max(c[2])]
        });
    for (value, mm) in far.iter().zip([40.0, 20.0, 10.0]) {
        assert!((value * 25.4 - mm).abs() < 1e-12, "{far:?}");
    }
    // The tolerance of an inch document is in inches too.
    let tolerance = run(&mut s, "document.inspect", json!({}))["tolerance"]["linear"]
        .as_f64()
        .unwrap();
    assert_eq!(result["uncertainty"], tolerance);
    assert_eq!(
        only(&step, "UNCERTAINTY_MEASURE_WITH_UNIT")[0],
        Param::typed("LENGTH_MEASURE", Param::Real(tolerance))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn what_cannot_be_exported_fails_with_the_reason() {
    let (mut s, dir) = session("refused");
    let refused = |s: &mut Session, params: Value, words: &[&str]| {
        let error = s
            .execute("file.export", &params)
            .expect_err(&params.to_string())
            .to_string();
        for word in words {
            assert!(error.contains(word), "{params}: {error}");
        }
    };
    // No document; then a document without bodies.
    refused(&mut s, json!({"path": "a.step"}), &["no document"]);
    run(&mut s, "file.new", json!({}));
    refused(&mut s, json!({"path": "a.step"}), &["there are no bodies"]);
    run(&mut s, "feature.block", json!({"size": [1, 2, 3]}));
    refused(
        &mut s,
        json!({"path": "a.step", "tolerance": 0.1}),
        &["`tolerance` is for mesh formats"],
    );
    refused(&mut s, json!({"path": "../a.step"}), &["../a.step"]);
    refused(&mut s, json!({"path": "/etc/a.step"}), &["/etc/a.step"]);
    refused(&mut s, json!({"path": "a.step", "format": 3}), &["format"]);
    refused(&mut s, json!({"path": 7}), &["path"]);
    refused(
        &mut s,
        json!({"path": "a.step", "schema": "ap242"}),
        &["schema"],
    );
    refused(
        &mut s,
        json!({"path": "a.dwg"}),
        &["cannot tell the format", ".step"],
    );
    refused(
        &mut s,
        json!({"path": "a.step", "format": "iges"}),
        &["unknown format `iges`", "step"],
    );
    assert!(!dir.join("a.step").exists(), "nothing was written");
    // A failed feature has no body: the others are exported, and a
    // document whose only feature failed has nothing to export.
    run(
        &mut s,
        "feature.block",
        json!({"size": [1, "nonsense", 3], "name": "Bad"}),
    );
    let result = run(&mut s, "file.export", json!({"path": "a.step"}));
    assert_eq!(result["bodies"], 1);
    std::fs::remove_dir_all(&dir).unwrap();
}
