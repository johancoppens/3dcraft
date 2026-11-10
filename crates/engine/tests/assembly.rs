//! Assemblies (docs/design/assemblies.md): instances of other documents,
//! configurations, bodies picked from a multi-body source, mates through
//! persistent names across files, subassemblies, pack and unpack, the
//! STEP export with structure, the assembly's measures, and the
//! commands' refusals; and a four-bar linkage of one bar part in two
//! configurations, solved by track B's `craft3d-assembly`.

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
        let block = 20.0 * 10.0 * 6.0 * scale.ppowi(3);
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
    // Invisible: left out of every export, as of the scene.
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "C", "position": [100, 0, 0], "visible": false }),
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
    let features = imported["bodies"]
        .as_array()
        .unwrap_or_else(|| panic!("{imported}"));
    assert_eq!(features.len(), 2, "{imported}");
    let doc = run(&mut s, "document.inspect", json!({}));
    let bodies = doc["bodies"].as_array().unwrap();
    assert_eq!(bodies.len(), 2, "{doc}");
    // Each body read back where its instance is placed: A as the part
    // is, B turned a quarter about z and raised by 30.
    let mut wanted = vec![boxed(&i, "A"), boxed(&i, "B")];
    assert!(close(wanted[1].0[0], -10.0, 1.0), "{i}");
    for body in bodies {
        assert!(
            close(body["volume"].as_f64().unwrap(), 1200.0, 1.0),
            "{body}"
        );
        let b = &body["bbox"];
        let got: [[f64; 3]; 2] = [0, 1].map(|k| [0, 1, 2].map(|j| b[k][j].as_f64().unwrap()));
        let at = wanted
            .iter()
            .position(|(lo, hi)| {
                (0..3).all(|j| close(lo[j], got[0][j], 100.0) && close(hi[j], got[1][j], 100.0))
            })
            .unwrap_or_else(|| panic!("a body at {got:?}, the instances at {wanted:?}"));
        wanted.remove(at);
    }
    assert!(wanted.is_empty());
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
    // Undriven, the arm stays where the solve finds it nearest its start;
    // driven to 0 it lies along its frame's zero.
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Hinge", "value": "0 deg" }),
    );
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
    // A drag: its first value a step of its own, the later ones merged
    // into it, so one undo takes the whole drag back.
    for (k, degrees) in [10, 20, 30, 40].into_iter().enumerate() {
        run(
            &mut s,
            "joint.set",
            json!({ "mate": "Hinge", "value": format!("{degrees} deg"), "merge": k > 0 }),
        );
    }
    let solved = run(&mut s, "assembly.solve", json!({}));
    assert!(
        close(
            solved["mates"][1]["value"].as_f64().unwrap(),
            40f64.to_radians(),
            1.0
        ),
        "{solved}"
    );
    run(&mut s, "edit.undo", json!({}));
    let solved = run(&mut s, "assembly.solve", json!({}));
    assert_eq!(solved["mates"][1]["driven"], false, "{solved}");
    // merge folds only into a joint.set of the same variable.
    run(
        &mut s,
        "variable.set",
        json!({ "name": "other", "expr": "1" }),
    );
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Hinge", "value": "5 deg", "merge": true }),
    );
    run(&mut s, "edit.undo", json!({}));
    let solved = run(&mut s, "assembly.solve", json!({}));
    assert_eq!(solved["mates"][1]["driven"], false, "{solved}");
    let vars = run(&mut s, "document.inspect", json!({ "measures": false }))["variables"].clone();
    assert!(vars.to_string().contains("\"other\""), "{vars}");
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

/// A flexible subassembly gives its instances and mates to the parent's
/// solve (named `Sub/Inner`); placed the same as rigid while nothing in
/// the parent moves them, and a parent's mate can name its instances.
#[test]
fn a_flexible_subassembly_is_solved_with_its_parent() {
    let dir = workdir("assembly", "flexible");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [10.0, 2.0, 2.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Base" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Arm" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "Base/F1:face(0)" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "Base/F1:face(1)", "b": "Arm/F1:face(0)", "name": "Hinge" }),
    );
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Hinge", "value": "90 deg" }),
    );
    run(&mut s, "file.save", json!({ "path": "leg.3dcraftasm" }));
    let mut boxes = Vec::new();
    for mode in ["rigid", "flexible"] {
        run(&mut s, "assembly.new", json!({}));
        run(
            &mut s,
            "instance.add",
            json!({ "source": "leg.3dcraftasm", "name": "Leg", "mode": mode, "position": [0, 0, 5] }),
        );
        run(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": "Lid" }),
        );
        let m = run(
            &mut s,
            "mate.add",
            json!({ "kind": "fixed", "a": "Leg/Arm/F1:face(1)", "b": "Lid/F1:face(0)" }),
        );
        assert_eq!(m["status"], "ok", "{mode}: {m}");
        let i = inspect(&mut s);
        assert_eq!(i["bodies"], 3, "{mode}: {i}");
        let solved = run(&mut s, "assembly.solve", json!({}));
        assert_eq!(solved["ok"], true, "{mode}: {solved}");
        if mode == "flexible" {
            let names: Vec<&str> = solved["solved"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d.as_str().unwrap())
                .collect();
            assert!(names.contains(&"Leg/Arm"), "{solved}");
        }
        boxes.push((boxed(&i, "Leg"), boxed(&i, "Lid")));
    }
    let ((l0, h0), (l1, h1)) = (boxes[0].0, boxes[1].0);
    for k in 0..3 {
        assert!(
            close(l0[k], l1[k], 1.0) && close(h0[k], h1[k], 1.0),
            "{boxes:?}"
        );
    }
    let ((a, b), (c, d)) = (boxes[0].1, boxes[1].1);
    for k in 0..3 {
        assert!(
            close(a[k], c[k], 1.0) && close(b[k], d[k], 1.0),
            "{boxes:?}"
        );
    }
}

