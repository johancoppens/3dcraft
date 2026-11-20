//! The window says of an assembly what its check says
//! (docs/design/assemblies.md › *Findings*, 2026-11-20): the Assembly
//! tab's heading says *Solved* only while `document.check` lists nothing
//! wrong with the assembly, and otherwise how many problems it lists (or
//! *Not solved*), each in the check's words; what is wrong is found on the
//! row it names (an instance a flexible subassembly could not place, what
//! is wrong in a subassembly itself, a coupling that takes no part); and a
//! form of the assembly's public parameters says *Valid* only then too.
//! Before, the heading read the solve of what was placed alone, couplings
//! and a subassembly's mates were shown nowhere, a flexible subassembly's
//! instance that was not placed was not listed, and the form weighed only
//! the instances that were not placed. Driven through real frames of the
//! shell, the host's part played here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, FeatureState, Session, Workspace};
use craft3d_ui_egui::shell::HELD_MARK;
use craft3d_ui_egui::{RendererChoice, Shell, TreeRow};
use egui::{Pos2, Rect, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap()
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

struct Window {
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    /// One frame, and the host's part between frames: the form's commit
    /// run and handed back.
    fn frame(&mut self, shell: &mut Shell, session: &mut Session) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 760.0))),
            time: Some(self.time),
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
        if let Some(asked) = shell.take_command() {
            let result = session
                .execute(asked.command, &asked.params)
                .map_err(|e| e.message().to_string());
            shell.command_done(result);
        }
    }

    fn frames(&mut self, shell: &mut Shell, session: &mut Session) {
        for _ in 0..4 {
            self.frame(shell, session);
        }
    }
}

/// A session that reads and writes under a folder of its own.
fn session(name: &str) -> Session {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ui-asm-verdict-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    })
}

/// Saves a document with one block `Base` of `size` at `path`, closes it.
fn block_part(s: &mut Session, path: &str, size: [f64; 3]) {
    run(s, "file.new", json!({ "units": "mm" }));
    run(s, "feature.block", json!({ "name": "Base", "size": size }));
    run(s, "file.save", json!({ "path": path }));
    run(s, "file.close", json!({}));
}

/// A plate 30 × 20 × 4 whose fillet `Plate` has the published radius
/// `corner` (0.5 to 30 mm, 1 mm by default), saved at `plate.3dcraft`. A
/// radius of 12 mm fails the fillet; one of 40 mm its part refuses.
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

/// A shell showing the active document on its Assembly tab, synced.
fn shown(s: &mut Session) -> (Shell, Window) {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.set_parameters_single_thread(false);
    let mut window = Window::new();
    window.frames(&mut shell, s);
    call(
        &mut shell,
        s,
        "ui.set",
        json!({ "panels": { "sideTab": "assembly" } }),
    );
    window.frames(&mut shell, s);
    (shell, window)
}

/// The problems `document.check` lists for the document, in its words.
fn checked(s: &mut Session) -> Vec<String> {
    let checked = run(s, "document.check", json!({}));
    checked["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["problem"].as_str().unwrap().to_string())
        .collect()
}

/// The verdict as `ui.get` gives it, held to what the check says: the
/// same problems in the same words, ok when it lists none, and the
/// heading's words.
fn verdict(shell: &mut Shell, s: &mut Session) -> Value {
    let got = call(shell, s, "ui.get", json!({}));
    let assembly = got["assembly"].clone();
    let problems = checked(s);
    assert_eq!(assembly["problems"], json!(problems), "{assembly}");
    assert_eq!(assembly["ok"], problems.is_empty(), "{assembly}");
    let mine = shell.assembly_verdict().expect("an assembly's verdict");
    assert_eq!(assembly["text"], mine.text.as_str());
    assembly
}

fn row<'a>(shell: &'a Shell, name: &str) -> &'a TreeRow {
    shell
        .tree_rows()
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no row {name} in {:?}", shell.tree_rows()))
}

