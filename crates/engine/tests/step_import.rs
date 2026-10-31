//! STEP import through the command layer (docs/design/step-import.md):
//! `file.import` adds one import feature per solid of the file, which
//! regenerate from the bytes the document holds; a saved and reopened
//! document is the same; undo and redo; names that a second import of the
//! same file keeps; refusals.
//!
//! Two opt-in tests run in release with `cargo xtask freecad` (the
//! Strix's gate):
//! - `every_body_of_the_showcase_reads_back` exports every body of every
//!   showcase model and reads it back, in three units;
//! - `freecad_files_read_with_freecads_volume` lets FreeCAD (an
//!   independent kernel, `CRAFT3D_FREECAD`) make solids of its own
//!   (primitives, booleans, fillets, chamfers, revolutions, lofts), write
//!   them as STEP and measure them; each must read here with FreeCAD's
//!   volume and area, or be refused with a reason, never read wrong.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::plain::run;
use common::{error, session_in, workdir};
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A plate with a hole and a ball beside it, exported to `out.step` in
/// `dir`.
fn exported(dir: &Path) -> (Vec<Value>, PathBuf) {
    let mut s = session_in(dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 30, 10], "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 6, "height": 20, "position": [20, 15, -5], "name": "Pin"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"target": "Plate", "tool": "Pin", "op": "subtract"}),
    );
    run(
        &mut s,
        "feature.sphere",
        json!({"radius": 8, "position": [70, 0, 0], "name": "Ball"}),
    );
    run(
        &mut s,
        "feature.torus",
        json!({"major": 10, "minor": 3, "position": [0, 70, 0], "name": "Ring"}),
    );
    run(&mut s, "file.export", json!({"path": "out.step"}));
    let bodies = bodies(&mut s);
    (bodies, dir.join("out.step"))
}

fn bodies(s: &mut Session) -> Vec<Value> {
    let inspected = run(s, "document.inspect", json!({}));
    inspected["bodies"].as_array().unwrap().clone()
}

fn close(a: &Value, b: &Value, what: &str) {
    let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
    assert!(
        (a - b).abs() <= 1e-7 * a.abs().max(b.abs()),
        "{what}: {a} and {b}"
    );
}

fn same_bodies(ours: &[Value], theirs: &[Value], scale: f64) {
    assert_eq!(ours.len(), theirs.len());
    for (a, b) in ours.iter().zip(theirs) {
        for key in ["faces", "edges", "vertices", "shells", "kind"] {
            assert_eq!(a[key], b[key], "{key} of {} and {}", a["name"], b["name"]);
        }
        let volume = a["volume"].as_f64().unwrap() * scale.ppowi(3);
        let area = a["area"].as_f64().unwrap() * scale.ppowi(2);
        close(&json!(volume), &b["volume"], "volume");
        close(&json!(area), &b["area"], "area");
    }
}

#[test]
fn an_imported_file_regenerates_saves_reopens_undoes_and_redoes() {
    let dir = workdir("step-import", "command");
    let (ours, _) = exported(&dir);
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let result = run(&mut s, "file.import", json!({"path": "out.step"}));
    let made = result["bodies"].as_array().unwrap();
    assert_eq!(made.len(), 3, "{result}");
    for body in made {
        assert_eq!(body["status"], "ok", "{body}");
        assert_eq!(body["import"]["fileUnit"], "MILLI METRE");
        assert_eq!(body["import"]["mergedVertices"], 0);
    }
    assert_eq!(made[0]["import"]["stepName"], "Boolean1");
    assert_eq!(made[0]["name"], "Boolean1");
    assert_eq!(made[1]["name"], "Ball");
    let theirs = bodies(&mut s);
    same_bodies(&ours, &theirs, 1.0);
    // Named by their order in the file.
    let names: Vec<String> = theirs[0]["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    assert_eq!(names[0], "F1:face(0)");

    // Saved and reopened: the same bodies, from the bytes in the file;
    // the STEP file itself is gone by then.
    run(&mut s, "file.save", json!({"path": "imported.3dcraft"}));
    std::fs::remove_file(dir.join("out.step")).unwrap();
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "imported.3dcraft"}));
    assert_eq!(bodies(&mut t), theirs);
    // Saving again writes the same bytes.
    run(&mut t, "file.save", json!({"path": "again.3dcraft"}));
    assert_eq!(
        std::fs::read(dir.join("imported.3dcraft")).unwrap(),
        std::fs::read(dir.join("again.3dcraft")).unwrap()
    );

    // One undo step for the whole import.
    run(&mut s, "edit.undo", json!({}));
    assert!(bodies(&mut s).is_empty());
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(bodies(&mut s), theirs);

    // The document's own unit: the same file read into inches.
    let (_, path) = exported(&dir);
    let mut u = session_in(&dir);
    run(&mut u, "file.new", json!({"units": "in"}));
    run(
        &mut u,
        "file.import",
        json!({"path": path.file_name().unwrap().to_str().unwrap()}),
    );
    same_bodies(&ours, &bodies(&mut u), 1.0 / 25.4);
}