/// The acceptance example: a four-bar linkage made from one bar part in
/// two configurations (a long bar for the ground and the coupler, a short
/// one for the crank and the rocker: a parallelogram), its pivots datum
/// axes of the part. Driven by the crank, the closed loop is solved as a
/// whole, and the coupler stays parallel to the ground, carried by the
/// crank's tip: closed form.
#[test]
fn a_four_bar_linkage_of_one_bar_part_follows_its_crank() {
    let dir = workdir("assembly", "four-bar");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "len", "expr": "24 mm" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "len", "min": "10 mm", "max": "500 mm" }),
    );
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Bar", "size": ["len", 4, 2] }),
    );
    run(
        &mut s,
        "datum.axis",
        json!({ "name": "End1", "through": [[2, 2, 0], [2, 2, 1]] }),
    );
    run(
        &mut s,
        "datum.axis",
        json!({ "name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]] }),
    );
    run(&mut s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    let (a, d) = (20.0_f64, 60.0_f64);
    for theta in [60.0_f64, 135.0, 250.0] {
        let t = theta.to_radians();
        let (c, n) = (t.pcos(), t.psin());
        let tip = [a * c, a * n];
        // Starts near the parallelogram, off by a little.
        let crank = [-(2.0 * c - 2.0 * n) + 0.3, -(2.0 * n + 2.0 * c) - 0.2, 0.0];
        let coupler = [tip[0] - 2.0 + 0.4, tip[1] - 2.0 + 0.3, 0.0];
        let rocker = [
            d - (2.0 * c - 2.0 * n) - 0.2,
            -(2.0 * n + 2.0 * c) + 0.4,
            0.0,
        ];
        run(&mut s, "assembly.new", json!({}));
        let bar = |name: &str, len: &str, position: [f64; 3], rotation: f64| {
            json!({ "source": "bar.3dcraft", "name": name, "body": "Bar", "params": { "len": len },
                    "position": position, "rotation": [0, 0, rotation] })
        };
        run(
            &mut s,
            "instance.add",
            bar("Ground", "64 mm", [-2.0, -2.0, 0.0], 0.0),
        );
        run(&mut s, "instance.add", bar("Crank", "24 mm", crank, theta));
        run(
            &mut s,
            "instance.add",
            bar("Coupler", "64 mm", coupler, 0.0),
        );
        run(
            &mut s,
            "instance.add",
            bar("Rocker", "24 mm", rocker, theta),
        );
        for (name, a, b) in [
            ("Base", "world/z", "Ground/End1"),
            ("Drive", "Ground/End1", "Crank/End1"),
            ("Elbow", "Crank/End2", "Coupler/End1"),
            ("Knee", "Coupler/End2", "Rocker/End2"),
            ("Hip", "Ground/End2", "Rocker/End1"),
        ] {
            let kind = if name == "Base" { "fixed" } else { "revolute" };
            run(
                &mut s,
                "mate.add",
                json!({ "kind": kind, "a": a, "b": b, "name": name, "flip": true }),
            );
        }
        run(
            &mut s,
            "joint.set",
            json!({ "mate": "Drive", "value": format!("{theta} deg") }),
        );
        let i = inspect(&mut s);
        assert_eq!(i["configurations"], 2, "{i}");
        let solved = run(&mut s, "assembly.solve", json!({}));
        assert_eq!(solved["ok"], true, "{theta}: {solved}");
        assert_eq!(solved["dof"], 0, "{theta}: {solved}");
        let (lo, hi) = boxed(&i, "Coupler");
        let want = [tip[0] - 2.0, tip[1] - 2.0];
        assert!(
            close(lo[0], want[0], 100.0) && close(lo[1], want[1], 100.0),
            "{theta}: {lo:?} {want:?}"
        );
        assert!(
            close(hi[0] - lo[0], 64.0, 100.0) && close(hi[1] - lo[1], 4.0, 100.0),
            "{theta}: parallel {lo:?} {hi:?}"
        );
    }
}

/// What a viewer draws follows the solve: the scene of an assembly holds
/// its placed bodies, and a joint set moves its box (the window's view
/// and `ui.fit` frame that box).
#[test]
fn the_scene_of_an_assembly_follows_its_joints() {
    let dir = workdir("assembly", "scene");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [10.0, 2.0, 2.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Base" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Arm" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "Base/F1:face(0)" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "Base/F1:face(1)", "b": "Arm/F1:face(0)", "name": "Hinge" }),
    );
    let mut boxes = Vec::new();
    for angle in ["0 deg", "90 deg"] {
        run(
            &mut s,
            "joint.set",
            json!({ "mate": "Hinge", "value": angle }),
        );
        let scene = s.scene(None).unwrap().expect("a scene");
        assert_eq!(scene.bodies.len(), 2, "{angle}");
        let (lo, hi) = scene.bbox.corners().expect("a box");
        boxes.push((lo, hi));
    }
    let ((lo0, hi0), (lo1, hi1)) = (boxes[0], boxes[1]);
    assert!(
        (lo0.x - lo1.x).abs() > 1.0 || (lo0.y - lo1.y).abs() > 1.0 || (hi0.x - hi1.x).abs() > 1.0,
        "the drawn box did not move: {boxes:?}"
    );
}