/// Case 1: an instance that is not placed and a mate that fails. The
/// solve of what is placed (the base) succeeds, so the heading said
/// *Solved · 0 DOF* before, its rows red under it; now *2 failed*, and
/// *1 failed* for a mate that fails by itself, *Solved* once nothing is
/// wrong.
#[test]
fn the_heading_is_solved_only_while_the_check_finds_nothing_wrong() {
    let mut s = session("heading");
    block_part(&mut s, "block.3dcraft", [20.0, 10.0, 6.0]);
    filleted_plate(&mut s);
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Base" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "Base/F1:face(0)", "name": "Ground" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "plate.3dcraft", "name": "Plate", "params": { "corner": "40 mm" } }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "Base/F1:face(1)", "b": "Plate/F1:face(1)", "name": "OnBase", "flip": true }),
    );
    let (mut shell, mut window) = shown(&mut s);
    // The solve of what is placed succeeds: what said Solved before.
    let outline = shell.assembly_outline().unwrap().clone();
    assert_eq!(outline.solve_error, None, "{outline:?}");
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["ok"], false, "{said}");
    assert_eq!(said["text"], "2 failed", "{said}");
    let problems = said["problems"].as_array().unwrap();
    assert!(
        problems[0]
            .as_str()
            .unwrap()
            .starts_with("instance `Plate` is not placed: "),
        "{said}"
    );
    assert!(
        problems[1].as_str().unwrap().starts_with("mate `OnBase`: "),
        "{said}"
    );
    // Each where it is named: the instance's row and the mate's, red.
    let plate = row(&shell, "Plate");
    assert_eq!(plate.state, FeatureState::Failed, "{plate:?}");
    assert!(plate.error.is_some(), "{plate:?}");
    let on_base = outline.mates.iter().find(|m| m.name == "OnBase").unwrap();
    assert_eq!(on_base.status, "failed", "{on_base:?}");
    assert!(
        shell.mate_row_hits().iter().any(|(n, _)| n == "OnBase"),
        "{:?}",
        shell.mate_row_hits()
    );

    // Placed, and its mate on a face of the blank the fillet took: that
    // mate alone.
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "Plate", "params": { "corner": "1 mm" } }),
    );
    window.frames(&mut shell, &mut s);
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["text"], "1 failed", "{said}");
    assert_eq!(row(&shell, "Plate").state, FeatureState::Ok);

    // On the plate's own face: nothing wrong, Solved.
    run(
        &mut s,
        "mate.edit",
        json!({ "name": "OnBase", "b": "Plate/F2:copy(0)/F1:face(1)" }),
    );
    window.frames(&mut shell, &mut s);
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["ok"], true, "{said}");
    assert_eq!(said["text"], "Solved · 0 DOF", "{said}");
    assert_eq!(said["problems"], json!([]), "{said}");

    // A feature of the plate's part that fails (A-023): placed, marked ⚠,
    // and not Solved (its mate's face is the fillet's, gone with it).
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "Plate", "params": { "corner": "12 mm" } }),
    );
    window.frames(&mut shell, &mut s);
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["ok"], false, "{said}");
    assert!(
        said["problems"][0]
            .as_str()
            .unwrap()
            .starts_with("instance `Plate`: its feature `Plate` (F2) failed"),
        "{said}"
    );
    let plate = row(&shell, "Plate");
    assert_eq!(plate.state, FeatureState::Ok, "{plate:?}");
    assert!(plate.status.contains(HELD_MARK), "{plate:?}");
}

