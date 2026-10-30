//! The assembly gaps closed before the first release
//! (docs/design/assemblies.md): world references checked when a mate is
//! made, linear couplings of any number of motions, the solved values of
//! joints and poses as measures (and so in formulas through outputs), an
//! overlap of many bodies through a broad phase of boxes, an unsaved
//! assembly with access to every file, and a flexible subassembly's own
//! drives and couplings in its parent. Held to closed forms.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{error, session_in, workdir};
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

/// Saves a block `Base` of `size` with a datum axis `Axis` along z through
/// the origin at `path`, and closes it.
fn block_part(s: &mut Session, path: &str, size: [f64; 3]) {
    run(s, "file.new", json!({}));
    run(s, "feature.block", json!({ "name": "Base", "size": size }));
    run(
        s,
        "datum.axis",
        json!({ "name": "Axis", "through": [[0, 0, 0], [0, 0, 1]] }),
    );
    run(s, "file.save", json!({ "path": path }));
    run(s, "file.close", json!({}));
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

/// The status of the assertion `name`.
fn assertion(s: &mut Session, name: &str) -> Value {
    let list = run(s, "assert.list", json!({}));
    list["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no assertion {name} in {list}"))
}

/// An assembly of `names`, each an instance of `block.3dcraft` on an axis
/// of its own: `<name>` turns about the world's z (revolute `R<name>`).
fn turning(s: &mut Session, names: &[&str]) {
    run(s, "assembly.new", json!({}));
    for name in names {
        run(
            s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": name }),
        );
        run(
            s,
            "mate.add",
            json!({ "kind": "revolute", "a": "world/z", "b": format!("{name}/Axis"),
                    "name": format!("R{name}"), "flip": true }),
        );
    }
}

/// A mate to a frame the world does not have is refused when it is made
/// or edited, by name, and nothing is added.
#[test]
fn a_world_reference_is_checked_when_the_mate_is_made() {
    let dir = workdir("asm-gaps", "world");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [10.0, 2.0, 2.0]);
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "A" }),
    );
    let e = error(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/-x", "b": "A/Axis" }),
    );
    assert!(
        e.contains("the world has no `-x`") && e.contains("world/zx"),
        "{e}"
    );
    let solved = run(&mut s, "assembly.solve", json!({}));
    assert_eq!(solved["mates"].as_array().unwrap().len(), 0, "{solved}");
    for good in ["xy", "yz", "zx", "x", "y", "z", "origin"] {
        let m = run(
            &mut s,
            "mate.add",
            json!({ "kind": "fixed", "a": format!("world/{good}"), "b": "A/Axis", "name": "M" }),
        );
        assert_eq!(m["status"], "ok", "{good}: {m}");
        let e = error(
            &mut s,
            "mate.edit",
            json!({ "name": "M", "b": "world/upward" }),
        );
        assert!(e.contains("the world has no `upward`"), "{e}");
        run(&mut s, "mate.delete", json!({ "name": "M" }));
    }
}