/// An absolute source path is kept absolute, never turned into a
/// relative one.
#[test]
fn an_absolute_source_path_stays_absolute() {
    let dir = workdir("assembly", "absolute");
    let mut s = common::session_in(&dir);
    block_part(&mut s, "block.3dcraft", [1.0, 1.0, 1.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    let absolute = dir.join("block.3dcraft").to_string_lossy().to_string();
    match s.execute("instance.add", &json!({ "source": absolute })) {
        Ok(added) => assert_eq!(added["status"], "ok", "{added}"),
        Err(e) => {
            let e = e.to_string();
            assert!(!e.contains("cannot be read"), "{e}");
            assert!(e.contains("read root") || e.contains("absolute"), "{e}");
        }
    }
}

/// The width along x of an instance's placed bodies.
fn width(inspected: &Value, name: &str) -> f64 {
    let (lo, hi) = boxed(inspected, name);
    hi[0] - lo[0]
}

/// A subassembly is placed again when one of its own sources changes
/// (`assembly.reload` lists it), and two copies of one subassembly file
/// in two folders are two configurations, each with its own folder's
/// parts.
#[test]
fn a_subassembly_follows_its_own_parts_and_its_folder() {
    let dir = workdir("assembly", "subcache");
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let mut s = session_in(&dir);
    block_part(&mut s, "a/part.3dcraft", [2.0, 2.0, 2.0], 1e-6);
    block_part(&mut s, "b/part.3dcraft", [4.0, 4.0, 4.0], 1e-6);
    block_part(&mut s, "big.3dcraft", [8.0, 8.0, 8.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "a/part.3dcraft", "name": "P" }),
    );
    run(&mut s, "file.save", json!({ "path": "a/sub.3dcraftasm" }));
    run(&mut s, "file.close", json!({}));
    // The same bytes in another folder, beside another part.
    std::fs::copy(dir.join("a/sub.3dcraftasm"), dir.join("b/sub.3dcraftasm")).unwrap();
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "a/sub.3dcraftasm", "name": "SubA" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "b/sub.3dcraftasm", "name": "SubB", "position": [20, 0, 0] }),
    );
    let i = inspect(&mut s);
    assert!(close(width(&i, "SubA"), 2.0, 1.0), "{i}");
    assert!(close(width(&i, "SubB"), 4.0, 1.0), "{i}");
    assert_ne!(
        instance(&i, "SubA")["configuration"],
        instance(&i, "SubB")["configuration"]
    );
    // The copy records the hash of a's part, and b's differs: SubB has
    // changed below it from the start; SubA not yet.
    assert_eq!(instance(&i, "SubA")["changed"], false, "{i}");
    assert_eq!(instance(&i, "SubB")["changed"], true, "{i}");
    // The part of the first subassembly edited.
    std::fs::copy(dir.join("big.3dcraft"), dir.join("a/part.3dcraft")).unwrap();
    let reloaded = run(&mut s, "assembly.reload", json!({}));
    assert_eq!(
        reloaded["filesChanged"],
        json!(["a/part.3dcraft"]),
        "{reloaded}"
    );
    assert_eq!(reloaded["changed"], json!(["SubA", "SubB"]), "{reloaded}");
    let i = inspect(&mut s);
    assert!(close(width(&i, "SubA"), 8.0, 1.0), "{i}");
    assert!(close(width(&i, "SubB"), 4.0, 1.0), "{i}");
}

/// The source paths are rewritten for the folder an assembly is saved
/// into only once its file is written, and for every state undo can go
/// back to: a failed save and an undo after Save As keep each instance
/// on its own file.
#[test]
fn a_failed_save_and_an_undo_after_save_as_keep_the_sources() {
    let dir = workdir("assembly", "rebase");
    std::fs::create_dir_all(dir.join("parts")).unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    // A file where a folder would have to be: a save beneath it fails.
    std::fs::write(dir.join("blocked"), b"not a folder").unwrap();
    let mut s = session_in(&dir);
    block_part(&mut s, "parts/block.3dcraft", [3.0, 2.0, 1.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "parts/block.3dcraft", "name": "A" }),
    );
    run(&mut s, "file.save", json!({ "path": "top.3dcraftasm" }));
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "A", "position": [5, 0, 0] }),
    );
    let failed = s.execute("file.save", &json!({ "path": "blocked/top.3dcraftasm" }));
    assert!(failed.is_err(), "{failed:?}");
    let i = inspect(&mut s);
    assert_eq!(instance(&i, "A")["status"], "ok", "{i}");
    assert_eq!(instance(&i, "A")["source"]["path"], "parts/block.3dcraft");
    // Save As into another folder, then undo the move.
    run(&mut s, "file.save", json!({ "path": "sub/top.3dcraftasm" }));
    let i = inspect(&mut s);
    assert_eq!(instance(&i, "A")["status"], "ok", "{i}");
    assert_eq!(
        instance(&i, "A")["source"]["path"],
        "../parts/block.3dcraft"
    );
    run(&mut s, "edit.undo", json!({}));
    let i = inspect(&mut s);
    assert_eq!(instance(&i, "A")["status"], "ok", "{i}");
    assert_eq!(
        instance(&i, "A")["source"]["path"],
        "../parts/block.3dcraft"
    );
    assert!(close(boxed(&i, "A").0[0], 0.0, 1.0), "{i}");
    run(&mut s, "edit.redo", json!({}));
    let i = inspect(&mut s);
    assert_eq!(instance(&i, "A")["status"], "ok", "{i}");
    assert!(close(boxed(&i, "A").0[0], 5.0, 1.0), "{i}");
}

/// A coupling that cannot take part (a mate it names failed, or its value
/// does not evaluate) fails by name, and the solve is not ok.
#[test]
fn a_coupling_that_cannot_take_part_fails_by_name() {
    let dir = workdir("assembly", "couplings");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [10.0, 2.0, 2.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    for name in ["Base", "Arm", "Arm2"] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": name }),
        );
    }
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "Base/F1:face(0)" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "Base/F1:face(1)", "b": "Arm/F1:face(0)", "name": "Hinge" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "Arm/F1:face(1)", "b": "Arm2/F1:face(0)", "name": "Hinge2" }),
    );
    let broken = run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "Base/F1:face(1)", "b": "Arm/F1:face(77)", "name": "Broken" }),
    );
    assert_eq!(broken["status"], "failed", "{broken}");
    let added = run(
        &mut s,
        "coupling.add",
        json!({ "kind": "gear", "mates": ["Hinge", "Broken"], "value": 2, "name": "Lost" }),
    );
    assert_eq!(added["status"], "failed", "{added}");
    let why = added["error"].as_str().unwrap();
    assert!(
        why.contains("Broken") && why.contains("face(77)"),
        "{added}"
    );
    run(
        &mut s,
        "coupling.add",
        json!({ "kind": "gear", "mates": ["Hinge", "Hinge2"], "value": "nosuch * 2", "name": "Unvalued" }),
    );
    let solved = run(&mut s, "assembly.solve", json!({}));
    assert_eq!(solved["ok"], false, "{solved}");
    let failed = solved["failed"].as_array().unwrap();
    for name in ["Lost", "Unvalued"] {
        assert!(
            failed.iter().any(|f| f["coupling"] == name),
            "{name}: {solved}"
        );
    }
    let i = inspect(&mut s);
    let unvalued = i["couplings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Unvalued")
        .unwrap();
    assert_eq!(unvalued["status"], "failed", "{i}");
    assert!(
        unvalued["error"].as_str().unwrap().contains("nosuch"),
        "{i}"
    );
}