/// Case 2: a coupling that takes no part. Every mate holds and the solve
/// succeeds, so the heading said *Solved · 1 DOF* and the coupling was
/// shown nowhere; now *1 failed*, and the coupling is a row after the
/// mates, with why. A coupling that takes part is not listed.
#[test]
fn a_coupling_that_takes_no_part_is_a_row_after_the_mates() {
    let mut s = session("coupling");
    block_part(&mut s, "block.3dcraft", [20.0, 10.0, 6.0]);
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
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
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "A/F1:face(0)", "name": "Ground" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "A/F1:face(1)", "b": "B/F1:face(0)", "name": "Turn" }),
    );
    let c = run(
        &mut s,
        "coupling.add",
        json!({ "kind": "linear", "name": "NoSlide", "terms": [{ "mate": "Turn", "motion": "distance" }] }),
    );
    assert_eq!(c["status"], "failed", "{c}");
    let (mut shell, mut window) = shown(&mut s);
    let outline = shell.assembly_outline().unwrap().clone();
    assert_eq!(outline.solve_error, None, "{outline:?}");
    assert!(
        outline.mates.iter().all(|m| m.status == "ok"),
        "{outline:?}"
    );
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["text"], "1 failed", "{said}");
    assert!(
        said["problems"][0]
            .as_str()
            .unwrap()
            .starts_with("coupling `NoSlide`: "),
        "{said}"
    );
    let coupling = outline
        .couplings
        .iter()
        .find(|c| c.name == "NoSlide")
        .unwrap();
    let why = coupling.error.clone().expect("why it takes no part");
    assert_eq!(said["problems"][0], format!("coupling `NoSlide`: {why}"));
    // Drawn after the mates, on the Assembly tab.
    let couplings = shell.coupling_row_hits().to_vec();
    assert_eq!(couplings.len(), 1, "{couplings:?}");
    assert_eq!(couplings[0].0, "NoSlide");
    let last_mate = shell
        .mate_row_hits()
        .iter()
        .map(|(_, r)| r.bottom())
        .fold(f32::MIN, f32::max);
    assert!(couplings[0].1.top() >= last_mate, "{couplings:?}");

    // Gone: Solved again, and no coupling row.
    run(&mut s, "coupling.delete", json!({ "name": "NoSlide" }));
    window.frames(&mut shell, &mut s);
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["text"], "Solved · 1 DOF", "{said}");
    assert!(shell.coupling_row_hits().is_empty());
}

/// Saves `inner.3dcraftasm` (the plate its part refuses, and its mate
/// `Ground` that fails with it) and `unsolved.3dcraftasm` (a block held to
/// the plane twice, 5 apart: its solve fails).
fn inner_assemblies(s: &mut Session) {
    filleted_plate(s);
    block_part(s, "block.3dcraft", [20.0, 10.0, 6.0]);
    run(s, "assembly.new", json!({ "units": "mm" }));
    run(
        s,
        "instance.add",
        json!({ "source": "plate.3dcraft", "name": "Plate", "params": { "corner": "40 mm" } }),
    );
    run(
        s,
        "mate.add",
        json!({ "kind": "fixed", "name": "Ground", "a": "world/xy", "b": "Plate/F1:face(1)", "flip": true }),
    );
    run(s, "file.save", json!({ "path": "inner.3dcraftasm" }));
    run(s, "file.close", json!({}));
    run(s, "assembly.new", json!({ "units": "mm" }));
    run(
        s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "A" }),
    );
    for (name, offset) in [("Ground", 0), ("Lifted", 5)] {
        run(
            s,
            "mate.add",
            json!({ "kind": "fixed", "a": "world/xy", "b": "A/F1:face(0)", "name": name, "offset": offset }),
        );
    }
    run(s, "file.save", json!({ "path": "unsolved.3dcraftasm" }));
    run(s, "file.close", json!({}));
}

