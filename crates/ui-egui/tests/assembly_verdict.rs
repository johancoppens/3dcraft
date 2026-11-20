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

    // A feature of the plate's part that fails (A-023): placed, not Solved
    // (its mate's face is the fillet's, gone with it), and its row carries
    // the dot after its name; the dim mark after the source says held only.
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
    assert!(plate.marked, "{plate:?}");
    // `ui.get` reports the ⚠ after its source as since 0.2.0; the panel
    // paints it for held only.
    assert!(plate.status.contains(HELD_MARK), "{plate:?}");
    assert!(!row(&shell, "Base").marked);
    // The sections' notes add up to the heading: the instance under
    // Instances, the mate under Mates.
    assert_eq!(said["text"], "2 failed", "{said}");
    assert_eq!(said["instancesFailed"], 1, "{said}");
    assert_eq!(said["matesFailed"], 1, "{said}");
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
        assert!(sub.marked, "{mode}: {sub:?}");
        assert!(!sub.status.contains(HELD_MARK), "{mode}: {sub:?}");
        // Both under Instances: the subassembly's own mate is its row's.
        assert_eq!(said["instancesFailed"], 2, "{mode}: {said}");
        assert_eq!(said["matesFailed"], 0, "{mode}: {said}");
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
        // `ui.get` lists the lines with the row, and its dot.
        let got = call(&mut shell, &mut s, "ui.get", json!({}));
        assert_eq!(got["features"][0]["inside"], json!(sub.inside), "{mode}");
        assert_eq!(got["features"][0]["marked"], true, "{mode}");
        assert_eq!(got["features"][1]["marked"], false, "{mode}: red already");
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
        // The dot on the subassembly and on the one above it: what is
        // wrong two levels down is seen from the top.
        let marks: Vec<bool> = shell.tree_rows().iter().map(|r| r.marked).collect();
        assert_eq!(marks, [true, true, false], "{outer} {inner}");
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
    hinged(s);
    run(s, "joint.set", json!({ "mate": "Hinge", "value": angle }));
    run(
        s,
        "variable.publish",
        json!({ "name": "Hinge_angle", "min": "0 deg", "max": "180 deg" }),
    );
}