/// A pinned instance keeps the bytes it is pinned to in the assembly: it
/// shows them after its file changed, `assembly.pack` packs them (never
/// the file's new bytes), and the saved assembly shows them without the
/// file.
#[test]
fn a_pinned_instance_keeps_its_bytes_through_a_change_and_a_pack() {
    let dir = workdir("assembly", "pin");
    let mut s = session_in(&dir);
    block_part(&mut s, "part.3dcraft", [2.0, 2.0, 2.0], 1e-6);
    block_part(&mut s, "big.3dcraft", [8.0, 8.0, 8.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    let pinned = run(
        &mut s,
        "instance.add",
        json!({ "source": "part.3dcraft", "name": "P", "pinned": true }),
    );
    let hash = pinned["pinned"].clone();
    assert!(hash.is_string(), "{pinned}");
    run(
        &mut s,
        "instance.add",
        json!({ "source": "part.3dcraft", "name": "F", "position": [20, 0, 0] }),
    );
    let i = inspect(&mut s);
    assert_eq!(i["packed"][0]["hash"], hash, "{i}");
    std::fs::copy(dir.join("big.3dcraft"), dir.join("part.3dcraft")).unwrap();
    let reloaded = run(&mut s, "assembly.reload", json!({}));
    assert_eq!(reloaded["failed"], json!([]), "{reloaded}");
    let i = inspect(&mut s);
    assert!(close(width(&i, "P"), 2.0, 1.0), "{i}");
    assert!(close(width(&i, "F"), 8.0, 1.0), "{i}");
    run(&mut s, "assembly.pack", json!({}));
    let i = inspect(&mut s);
    assert_eq!(instance(&i, "P")["source"]["hash"], hash, "{i}");
    assert_eq!(instance(&i, "P")["pinned"], hash, "{i}");
    assert!(close(width(&i, "P"), 2.0, 1.0), "{i}");
    assert!(close(width(&i, "F"), 8.0, 1.0), "{i}");
    run(&mut s, "file.save", json!({ "path": "pinned.3dcraftasm" }));
    run(&mut s, "file.close", json!({}));
    std::fs::remove_file(dir.join("part.3dcraft")).unwrap();
    run(&mut s, "file.open", json!({ "path": "pinned.3dcraftasm" }));
    let i = inspect(&mut s);
    assert!(close(width(&i, "P"), 2.0, 1.0), "{i}");
    assert!(close(width(&i, "F"), 8.0, 1.0), "{i}");
}

/// Assertions follow an instance's rename; an instance an assertion names
/// is not deleted, so a later rename can never hand its checks to
/// another instance.
#[test]
fn a_rename_takes_the_checks_along_and_never_hands_them_on() {
    let dir = workdir("assembly", "rename");
    let mut s = session_in(&dir);
    block_part(&mut s, "cube.3dcraft", [10.0, 10.0, 10.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    for (name, x) in [("A", 0), ("B", 15), ("C", 40)] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "cube.3dcraft", "name": name, "position": [x, 0, 0] }),
        );
    }
    let gap = run(
        &mut s,
        "assert.add",
        json!({ "name": "gap", "measure": "clearance", "from": "A", "to": "B", "min": "1 mm" }),
    );
    assert!(close(gap["measured"].as_f64().unwrap(), 5.0, 1.0), "{gap}");
    let refused = error(&mut s, "instance.delete", json!({ "name": "B" }));
    assert!(refused.contains("gap"), "{refused}");
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "A", "rename": "Left" }),
    );
    // C takes the old name: the check stays on Left.
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "C", "rename": "A" }),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    let gap = &listed["assertions"][0];
    assert_eq!(gap["status"], "passed", "{listed}");
    assert!(
        close(gap["measured"].as_f64().unwrap(), 5.0, 1.0),
        "{listed}"
    );
}

/// A mate on a body the source hides fails by name (the instance shows
/// only the visible bodies), unless the instance picks that body.
#[test]
fn a_mate_on_a_hidden_body_of_the_source_fails_unless_it_is_picked() {
    let dir = workdir("assembly", "hidden");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Shown", "size": [2, 2, 2] }),
    );
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Kept", "size": [1, 1, 1], "position": [5, 0, 0] }),
    );
    run(
        &mut s,
        "feature.visible",
        json!({ "feature": "Kept", "visible": false }),
    );
    run(&mut s, "file.save", json!({ "path": "two.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "two.3dcraft", "name": "All" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "two.3dcraft", "name": "Picked", "body": "Kept" }),
    );
    let hidden = run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "All/F2:face(0)", "name": "OnHidden" }),
    );
    assert_eq!(hidden["status"], "failed", "{hidden}");
    assert!(
        hidden["error"].as_str().unwrap().contains("hides"),
        "{hidden}"
    );
    let picked = run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "Picked/F2:face(0)", "name": "OnPicked" }),
    );
    assert_eq!(picked["status"], "ok", "{picked}");
}

