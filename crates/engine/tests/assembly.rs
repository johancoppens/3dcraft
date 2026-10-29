//! Assemblies (docs/design/assemblies.md): instances of other documents,
//! configurations, bodies picked from a multi-body source, mates through
//! persistent names across files, subassemblies, pack and unpack, the
//! STEP export with structure, the assembly's measures, and the
//! commands' refusals. The solver is the stand-in (`asm_solve.rs`) until
//! track B's lands.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{error, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// Saves a document with one block `Base` of `size` at `path`, closes it.
fn block_part(s: &mut Session, path: &str, size: [f64; 3], tolerance: f64) {
    run(s, "file.new", json!({ "tolerance": tolerance }));
    run(s, "feature.block", json!({ "name": "Base", "size": size }));
    run(s, "file.save", json!({ "path": path }));
    run(s, "file.close", json!({}));
}

fn inspect(s: &mut Session) -> Value {
    run(s, "assembly.inspect", json!({ "measures": true }))
}

fn instance<'a>(inspected: &'a Value, name: &str) -> &'a Value {
    inspected["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == name)
        .unwrap_or_else(|| panic!("no instance {name} in {inspected}"))
}

fn volume(inspected: &Value, name: &str) -> f64 {
    instance(inspected, name)["measured"]["volume"]
        .as_f64()
        .unwrap()
}

/// The box of an instance's placed bodies: [[lo], [hi]].
fn boxed(inspected: &Value, name: &str) -> ([f64; 3], [f64; 3]) {
    let b = &instance(inspected, name)["measured"]["box"];
    let v = |k: usize| -> [f64; 3] {
        [
            b[k][0].as_f64().unwrap(),
            b[k][1].as_f64().unwrap(),
            b[k][2].as_f64().unwrap(),
        ]
    };
    (v(0), v(1))
}

fn close(a: f64, b: f64, scale: f64) -> bool {
    (a - b).abs() <= 1e-6 * scale.max(1e-300)
}

/// A stack of two blocks: the first on the world's xy plane, the second
/// on the first's top, by fixed mates between faces named through their
/// instances. At three scales.
#[test]
fn a_stack_of_two_instances_is_placed_by_its_mates_at_three_scales() {
    for scale in [1e-3, 1.0, 1e6] {
        let dir = workdir("assembly", &format!("stack-{scale:e}"));
        let mut s = session_in(&dir);
        let tolerance = 1e-6 * scale;
        block_part(
            &mut s,
            "block.3dcraft",
            [20.0 * scale, 10.0 * scale, 6.0 * scale],
            tolerance,
        );
        run(&mut s, "assembly.new", json!({ "tolerance": tolerance }));
        run(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": "A" }),
        );
        run(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": "B" }),
        );
        let m = run(
            &mut s,
            "mate.add",
            json!({ "kind": "fixed", "a": "world/xy", "b": "A/F1:face(0)", "name": "Ground" }),
        );
        assert_eq!(m["status"], "ok", "{m}");
        let m = run(
            &mut s,
            "mate.add",
            json!({ "kind": "fixed", "a": "A/F1:face(1)", "b": "B/F1:face(0)", "name": "OnTop" }),
        );
        assert_eq!(m["status"], "ok", "{m}");
        let i = inspect(&mut s);
        assert_eq!(i["configurations"], 1, "{i}");
        assert_eq!(i["regenerations"], 1, "{i}");
        assert_eq!(i["bodies"], 2, "{i}");
        let (alo, ahi) = boxed(&i, "A");
        let (blo, bhi) = boxed(&i, "B");
        // A on the plane, its bottom's centroid at the origin; B on top.
        assert!(
            close(alo[2], 0.0, scale) && close(ahi[2], 6.0 * scale, scale),
            "{alo:?} {ahi:?}"
        );
        assert!(
            close(blo[2], 6.0 * scale, scale) && close(bhi[2], 12.0 * scale, scale),
            "{blo:?} {bhi:?}"
        );
        for k in 0..2 {
            assert!(
                close(alo[k] + ahi[k], 0.0, scale),
                "A not centred: {alo:?} {ahi:?}"
            );
            assert!(
                close(blo[k] + bhi[k], 0.0, scale),
                "B not centred: {blo:?} {bhi:?}"
            );
        }
        let block = 20.0 * 10.0 * 6.0 * scale.powi(3);
        assert!((volume(&i, "A") - block).abs() <= 1e-9 * block);
        let solved = run(&mut s, "assembly.solve", json!({}));
        assert_eq!(solved["ok"], true, "{solved}");
        assert_eq!(solved["floating"], false, "{solved}");
        // Saved and opened again: the same.
        run(&mut s, "file.save", json!({ "path": "stack.3dcraftasm" }));
        run(&mut s, "file.close", json!({}));
        let opened = run(&mut s, "file.open", json!({ "path": "stack.3dcraftasm" }));
        assert_eq!(opened["assembly"], true, "{opened}");
        let again = inspect(&mut s);
        assert_eq!(boxed(&again, "B"), (blo, bhi));
    }
}