/// A linear coupling ties any number of motions: two sliders (one moves
/// twice the other), a rack of a slider and a revolute in mixed units, and
/// three revolutes as a planetary stage by Willis' formula; refusals by
/// name; the terms survive a save, a rename and a reopen.
#[test]
fn a_linear_coupling_ties_any_number_of_motions() {
    let dir = workdir("asm-gaps", "linear");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [4.0, 4.0, 2.0]);

    // Two sliders: dA = 2 dB.
    run(&mut s, "assembly.new", json!({}));
    for (name, axis) in [("A", "world/x"), ("B", "world/y")] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": name }),
        );
        run(
            &mut s,
            "mate.add",
            json!({ "kind": "slider", "a": axis, "b": format!("{name}/Axis"),
                    "name": format!("S{name}"), "flip": true }),
        );
    }
    let c = run(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "name": "Twice",
                "terms": [{ "mate": "SA" }, { "mate": "SB", "motion": "distance", "coefficient": "-2" }] }),
    );
    assert_eq!(c["status"], "ok", "{c}");
    assert_eq!(c["mates"], json!(["SA", "SB"]), "{c}");
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "SB", "value": "5 mm" }),
    );
    let j = run(&mut s, "measure.joint", json!({ "mate": "SA" }));
    assert!(close(number(&j["values"]["distance"]), 10.0, 1e-6), "{j}");
    let solved = run(&mut s, "assembly.solve", json!({}));
    assert_eq!(
        (&solved["ok"], &solved["dof"]),
        (&json!(true), &json!(0)),
        "{solved}"
    );

    // Refusals, each by name, the assembly unchanged.
    let e = error(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "mates": ["SA", "SB"], "terms": [{ "mate": "SA" }] }),
    );
    assert!(e.contains("in `terms`"), "{e}");
    let e = error(
        &mut s,
        "coupling.add",
        json!({ "kind": "gear", "mates": ["SA", "SB"], "value": 1, "terms": [] }),
    );
    assert!(e.contains("only a linear coupling"), "{e}");
    let e = error(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "terms": [{ "mate": "SA", "turns": 1 }] }),
    );
    assert!(e.contains("`turns`"), "{e}");
    let e = error(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "terms": [{ "mate": "SA", "motion": "twist" }] }),
    );
    assert!(e.contains("`twist` is not a free motion"), "{e}");
    let e = error(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "terms": [] }),
    );
    assert!(e.contains("1 to 64 terms"), "{e}");
    let e = error(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "terms": [{ "mate": "Nobody" }] }),
    );
    assert!(e.contains("Nobody"), "{e}");
    // Kept but failed by name: a motion the mate does not have, terms
    // that do not agree in dimension.
    let c = run(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "name": "NoAngle", "terms": [{ "mate": "SA", "motion": "angle" }] }),
    );
    assert_eq!(c["status"], "failed", "{c}");
    assert!(
        c["error"]
            .as_str()
            .unwrap()
            .contains("has no free motion `angle`"),
        "{c}"
    );
    run(&mut s, "coupling.delete", json!({ "name": "NoAngle" }));
    let c = run(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "name": "Mixed",
                "terms": [{ "mate": "SA" }, { "mate": "SB", "coefficient": "2 mm" }] }),
    );
    assert_eq!(c["status"], "failed", "{c}");
    assert!(
        c["error"]
            .as_str()
            .unwrap()
            .contains("not of the dimension"),
        "{c}"
    );
    run(&mut s, "coupling.delete", json!({ "name": "Mixed" }));
    // A coefficient of a mass or a time leaves the lengths alike and is
    // still another dimension: refused.
    for unit in ["2 kg", "3 s"] {
        let c = run(
            &mut s,
            "coupling.add",
            json!({ "kind": "linear", "name": "Heavy",
                    "terms": [{ "mate": "SA" }, { "mate": "SB", "coefficient": unit }] }),
        );
        assert_eq!(c["status"], "failed", "{unit}: {c}");
        assert!(
            c["error"]
                .as_str()
                .unwrap()
                .contains("not of the dimension"),
            "{unit}: {c}"
        );
        run(&mut s, "coupling.delete", json!({ "name": "Heavy" }));
    }

    // A mate a term names is not deleted; a rename takes the term along;
    // the terms are saved and read back.
    let e = error(&mut s, "mate.delete", json!({ "name": "SB" }));
    assert!(e.contains("`Twice`"), "{e}");
    run(
        &mut s,
        "mate.edit",
        json!({ "name": "SB", "rename": "Feed" }),
    );
    run(&mut s, "file.save", json!({ "path": "sliders.3dcraftasm" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({ "path": "sliders.3dcraftasm" }));
    let i = run(&mut s, "assembly.inspect", json!({}));
    let twice = &i["couplings"][0];
    assert_eq!(twice["kind"], "linear", "{i}");
    assert_eq!(twice["terms"][1]["mate"], "Feed", "{i}");
    assert_eq!(twice["terms"][1]["coefficient"], "-2", "{i}");
    let j = run(&mut s, "measure.joint", json!({ "mate": "SA" }));
    assert!(close(number(&j["values"]["distance"]), 10.0, 1e-6), "{j}");

    // A rack in mixed units: d − 5 mm · θ = 0, θ = 90°: d = 5π/2.
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Rack" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Pinion", "position": [0, 20, 0] }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "slider", "a": "world/x", "b": "Rack/Axis", "name": "Slide", "flip": true }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "world/z", "b": "Pinion/Axis", "name": "Turn", "flip": true }),
    );
    let c = run(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "terms": [{ "mate": "Slide" }, { "mate": "Turn", "coefficient": "-5 mm" }] }),
    );
    assert_eq!(c["status"], "ok", "{c}");
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Turn", "value": "90 deg" }),
    );
    let j = run(&mut s, "measure.joint", json!({ "mate": "Slide" }));
    let want = 5.0 * std::f64::consts::FRAC_PI_2;
    assert!(
        close(number(&j["values"]["distance"]), want, 1e-6),
        "{j} {want}"
    );

    // A planetary stage: Zs θs + Zr θr − (Zs + Zr) θc = 0 (Willis), the sun
    // turned 90°, the ring held at 0: the carrier turns 90° · 12 / 48.
    turning(&mut s, &["Sun", "Ring", "Carrier"]);
    let c = run(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "name": "Stage", "value": "0 deg",
                "terms": [{ "mate": "RSun", "coefficient": "12" },
                          { "mate": "RRing", "coefficient": "36" },
                          { "mate": "RCarrier", "coefficient": "-48" }] }),
    );
    assert_eq!(c["status"], "ok", "{c}");
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "RSun", "value": "90 deg" }),
    );
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "RRing", "value": "0 deg" }),
    );
    let j = run(&mut s, "measure.joint", json!({ "mate": "RCarrier" }));
    assert!(close(number(&j["values"]["angle"]), 22.5, 1e-6), "{j}");
    let solved = run(&mut s, "assembly.solve", json!({}));
    assert_eq!(
        (&solved["ok"], &solved["dof"]),
        (&json!(true), &json!(0)),
        "{solved}"
    );
}