/// The window's case: an assembly opened with unrestricted access, then
/// the workspace narrowed to a root, the display scene refined
/// elsewhere; the Preview and Display scenes it is drawn from follow
/// joint.set, directly and built a piece at a time.
#[test]
fn the_shown_scenes_follow_the_joints_after_the_workspace_narrows() {
    use craft3d_engine::{Access, SceneQuality, Workspace};
    let dir = workdir("assembly", "window");
    {
        let mut s = session_in(&dir);
        block_part(&mut s, "block.3dcraft", [10.0, 2.0, 2.0], 1e-6);
        run(&mut s, "assembly.new", json!({}));
        for name in ["Base", "Arm"] {
            run(
                &mut s,
                "instance.add",
                json!({ "source": "block.3dcraft", "name": name }),
            );
        }
        run(
            &mut s,
            "mate.add",
            json!({ "kind": "fixed", "a": "world/xy", "b": "Base/F1:face(0)" }),
        );
        run(
            &mut s,
            "mate.add",
            json!({ "kind": "revolute", "a": "Base/F1:face(1)", "b": "Arm/F1:face(0)", "name": "Hinge" }),
        );
        run(&mut s, "file.save", json!({ "path": "arm.3dcraftasm" }));
    }
    for stepwise in [false, true] {
        let mut s = Session::new(Workspace {
            read: Access::Unrestricted,
            write: Access::Unrestricted,
        });
        let absolute = dir.join("arm.3dcraftasm").to_string_lossy().to_string();
        run(&mut s, "file.open", json!({ "path": absolute }));
        s.set_workspace(Workspace {
            read: Access::Root(dir.clone()),
            write: Access::Root(dir.clone()),
        });
        s.set_refine_elsewhere(true);
        s.set_stepwise(stepwise);
        let mut boxes = Vec::new();
        for angle in ["0 deg", "90 deg"] {
            run(
                &mut s,
                "joint.set",
                json!({ "mate": "Hinge", "value": angle }),
            );
            for _ in 0..1000 {
                if s.prepare_some(SceneQuality::display(), || false).is_none() {
                    break;
                }
            }
            for quality in [SceneQuality::Preview, SceneQuality::display()] {
                let scene = s.scene_with(quality).unwrap().expect("a scene");
                assert_eq!(scene.bodies.len(), 2, "{angle} {quality:?}");
                let (lo, hi) = scene.bbox.corners().expect("a box");
                boxes.push((quality, lo, hi));
            }
        }
        let i = inspect(&mut s);
        assert_eq!(i["solve"]["ok"], true, "{i}");
        for k in 0..2 {
            let (q, lo0, hi0) = boxes[k];
            let (_, lo1, hi1) = boxes[k + 2];
            assert!(
                (lo0.x - lo1.x).abs() > 1.0
                    || (lo0.y - lo1.y).abs() > 1.0
                    || (hi0.x - hi1.x).abs() > 1.0
                    || (hi0.y - hi1.y).abs() > 1.0,
                "stepwise {stepwise}, {q:?}: the drawn box did not move: {boxes:?}"
            );
        }
    }
}

/// An assembly shows its placed parts; with a timeline it plays over `t`
/// (motion of assemblies, docs/design/motion.md › *Assemblies*), and
/// what `document.inspect` sees stays the placed instance at rest.
#[test]
fn an_assembly_shows_its_placed_parts_and_plays_over_t() {
    let dir = workdir("assembly", "no-motion");
    let mut s = session_in(&dir);
    block_part(&mut s, "base.3dcraft", [10.0, 4.0, 2.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    run(&mut s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "base.3dcraft", "name": "B1" }),
    );
    let shown = run(&mut s, "view.motion", json!({ "t": 0.5 }));
    assert_eq!(shown["t"], 0.5, "{shown}");
    assert!(s.timeline().is_ok());
    let shown = run(&mut s, "document.inspect", json!({}));
    let bodies = shown["bodies"].as_array().unwrap();
    assert_eq!(bodies.len(), 1, "{shown}");
    assert!(
        close(bodies[0]["volume"].as_f64().unwrap(), 80.0, 1.0),
        "{shown}"
    );
}

/// A selection on an assembly stays on its instance (rule 4): the placed
/// part numbers its features in instance order, so deleting an instance
/// before it gives its feature id to another instance; the item is found
/// by its instance, and an item of an instance that is gone drops out,
/// said in dropped, never moving to another instance.
#[test]
fn a_selection_on_an_assembly_stays_on_its_instance() {
    let dir = workdir("assembly", "selection-instance");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [10.0, 4.0, 2.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    for (name, x) in [("A", 0), ("B", 20), ("C", 40)] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": name, "position": [x, 0, 0] }),
        );
    }
    let set = run(
        &mut s,
        "select.set",
        json!({ "items": [
            { "kind": "body", "name": "B" },
            { "kind": "face", "name": "F1:face(0)", "feature": "C" },
        ] }),
    );
    assert_eq!(set["count"], 2, "{set}");
    run(&mut s, "instance.delete", json!({ "name": "A" }));
    let got = run(&mut s, "select.get", json!({ "context": false }));
    let items = got["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{got}");
    assert_eq!(items[0]["kind"], "body", "{got}");
    assert_eq!(items[0]["name"], "B", "{got}");
    // C's face is on C's placed feature, whatever its id is now.
    let inspected = inspect(&mut s);
    let c_features = instance(&inspected, "C")["bodies"].clone();
    assert!(
        c_features
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == &items[1]["feature"]),
        "{got} {c_features}"
    );
    assert_eq!(items[1]["name"], "F1:face(0)", "{got}");
    // An instance that is gone takes its item with it.
    run(&mut s, "instance.delete", json!({ "name": "B" }));
    let got = run(&mut s, "select.get", json!({ "context": false }));
    let items = got["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{got}");
    assert_eq!(items[0]["kind"], "face", "{got}");
    let dropped = got["dropped"].to_string();
    assert!(dropped.contains("`B`"), "{got}");
}