/// Cases 3 and 4: inside a subassembly, rigid or flexible, an instance
/// that is not placed and the mate on it. The top's solve succeeds, so
/// the heading said *Solved*; the mate was shown nowhere, and in a
/// flexible subassembly the instance was not listed either (only what is
/// placed in the parent was). Now *2 failed*; the instance is a red row
/// under its subassembly either way, and the subassembly's row is marked
/// ⚠ with what is wrong in it itself under it, in the check's words.
#[test]
fn what_is_wrong_inside_a_subassembly_is_on_its_rows() {
    let mut s = session("inside");
    inner_assemblies(&mut s);
    for mode in ["rigid", "flexible"] {
        run(&mut s, "assembly.new", json!({ "units": "mm" }));
        run(
            &mut s,
            "instance.add",
            json!({ "source": "inner.3dcraftasm", "name": "Sub", "mode": mode }),
        );
        let (mut shell, _window) = shown(&mut s);
        assert_eq!(
            shell.assembly_outline().unwrap().solve_error,
            None,
            "{mode}"
        );
        let said = verdict(&mut shell, &mut s);
        assert_eq!(said["text"], "2 failed", "{mode}: {said}");
        let names: Vec<(&str, usize)> = shell
            .tree_rows()
            .iter()
            .map(|r| (r.name.as_str(), r.depth))
            .collect();
        assert_eq!(names, [("Sub", 0), ("Plate", 1)], "{mode}");
        let plate = row(&shell, "Plate");
        assert_eq!(plate.state, FeatureState::Failed, "{mode}: {plate:?}");
        let why = plate.error.clone().unwrap();
        assert_eq!(
            said["problems"][0],
            format!("instance `Sub`: its instance `Plate` is not placed: {why}"),
            "{mode}"
        );
        let sub = row(&shell, "Sub").clone();
        assert_eq!(sub.state, FeatureState::Ok, "{mode}: {sub:?}");
        assert!(sub.status.contains(HELD_MARK), "{mode}: {sub:?}");
        assert_eq!(sub.inside.len(), 1, "{mode}: {sub:?}");
        assert!(
            sub.inside[0].starts_with("its mate `Ground`: "),
            "{mode}: {sub:?}"
        );
        assert_eq!(
            said["problems"][1],
            format!("instance `Sub`: {}", sub.inside[0]),
            "{mode}"
        );
        // `ui.get` lists the lines with the row.
        let got = call(&mut shell, &mut s, "ui.get", json!({}));
        assert_eq!(got["features"][0]["inside"], json!(sub.inside), "{mode}");
        run(&mut s, "file.close", json!({ "discard": true }));

        // A subassembly that does not solve: a rigid one says so under its
        // row; a flexible one's mates contradict in the top's solve.
        run(&mut s, "assembly.new", json!({ "units": "mm" }));
        run(
            &mut s,
            "instance.add",
            json!({ "source": "unsolved.3dcraftasm", "name": "Sub", "mode": mode }),
        );
        let (mut shell, _window) = shown(&mut s);
        let said = verdict(&mut shell, &mut s);
        let sub = row(&shell, "Sub");
        if mode == "rigid" {
            assert_eq!(said["text"], "1 failed", "{said}");
            assert_eq!(sub.inside.len(), 1, "{sub:?}");
            assert!(sub.inside[0].starts_with("it does not solve: "), "{sub:?}");
            assert_eq!(
                said["problems"][0],
                format!("instance `Sub`: {}", sub.inside[0])
            );
        } else {
            assert_eq!(said["text"], "Not solved", "{said}");
            assert!(sub.inside.is_empty(), "{sub:?}");
        }
        run(&mut s, "file.close", json!({ "discard": true }));
    }
}