#[test]
fn a_second_import_of_the_same_file_names_its_faces_the_same_way() {
    let dir = workdir("step-import", "names");
    exported(&dir);
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "file.import",
        json!({"path": "out.step", "names": ["A", "B", "C"]}),
    );
    run(&mut s, "file.import", json!({"path": "out.step"}));
    let all = bodies(&mut s);
    assert_eq!(all.len(), 6);
    assert_eq!(all[0]["name"], "A");
    let local = |body: &Value| -> Vec<String> {
        body["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| {
                let n = n.as_str().unwrap();
                n.split_once(':').unwrap().1.to_string()
            })
            .collect()
    };
    for k in 0..3 {
        assert_eq!(local(&all[k]), local(&all[k + 3]));
    }
    // A later feature refers to an imported face by its name.
    let inspected = run(&mut s, "document.inspect", json!({}));
    let id = inspected["features"][0]["id"].as_str().unwrap().to_string();
    let result = run(
        &mut s,
        "feature.fillet",
        json!({"edges": [[format!("{id}:face(0)"), format!("{id}:face(2)")]], "radius": 1}),
    );
    assert!(
        result["status"] == "ok" || result["status"] == "failed",
        "{result}"
    );
}

#[test]
fn what_cannot_be_imported_is_refused_and_changes_nothing() {
    let dir = workdir("step-import", "refused");
    exported(&dir);
    std::fs::write(dir.join("empty.step"), b"").unwrap();
    std::fs::write(
        dir.join("nothing.step"),
        b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','',(''),(''),'','','');\nFILE_SCHEMA(('X'));\nENDSEC;\nDATA;\n#1=CARTESIAN_POINT('',(0.,0.,0.));\nENDSEC;\nEND-ISO-10303-21;\n",
    )
    .unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (params, says) in [
        (json!({}), "path"),
        (json!({"path": 3}), "path"),
        (json!({"path": "missing.step"}), "missing.step"),
        (json!({"path": "empty.step"}), "not a readable STEP file"),
        (json!({"path": "nothing.step"}), "no solids or sheets"),
        (json!({"path": "out.step", "names": ["one"]}), "names"),
        (json!({"path": "out.step", "names": "one"}), "names"),
        (json!({"path": "out.step", "names": ["a", "a", "b"]}), "a"),
        (json!({"path": "out.step", "extra": 1}), "extra"),
        (json!({"path": "../out.step"}), ""),
    ] {
        let e = error(&mut s, "file.import", params.clone());
        assert!(e.contains(says), "{params}: {e}");
        assert!(bodies(&mut s).is_empty(), "{params} changed the document");
    }
    // A vertex moved off its edges by ten times the cap on an entity's
    // tolerance is refused with the gap (crates/io/tests/step_import.rs
    // has the file-level variants, and a gap within the cap read).
    let text = std::fs::read_to_string(dir.join("out.step")).unwrap();
    let moved = text.replacen("CARTESIAN_POINT('',(40.,", "CARTESIAN_POINT('',(40.01,", 1);
    assert_ne!(moved, text);
    std::fs::write(dir.join("moved.step"), moved).unwrap();
    let e = error(&mut s, "file.import", json!({"path": "moved.step"}));
    assert!(e.contains("refused rather than moved"), "{e}");
    assert!(bodies(&mut s).is_empty());
}

/// A block whose file puts a corner 1e-4 mm off its edges (a hundred
/// tolerances, within the cap) is imported loose, and a fillet, a chamfer,
/// a shell and an offset on it carry its tolerances: each is valid with
/// them, and `document.check` lists them (tolerant modelling, slice 2).
#[test]
fn operations_on_a_loose_import_carry_its_tolerances() {
    let dir = workdir("step-import", "loose-operations");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "feature.block", json!({"size": [10, 20, 30]}));
    run(&mut s, "file.export", json!({"path": "block.step"}));
    let text = std::fs::read_to_string(dir.join("block.step")).unwrap();
    let moved = text.replacen(
        "CARTESIAN_POINT('',(10.,",
        "CARTESIAN_POINT('',(10.0001,",
        1,
    );
    assert_ne!(moved, text);
    std::fs::write(dir.join("loose.step"), moved).unwrap();
    for (command, params) in [
        (
            "feature.fillet",
            json!({"edges": [["F1:face(1)", "F1:face(3)"]], "radius": 2}),
        ),
        (
            "feature.chamfer",
            json!({"edges": [["F1:face(1)", "F1:face(4)"]], "distance": 1}),
        ),
        (
            "feature.shell",
            json!({"source": "F1", "thickness": 1, "open": ["F1:face(1)"]}),
        ),
        (
            "feature.offsetFace",
            json!({"faces": ["F1:face(4)"], "distance": 1}),
        ),
    ] {
        let mut s = session_in(&dir);
        run(&mut s, "file.new", json!({"units": "mm"}));
        let imported = run(&mut s, "file.import", json!({"path": "loose.step"}));
        assert!(
            imported["bodies"][0]["import"]["looseVertices"]
                .as_u64()
                .unwrap()
                >= 1,
            "{imported}"
        );
        run(&mut s, command, params);
        let check = run(&mut s, "document.check", json!({}));
        assert_eq!(check["ok"], true, "{command}: {check}");
        let loose = check["loose"].as_array().unwrap();
        assert!(
            loose.iter().any(|l| l["feature"] == "F2"),
            "{command}: {check}"
        );
        for l in loose {
            assert!(l["largest"].as_f64().unwrap() <= 1e-3, "{command}: {check}");
        }
    }
}

/// The deep check of a loose import meshes it and holds its boundary
/// points to the tolerances of the entities they lie on (tolerant
/// modelling, slice 3): the corner 1e-4 mm off its edges passes, and the
/// body is counted under looseMeshesChecked.
#[test]
fn the_deep_check_meshes_a_loose_import_on_its_tolerances() {
    let dir = workdir("step-import", "loose-mesh");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "feature.block", json!({"size": [10, 20, 30]}));
    run(&mut s, "file.export", json!({"path": "block.step"}));
    let text = std::fs::read_to_string(dir.join("block.step")).unwrap();
    let moved = text.replacen(
        "CARTESIAN_POINT('',(10.,",
        "CARTESIAN_POINT('',(10.0001,",
        1,
    );
    std::fs::write(dir.join("loose.step"), moved).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "file.import", json!({"path": "loose.step"}));
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    assert_eq!(check["looseMeshesChecked"], 1, "{check}");
}