/// One source in two configurations (a public parameter set by the
/// instances): two regenerations for three instances.
#[test]
fn two_configurations_of_one_source_are_built_once_each() {
    let dir = workdir("assembly", "configurations");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "len", "expr": "40 mm" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "len", "min": "1 mm", "max": "500 mm" }),
    );
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Bar", "size": ["len", 4, 2] }),
    );
    run(&mut s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "long", "expr": "50 mm" }),
    );
    for (name, len) in [("L1", "30 mm"), ("L2", "long"), ("L3", "30 mm")] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "bar.3dcraft", "name": name, "params": { "len": len } }),
        );
    }
    let i = inspect(&mut s);
    assert_eq!(i["configurations"], 2, "{i}");
    assert_eq!(i["regenerations"], 2, "{i}");
    assert!(close(volume(&i, "L1"), 240.0, 1.0));
    assert!(close(volume(&i, "L2"), 400.0, 1.0));
    assert!(close(volume(&i, "L3"), 240.0, 1.0));
    // The assembly's variable drives its configuration.
    run(
        &mut s,
        "variable.set",
        json!({ "name": "long", "expr": "60 mm" }),
    );
    let i = inspect(&mut s);
    assert!(close(volume(&i, "L2"), 480.0, 1.0));
    assert_eq!(i["regenerations"], 3, "only the changed configuration: {i}");
    // A parameter the source does not publish fails the instance by name.
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "L3", "params": { "width": "3 mm" } }),
    );
    let i = inspect(&mut s);
    let failed = instance(&i, "L3");
    assert_eq!(failed["status"], "failed");
    assert!(
        failed["error"].as_str().unwrap().contains("width"),
        "{failed}"
    );
}

/// Two bodies picked from one multi-body source: one regeneration, each
/// body where the source puts it; a body that is gone fails by name.
#[test]
fn two_bodies_picked_from_one_source_share_its_regeneration() {
    let dir = workdir("assembly", "bodies");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Left", "size": [2, 2, 2] }),
    );
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Right", "size": [4, 4, 4], "position": [10, 0, 0] }),
    );
    run(&mut s, "file.save", json!({ "path": "pair.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "pair.3dcraft", "name": "P1", "body": "Left" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "pair.3dcraft", "name": "P2", "body": "F2" }),
    );
    let i = inspect(&mut s);
    assert_eq!(i["regenerations"], 1, "{i}");
    assert_eq!(i["bodies"], 2, "{i}");
    assert!(close(volume(&i, "P1"), 8.0, 1.0));
    assert!(close(volume(&i, "P2"), 64.0, 1.0));
    let (lo, _) = boxed(&i, "P2");
    assert!(close(lo[0], 10.0, 1.0), "where the source puts it: {lo:?}");
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "P2", "body": "Gone" }),
    );
    let i = inspect(&mut s);
    let p2 = instance(&i, "P2");
    assert_eq!(p2["status"], "failed");
    assert!(p2["error"].as_str().unwrap().contains("Gone"), "{p2}");
    assert_eq!(i["bodies"], 1, "never another body: {i}");
}