/// Two subassemblies down, each rigid or flexible: the rows reach the
/// plate that is not placed, and the mate on it is said under the row of
/// the subassembly it is in. A flexible subassembly inside a flexible one
/// is a row of its own (before, the list went from the outer one to the
/// inner one's instances).
#[test]
fn what_is_wrong_two_subassemblies_down_is_on_its_rows() {
    let mut s = session("two-deep");
    inner_assemblies(&mut s);
    for (outer, inner) in [
        ("rigid", "rigid"),
        ("rigid", "flexible"),
        ("flexible", "rigid"),
        ("flexible", "flexible"),
    ] {
        let mid = format!("mid-{outer}-{inner}.3dcraftasm");
        run(&mut s, "assembly.new", json!({ "units": "mm" }));
        run(
            &mut s,
            "instance.add",
            json!({ "source": "inner.3dcraftasm", "name": "Sub", "mode": inner }),
        );
        run(&mut s, "file.save", json!({ "path": mid }));
        run(&mut s, "file.close", json!({}));
        run(&mut s, "assembly.new", json!({ "units": "mm" }));
        run(
            &mut s,
            "instance.add",
            json!({ "source": mid, "name": "Mid", "mode": outer }),
        );
        let (mut shell, _window) = shown(&mut s);
        let said = verdict(&mut shell, &mut s);
        assert_eq!(said["text"], "2 failed", "{outer} {inner}: {said}");
        let rows: Vec<(&str, usize, FeatureState)> = shell
            .tree_rows()
            .iter()
            .map(|r| (r.name.as_str(), r.depth, r.state))
            .collect();
        assert_eq!(
            rows,
            [
                ("Mid", 0, FeatureState::Ok),
                ("Sub", 1, FeatureState::Ok),
                ("Plate", 2, FeatureState::Failed),
            ],
            "{outer} {inner}"
        );
        assert!(row(&shell, "Mid").inside.is_empty(), "{outer} {inner}");
        let sub = row(&shell, "Sub");
        assert_eq!(sub.inside.len(), 1, "{outer} {inner}: {sub:?}");
        assert_eq!(
            said["problems"][1],
            format!("instance `Mid`: its instance `Sub`: {}", sub.inside[0]),
            "{outer} {inner}"
        );
        run(&mut s, "file.close", json!({ "discard": true }));
    }
}

/// The form's words as `ui.get` gives them.
fn form(shell: &mut Shell, s: &mut Session) -> Value {
    call(shell, s, "ui.get", json!({}))["parameters"].clone()
}

/// Case 5: a form of an assembly's public parameters. A value that fails
/// a feature of an instance's part (placed with the bodies from before
/// it, its row not red) breaks the model; before, the form weighed only
/// the rows of instances that were not placed and said *Valid*.
#[test]
fn a_value_that_fails_a_feature_of_an_instance_breaks_the_assembly() {
    let mut s = session("form-feature");
    filleted_plate(&mut s);
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "corner", "expr": "1 mm" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "corner", "min": "0.5 mm", "max": "30 mm" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "plate.3dcraft", "name": "Plate", "params": { "corner": "corner" } }),
    );
    let (mut shell, mut window) = shown(&mut s);
    let header = form(&mut shell, &mut s);
    assert_eq!(header["jig"], true, "{header}");
    assert_eq!(header["status"], "Valid", "{header}");
    assert_eq!(verdict(&mut shell, &mut s)["ok"], true);
    let answer = call(
        &mut shell,
        &mut s,
        "ui.param",
        json!({ "name": "corner", "value": 12 }),
    );
    assert_eq!(answer["sent"], 12.0, "{answer}");
    window.frames(&mut shell, &mut s);
    let plate = row(&shell, "Plate").clone();
    assert_eq!(plate.state, FeatureState::Ok, "{plate:?}");
    assert!(plate.failed.is_some(), "{plate:?}");
    let header = form(&mut shell, &mut s);
    assert_eq!(header["trouble"]["broke"], true, "{header}");
    assert_eq!(header["trouble"]["name"], "corner", "{header}");
    assert_eq!(header["status"], "Breaks the model", "{header}");
    let said = verdict(&mut shell, &mut s);
    assert_eq!(
        header["trouble"]["message"], said["problems"][0],
        "{header}"
    );
    // Back to last good: Valid again.
    call(&mut shell, &mut s, "ui.param", json!({ "lastGood": true }));
    window.frames(&mut shell, &mut s);
    let header = form(&mut shell, &mut s);
    assert_eq!(header["trouble"], Value::Null, "{header}");
    assert_eq!(header["status"], "Valid", "{header}");
}