#[test]
fn import_needs_a_document_and_files() {
    let mut s = Session::new(Workspace {
        read: craft3d_engine::Access::None,
        write: craft3d_engine::Access::None,
    });
    let e = error(&mut s, "file.import", json!({"path": "a.step"}));
    assert!(!e.is_empty());
    run(&mut s, "file.new", json!({"units": "mm"}));
    let e = error(&mut s, "file.import", json!({"path": "a.step"}));
    assert!(!e.is_empty());
}

#[test]
fn an_import_that_would_make_a_document_too_large_to_open_is_refused() {
    // Three files of 90 MB (a small solid and a long comment, each
    // different) would make a .3dcraft file over the 256 MiB that are
    // read: the third is refused, and the document still saves.
    let dir = workdir("step-import", "large");
    let (_, path) = exported(&dir);
    let text = std::fs::read_to_string(&path).unwrap();
    let at = text.find("DATA;").unwrap();
    for k in 0..3 {
        let comment = format!(
            "/* {} */\n",
            char::from(b'a' + k).to_string().repeat(90 << 20)
        );
        let padded = format!("{}{comment}{}", &text[..at], &text[at..]);
        std::fs::write(dir.join(format!("big{k}.step")), padded).unwrap();
    }
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "file.import", json!({"path": "big0.step"}));
    run(&mut s, "file.import", json!({"path": "big1.step"}));
    let e = error(&mut s, "file.import", json!({"path": "big2.step"}));
    assert!(e.contains("opened again"), "{e}");
    assert_eq!(bodies(&mut s).len(), 6);
    run(&mut s, "file.save", json!({"path": "big.3dcraft"}));
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "big.3dcraft"}));
    assert_eq!(bodies(&mut t).len(), 6);
    for k in 0..3 {
        let _ = std::fs::remove_file(dir.join(format!("big{k}.step")));
    }
    let _ = std::fs::remove_file(dir.join("big.3dcraft"));
}

// ------------------------------------------- assemblies and colours