/// The solved value of a free motion and an instance's pose, as commands
/// and as measures of assertions and outputs, against closed forms; an
/// output of a joint's angle is used in a formula; a rename of the mate
/// follows; refusals by name.
#[test]
fn joints_and_poses_are_measured_and_readable_in_formulas() {
    let dir = workdir("asm-gaps", "measures");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [10.0, 2.0, 2.0]);
    let e = error(&mut s, "measure.joint", json!({ "mate": "Hinge" }));
    assert!(
        e.contains("no document") || e.contains("not an assembly"),
        "{e}"
    );
    run(&mut s, "file.new", json!({}));
    let e = error(&mut s, "measure.pose", json!({ "instance": "A" }));
    assert!(e.contains("not an assembly"), "{e}");

    // Driven at 30°, and free where it was placed: 45° about z, at (3, 4).
    turning(&mut s, &["Driven"]);
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Free", "position": [3, 4, 0], "rotation": [0, 0, 45] }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "cylindrical", "a": "world/z", "b": "Free/Axis", "name": "Spin", "flip": true }),
    );
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "RDriven", "value": "30 deg" }),
    );
    let j = run(&mut s, "measure.joint", json!({ "mate": "RDriven" }));
    assert!(close(number(&j["values"]["angle"]), 30.0, 1e-6), "{j}");
    assert_eq!(
        (&j["kind"], &j["driven"]),
        (&json!("revolute"), &json!(true))
    );
    let p = run(&mut s, "measure.pose", json!({ "instance": "Driven" }));
    assert!(close(number(&p["angle"]), 30.0, 1e-6), "{p}");
    assert!(close(number(&p["axis"][2]).abs(), 1.0, 1e-9), "{p}");
    for k in 0..3 {
        assert!(close(number(&p["position"][k]), 0.0, 1e-9), "{p}");
    }
    // A free motion's solved value: the cylindrical mate leaves the turn
    // where the instance was placed (both frames' x is the world's x), and
    // moves it onto the axis.
    let j = run(&mut s, "measure.joint", json!({ "mate": "Spin" }));
    assert_eq!(j["driven"], false, "{j}");
    assert!(close(number(&j["values"]["angle"]), 45.0, 1e-6), "{j}");
    assert!(close(number(&j["values"]["distance"]), 0.0, 1e-6), "{j}");
    let p = run(&mut s, "measure.pose", json!({ "instance": "Free" }));
    assert!(close(number(&p["angle"]), 45.0, 1e-6), "{p}");
    assert!(close(number(&p["position"][0]), 0.0, 1e-6), "{p}");
    assert!(close(number(&p["position"][1]), 0.0, 1e-6), "{p}");
    // A point through an instance: the far end of the driven block's axis
    // datum is its origin, on the world's z.
    let p = run(&mut s, "measure.pose", json!({ "point": "Driven/Axis" }));
    assert!(close(number(&p["position"][0]), 0.0, 1e-9), "{p}");

    // As assertions and outputs.
    run(
        &mut s,
        "assert.add",
        json!({ "name": "turned", "measure": "jointAngle", "mate": "RDriven", "eq": "30 deg", "tol": "1e-6 deg" }),
    );
    run(
        &mut s,
        "assert.add",
        json!({ "name": "round", "measure": "rotation", "instance": "Driven", "eq": "30 deg", "tol": "1e-6 deg" }),
    );
    run(
        &mut s,
        "assert.add",
        json!({ "name": "free", "measure": "jointDistance", "mate": "Spin", "min": "-1 mm", "max": "1 mm" }),
    );
    run(
        &mut s,
        "assert.add",
        json!({ "name": "there", "measure": "position", "instance": "Free", "axis": "z", "min": "-1 mm", "max": "1 mm" }),
    );
    for name in ["turned", "round", "free", "there"] {
        let a = assertion(&mut s, name);
        assert_eq!(a["status"], "passed", "{name}: {a}");
    }
    run(
        &mut s,
        "variable.measure",
        json!({ "name": "crank", "measure": "jointAngle", "mate": "RDriven" }),
    );
    run(
        &mut s,
        "variable.output",
        json!({ "name": "twice", "expr": "2 * crank" }),
    );
    let outputs = run(&mut s, "output.list", json!({}));
    let twice = outputs["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == "twice")
        .cloned()
        .unwrap();
    assert_eq!(twice["status"], "known", "{outputs}");
    // The output follows the joint.
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "RDriven", "value": "50 deg" }),
    );
    let a = assertion(&mut s, "turned");
    assert_eq!(a["status"], "failed", "{a}");
    // A rename of the mate takes the checks along; one that names it
    // keeps it from being deleted.
    run(
        &mut s,
        "mate.edit",
        json!({ "name": "RDriven", "rename": "Crank" }),
    );
    let a = assertion(&mut s, "turned");
    assert_ne!(a["status"], "error", "{a}");
    let e = error(&mut s, "mate.delete", json!({ "name": "Crank" }));
    assert!(
        e.contains("assertion `turned`") && e.contains("output `crank`"),
        "{e}"
    );

    // Refusals and errors by name.
    let e = error(&mut s, "measure.joint", json!({ "mate": "Nobody" }));
    assert!(e.contains("no mate `Nobody`"), "{e}");
    let e = error(
        &mut s,
        "measure.pose",
        json!({ "instance": "Free", "point": "Free/Axis" }),
    );
    assert!(e.contains("not both"), "{e}");
    let e = error(&mut s, "measure.pose", json!({}));
    assert!(e.contains("required"), "{e}");
    run(
        &mut s,
        "assert.add",
        json!({ "name": "slides", "measure": "jointDistance", "mate": "Crank", "min": "0 mm" }),
    );
    let a = assertion(&mut s, "slides");
    assert_eq!(a["status"], "error", "{a}");
    assert!(
        a["message"]
            .as_str()
            .unwrap()
            .contains("has no length motion"),
        "{a}"
    );
}