/// A base held to the world and an arm on a hinge between 0° and 90°,
/// nothing driving it yet.
fn hinged(s: &mut Session) {
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

/// No row of `shell` is failed, marked (the dot, or ⚠) or says anything
/// under it, and no coupling row is drawn.
fn nothing_marked(shell: &Shell, what: &str) {
    for r in shell.tree_rows() {
        assert_eq!(r.state, FeatureState::Ok, "{what}: {r:?}");
        assert!(r.inside.is_empty(), "{what}: {r:?}");
        assert!(!r.marked, "{what}: {r:?}");
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

/// The last assembly closed leaves no verdict behind: on the start screen
/// `ui.get` › `assembly` is null and the shell holds no outline. Before
/// (review, 2026-11-20), the outline was read again only with a scene, so
/// the closed assembly's *Not solved* stayed under `assembly`.
#[test]
fn closing_the_last_assembly_leaves_no_verdict() {
    let mut s = session("closed");
    hinge(&mut s, "120 deg");
    let (mut shell, mut window) = shown(&mut s);
    assert_eq!(verdict(&mut shell, &mut s)["text"], "Not solved");
    run(&mut s, "file.close", json!({ "discard": true }));
    window.frames(&mut shell, &mut s);
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    assert_eq!(got["startScreen"], true, "{}", got["startScreen"]);
    assert_eq!(got["assembly"], Value::Null, "{}", got["assembly"]);
    assert!(shell.assembly_outline().is_none());
}

/// The longest run of digits in `text`: a number written out to its last
/// bit (2.0943951023931953) has many, one the window writes has few.
fn longest_digits(text: &str) -> usize {
    text.split(|c: char| !c.is_ascii_digit())
        .map(str::len)
        .max()
        .unwrap_or(0)
}

/// The form says why, under its header, in the window's words: the first
/// problem it stands on, without the check's backticks, and how many more
/// the Assembly tab lists. `ui.get` › `assembly.problems` keeps the
/// check's own text. Before, the header said *2 failed* alone.
#[test]
fn the_form_says_why_in_the_window_s_words() {
    let mut s = session("form-why");
    block_part(&mut s, "block.3dcraft", [20.0, 10.0, 6.0]);
    filleted_plate(&mut s);
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "corner", "expr": "40 mm" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "corner", "min": "0.5 mm", "max": "50 mm" }),
    );
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
        json!({ "source": "plate.3dcraft", "name": "Plate", "params": { "corner": "corner" } }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "Base/F1:face(1)", "b": "Plate/F2:copy(0)/F1:face(1)", "name": "OnBase", "flip": true }),
    );
    let (mut shell, mut window) = shown(&mut s);
    let said = verdict(&mut shell, &mut s);
    let raw = said["problems"][0].as_str().unwrap().to_string();
    assert!(raw.starts_with("instance `Plate` is not placed: "), "{raw}");
    let header = form(&mut shell, &mut s);
    assert_eq!(header["status"], "2 failed", "{header}");
    let why = header["why"].as_str().unwrap_or_else(|| panic!("{header}"));
    let (first, more) = why.split_once('\n').unwrap_or_else(|| panic!("{why}"));
    assert!(first.starts_with("instance Plate is not placed: "), "{why}");
    assert!(first.contains("corner: its value 40 mm"), "{why}");
    assert!(!why.contains('`'), "{why}");
    assert_eq!(more, "and 1 more on the Assembly tab");
    // A value the part takes: nothing to say, Valid.
    call(
        &mut shell,
        &mut s,
        "ui.param",
        json!({ "name": "corner", "value": 1 }),
    );
    window.frames(&mut shell, &mut s);
    let header = form(&mut shell, &mut s);
    assert_eq!(header["status"], "Valid", "{header}");
    assert_eq!(header["why"], Value::Null, "{header}");
}

/// A failed solve beside other problems: the heading says both (*Not
/// solved · 2 failed*), the sections' notes count the two others (an
/// instance under Instances, a mate under Mates), and the form's line
/// writes the solve's numbers as the window writes them while
/// `ui.get` › `assembly.problems` keeps the check's. Before, *Not solved*
/// alone, and the solve's radians to their last bit everywhere.
#[test]
fn a_failed_solve_beside_other_problems_is_said_with_them() {
    let mut s = session("solve-and-more");
    filleted_plate(&mut s);
    hinge(&mut s, "120 deg");
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
    let (mut shell, _window) = shown(&mut s);
    let said = verdict(&mut shell, &mut s);
    assert_eq!(said["text"], "Not solved · 2 failed", "{said}");
    assert_eq!(said["instancesFailed"], 1, "{said}");
    assert_eq!(said["matesFailed"], 1, "{said}");
    let problems = said["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 3, "{said}");
    let solve = problems[2].as_str().unwrap();
    assert!(
        solve.starts_with("the assembly does not solve: "),
        "{solve}"
    );
    assert!(longest_digits(solve) > 10, "the check's own: {solve}");
    let header = form(&mut shell, &mut s);
    assert_eq!(header["status"], "Not solved · 2 failed", "{header}");
    let why = header["why"].as_str().unwrap_or_else(|| panic!("{header}"));
    // The header leads with the solve, and so does the line under it, its
    // numbers as the window writes them.
    let (first, more) = why.split_once('\n').unwrap_or_else(|| panic!("{why}"));
    assert!(first.starts_with("the assembly does not solve: "), "{why}");
    assert!(longest_digits(first) <= 6, "{why}");
    assert_eq!(more, "and 2 more on the Assembly tab");
    assert!(!why.contains('`'), "{why}");
}

/// The form judges the assembly at rest, as `document.check` does: a plate
/// whose corner follows the timeline (40 mm at rest, past its part's range
/// of 30, so it is not placed; 1 mm at t = 0.5), shown at that moment, is
/// *Solved* in the heading (which speaks of what is shown) and *1 failed* in
/// the form. Before (round one), the form read the placement shown and said
/// *Valid* there. (A motion whose rest does not solve is not shown at all:
/// `view.motion` refuses it.)
#[test]
fn the_form_weighs_the_assembly_at_rest_while_a_moment_is_shown() {
    let mut s = session("form-at-rest");
    block_part(&mut s, "block.3dcraft", [20.0, 10.0, 6.0]);
    filleted_plate(&mut s);
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(&mut s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "big", "expr": "40 mm" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "big", "min": "0.5 mm", "max": "50 mm" }),
    );
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
        json!({ "source": "plate.3dcraft", "name": "Plate", "params": { "corner": "big - 78 mm * t" } }),
    );
    let (mut shell, mut window) = shown(&mut s);
    assert_eq!(verdict(&mut shell, &mut s)["text"], "1 failed");
    assert_eq!(form(&mut shell, &mut s)["status"], "1 failed");
    // At t = 0.5 the corner is 1 mm: the moment shown is right.
    run(&mut s, "view.motion", json!({ "t": 0.5 }));
    window.frames(&mut shell, &mut s);
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    assert_eq!(got["assembly"]["ok"], true, "{}", got["assembly"]);
    // The check reads the document at rest, and so does the form.
    let checked = run(&mut s, "document.check", json!({}));
    assert_eq!(checked["ok"], false, "{checked}");
    assert_eq!(
        got["parameters"]["status"], "1 failed",
        "{}",
        got["parameters"]
    );
    assert!(
        got["parameters"]["why"]
            .as_str()
            .is_some_and(|w| w.starts_with("instance Plate is not placed: ")),
        "{}",
        got["parameters"]
    );
}