/// The part file `part.step` in `dir` (a coloured block), wrapped in an
/// assembly that places it at `places` (offsets along x), written as
/// `assembly.step`: the structure other writers use (a product per part
/// and assembly, `NEXT_ASSEMBLY_USAGE_OCCURRENCE`,
/// `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION`, a representation relationship
/// with an `ITEM_DEFINED_TRANSFORMATION`).
fn assembly_file(dir: &Path, places: &[f64]) {
    use craft3d_step::{Entity, Instance, Param, Record};
    let mut s = session_in(dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [4, 2, 1], "name": "Peg"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Peg", "color": "#c86432"}),
    );
    run(&mut s, "file.export", json!({"path": "part.step"}));
    let mut file = craft3d_step::read(&std::fs::read(dir.join("part.step")).unwrap()).unwrap();
    let mut next = file.data.iter().map(|i| i.id).max().unwrap() + 1;
    let find = |file: &craft3d_step::StepFile, keyword: &str| {
        file.data.iter().find(|i| i.entity.is(keyword)).unwrap().id
    };
    let mut add = |file: &mut craft3d_step::StepFile, records: Vec<Record>| {
        let id = next;
        next += 1;
        let entity = if records.len() == 1 {
            Entity::Simple(records.into_iter().next().unwrap())
        } else {
            Entity::Complex(records)
        };
        file.data.push(Instance { id, entity });
        id
    };
    let r = |k: &str, p: Vec<Param>| Record::new(k, p);
    let text = Param::string;
    let context = find(&file, "GLOBAL_UNIT_ASSIGNED_CONTEXT");
    let part = find(&file, "PRODUCT_DEFINITION");
    let brep = find(&file, "ADVANCED_BREP_SHAPE_REPRESENTATION");
    let pc = find(&file, "PRODUCT_CONTEXT");
    let dc = find(&file, "PRODUCT_DEFINITION_CONTEXT");
    let mut frame = |file: &mut craft3d_step::StepFile, x: f64| {
        let o = add(
            file,
            vec![r(
                "CARTESIAN_POINT",
                vec![text(""), Param::reals([x, 0.0, 0.0])],
            )],
        );
        let z = add(
            file,
            vec![r(
                "DIRECTION",
                vec![text(""), Param::reals([0.0, 0.0, 1.0])],
            )],
        );
        let d = add(
            file,
            vec![r(
                "DIRECTION",
                vec![text(""), Param::reals([1.0, 0.0, 0.0])],
            )],
        );
        add(
            file,
            vec![r(
                "AXIS2_PLACEMENT_3D",
                vec![text(""), Param::Ref(o), Param::Ref(z), Param::Ref(d)],
            )],
        )
    };
    let part_origin = frame(&mut file, 0.0);
    let root_origin = frame(&mut file, 0.0);
    let placed: Vec<u64> = places.iter().map(|&x| frame(&mut file, x)).collect();
    let mut add = |file: &mut craft3d_step::StepFile, records: Vec<Record>| {
        let id = next;
        next += 1;
        let entity = if records.len() == 1 {
            Entity::Simple(records.into_iter().next().unwrap())
        } else {
            Entity::Complex(records)
        };
        file.data.push(Instance { id, entity });
        id
    };
    let part_rep = add(
        &mut file,
        vec![r(
            "SHAPE_REPRESENTATION",
            vec![text("Peg"), Param::refs([part_origin]), Param::Ref(context)],
        )],
    );
    add(
        &mut file,
        vec![r(
            "SHAPE_REPRESENTATION_RELATIONSHIP",
            vec![text(""), text(""), Param::Ref(part_rep), Param::Ref(brep)],
        )],
    );
    let product = add(
        &mut file,
        vec![r(
            "PRODUCT",
            vec![text("Rack"), text("Rack"), text(""), Param::refs([pc])],
        )],
    );
    let formation = add(
        &mut file,
        vec![r(
            "PRODUCT_DEFINITION_FORMATION",
            vec![text(""), text(""), Param::Ref(product)],
        )],
    );
    let root = add(
        &mut file,
        vec![r(
            "PRODUCT_DEFINITION",
            vec![
                text("design"),
                text(""),
                Param::Ref(formation),
                Param::Ref(dc),
            ],
        )],
    );
    let shape = add(
        &mut file,
        vec![r(
            "PRODUCT_DEFINITION_SHAPE",
            vec![text(""), text(""), Param::Ref(root)],
        )],
    );
    let mut items = vec![root_origin];
    items.extend(&placed);
    let root_rep = add(
        &mut file,
        vec![r(
            "SHAPE_REPRESENTATION",
            vec![text("Rack"), Param::refs(items), Param::Ref(context)],
        )],
    );
    add(
        &mut file,
        vec![r(
            "SHAPE_DEFINITION_REPRESENTATION",
            vec![Param::Ref(shape), Param::Ref(root_rep)],
        )],
    );
    for (k, at) in placed.iter().enumerate() {
        let idt = add(
            &mut file,
            vec![r(
                "ITEM_DEFINED_TRANSFORMATION",
                vec![text(""), text(""), Param::Ref(*at), Param::Ref(part_origin)],
            )],
        );
        let rrwt = add(
            &mut file,
            vec![
                r(
                    "REPRESENTATION_RELATIONSHIP",
                    vec![
                        text(""),
                        text(""),
                        Param::Ref(part_rep),
                        Param::Ref(root_rep),
                    ],
                ),
                r(
                    "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION",
                    vec![Param::Ref(idt)],
                ),
                r("SHAPE_REPRESENTATION_RELATIONSHIP", vec![]),
            ],
        );
        let tag = format!("{}", k + 1);
        let nauo = add(
            &mut file,
            vec![r(
                "NEXT_ASSEMBLY_USAGE_OCCURRENCE",
                vec![
                    text(&tag),
                    text(&tag),
                    text(""),
                    Param::Ref(root),
                    Param::Ref(part),
                    Param::Omitted,
                ],
            )],
        );
        let pds = add(
            &mut file,
            vec![r(
                "PRODUCT_DEFINITION_SHAPE",
                vec![text(""), text(""), Param::Ref(nauo)],
            )],
        );
        add(
            &mut file,
            vec![r(
                "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION",
                vec![Param::Ref(rrwt), Param::Ref(pds)],
            )],
        );
    }
    std::fs::write(
        dir.join("assembly.step"),
        craft3d_step::write(&file).unwrap(),
    )
    .unwrap();
}

/// An assembly placing one part three times: three import features in
/// one undo step, each body where its placement puts it, named by its
/// place, with the part's colour; the same file again gives the same
/// names; saved and reopened, the same bodies (one blob in the file).
#[test]
fn an_assembly_imports_one_feature_per_placement_with_its_colour() {
    let dir = workdir("step-import", "assembly");
    let places = [0.0, 10.0, 25.0];
    assembly_file(&dir, &places);
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let result = run(&mut s, "file.import", json!({"path": "assembly.step"}));
    let made = result["bodies"].as_array().unwrap();
    assert_eq!(made.len(), 3, "{result}");
    for (k, body) in made.iter().enumerate() {
        assert_eq!(body["status"], "ok", "{body}");
        assert_eq!(body["import"]["instance"], format!("Rack/Peg#{}", k + 1));
        assert_eq!(body["import"]["color"], "#c86432");
        assert_eq!(body["name"], format!("Peg#{}", k + 1));
    }
    let all = bodies(&mut s);
    assert_eq!(all.len(), 3);
    for (body, x) in all.iter().zip(places) {
        let centroid = body["centroid"].as_array().unwrap();
        let cx = centroid[0].as_f64().unwrap();
        assert!((cx - (x + 2.0)).abs() < 1e-9, "{body}");
        assert!(body.to_string().contains("#c86432"), "{body}");
    }
    // One undo step for the features and their colours.
    run(&mut s, "edit.undo", json!({}));
    assert!(bodies(&mut s).is_empty());
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(bodies(&mut s), all);
    // Saved and reopened without the file: the same.
    run(&mut s, "file.save", json!({"path": "rack.3dcraft"}));
    std::fs::remove_file(dir.join("assembly.step")).unwrap();
    let mut t = session_in(&dir);
    run(&mut t, "file.open", json!({"path": "rack.3dcraft"}));
    assert_eq!(bodies(&mut t), all);
}

// ------------------------------------------------- the opt-in oracles

fn showcase() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/showcase");
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "3dcraft"))
        .collect();
    found.sort();
    if let Ok(only) = std::env::var("CRAFT3D_IMPORT_MODELS") {
        found.retain(|p| {
            only.split(',')
                .any(|name| p.file_stem().is_some_and(|s| s == name))
        });
    }
    found
}