/// The path of `to` relative to the directory `from` (both absolute).
fn relative_path(from: &std::path::Path, to: &std::path::Path) -> std::path::PathBuf {
    let a: Vec<_> = from.components().collect();
    let b: Vec<_> = to.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = std::path::PathBuf::new();
    for _ in common..a.len() {
        out.push("..");
    }
    for part in &b[common..] {
        out.push(part);
    }
    out
}

/// The window found on 2026-10-30: roots given relative
/// (`--automation-read-root target/work`) and an assembly kept by its
/// absolute path (opened or saved by the person) take a source named
/// relative to the root, as an instance's path relative to the
/// assembly's folder; the assembly then reads it by its absolute path
/// beneath the relative root.
#[test]
fn a_source_beneath_a_relative_root_joins_an_assembly_kept_by_its_absolute_path() {
    let dir = workdir("assembly", "relative-root");
    std::fs::create_dir_all(dir.join("demo")).unwrap();
    let cwd = std::env::current_dir().unwrap();
    let relative_root = relative_path(&cwd, &dir);
    assert!(relative_root.is_relative(), "{}", relative_root.display());
    let mut s = Session::new(craft3d_engine::Workspace {
        read: craft3d_engine::Access::Root(relative_root.clone()),
        write: craft3d_engine::Access::Root(relative_root),
    });
    block_part(&mut s, "demo/parts.3dcraft", [10.0, 10.0, 10.0], 1e-6);
    block_part(&mut s, "demo/other.3dcraft", [20.0, 10.0, 10.0], 1e-6);
    run(&mut s, "assembly.new", json!({}));
    let absolute = craft3d_engine::canonical(&dir.join("demo"))
        .unwrap()
        .join("planetary.3dcraftasm");
    run(
        &mut s,
        "file.save",
        json!({ "path": absolute.to_str().unwrap() }),
    );
    let added = run(
        &mut s,
        "instance.add",
        json!({ "source": "demo/parts.3dcraft", "name": "Ring", "body": "Base" }),
    );
    assert_eq!(added["name"], "Ring", "{added}");
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "Ring", "source": "demo/other.3dcraft" }),
    );
    let i = inspect(&mut s);
    let ring = instance(&i, "Ring");
    assert_eq!(ring["status"], "ok", "{ring}");
    // Kept relative to the assembly's folder, beside it.
    assert_eq!(ring["source"]["path"], "other.3dcraft", "{ring}");
    assert!(close(volume(&i, "Ring"), 2000.0, 2000.0), "{ring}");
}

/// A plate 30 × 20 × 4 whose fillet `Plate` has the published radius
/// `corner` (0.5 to 30 mm, 1 mm by default), saved at `plate.3dcraft`.
fn filleted_plate(s: &mut Session) {
    run(s, "file.new", json!({ "units": "mm" }));
    run(
        s,
        "variable.set",
        json!({ "name": "corner", "expr": "1 mm" }),
    );
    run(
        s,
        "variable.publish",
        json!({ "name": "corner", "min": "0.5 mm", "max": "30 mm" }),
    );
    run(
        s,
        "feature.block",
        json!({ "name": "Blank", "size": ["30 mm", "20 mm", "4 mm"] }),
    );
    run(
        s,
        "feature.fillet",
        json!({ "name": "Plate", "edges": [["F1:face(0)", "F1:face(2)"]], "radius": "corner" }),
    );
    run(s, "file.save", json!({ "path": "plate.3dcraft" }));
    run(s, "file.close", json!({}));
}

/// A-023: an instance whose configuration has a failed feature is placed
/// with the bodies from before it, as the part shows them, but carries
/// the failure by name (the part's own message) and the assembly's check
/// is not ok, also through a subassembly. A valid value is ok.
#[test]
fn an_instance_whose_part_has_a_failed_feature_says_so_and_is_still_placed() {
    let dir = workdir("assembly", "failed-feature");
    let mut s = session_in(&dir);
    filleted_plate(&mut s);
    // The part's own message for a radius of 12 on a plate 4 thick.
    run(&mut s, "file.open", json!({ "path": "plate.3dcraft" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "corner", "expr": "12 mm" }),
    );
    let alone = run(&mut s, "document.check", json!({}));
    assert_eq!(alone["ok"], false, "{alone}");
    let own = alone["problems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "Plate")
        .unwrap_or_else(|| panic!("the part's check names its fillet: {alone}"))["problem"]
        .as_str()
        .unwrap()
        .to_string();
    run(&mut s, "file.close", json!({ "discard": true }));
    let assemble = |s: &mut Session, corner: &str| {
        run(s, "assembly.new", json!({ "units": "mm" }));
        run(
            s,
            "instance.add",
            json!({ "source": "plate.3dcraft", "name": "Plate", "params": { "corner": corner } }),
        );
        run(
            s,
            "mate.add",
            json!({ "kind": "fixed", "name": "Ground", "a": "world/xy", "b": "Plate/F1:face(1)", "flip": true }),
        );
    };
    assemble(&mut s, "12 mm");
    let inspected = inspect(&mut s);
    let row = instance(&inspected, "Plate");
    // Placed: not an error, and its body is the blank's, 30 · 20 · 4.
    assert_eq!(row["status"], "ok", "{row}");
    assert_eq!(row["error"], Value::Null, "{row}");
    assert_eq!(row["bodies"].as_array().unwrap().len(), 1, "{row}");
    assert!(close(volume(&inspected, "Plate"), 2400.0, 2400.0), "{row}");
    let failed = row["featureFailed"]
        .as_str()
        .unwrap_or_else(|| panic!("{row}"));
    assert!(failed.contains("`Plate` (F2)"), "{failed}");
    assert!(
        failed.ends_with(&own),
        "the part's own message: {failed} / {own}"
    );
    let checked = run(&mut s, "document.check", json!({}));
    assert_eq!(checked["ok"], false, "{checked}");
    let problem = &checked["problems"][0];
    assert_eq!(problem["instance"], "Plate", "{checked}");
    let text = problem["problem"].as_str().unwrap();
    assert!(
        text.contains("instance `Plate`") && text.contains("`Plate` (F2)"),
        "{text}"
    );
    run(&mut s, "file.save", json!({ "path": "inner.3dcraftasm" }));
    run(&mut s, "file.close", json!({}));
    // Through a subassembly: the top's instance names the inner one.
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "inner.3dcraftasm", "name": "Sub" }),
    );
    let inspected = inspect(&mut s);
    let row = instance(&inspected, "Sub");
    assert_eq!(row["status"], "ok", "{row}");
    let failed = row["featureFailed"]
        .as_str()
        .unwrap_or_else(|| panic!("{row}"));
    assert!(
        failed.starts_with("its instance `Plate`: ") && failed.contains("`Plate` (F2)"),
        "{failed}"
    );
    let checked = run(&mut s, "document.check", json!({}));
    assert_eq!(checked["ok"], false, "{checked}");
    assert_eq!(checked["problems"][0]["instance"], "Sub", "{checked}");
    run(&mut s, "file.close", json!({ "discard": true }));
    // A value the part takes: nothing failed, the check is ok.
    assemble(&mut s, "1 mm");
    let inspected = inspect(&mut s);
    let row = instance(&inspected, "Plate");
    assert_eq!(row["featureFailed"], Value::Null, "{row}");
    let checked = run(&mut s, "document.check", json!({}));
    assert_eq!(checked["ok"], true, "{checked}");
}