/// An edited source is followed after a reload; a face a mate names that
/// the edit removed fails that mate by its name, and never retargets.
#[test]
fn an_edited_source_is_followed_and_a_removed_face_fails_its_mate_by_name() {
    let dir = workdir("assembly", "edited");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [20.0, 10.0, 6.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "A" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "B" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "A/F1:face(0)" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "A/F1:face(1)", "b": "B/F1:face(0)", "name": "OnTop" }),
    );
    run(&mut s, "file.save", json!({ "path": "stack.3dcraftasm" }));
    // The part made taller, in its own document.
    block_part(&mut s, "block.3dcraft", [20.0, 10.0, 8.0], 1e-6);
    run(
        &mut s,
        "file.activate",
        json!({ "path": "stack.3dcraftasm" }),
    );
    let reloaded = run(&mut s, "assembly.reload", json!({}));
    assert_eq!(reloaded["changed"], json!(["A", "B"]), "{reloaded}");
    let i = inspect(&mut s);
    assert!(close(volume(&i, "A"), 1600.0, 1.0));
    let (blo, _) = boxed(&i, "B");
    assert!(close(blo[2], 8.0, 1.0), "{blo:?}");
    assert_eq!(instance(&i, "A")["changed"], true);
    // The top cut away by a later feature: the face is gone.
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Base", "size": [20, 10, 6] }),
    );
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Tool", "size": [40, 40, 10], "position": [-10, -10, 3] }),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({ "op": "subtract", "target": "Base", "tool": "Tool" }),
    );
    run(&mut s, "file.save", json!({ "path": "block.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(
        &mut s,
        "file.activate",
        json!({ "path": "stack.3dcraftasm" }),
    );
    run(&mut s, "assembly.reload", json!({}));
    let solved = run(&mut s, "assembly.solve", json!({}));
    let on_top = solved["mates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "OnTop")
        .unwrap();
    assert_eq!(on_top["status"], "failed", "{solved}");
    assert!(
        on_top["error"].as_str().unwrap().contains("F1:face(1)"),
        "{on_top}"
    );
    // A pinned instance keeps what it was pinned to: refused by name when
    // the file moved on and nothing packed holds the old bytes.
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "B", "pinned": true }),
    );
    let i = inspect(&mut s);
    assert_eq!(instance(&i, "B")["status"], "ok", "{i}");
}

/// A subassembly placed rigidly, and a cycle of subassemblies refused.
#[test]
fn a_subassembly_is_placed_and_a_cycle_is_refused() {
    let dir = workdir("assembly", "cycle");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [2.0, 2.0, 2.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Cube", "position": [5, 0, 0] }),
    );
    run(&mut s, "file.save", json!({ "path": "inner.3dcraftasm" }));
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "inner.3dcraftasm", "name": "Sub", "position": [0, 10, 0] }),
    );
    let i = inspect(&mut s);
    assert_eq!(instance(&i, "Sub")["subassembly"], true);
    let (lo, _) = boxed(&i, "Sub");
    assert!(close(lo[0], 5.0, 1.0) && close(lo[1], 10.0, 1.0), "{lo:?}");
    // A mate through the subassembly's instance.
    let m = run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "Sub/Cube/F1:face(0)" }),
    );
    assert_eq!(m["status"], "ok", "{m}");
    run(&mut s, "file.save", json!({ "path": "outer.3dcraftasm" }));
    run(
        &mut s,
        "file.activate",
        json!({ "path": "inner.3dcraftasm" }),
    );
    let refused = error(
        &mut s,
        "instance.add",
        json!({ "source": "outer.3dcraftasm" }),
    );
    assert!(refused.contains("cycle"), "{refused}");
    // Itself.
    let refused = error(
        &mut s,
        "instance.add",
        json!({ "source": "inner.3dcraftasm" }),
    );
    assert!(refused.contains("cycle"), "{refused}");
}

/// Packed, the assembly carries its sources and rebuilds without them;
/// unpacked, it refers to files again with the same hashes.
#[test]
fn pack_and_unpack_keep_the_assembly_the_same() {
    let dir = workdir("assembly", "pack");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [20.0, 10.0, 6.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "A" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "B", "position": [0, 0, 6] }),
    );
    let before = inspect(&mut s);
    let hash = instance(&before, "A")["source"]["hash"].clone();
    let packed = run(&mut s, "assembly.pack", json!({}));
    assert_eq!(packed["packed"].as_array().unwrap().len(), 1, "{packed}");
    run(&mut s, "file.save", json!({ "path": "packed.3dcraftasm" }));
    run(&mut s, "file.close", json!({}));
    std::fs::remove_file(dir.join("block.3dcraft")).unwrap();
    run(&mut s, "file.open", json!({ "path": "packed.3dcraftasm" }));
    let after = inspect(&mut s);
    for name in ["A", "B"] {
        assert_eq!(instance(&after, name)["source"]["kind"], "packed");
        assert_eq!(instance(&after, name)["source"]["hash"], hash);
        assert_eq!(boxed(&after, name), boxed(&before, name));
        assert_eq!(volume(&after, name), volume(&before, name));
    }
    let unpacked = run(&mut s, "assembly.unpack", json!({ "folder": "parts" }));
    assert_eq!(
        unpacked["written"].as_array().unwrap().len(),
        1,
        "{unpacked}"
    );
    assert!(dir.join("parts/A.3dcraft").is_file());
    let again = inspect(&mut s);
    for name in ["A", "B"] {
        assert_eq!(instance(&again, name)["source"]["kind"], "file");
        assert_eq!(instance(&again, name)["source"]["hash"], hash);
        assert_eq!(boxed(&again, name), boxed(&before, name));
    }
}