/// Oracle (a) on the showcase: every body of every model, exported as
/// the export writes it and read back into micrometres, millimetres and
/// metres, has the same counts and, scaled, the same volume and area.
/// The reading is `craft3d-stepread` itself (regeneration adds the check
/// that a body does not pass through itself, which `self_check.rs` holds
/// for the models already).
#[test]
#[ignore = "whole models, in release: cargo xtask freecad"]
fn every_body_of_the_showcase_reads_back() {
    use craft3d_io::{StepBody, StepOptions, export_step};
    let mut problems = Vec::new();
    let mut read = 0;
    let mut unproven = 0;
    for path in showcase() {
        let model = path.file_stem().unwrap().to_string_lossy().to_string();
        let doc = craft3d_format::load(&path).unwrap();
        let regen = craft3d_regen::regenerate(&doc);
        let unit = match doc.units.length {
            craft3d_doc::LengthUnit::Micrometre => craft3d_meshio::Unit::Micron,
            craft3d_doc::LengthUnit::Millimetre => craft3d_meshio::Unit::Millimeter,
            craft3d_doc::LengthUnit::Centimetre => craft3d_meshio::Unit::Centimeter,
            craft3d_doc::LengthUnit::Metre => craft3d_meshio::Unit::Meter,
            craft3d_doc::LengthUnit::Inch => craft3d_meshio::Unit::Inch,
            craft3d_doc::LengthUnit::Foot => craft3d_meshio::Unit::Foot,
        };
        let tolerance =
            craft3d_math::Tolerance::new(doc.tolerance.linear, doc.tolerance.angular).unwrap();
        let own = craft3d_regen::metres_per_unit(doc.units.length);
        for feature in &doc.features {
            let Some(body) = regen.feature(feature.id).and_then(|o| o.own_body()) else {
                continue;
            };
            let what = format!("{model}/{}", feature.name);
            let options = StepOptions {
                length_unit: unit,
                tolerance,
                ..StepOptions::default()
            };
            let bytes = match export_step(&[StepBody::new(&feature.name, body)], &options) {
                Ok((bytes, _)) => bytes,
                Err(e) => {
                    problems.push(format!("{what}: export: {e}"));
                    continue;
                }
            };
            let parsed = match craft3d_stepread::parse(&bytes) {
                Ok(parsed) => parsed,
                Err(e) => {
                    problems.push(format!("{what}: {e}"));
                    continue;
                }
            };
            let ours = craft3d_topo::mass_properties(body);
            for metres in [1e-6, 1e-3, 1.0] {
                let factor = own / metres;
                let tolerance = craft3d_math::Tolerance::new(
                    doc.tolerance.linear * factor,
                    doc.tolerance.angular,
                )
                .unwrap();
                let options = craft3d_stepread::Options {
                    metres_per_unit: metres,
                    tolerance,
                    loosest: 1000.0 * tolerance.linear,
                };
                let (mut faces, mut edges, mut vertices, mut volume, mut area) =
                    (0, 0, 0, 0.0, 0.0);
                let mut failed = false;
                for k in 0..parsed.items().len() {
                    match parsed.build(k, &options) {
                        Ok(done) => {
                            unproven += done.report.unproven_edges;
                            if done.report.loose_edges + done.report.loose_vertices > 0 {
                                problems.push(format!(
                                    "{what} in {metres} m: our own export read with loose entities: {:?}",
                                    done.report
                                ));
                            }
                            faces += done.body.faces.len();
                            edges += done.body.edges.len();
                            vertices += done.body.vertices.len();
                            if let Ok(m) = craft3d_topo::mass_properties(&done.body) {
                                volume += m.volume;
                                area += m.area;
                            }
                        }
                        Err(e) => {
                            problems.push(format!("{what} in {metres} m: {e}"));
                            failed = true;
                        }
                    }
                }
                if failed {
                    continue;
                }
                if (faces, edges, vertices)
                    != (body.faces.len(), body.edges.len(), body.vertices.len())
                {
                    problems.push(format!(
                        "{what}: counts {:?} became {:?}",
                        (body.faces.len(), body.edges.len(), body.vertices.len()),
                        (faces, edges, vertices)
                    ));
                }
                if let Ok(ours) = &ours {
                    let (v, a) = (ours.volume * factor.ppowi(3), ours.area * factor.ppowi(2));
                    if (v - volume).abs() > 1e-7 * v.abs().max(volume.abs())
                        || (a - area).abs() > 1e-7 * a.abs().max(area.abs())
                    {
                        problems.push(format!(
                            "{what} in {metres} m: volume {v} became {volume}, area {a} became {area}"
                        ));
                    }
                }
                read += 1;
            }
        }
    }
    println!(
        "{read} bodies read back; {} problems; {unproven} edges no proof was found for (sampled within the tolerance)",
        problems.len()
    );
    for p in &problems {
        println!("  {p}");
    }
    assert!(read > 0);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    // Every edge proven (slice 2: 35 were accepted on samples before).
    assert_eq!(unproven, 0, "edges accepted on samples alone");
}

/// The FreeCAD script: solids made by FreeCAD's own primitives,
/// booleans, fillets, chamfers, revolutions and lofts, each written as
/// STEP by FreeCAD and measured by it.
const FREECAD_SCRIPT: &str = r#"
import os, json, traceback
import FreeCAD, Part
from FreeCAD import Vector as V
out = os.environ["CRAFT3D_FC_DIR"]
cases = {}
def add(name, make):
    try:
        shape = make()
        path = os.path.join(out, name + ".step")
        shape.exportStep(path)
        cases[name] = {"volume": shape.Volume, "area": shape.Area,
                       "faces": len(shape.Faces), "edges": len(shape.Edges),
                       "vertices": len(shape.Vertexes), "solids": len(shape.Solids),
                       "valid": shape.isValid()}
    except Exception:
        cases[name] = {"error": traceback.format_exc()}