/// A base and an arm on a hinge between 0° and 90°, its angle the public
/// parameter `Hinge_angle` (0° to 180°) at `angle`.
fn hinge(s: &mut Session, angle: &str) {
    block_part(s, "block.3dcraft", [10.0, 2.0, 2.0]);
    run(s, "assembly.new", json!({ "units": "mm" }));
    for name in ["Base", "Arm"] {
        run(
            s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": name }),
        );
    }
    run(
        s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "Base/F1:face(0)", "name": "Ground" }),
    );
    run(
        s,
        "mate.add",
        json!({ "kind": "revolute", "a": "Base/F1:face(1)", "b": "Arm/F1:face(0)", "name": "Hinge", "limits": ["0 deg", "90 deg"] }),
    );
    run(s, "joint.set", json!({ "mate": "Hinge", "value": angle }));
    run(
        s,
        "variable.publish",
        json!({ "name": "Hinge_angle", "min": "0 deg", "max": "180 deg" }),
    );
}

/// Case 5: an assembly that opens unsolved (its hinge past its limit) is
/// not said *Valid*, whatever the last good state: its form says *Not
/// solved*, as its heading does; a value inside the limit mends it, and
/// one past it again breaks it. Before, the form said *Valid* throughout
/// but for the one commit, no instance row being red.
#[test]
fn a_form_of_an_assembly_is_valid_only_while_its_check_is_ok() {
    let mut s = session("form-solve");
    hinge(&mut s, "120 deg");
    let (mut shell, mut window) = shown(&mut s);
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["text"], "Not solved", "{said}");
    assert!(
        said["problems"][0]
            .as_str()
            .unwrap()
            .starts_with("the assembly does not solve: "),
        "{said}"
    );
    let header = form(&mut shell, &mut s);
    assert_eq!(header["jig"], true, "{header}");
    assert_eq!(header["trouble"], Value::Null, "{header}");
    assert_eq!(header["status"], "Not solved", "{header}");
    call(
        &mut shell,
        &mut s,
        "ui.param",
        json!({ "name": "Hinge_angle", "value": 45 }),
    );
    window.frames(&mut shell, &mut s);
    assert_eq!(verdict(&mut shell, &mut s)["ok"], true);
    let header = form(&mut shell, &mut s);
    assert_eq!(header["status"], "Valid", "{header}");
    call(
        &mut shell,
        &mut s,
        "ui.param",
        json!({ "name": "Hinge_angle", "value": 150 }),
    );
    window.frames(&mut shell, &mut s);
    let header = form(&mut shell, &mut s);
    assert_eq!(header["trouble"]["broke"], true, "{header}");
    assert_eq!(header["status"], "Breaks the model", "{header}");
    assert!(
        header["trouble"]["message"]
            .as_str()
            .unwrap()
            .starts_with("the assembly does not solve: "),
        "{header}"
    );
}

/// No row of `shell` is failed, marked ⚠ or says anything under it, and no
/// coupling row is drawn.
fn nothing_marked(shell: &Shell, what: &str) {
    for r in shell.tree_rows() {
        assert_eq!(r.state, FeatureState::Ok, "{what}: {r:?}");
        assert!(r.inside.is_empty(), "{what}: {r:?}");
        assert!(!r.status.contains(HELD_MARK), "{what}: {r:?}");
    }
    assert!(shell.coupling_row_hits().is_empty(), "{what}");
}