/// The STEP export keeps the structure: one part, placed twice, read back
/// by our STEP import as two bodies where the assembly put them.
#[test]
fn the_step_export_of_an_assembly_reads_back_as_its_placed_bodies() {
    let dir = workdir("assembly", "step");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [20.0, 10.0, 6.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "A" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "B", "position": [0, 0, 30], "rotation": [0, 0, 90] }),
    );
    let i = inspect(&mut s);
    let exported = run(&mut s, "file.export", json!({ "path": "stack.step" }));
    assert_eq!(exported["parts"], 1, "{exported}");
    assert_eq!(exported["occurrences"], 2, "{exported}");
    // The meshes of the placed bodies, too.
    for path in ["stack.stl", "stack.3mf", "stack.obj"] {
        run(&mut s, "file.export", json!({ "path": path }));
        assert!(dir.join(path).is_file());
    }
    run(&mut s, "file.new", json!({}));
    let imported = run(&mut s, "file.import", json!({ "path": "stack.step" }));
    let features = imported.as_array().unwrap_or_else(|| panic!("{imported}"));
    assert_eq!(features.len(), 2, "{imported}");
    let doc = run(&mut s, "document.inspect", json!({}));
    let bodies = doc["bodies"].as_array().unwrap();
    assert_eq!(bodies.len(), 2, "{doc}");
    for body in bodies {
        assert!(
            close(body["volume"].as_f64().unwrap(), 1200.0, 1.0),
            "{body}"
        );
    }
    let _ = i;
}

/// The assembly's measures on its placed bodies, by instance name:
/// clearance, overlap and mass, in closed form.
#[test]
fn interference_clearance_and_mass_of_instances_in_closed_form() {
    let dir = workdir("assembly", "measures");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Cube", "size": [10, 10, 10] }),
    );
    run(
        &mut s,
        "body.material",
        json!({ "body": "Cube", "name": "steel", "density": 8000 }),
    );
    run(&mut s, "file.save", json!({ "path": "cube.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "cube.3dcraft", "name": "A" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "cube.3dcraft", "name": "B", "position": [15, 0, 0] }),
    );
    let gap = run(
        &mut s,
        "assert.add",
        json!({ "name": "gap", "measure": "clearance", "from": "A", "to": "B", "min": "4 mm" }),
    );
    assert_eq!(gap["status"], "passed", "{gap}");
    assert!(close(gap["measured"].as_f64().unwrap(), 5.0, 1.0), "{gap}");
    let mass = run(
        &mut s,
        "assert.add",
        json!({ "name": "mass", "measure": "mass", "of": ["A", "B"], "eq": 0.016, "tol": 1e-9 }),
    );
    assert_eq!(
        mass["status"], "passed",
        "8000 kg/m3 x 2 x 1e-6 m3 is 0.016 kg: {mass}"
    );
    // B moved into A by 3 mm: they overlap by 300 mm^3.
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "B", "position": [7, 0, 0] }),
    );
    let overlap = run(
        &mut s,
        "assert.add",
        json!({ "name": "clear", "measure": "overlap", "of": ["A"], "to": ["B"], "is": false }),
    );
    let message = overlap["message"].as_str().unwrap_or_default();
    assert_eq!(overlap["status"], "failed", "{overlap}");
    assert!(message.contains("300"), "{overlap}");
}