box = lambda: Part.makeBox(40, 30, 20)
pin = lambda: Part.makeCylinder(6, 40, V(20, 15, -10))
add("box", box)
add("cylinder", lambda: Part.makeCylinder(5, 12))
add("sphere", lambda: Part.makeSphere(10))
add("torus", lambda: Part.makeTorus(20, 5))
add("cone_frustum", lambda: Part.makeCone(10, 4, 15))
add("cone_apex", lambda: Part.makeCone(10, 0, 15))
add("box_minus_pin", lambda: box().cut(pin()))
add("box_plus_ball", lambda: box().fuse(Part.makeSphere(12, V(40, 30, 20))))
add("box_common_ball", lambda: box().common(Part.makeSphere(25, V(20, 15, 10))))
def blended(how, size):
    b = box()
    return getattr(b, how)(size, b.Edges)
add("box_filleted", lambda: blended("makeFillet", 3))
add("box_chamfered", lambda: blended("makeChamfer", 2))
def cut_filleted():
    s = box().cut(pin())
    edges = [e for e in s.Edges if abs(e.BoundBox.ZMax - 20) < 1e-6 and abs(e.BoundBox.ZMin - 20) < 1e-6]
    return s.makeFillet(2, edges)
add("box_minus_pin_filleted", cut_filleted)
def revolved():
    pts = [V(5, 0, 0), V(12, 0, 0), V(12, 0, 4), V(8, 0, 10), V(5, 0, 10)]
    face = Part.Face(Part.makePolygon(pts + [pts[0]]))
    return face.revolve(V(0, 0, 0), V(0, 0, 1), 360)
add("revolved_profile", revolved)
def spline_revolved():
    curve = Part.BSplineCurve()
    curve.interpolate([V(10, 0, 0), V(14, 0, 5), V(9, 0, 12), V(11, 0, 18)])
    edge = curve.toShape()
    top = Part.makeLine(V(11, 0, 18), V(0, 0, 18))
    axis = Part.makeLine(V(0, 0, 18), V(0, 0, 0))
    bottom = Part.makeLine(V(0, 0, 0), V(10, 0, 0))
    face = Part.Face(Part.Wire([edge, top, axis, bottom]))
    return face.revolve(V(0, 0, 0), V(0, 0, 1), 360)
add("spline_revolved", spline_revolved)
def lofted():
    a = Part.Wire(Part.makeCircle(10, V(0, 0, 0)))
    b = Part.Wire(Part.makePolygon([V(-6, -6, 20), V(6, -6, 20), V(6, 6, 20), V(-6, 6, 20), V(-6, -6, 20)]))
    return Part.makeLoft([a, b], True)
add("loft_circle_square", lofted)
add("two_solids", lambda: Part.makeCompound([box(), Part.makeSphere(5, V(80, 0, 0))]))
with open(os.path.join(out, "cases.json"), "w") as f:
    json.dump(cases, f)
"#;

#[test]
#[ignore = "needs FreeCAD (CRAFT3D_FREECAD): cargo xtask freecad"]
fn freecad_files_read_with_freecads_volume() {
    let freecad = std::env::var("CRAFT3D_FREECAD").expect(
        "set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable to run this test",
    );
    let dir = workdir("step-import", "freecad");
    let script = dir.join("make_solids.py");
    std::fs::write(&script, FREECAD_SCRIPT).unwrap();
    let mut command = std::process::Command::new(&freecad);
    if freecad.ends_with(".AppImage") {
        command.arg("freecadcmd");
    }
    let output = command
        .arg(&script)
        .env("CRAFT3D_FC_DIR", &dir)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {freecad}: {e}"));
    let cases: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("cases.json")).unwrap_or_else(|_| {
            panic!(
                "FreeCAD wrote no cases ({}):\n{}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        }),
    )
    .unwrap();
    let mut agreed = Vec::new();
    let mut refused = Vec::new();
    let mut wrong = Vec::new();
    let mut report =
        String::from("| file | FreeCAD volume | ours | verdict |\n|---|---|---|---|\n");
    for (name, case) in cases.as_object().unwrap() {
        if let Some(e) = case.get("error") {
            panic!("FreeCAD could not make {name}: {e}");
        }
        let mut s = session_in(&dir);
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 1e-6}),
        );
        let expected = case["volume"].as_f64().unwrap();
        let expected_area = case["area"].as_f64().unwrap();
        let verdict = match s.execute("file.import", &json!({"path": format!("{name}.step")})) {
            Err(e) => {
                refused.push(format!("{name}: {e}"));
                format!("refused: {e}")
            }
            Ok(result) => {
                let made = result["bodies"].as_array().unwrap();
                let failed: Vec<&Value> = made.iter().filter(|b| b["status"] != "ok").collect();
                if !failed.is_empty() {
                    refused.push(format!("{name}: {}", failed[0]["error"]));
                    format!("failed: {}", failed[0]["error"])
                } else {
                    let all = bodies(&mut s);
                    let volume: f64 = all.iter().map(|b| b["volume"].as_f64().unwrap()).sum();
                    let area: f64 = all.iter().map(|b| b["area"].as_f64().unwrap()).sum();
                    let ok = (volume - expected).abs() <= 1e-6 * expected
                        && (area - expected_area).abs() <= 1e-6 * expected_area
                        && all.len() as u64 == case["solids"].as_u64().unwrap();
                    if ok {
                        agreed.push(name.clone());
                        format!("agrees ({volume})")
                    } else {
                        wrong.push(format!(
                            "{name}: FreeCAD {expected} / {expected_area} in {} solids, ours {volume} / {area} in {}",
                            case["solids"],
                            all.len()
                        ));
                        format!("DIFFERS: {volume} / {area}")
                    }
                }
            }
        };
        report.push_str(&format!("| {name} | {expected} | | {verdict} |\n"));
    }
    std::fs::write(dir.join("report.md"), &report).unwrap();
    println!("{report}");
    println!("agreed: {agreed:?}\nrefused: {refused:#?}\nwrong: {wrong:#?}");
    // Never a wrong body; refusals are work owed (listed), but the plain
    // solids must read.
    assert!(wrong.is_empty(), "{wrong:#?}");
    for must in [
        "box",
        "cylinder",
        "sphere",
        "torus",
        "cone_frustum",
        "box_minus_pin",
    ] {
        assert!(
            agreed.iter().any(|a| a == must),
            "{must} did not read: {refused:#?}"
        );
    }
}