/// The heading never makes the panel wider than it is (the UX pass, round
/// three): its longest words, *Not solved · 2 failed* with *1 of 3 checks
/// fails* beside them, at a panel of 200 and of 260 points, and every row
/// under it (instances, mates, the checks' statuses) inside the panel.
/// Before, the heading was one line that neither wrapped nor was cut, and
/// every row under it was laid out to its width and cut at the panel's
/// edge. The checks part is what `ui.get` › `assembly.checks` says.
#[test]
fn the_heading_keeps_to_the_panel_s_width() {
    let mut s = session("narrow");
    filleted_plate(&mut s);
    hinge(&mut s, "120 deg");
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
    for (name, of, key, bound) in [
        ("base", "Base", "min", 1),
        ("arm", "Arm", "min", 1),
        ("tiny", "Base", "max", 1),
    ] {
        run(
            &mut s,
            "assert.add",
            json!({ "name": name, "measure": "volume", "of": of, key: bound }),
        );
    }
    let (mut shell, mut window) = shown(&mut s);
    for width in [200.0, 260.0] {
        call(
            &mut shell,
            &mut s,
            "ui.set",
            json!({ "panels": { "treeWidth": width } }),
        );
        window.frames(&mut shell, &mut s);
        window.frames(&mut shell, &mut s);
        let got = call(&mut shell, &mut s, "ui.get", json!({}));
        assert_eq!(got["assembly"]["text"], "Not solved · 2 failed", "{width}");
        assert_eq!(got["assembly"]["checks"], "1 of 3 checks fails", "{width}");
        let panel = shell.side_hits().panel.expect("the side panel");
        let mut rects: Vec<Rect> = shell.tree_row_hits().iter().map(|(_, r)| *r).collect();
        rects.extend(shell.mate_row_hits().iter().map(|(_, r)| *r));
        rects.extend(shell.side_hits().statuses.iter().copied());
        assert!(rects.len() >= 4, "{width}: {rects:?}");
        for rect in rects {
            assert!(
                rect.right() <= panel.right() + 0.5,
                "{width}: a row {rect:?} past the panel {panel:?}"
            );
        }
    }
}