/// An overlap of more bodies than 500 pairs: the pairs whose boxes do not
/// meet cost nothing, and the one pair that overlaps is found; more pairs
/// that meet than one overlap checks is an error that says so, never a
/// part of them.
#[test]
fn an_overlap_of_many_bodies_checks_the_pairs_whose_boxes_meet() {
    let dir = workdir("asm-gaps", "overlap");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [2.0, 2.0, 2.0]);
    run(&mut s, "assembly.new", json!({}));
    // 40 blocks 3 apart along x (780 pairs), the last one onto the one
    // before it by half its width.
    for k in 0..40 {
        let x = if k == 39 {
            3.0 * 38.0 + 1.0
        } else {
            3.0 * f64::from(k)
        };
        run(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": format!("B{k}"), "position": [x, 0, 0] }),
        );
    }
    run(
        &mut s,
        "assert.add",
        json!({ "name": "apart", "measure": "overlap", "is": false }),
    );
    let a = assertion(&mut s, "apart");
    assert_eq!(a["status"], "failed", "{a}");
    let message = a["message"].as_str().unwrap();
    assert!(message.contains("B38") && message.contains("B39"), "{a}");
    let v = run(
        &mut s,
        "variable.measure",
        json!({ "name": "clashes", "measure": "overlap" }),
    );
    assert!(v.to_string().contains("clashes"), "{v}");
    let outputs = run(&mut s, "output.list", json!({}));
    let clashes = &outputs["outputs"][0];
    assert_eq!(clashes["status"], "known", "{outputs}");

    // 70 blocks in one place: 2415 pairs whose boxes meet.
    run(&mut s, "assembly.new", json!({}));
    for k in 0..70 {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": format!("C{k}") }),
        );
    }
    run(
        &mut s,
        "assert.add",
        json!({ "name": "apart", "measure": "overlap", "is": false }),
    );
    let a = assertion(&mut s, "apart");
    assert_eq!(a["status"], "error", "{a}");
    assert!(
        a["message"].as_str().unwrap().contains("more than 2000"),
        "{a}"
    );
}