/// The FreeCAD script for assemblies and colours: a part placed several
/// times by links in an `App::Part` (one level, and a sub-assembly placed
/// twice), written by FreeCAD's own STEP writer (`Import.export`, its
/// assembly structure with the part shared), measured by FreeCAD: the
/// solids, their volume and their centroids where the assembly puts them.
/// A colour is given where FreeCAD has a way to give it without its GUI;
/// whether the file holds one is recorded, not assumed.
const FREECAD_ASSEMBLY_SCRIPT: &str = r#"
import os, json, traceback
import FreeCAD, Part, Import
from FreeCAD import Vector as V
out = os.environ["CRAFT3D_FC_DIR"]
cases = {}
def colour(obj):
    rgb = (0.8, 0.4, 0.2)
    done = False
    try:
        if obj.ViewObject is not None:
            obj.ViewObject.ShapeColor = rgb
            done = True
    except Exception:
        pass
    for prop in ("ShapeAppearance", "ShapeColor", "Color"):
        try:
            if hasattr(obj, prop):
                setattr(obj, prop, rgb)
                done = True
        except Exception:
            pass
    return done
def add(name, make):
    try:
        doc = FreeCAD.newDocument(name)
        top = make(doc)
        doc.recompute()
        path = os.path.join(out, name + ".step")
        Import.export([top], path)
        shape = Part.getShape(top)
        solids = shape.Solids
        text = open(path).read()
        cases[name] = {"volume": sum(s.Volume for s in solids),
                       "solids": len(solids),
                       "centroids": [[c.x, c.y, c.z] for c in (s.CenterOfMass for s in solids)],
                       "colours": "COLOUR_RGB" in text,
                       "assembly": "NEXT_ASSEMBLY_USAGE_OCCURRENCE" in text}
    except Exception:
        cases[name] = {"error": traceback.format_exc()}
def peg(doc):
    part = doc.addObject("Part::Feature", "Peg")
    part.Shape = Part.makeBox(4, 2, 1)
    colour(part)
    return part
def placed(doc, parent, target, name, x, y, z, axis, angle):
    link = doc.addObject("App::Link", name)
    link.LinkedObject = target
    link.Placement = FreeCAD.Placement(V(x, y, z), FreeCAD.Rotation(axis, angle))
    parent.addObject(link)
def rack(doc):
    part = peg(doc)
    asm = doc.addObject("App::Part", "Rack")
    placed(doc, asm, part, "Peg1", 0, 0, 0, V(0, 0, 1), 0)
    placed(doc, asm, part, "Peg2", 10, 0, 0, V(0, 0, 1), 90)
    placed(doc, asm, part, "Peg3", 0, 20, 5, V(1, 1, 0), 45)
    return asm
def robot(doc):
    part = peg(doc)
    arm = doc.addObject("App::Part", "Arm")
    placed(doc, arm, part, "PegA", 0, 0, 0, V(0, 0, 1), 30)
    placed(doc, arm, part, "PegB", 6, 0, 0, V(0, 1, 0), 90)
    root = doc.addObject("App::Part", "Robot")
    placed(doc, root, arm, "Left", 0, 0, 0, V(0, 0, 1), 0)
    placed(doc, root, arm, "Right", 30, 10, 0, V(0, 0, 1), 180)
    return root
add("rack", rack)
add("robot", robot)
with open(os.path.join(out, "cases.json"), "w") as f:
    json.dump(cases, f)
"#;