/// A revolute mate driven by its joint variable: the second bar turns
/// about the first's end by the angle set.
#[test]
fn a_joint_set_turns_a_revolute_mate() {
    let dir = workdir("assembly", "joint");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Bar", "size": [10, 2, 2] }),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({ "name": "Pin", "radius": 0.5, "height": 2, "position": [9, 1, 0] }),
    );
    run(&mut s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "bar.3dcraft", "name": "Base", "body": "Bar" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "bar.3dcraft", "name": "Arm", "body": "Bar" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "Base/F1:face(0)", "flip": true }),
    );
    let joint = run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "Base/F1:face(1)", "b": "Arm/F1:face(0)", "name": "Hinge" }),
    );
    assert_eq!(joint["status"], "ok", "{joint}");
    assert_eq!(joint["variables"], json!(["Hinge_angle"]));
    let i = inspect(&mut s);
    let (lo0, hi0) = boxed(&i, "Arm");
    let set = run(
        &mut s,
        "joint.set",
        json!({ "mate": "Hinge", "value": "90 deg" }),
    );
    assert_eq!(set["driven"], true, "{set}");
    assert!(
        close(
            set["value"].as_f64().unwrap(),
            std::f64::consts::FRAC_PI_2,
            1.0
        ),
        "{set}"
    );
    let i = inspect(&mut s);
    let (lo1, hi1) = boxed(&i, "Arm");
    // A quarter turn about z swaps the arm's extents along x and y.
    assert!(
        close(hi0[0] - lo0[0], hi1[1] - lo1[1], 1.0),
        "{lo0:?} {hi0:?} {lo1:?} {hi1:?}"
    );
    // The variable is the assembly's, usable in formulas.
    let vars = run(
        &mut s,
        "variable.set",
        json!({ "name": "twice", "expr": "2 * Hinge_angle" }),
    );
    assert!(vars.is_object());
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Hinge", "value": null }),
    );
    let solved = run(&mut s, "assembly.solve", json!({}));
    let hinge = &solved["mates"][1];
    assert_eq!(hinge["driven"], false, "{solved}");
}

/// What every command refuses, by name, never a panic.
#[test]
fn the_assembly_commands_refuse_what_they_cannot_do() {
    let dir = workdir("assembly", "refusals");
    let mut s = session_in(&dir);
    // A part is not an assembly.
    run(&mut s, "file.new", json!({}));
    for id in [
        "instance.add",
        "mate.add",
        "assembly.solve",
        "assembly.pack",
        "joint.set",
    ] {
        let e = error(&mut s, id, json!({ "source": "x.3dcraft" }));
        assert!(!e.is_empty());
    }
    block_part(&mut s, "block.3dcraft", [1.0, 1.0, 1.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    // No such file; not a document.
    assert!(
        error(&mut s, "instance.add", json!({ "source": "none.3dcraft" })).contains("none.3dcraft")
    );
    std::fs::write(dir.join("junk.3dcraft"), b"junk").unwrap();
    assert!(
        error(&mut s, "instance.add", json!({ "source": "junk.3dcraft" }))
            .contains("not a 3DCraft document")
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "A" }),
    );
    // Names.
    assert!(
        error(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": "A" })
        )
        .contains("already")
    );
    assert!(
        error(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": "world" })
        )
        .contains("reserved")
    );
    assert!(
        error(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": "a/b" })
        )
        .contains("/")
    );
    // Mates.
    assert!(
        error(
            &mut s,
            "mate.add",
            json!({ "kind": "hinge", "a": "A/F1:face(0)", "b": "world/xy" })
        )
        .contains("kind")
    );
    assert!(
        error(
            &mut s,
            "mate.add",
            json!({ "kind": "fixed", "a": "A", "b": "world/xy" })
        )
        .contains("Instance/")
    );
    assert!(
        error(
            &mut s,
            "mate.add",
            json!({ "kind": "fixed", "a": "Gone/F1:face(0)", "b": "world/xy" })
        )
        .contains("Gone")
    );
    assert!(
        error(
            &mut s,
            "mate.add",
            json!({ "kind": "fixed", "a": "A/F1:face(0)", "b": "A/F1:face(1)" })
        )
        .contains("itself")
    );
    // A mate on a name that is not there is kept, failed by its name.
    let m = run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "A/F1:face(77)" }),
    );
    assert_eq!(m["status"], "failed");
    assert!(m["error"].as_str().unwrap().contains("F1:face(77)"), "{m}");
    assert!(error(&mut s, "instance.delete", json!({ "name": "A" })).contains("delete them first"));
    assert!(
        error(&mut s, "joint.set", json!({ "mate": "Mate1", "value": 1 }))
            .contains("no free motion")
    );
    assert!(
        error(
            &mut s,
            "coupling.add",
            json!({ "kind": "gear", "mates": ["Mate1", "Nope"], "value": 2 })
        )
        .contains("Nope")
    );
    assert!(error(&mut s, "assembly.unpack", json!({})).contains("no packed"));
    // Saving under the wrong extension, both ways.
    assert!(error(&mut s, "file.save", json!({ "path": "a.3dcraft" })).contains(".3dcraftasm"));
    run(&mut s, "file.new", json!({}));
    assert!(error(&mut s, "file.save", json!({ "path": "p.3dcraftasm" })).contains("part"));
    // An assembly has no features of its own.
    run(&mut s, "assembly.new", json!({}));
    assert!(error(&mut s, "feature.block", json!({ "size": [1, 1, 1] })).contains("assembly"));
}