/// With access to every file (the command line), an assembly without a
/// file places its sources from the folder they were read from, and keeps
/// doing so once it is saved.
#[test]
fn an_unsaved_assembly_with_access_to_every_file_places_its_sources() {
    let dir = workdir("asm-gaps", "unsaved");
    let mut rooted = session_in(&dir);
    block_part(&mut rooted, "block.3dcraft", [10.0, 2.0, 2.0]);
    let source = dir.join("block.3dcraft");
    let source = source.to_str().unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "assembly.new", json!({}));
    let added = run(
        &mut s,
        "instance.add",
        json!({ "source": source, "name": "A" }),
    );
    assert_eq!(added["status"], "ok", "{added}");
    let m = run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/z", "b": "A/Axis" }),
    );
    assert_eq!(m["status"], "ok", "{m}");
    let i = run(&mut s, "assembly.inspect", json!({ "measures": true }));
    assert_eq!(i["bodies"], 1, "{i}");
    let saved = dir.join("unsaved.3dcraftasm");
    run(
        &mut s,
        "file.save",
        json!({ "path": saved.to_str().unwrap() }),
    );
    let i = run(&mut s, "assembly.inspect", json!({}));
    assert_eq!(i["instances"][0]["status"], "ok", "{i}");
    assert_eq!(i["instances"][0]["source"]["path"], "block.3dcraft", "{i}");
}

/// A flexible subassembly's own drive is free in its parent unless the
/// parent sets what it depends on (a published parameter); its couplings
/// come along.
#[test]
fn a_flexible_subassembly_brings_its_couplings_and_leaves_its_drives_to_the_parent() {
    let dir = workdir("asm-gaps", "flexible");
    let mut s = session_in(&dir);
    block_part(&mut s, "block.3dcraft", [10.0, 2.0, 2.0]);
    // A leg: a base on the world, two arms on hinges coupled one to one,
    // the first driven by its published `swing`.
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
        json!({ "kind": "revolute", "a": "Base/F1:face(2)", "b": "Arm2/F1:face(0)", "name": "Hinge2" }),
    );
    run(
        &mut s,
        "coupling.add",
        json!({ "kind": "gear", "mates": ["Hinge", "Hinge2"], "value": 1 }),
    );
    run(
        &mut s,
        "variable.set",
        json!({ "name": "swing", "expr": "90 deg" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "swing", "min": "0 deg", "max": "180 deg" }),
    );
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Hinge", "value": "swing" }),
    );
    let leg = run(&mut s, "assembly.solve", json!({}));
    assert_eq!(
        (&leg["ok"], &leg["dof"]),
        (&json!(true), &json!(0)),
        "{leg}"
    );
    // Where the leg puts its arm at 90° and at 30°: its base is on the
    // world, so the parent puts it there too.
    let at = |s: &mut Session| run(s, "measure.pose", json!({ "instance": "Arm" }));
    let at90 = at(&mut s);
    run(
        &mut s,
        "variable.set",
        json!({ "name": "swing", "expr": "30 deg" }),
    );
    let at30 = at(&mut s);
    run(
        &mut s,
        "variable.set",
        json!({ "name": "swing", "expr": "90 deg" }),
    );
    run(&mut s, "file.save", json!({ "path": "leg.3dcraftasm" }));
    let same = |a: &Value, b: &Value| {
        close(number(&a["angle"]), number(&b["angle"]), 1e-6)
            && (0..3).all(|k| close(number(&a["position"][k]), number(&b["position"][k]), 1e-6))
            && (0..3).all(|k| close(number(&a["x"][k]), number(&b["x"][k]), 1e-6))
    };
    assert!(!same(&at90, &at30), "{at90} {at30}");

    // In a parent that sets nothing: the hinge is free, the arms still
    // turn together (one degree of freedom, not two), where the leg put
    // them.
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "leg.3dcraftasm", "name": "Leg", "mode": "flexible" }),
    );
    let solved = run(&mut s, "assembly.solve", json!({}));
    assert_eq!(
        (&solved["ok"], &solved["dof"]),
        (&json!(true), &json!(1)),
        "{solved}"
    );
    let p = run(&mut s, "measure.pose", json!({ "instance": "Leg/Arm" }));
    assert!(same(&p, &at90), "{p} {at90}");

    // A parent that sets `swing` drives the hinge: nothing is left free,
    // and the arm stands where the parent says.
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "Leg", "params": { "swing": "30 deg" } }),
    );
    let solved = run(&mut s, "assembly.solve", json!({}));
    assert_eq!(
        (&solved["ok"], &solved["dof"]),
        (&json!(true), &json!(0)),
        "{solved}"
    );
    let p = run(&mut s, "measure.pose", json!({ "instance": "Leg/Arm" }));
    assert!(same(&p, &at30), "{p} {at30}");
}