/// FreeCAD's assemblies and colours (slice 2): each placed solid of a
/// file FreeCAD writes reads as a body of its own where FreeCAD puts it
/// (its count, total volume and every centroid to 1e-6), and where
/// FreeCAD wrote a colour, the bodies have it. Never a body in the wrong
/// place; a file without an assembly structure or a colour is said so.
#[test]
#[ignore = "needs FreeCAD (CRAFT3D_FREECAD): cargo xtask freecad"]
fn freecad_assemblies_read_where_freecad_places_them() {
    let freecad = std::env::var("CRAFT3D_FREECAD").expect(
        "set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable to run this test",
    );
    let dir = workdir("step-import", "freecad-assembly");
    let script = dir.join("make_assemblies.py");
    std::fs::write(&script, FREECAD_ASSEMBLY_SCRIPT).unwrap();
    let mut command = std::process::Command::new(&freecad);
    if freecad.ends_with(".AppImage") {
        command.arg("freecadcmd");
    }
    let output = command
        .arg(&script)
        .env("CRAFT3D_FC_DIR", &dir)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {freecad}: {e}"));
    let cases: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("cases.json")).unwrap_or_else(|_| {
            panic!(
                "FreeCAD wrote no cases ({}):\n{}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        }),
    )
    .unwrap();
    let mut wrong = Vec::new();
    let mut said = Vec::new();
    for (name, case) in cases.as_object().unwrap() {
        if let Some(e) = case.get("error") {
            panic!("FreeCAD could not make {name}: {e}");
        }
        if case["assembly"] != true {
            said.push(format!("{name}: FreeCAD wrote no assembly structure"));
        }
        let mut s = session_in(&dir);
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 1e-6}),
        );
        let result = match s.execute("file.import", &json!({"path": format!("{name}.step")})) {
            Ok(result) => result,
            Err(e) => {
                wrong.push(format!("{name}: refused: {e}"));
                continue;
            }
        };
        let made = result["bodies"].as_array().unwrap();
        if let Some(failed) = made.iter().find(|b| b["status"] != "ok") {
            wrong.push(format!("{name}: failed: {}", failed["error"]));
            continue;
        }
        let all = bodies(&mut s);
        let volume: f64 = all.iter().map(|b| b["volume"].as_f64().unwrap()).sum();
        let expected = case["volume"].as_f64().unwrap();
        if all.len() as u64 != case["solids"].as_u64().unwrap()
            || (volume - expected).abs() > 1e-6 * expected
        {
            wrong.push(format!(
                "{name}: FreeCAD {} solids of {expected} in all, ours {} of {volume}",
                case["solids"],
                all.len()
            ));
            continue;
        }
        // Every FreeCAD centroid matched by one of ours, each used once.
        let mut ours: Vec<[f64; 3]> = all
            .iter()
            .map(|b| {
                let c = b["centroid"].as_array().unwrap();
                [
                    c[0].as_f64().unwrap(),
                    c[1].as_f64().unwrap(),
                    c[2].as_f64().unwrap(),
                ]
            })
            .collect();
        for c in case["centroids"].as_array().unwrap() {
            let c = [
                c[0].as_f64().unwrap(),
                c[1].as_f64().unwrap(),
                c[2].as_f64().unwrap(),
            ];
            let near = ours.iter().position(|o| {
                ((o[0] - c[0]).ppowi(2) + (o[1] - c[1]).ppowi(2) + (o[2] - c[2]).ppowi(2)).sqrt()
                    <= 1e-6 * 50.0
            });
            match near {
                Some(k) => {
                    ours.remove(k);
                }
                None => wrong.push(format!("{name}: no body at FreeCAD's centroid {c:?}")),
            }
        }
        if case["colours"] == true {
            for b in &all {
                if !b.to_string().contains("#cc6633") {
                    wrong.push(format!(
                        "{name}: {} has not FreeCAD's colour: {b}",
                        b["name"]
                    ));
                }
            }
        } else {
            said.push(format!("{name}: FreeCAD wrote no colour without its GUI"));
        }
    }
    println!("said: {said:#?}\nwrong: {wrong:#?}");
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The cap on an entity's own tolerance (`loosest`) is part of what a
/// result depends on (2026-10-31, the class-A review): a loose import (a
/// corner 1e-4 mm off its edges) united with a block is built under the
/// default cap of 1e-3 mm and refused under 5e-5; `document.setTolerance`
/// changing only the cap must rebuild them, not hand back the cached
/// bodies. Each state is held to the same document opened afresh from
/// its saved file.
#[test]
fn changing_only_the_cap_rebuilds_what_it_decides() {
    let dir = workdir("step-import", "loosest-cache");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "feature.block", json!({"size": [10, 20, 30]}));
    run(&mut s, "file.export", json!({"path": "block.step"}));
    let text = std::fs::read_to_string(dir.join("block.step")).unwrap();
    let moved = text.replacen(
        "CARTESIAN_POINT('',(10.,",
        "CARTESIAN_POINT('',(10.0001,",
        1,
    );
    assert_ne!(moved, text);
    std::fs::write(dir.join("loose.step"), moved).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "file.import", json!({"path": "loose.step"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [4, 4, 4], "position": [8, 8, 8], "name": "Lug"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "F1", "tool": "Lug", "name": "Joined"}),
    );
    let statuses = |s: &mut Session| -> Vec<(String, String)> {
        run(s, "document.inspect", json!({"measures": false}))["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["name"].as_str().unwrap().to_string(),
                    f["status"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    let fresh = |s: &mut Session, name: &str| -> Vec<(String, String)> {
        run(s, "file.save", json!({"path": name, "overwrite": true}));
        let mut other = session_in(&dir);
        run(&mut other, "file.open", json!({"path": name}));
        statuses(&mut other)
    };
    let before = statuses(&mut s);
    assert!(before.iter().all(|(_, st)| st == "ok"), "{before:?}");
    assert_eq!(fresh(&mut s, "wide.3dcraft"), before);
    // Only the cap changes: the loose import and what is built on it fail.
    run(&mut s, "document.setTolerance", json!({"loosest": 5e-5}));
    let tight = statuses(&mut s);
    let joined = tight.iter().find(|(n, _)| n == "Joined").unwrap();
    assert_eq!(joined.1, "failed", "{tight:?}");
    assert_eq!(fresh(&mut s, "tight.3dcraft"), tight);
    // And back: built again.
    run(&mut s, "document.setTolerance", json!({"loosest": null}));
    let wide = statuses(&mut s);
    assert_eq!(wide, before);
    assert_eq!(fresh(&mut s, "wide-again.3dcraft"), wide);
}