/// What the check leaves ok the window says *Solved* and *Valid* to, as
/// before: a hinge at either end of its limits, a fixed mate given twice
/// (redundant), an assembly no mate holds to the world (floating), and a
/// right one with a rigid and a flexible subassembly, a mate into the
/// rigid one and a gear coupling that takes part.
#[test]
fn limits_redundant_floating_and_right_assemblies_stay_solved_and_valid() {
    let mut s = session("right");
    hinge(&mut s, "90 deg");
    let (mut shell, mut window) = shown(&mut s);
    for at in [90, 0] {
        call(
            &mut shell,
            &mut s,
            "ui.param",
            json!({ "name": "Hinge_angle", "value": at }),
        );
        window.frames(&mut shell, &mut s);
        let said = verdict(&mut shell, &mut s);
        assert_eq!(said["ok"], true, "{at}: {said}");
        assert_eq!(said["text"], "Solved · 0 DOF", "{at}: {said}");
        assert_eq!(form(&mut shell, &mut s)["status"], "Valid", "{at}");
        nothing_marked(&shell, "limits");
    }
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "world/xy", "b": "Base/F1:face(0)", "name": "Again" }),
    );
    window.frames(&mut shell, &mut s);
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["ok"], true, "redundant: {said}");
    assert!(
        said["text"].as_str().unwrap().starts_with("Solved"),
        "{said}"
    );
    assert_eq!(form(&mut shell, &mut s)["status"], "Valid");
    nothing_marked(&shell, "redundant");
    run(&mut s, "file.close", json!({ "discard": true }));

    // Floating: nothing reaches the world.
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    for name in ["A", "B"] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": name }),
        );
    }
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "A/F1:face(1)", "b": "B/F1:face(0)", "name": "Turn" }),
    );
    assert_eq!(run(&mut s, "assembly.solve", json!({}))["floating"], true);
    let (mut shell, _window) = shown(&mut s);
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["ok"], true, "floating: {said}");
    assert!(
        said["text"].as_str().unwrap().starts_with("Solved"),
        "{said}"
    );
    nothing_marked(&shell, "floating");
    run(&mut s, "file.close", json!({ "discard": true }));

    // Right, with subassemblies and a coupling.
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
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
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Hinge", "value": "90 deg" }),
    );
    run(&mut s, "file.save", json!({ "path": "leg.3dcraftasm" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    for (name, mode, x) in [("Rigid", "rigid", 0), ("Flexible", "flexible", 30)] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "leg.3dcraftasm", "name": name, "mode": mode, "position": [x, 0, 5] }),
        );
    }
    run(
        &mut s,
        "instance.add",
        json!({ "source": "block.3dcraft", "name": "Lid" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "Rigid/Arm/F1:face(1)", "b": "Lid/F1:face(0)", "name": "OnArm" }),
    );
    for (name, mate, offset) in [("Wheel", "Spin", 20), ("Gear", "Mesh", 30)] {
        run(
            &mut s,
            "instance.add",
            json!({ "source": "block.3dcraft", "name": name }),
        );
        run(
            &mut s,
            "mate.add",
            json!({ "kind": "revolute", "a": "world/xy", "b": format!("{name}/F1:face(0)"), "name": mate, "offset": offset }),
        );
    }
    let c = run(
        &mut s,
        "coupling.add",
        json!({ "kind": "gear", "mates": ["Spin", "Mesh"], "value": "-1", "name": "Gears" }),
    );
    assert_eq!(c["status"], "ok", "{c}");
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Spin", "value": "30 deg" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "Spin_angle", "min": "0 deg", "max": "360 deg" }),
    );
    let (mut shell, _window) = shown(&mut s);
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["ok"], true, "right: {said}");
    assert!(
        said["text"].as_str().unwrap().starts_with("Solved"),
        "{said}"
    );
    assert_eq!(form(&mut shell, &mut s)["status"], "Valid");
    nothing_marked(&shell, "right");
    let outline = shell.assembly_outline().unwrap();
    assert_eq!(outline.couplings.len(), 1, "{outline:?}");
    assert_eq!(outline.couplings[0].error, None, "{outline:?}");
    // Its subassemblies' rows: each with its two instances under it.
    let names: Vec<&str> = shell.tree_rows().iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Rigid", "Base", "Arm", "Flexible", "Base", "Arm", "Lid", "Wheel", "Gear"
        ],
        "{names:?}"
    );
}