/// A part in inches placed in an assembly in millimetres is regenerated
/// there as large as it is (as `document.setUnit {keep: sizes}` converts
/// it), with its parameters given in either unit; a subassembly in
/// another unit is refused by name.
#[test]
fn a_part_in_another_length_unit_keeps_its_size() {
    let dir = workdir("asm-gaps", "units");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({ "units": "in" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "w", "expr": "1 in" }),
    );
    run(&mut s, "variable.publish", json!({ "name": "w" }));
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Cube", "size": ["w", 1, 1] }),
    );
    run(
        &mut s,
        "datum.axis",
        json!({ "name": "Axis", "through": [[0, 0, 0], [0, 0, 1]] }),
    );
    run(&mut s, "file.save", json!({ "path": "inch.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    let a = run(
        &mut s,
        "instance.add",
        json!({ "source": "inch.3dcraft", "name": "A" }),
    );
    assert_eq!(a["status"], "ok", "{a}");
    let b = run(
        &mut s,
        "instance.add",
        json!({ "source": "inch.3dcraft", "name": "B", "params": { "w": "50.8 mm" }, "position": [100, 0, 0] }),
    );
    assert_eq!(b["status"], "ok", "{b}");
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/z", "b": "A/Axis", "flip": true }),
    );
    let i = run(&mut s, "assembly.inspect", json!({ "measures": true }));
    let inch = 25.4_f64;
    let volume = |name: &str| {
        number(
            &i["instances"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["name"] == name)
                .unwrap()["measured"]["volume"],
        )
    };
    assert!(close(volume("A"), inch.powi(3), 1e-6 * inch.powi(3)), "{i}");
    assert!(
        close(volume("B"), 2.0 * inch.powi(3), 1e-6 * inch.powi(3)),
        "{i}"
    );
    // A frame of the converted part: its axis datum at the origin.
    let p = run(&mut s, "measure.pose", json!({ "instance": "A" }));
    assert!(close(number(&p["position"][0]), 0.0, 1e-9), "{p}");
    // A face's name survives the conversion: B's bottom (2 by 1 inches)
    // put on the world's xy plane, its centre at the origin.
    let m = run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "B/F1:face(0)" }),
    );
    assert_eq!(m["status"], "ok", "{m}");
    let p = run(&mut s, "measure.pose", json!({ "point": "B/F1:face(0)" }));
    for k in 0..3 {
        assert!(close(number(&p["position"][k]), 0.0, 1e-6), "{p}");
    }
    // And B's own origin lies a half of 2 by 1 inches away from it.
    let p = run(&mut s, "measure.pose", json!({ "instance": "B" }));
    let (x, y) = (number(&p["position"][0]), number(&p["position"][1]));
    assert!(close(x.hypot(y), (inch).hypot(inch / 2.0), 1e-6), "{p}");
    // A subassembly in inches is refused by name.
    run(&mut s, "file.save", json!({ "path": "mm.3dcraftasm" }));
    run(&mut s, "assembly.new", json!({ "units": "in" }));
    let sub = run(
        &mut s,
        "instance.add",
        json!({ "source": "mm.3dcraftasm", "name": "Sub" }),
    );
    assert_eq!(sub["status"], "failed", "{sub}");
    assert!(
        sub["error"]
            .as_str()
            .unwrap()
            .contains("subassembly in another length unit"),
        "{sub}"
    );
}