/// A-011: a value an assembly gives an instance is held to the part's
/// range, not to its step: 7√2 mm on a step of 0.1 mm places the bar at
/// that length; 20 mm, past its maximum of 15, is still refused; and
/// param.set in the part itself is still held to the step.
#[test]
fn an_instance_value_is_held_to_the_range_but_not_to_the_step() {
    let dir = workdir("assembly", "off-step");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "length", "expr": "10 mm" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "length", "min": "5 mm", "max": "15 mm", "step": "0.1 mm" }),
    );
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Bar", "size": ["length", "5 mm", "5 mm"] }),
    );
    let e = error(
        &mut s,
        "param.set",
        json!({ "values": { "length": "10.05 mm" } }),
    );
    assert!(e.contains("not on its step"), "{e}");
    run(&mut s, "file.save", json!({ "path": "stepped.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "computed", "expr": "sqrt(2) * 7 mm" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "stepped.3dcraft", "name": "Bar", "params": { "length": "computed" } }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "stepped.3dcraft", "name": "Long", "params": { "length": "20 mm" } }),
    );
    let inspected = inspect(&mut s);
    let bar = instance(&inspected, "Bar");
    assert_eq!(bar["status"], "ok", "{bar}");
    let (lo, hi) = boxed(&inspected, "Bar");
    let want = 7.0 * 2.0_f64.sqrt();
    assert!(close(hi[0] - lo[0], want, want), "{lo:?} {hi:?}");
    let long = instance(&inspected, "Long");
    assert_eq!(long["status"], "failed", "{long}");
    let why = long["error"].as_str().unwrap();
    assert!(why.contains("15 mm"), "the range is said: {why}");
}

/// The status and measured value of the assertion `name`.
fn asserted(s: &mut Session, name: &str) -> Value {
    let list = run(s, "assert.list", json!({}));
    list["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("no assertion {name} in {list}"))
        .clone()
}

/// A-010, A-031, A-007: one rule for naming a subject in an assembly. An
/// instance's name means all its bodies (a part of two bodies, a rigid
/// subassembly); `Instance/Body` names a body also when the part has one;
/// `Sub/Inner` and `Sub/Inner/Body` reach into a subassembly; and the pose
/// measures reach an instance inside a rigid subassembly by its path,
/// its pose that subassembly's composed with its own.
#[test]
fn instances_bodies_and_subassemblies_are_named_by_one_rule() {
    let dir = workdir("assembly", "subject-names");
    let mut s = session_in(&dir);
    // Two blocks 10 × 10 × 5 at x 0..10 and 20..30.
    run(&mut s, "file.new", json!({ "units": "mm" }));
    for (name, x) in [("First body", 0), ("Second body", 20)] {
        run(
            &mut s,
            "feature.block",
            json!({ "name": name, "size": [10, 10, 5], "position": [x, 0, 0] }),
        );
    }
    run(&mut s, "file.save", json!({ "path": "two.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    // A block 20 × 20 × 20.
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Block", "size": [20, 20, 20] }),
    );
    run(&mut s, "file.save", json!({ "path": "block.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    // A subassembly of two blocks: Inner at (5, 0, 0) turned 10° about z,
    // Twin at (100, 0, 0).
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Inner", "position": [5, 0, 0], "rotation": [0, 0, 10] }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Twin", "position": [100, 0, 0] }),
    );
    run(&mut s, "file.save", json!({ "path": "sub.3dcraftasm" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "two.3dcraft", "name": "Handle" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "two.3dcraft", "name": "Other", "position": [0, 0, 20] }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Cube", "position": [0, -100, 0] }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "sub.3dcraftasm", "name": "Sub", "position": [0, 100, 0], "rotation": [0, 0, 30] }),
    );
    let assert = |s: &mut Session, name: &str, measure: Value| {
        let mut measure = measure;
        measure["name"] = json!(name);
        run(s, "assert.add", measure);
        asserted(s, name)
    };
    // A-010: the instance of two bodies by its name, 15 above the other
    // (z 0..5 and 20..25); one of its bodies by its path, as before.
    let a = assert(
        &mut s,
        "handle",
        json!({ "measure": "clearance", "from": "Handle", "to": "Other", "min": "1 mm" }),
    );
    assert_eq!(a["status"], "passed", "{a}");
    assert!(close(a["measured"].as_f64().unwrap(), 15.0, 15.0), "{a}");
    let a = assert(
        &mut s,
        "first",
        json!({ "measure": "volume", "of": "Handle/First body", "eq": "500" }),
    );
    assert_eq!(a["status"], "passed", "{a}");
    let a = assert(
        &mut s,
        "both",
        json!({ "measure": "volume", "of": "Handle", "eq": "1000" }),
    );
    assert_eq!(a["status"], "passed", "{a}");
    // A part of one body by `Instance/Body`, as by its name.
    for (name, of) in [("cube", "Cube"), ("cube body", "Cube/Block")] {
        let a = assert(
            &mut s,
            name,
            json!({ "measure": "volume", "of": of, "eq": "8000" }),
        );
        assert_eq!(a["status"], "passed", "{of}: {a}");
    }
    // A subassembly by its name as `of` (both blocks) and as `from`; its
    // instances by `Sub/Inner` and `Sub/Inner/Block`.
    for (name, of, volume) in [
        ("sub", "Sub", "16000"),
        ("inner", "Sub/Inner", "8000"),
        ("inner body", "Sub/Inner/Block", "8000"),
    ] {
        let a = assert(
            &mut s,
            name,
            json!({ "measure": "volume", "of": of, "eq": volume }),
        );
        assert_eq!(a["status"], "passed", "{of}: {a}");
    }
    let a = assert(
        &mut s,
        "sub apart",
        json!({ "measure": "clearance", "from": "Sub", "to": "Cube", "min": "1 mm" }),
    );
    assert_eq!(a["status"], "passed", "{a}");
    // A-031: the pose of an instance in the rigid subassembly: turned
    // 30° + 10° about z, its origin at (0, 100, 0) + R(30°) (5, 0, 0).
    let a = assert(
        &mut s,
        "turned",
        json!({ "measure": "rotation", "instance": "Sub/Inner", "axis": "z", "eq": "40 deg", "tol": "1e-6 deg" }),
    );
    assert_eq!(a["status"], "passed", "{a}");
    let (sin, cos) = 30.0_f64.to_radians().psin_cos();
    let pose = run(&mut s, "measure.pose", json!({ "instance": "Sub/Inner" }));
    assert!(close(pose["angle"].as_f64().unwrap(), 40.0, 40.0), "{pose}");
    let at = &pose["position"];
    assert!(close(at[0].as_f64().unwrap(), 5.0 * cos, 5.0), "{pose}");
    assert!(
        close(at[1].as_f64().unwrap(), 100.0 + 5.0 * sin, 100.0),
        "{pose}"
    );
    // The subassembly's own name answers as before.
    let a = assert(
        &mut s,
        "sub turned",
        json!({ "measure": "rotation", "instance": "Sub", "axis": "z", "eq": "30 deg", "tol": "1e-6 deg" }),
    );
    assert_eq!(a["status"], "passed", "{a}");
    // A name of nothing is still an error.
    let a = assert(
        &mut s,
        "nobody",
        json!({ "measure": "volume", "of": "Sub/Nobody", "eq": "1" }),
    );
    assert_eq!(a["status"], "error", "{a}");
    // The query commands measure the placed bodies by the same names.
    let v = run(
        &mut s,
        "measure.clearance",
        json!({ "from": "Handle", "to": "Other" }),
    );
    assert!(close(v["clearance"].as_f64().unwrap(), 15.0, 15.0), "{v}");
}

/// Review of A-010 (M1): an assertion and an output on a name that is no
/// shown body's (an instance of two bodies, a one-body part's
/// `Instance/Body`, a subassembly, a path into one) are measured again
/// when the instance moves, as one on a shown body's name is: the key of
/// their answer holds the placed bodies the name resolves to.
#[test]
fn assertions_and_outputs_on_instance_names_follow_a_moved_instance() {
    let dir = workdir("assembly", "moved-subjects");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({ "units": "mm" }));
    for (name, x) in [("First", 0), ("Second", 20)] {
        run(
            &mut s,
            "feature.block",
            json!({ "name": name, "size": [10, 10, 5], "position": [x, 0, 0] }),
        );
    }
    run(&mut s, "file.save", json!({ "path": "two.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Block", "size": [20, 20, 20] }),
    );
    run(&mut s, "file.save", json!({ "path": "block.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Inner" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Twin", "position": [100, 0, 0] }),
    );
    run(&mut s, "file.save", json!({ "path": "sub.3dcraftasm" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    for (source, name) in [
        ("two.3dcraft", "Other"),
        ("block.3dcraft", "Cube"),
        ("sub.3dcraftasm", "Sub"),
    ] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": source, "name": name, "position": [0, 0, 20] }),
        );
    }
    let names = ["Other", "Cube/Block", "Sub", "Sub/Inner/Block"];
    for (k, of) in names.iter().enumerate() {
        run(
            &mut s,
            "assert.add",
            json!({ "name": format!("low{k}"), "measure": "boxMin", "of": of, "axis": "z", "eq": "20 mm" }),
        );
        run(
            &mut s,
            "variable.measure",
            json!({ "name": format!("z{k}"), "measure": "boxMin", "of": of, "axis": "z" }),
        );
    }
    let check = |s: &mut Session, z: f64| {
        let listed = run(s, "assert.list", json!({}));
        let outputs = run(s, "output.list", json!({}));
        for k in 0..names.len() {
            let a = listed["assertions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["name"] == format!("low{k}"))
                .unwrap()
                .clone();
            let want = if close(z, 20.0, 20.0) {
                "passed"
            } else {
                "failed"
            };
            assert_eq!(a["status"], want, "{} at z = {z}: {a}", names[k]);
            assert!(close(a["measured"].as_f64().unwrap(), z, 50.0), "{a}");
            let o = outputs["outputs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["name"] == format!("z{k}"))
                .unwrap()
                .clone();
            assert!(
                close(o["value"].as_f64().unwrap(), z, 50.0),
                "{} at z = {z}: {o}",
                names[k]
            );
        }
    };
    check(&mut s, 20.0);
    for z in [2.0, 50.0, 20.0] {
        for name in ["Other", "Cube", "Sub"] {
            run(
                &mut s,
                "instance.edit",
                json!({ "name": name, "position": [0, 0, z] }),
            );
        }
        check(&mut s, z);
    }
}