/// A source file that changes while a moment of the motion is shown (as
/// the window follows it when it gets the focus back): the form still
/// weighs the assembly at rest, placed again with the file read now, and
/// never waits for the moment to be let go of. A guard, not a fix: the
/// review's probe of round two (Check 44723852412) found the window
/// already said *1 failed* here, not *Regenerating…*; the page's two cases
/// were the stuck ones.
#[test]
fn a_source_followed_during_a_moment_leaves_the_form_its_verdict() {
    let mut s = session("follow-moment");
    block_part(&mut s, "block.3dcraft", [20.0, 10.0, 6.0]);
    filleted_plate(&mut s);
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(&mut s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "big", "expr": "40 mm" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "big", "min": "0.5 mm", "max": "50 mm" }),
    );
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
        json!({ "source": "plate.3dcraft", "name": "Plate", "params": { "corner": "big - 78 mm * t" } }),
    );
    let (mut shell, mut window) = shown(&mut s);
    assert_eq!(form(&mut shell, &mut s)["status"], "1 failed");
    run(&mut s, "view.motion", json!({ "t": 0.5 }));
    window.frames(&mut shell, &mut s);
    // The block's file changes beside it, and is followed.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ui-asm-verdict-follow-moment");
    let mut other = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    run(&mut other, "file.new", json!({ "units": "mm" }));
    run(
        &mut other,
        "feature.block",
        json!({ "name": "Base", "size": [20.0, 10.0, 9.0] }),
    );
    run(
        &mut other,
        "file.save",
        json!({ "path": "block.3dcraft", "overwrite": true }),
    );
    run(&mut s, "assembly.follow", json!({}));
    window.frames(&mut shell, &mut s);
    let said = form(&mut shell, &mut s);
    assert_eq!(said["status"], "1 failed", "{said}");
    assert!(
        said["why"]
            .as_str()
            .is_some_and(|w| w.starts_with("instance Plate is not placed: ")),
        "{said}"
    );
}

/// The verdict at rest is asked for only where a form reads it (round
/// four): an assembly without public parameters has nothing in a form's
/// place (`parameters.status` null), and one with them gets its verdict.
/// A moment shown after a state change places the state at rest once,
/// form or not, and the form's asking adds no placement of its own: the
/// motion's state is made from the state at rest
/// (`make_motion_state` › `scrubbed_regen` › `OpenDocument::placed`).
#[test]
fn only_a_form_asks_for_the_verdict_at_rest() {
    let mut placed = Vec::new();
    for with_form in [false, true] {
        let mut s = session(&format!("ask-{with_form}"));
        hinged(&mut s);
        run(&mut s, "variable.set", json!({ "name": "t", "expr": "0" }));
        run(
            &mut s,
            "variable.set",
            json!({ "name": "Hinge_angle", "expr": "90 deg * t" }),
        );
        run(
            &mut s,
            "variable.set",
            json!({ "name": "spare", "expr": "1 mm" }),
        );
        if with_form {
            run(
                &mut s,
                "variable.publish",
                json!({ "name": "spare", "min": "0 mm", "max": "10 mm" }),
            );
        }
        let (mut shell, mut window) = shown(&mut s);
        run(&mut s, "view.motion", json!({ "t": 0.5 }));
        window.frames(&mut shell, &mut s);
        // A state change while the moment is shown.
        run(&mut s, "document.info", json!({ "title": "Rig" }));
        let before = s.work_here().placements;
        window.frames(&mut shell, &mut s);
        let header = form(&mut shell, &mut s);
        placed.push(s.work_here().placements - before);
        assert!(s.motion().is_some(), "{with_form}: the moment is shown");
        if with_form {
            assert_eq!(header["status"], "Valid", "{header}");
        } else {
            assert_eq!(header["jig"], false, "{header}");
            assert_eq!(header["status"], Value::Null, "{header}");
        }
    }
    assert_eq!(placed[0], placed[1], "the form's asking placed: {placed:?}");
    assert!(placed[0] <= 1, "{placed:?}");
}
