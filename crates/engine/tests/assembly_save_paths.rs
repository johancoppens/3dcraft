//! An assembly saved into another folder (docs/design/assemblies.md,
//! *Pack and unpack*): each source path is rewritten so it still leads to
//! its file, whether the assembly's own path and the new one are relative
//! to the root or absolute, and the open assembly goes on placing its
//! instances from them. 2026-11-18: an assembly whose sources were
//! relative to the root, saved under an absolute path, had them written
//! as if the root were the file system's (as many `..` as the folder is
//! deep, then `model/v2/rod.3dcraft`), and the save answered ok. A save
//! that cannot write such a path is refused, with nothing written.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{error, session_in, workdir};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};

/// The part `model/v2/rod.3dcraft`: one block 4 × 4 and `length` long.
fn rod_part(s: &mut Session, length: f64) {
    rod_part_at(s, "model/v2/rod.3dcraft", length);
}

/// [`rod_part`] saved at `path`.
fn rod_part_at(s: &mut Session, path: &str, length: f64) {
    run(s, "file.new", json!({ "units": "mm" }));
    run(
        s,
        "feature.block",
        json!({ "name": "Rod", "size": [length, 4, 4] }),
    );
    run(s, "file.save", json!({ "path": path }));
    run(s, "file.close", json!({}));
}

/// The part `model/v2/rod.3dcraft` (one block 40 long), and an assembly
/// without a file holding one instance of it, `Rod`, added by its path
/// relative to the root.
fn rod_assembly(s: &mut Session) {
    rod_assembly_of(s, 40.0);
}

/// [`rod_assembly`] with a rod `length` long.
fn rod_assembly_of(s: &mut Session, length: f64) {
    rod_part(s, length);
    run(s, "assembly.new", json!({ "units": "mm" }));
    run(
        s,
        "instance.add",
        json!({ "source": "model/v2/rod.3dcraft", "name": "Rod" }),
    );
}

/// The open assembly's instance `Rod` has the source path `want` and is
/// placed from its file: one body, nothing held back. The inspection.
fn placed(s: &mut Session, want: &str, after: &str) -> Value {
    let i = run(s, "assembly.inspect", json!({ "measures": true }));
    let rod = &i["instances"][0];
    assert_eq!(rod["source"]["path"], want, "after {after}: {i}");
    assert_eq!(rod["status"], "ok", "after {after}: {i}");
    assert!(rod["held"].is_null(), "after {after}: {i}");
    assert_eq!(i["bodies"], 1, "after {after}: {i}");
    i
}

/// The source path the assembly file at `file` holds for its first
/// instance.
fn stored(file: &Path) -> String {
    let doc = craft3d_format::from_bytes(&std::fs::read(file).unwrap()).unwrap();
    let assembly = doc.assembly.unwrap();
    match &assembly.instances[0].source {
        craft3d_doc::Source::File { path, .. } => path.clone(),
        craft3d_doc::Source::Packed { .. } => panic!("{} holds a packed source", file.display()),
    }
}

/// The assembly file `file` (a path as a request gives it) opened in a
/// session of its own beneath `root`: its instance placed, one body.
fn opens(root: &Path, file: &str) {
    let mut s = session_in(root);
    run(&mut s, "file.open", json!({ "path": file }));
    let i = run(&mut s, "assembly.inspect", json!({}));
    assert_eq!(i["instances"][0]["status"], "ok", "{file}: {i}");
    assert_eq!(i["bodies"], 1, "{file}: {i}");
}

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_string()
}

fn folders(dir: &Path, names: &[&str]) {
    for name in names {
        std::fs::create_dir_all(dir.join(name)).unwrap();
    }
}

/// The five steps of the report (2026-11-18), over a root as an agent's
/// window has one: relative, absolute, relative again. Before the fix
/// the absolute save stored `../../../…/model/v2/rod.3dcraft` and the
/// instance said its source `/model/v2/rod.3dcraft` was outside the read
/// root.
#[test]
fn the_five_steps_of_the_report_keep_the_rod_found() {
    let dir = workdir("asm-save-paths", "report");
    folders(&dir, &["model/v2", "work/rel", "work/abs", "work/rel2"]);
    let want = "../../model/v2/rod.3dcraft";
    let mut s = session_in(&dir);
    rod_assembly(&mut s);
    placed(&mut s, "model/v2/rod.3dcraft", "instance.add");
    // 3: beneath the root, relative.
    run(
        &mut s,
        "file.save",
        json!({ "path": "work/rel/a.3dcraftasm" }),
    );
    assert_eq!(stored(&dir.join("work/rel/a.3dcraftasm")), want);
    placed(&mut s, want, "the relative save");
    // Moved, so undo has a state to go back to across the next save.
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "Rod", "position": [5, 0, 0] }),
    );
    // 4: beneath the root, by its absolute path.
    let absolute = dir.join("work/abs/c.3dcraftasm");
    let saved = run(&mut s, "file.save", json!({ "path": text(&absolute) }));
    assert_eq!(saved["path"], text(&absolute), "{saved}");
    assert_eq!(stored(&absolute), want);
    placed(&mut s, want, "the absolute save");
    run(&mut s, "edit.undo", json!({}));
    placed(&mut s, want, "an undo after the absolute save");
    run(&mut s, "edit.redo", json!({}));
    // 5: relative again.
    run(
        &mut s,
        "file.save",
        json!({ "path": "work/rel2/d.3dcraftasm" }),
    );
    assert_eq!(stored(&dir.join("work/rel2/d.3dcraftasm")), want);
    placed(&mut s, want, "the second relative save");
    // Each file opens on its own, by either kind of path.
    for file in [
        "work/rel/a.3dcraftasm",
        "work/abs/c.3dcraftasm",
        "work/rel2/d.3dcraftasm",
    ] {
        opens(&dir, file);
        opens(&dir, &text(&dir.join(file)));
    }
}

/// Relative to absolute, absolute to absolute, absolute to relative and
/// relative to absolute again, into folders one and three deep, the root
/// itself among them; and an assembly without a file saved straight under
/// an absolute path.
#[test]
fn saves_between_relative_and_absolute_paths_one_and_three_folders_deep() {
    for (deep, other) in [("a", "x"), ("a/b/c", "x/y/z")] {
        let dir = workdir("asm-save-paths", &format!("kinds-{}", deep.len()));
        folders(&dir, &["model/v2", deep, other]);
        let ups = "../".repeat(deep.split('/').count());
        let want = format!("{ups}model/v2/rod.3dcraft");
        let absolute = |p: &str| text(&dir.join(p));
        let mut s = session_in(&dir);
        rod_assembly(&mut s);
        let steps = [
            // No file yet: its sources are relative to the root.
            (absolute(&format!("{deep}/1.3dcraftasm")), want.clone()),
            (absolute(&format!("{other}/2.3dcraftasm")), want.clone()),
            (format!("{deep}/3.3dcraftasm"), want.clone()),
            (absolute(&format!("{other}/4.3dcraftasm")), want.clone()),
            (absolute("5.3dcraftasm"), "model/v2/rod.3dcraft".to_string()),
            (format!("{other}/6.3dcraftasm"), want.clone()),
            (
                "7.3dcraftasm".to_string(),
                "model/v2/rod.3dcraft".to_string(),
            ),
            (absolute(&format!("{deep}/8.3dcraftasm")), want.clone()),
        ];
        for (path, want) in steps {
            run(&mut s, "file.save", json!({ "path": path }));
            let file = if Path::new(&path).is_absolute() {
                Path::new(&path).to_path_buf()
            } else {
                dir.join(&path)
            };
            assert_eq!(stored(&file), want, "{path}");
            placed(&mut s, &want, &path);
            opens(&dir, &path);
        }
    }
}

/// An assembly opened by its absolute path (its sources climb with `..`)
/// saved under a relative path and again under its own: a save of a
/// save keeps what the first wrote.
#[test]
fn an_assembly_opened_by_its_absolute_path_saves_under_a_relative_one() {
    let dir = workdir("asm-save-paths", "opened");
    folders(&dir, &["model/v2", "work/rel", "work/other/deeper"]);
    let mut s = session_in(&dir);
    rod_assembly(&mut s);
    run(
        &mut s,
        "file.save",
        json!({ "path": "work/rel/a.3dcraftasm" }),
    );
    run(&mut s, "file.close", json!({}));
    let mut s = session_in(&dir);
    run(
        &mut s,
        "file.open",
        json!({ "path": text(&dir.join("work/rel/a.3dcraftasm")) }),
    );
    placed(&mut s, "../../model/v2/rod.3dcraft", "file.open");
    let want = "../../../model/v2/rod.3dcraft";
    run(
        &mut s,
        "file.save",
        json!({ "path": "work/other/deeper/b.3dcraftasm" }),
    );
    let b = dir.join("work/other/deeper/b.3dcraftasm");
    assert_eq!(stored(&b), want);
    placed(&mut s, want, "the relative save");
    // A save of the save, to its own file: nothing to rewrite.
    let again = run(&mut s, "file.save", json!({}));
    assert_eq!(again["path"], "work/other/deeper/b.3dcraftasm", "{again}");
    assert_eq!(stored(&b), want);
    placed(&mut s, want, "a save to its own file");
    // And its own file by its absolute path, then beside the first.
    run(&mut s, "file.save", json!({ "path": text(&b) }));
    assert_eq!(stored(&b), want);
    placed(&mut s, want, "a save to its own file by its absolute path");
    run(
        &mut s,
        "file.save",
        json!({ "path": "work/rel/c.3dcraftasm" }),
    );
    assert_eq!(
        stored(&dir.join("work/rel/c.3dcraftasm")),
        "../../model/v2/rod.3dcraft"
    );
    placed(&mut s, "../../model/v2/rod.3dcraft", "beside the first");
    for file in ["work/other/deeper/b.3dcraftasm", "work/rel/c.3dcraftasm"] {
        opens(&dir, file);
    }
}

/// In memory, as in the browser: paths are relative, the sources follow
/// each save, an absolute path is refused before anything is written, and
/// the bytes written open in another session.
#[test]
fn in_memory_the_sources_follow_each_save() {
    let mut s = Session::new(Workspace::memory());
    rod_assembly(&mut s);
    for (path, want) in [
        ("work/rel/a.3dcraftasm", "../../model/v2/rod.3dcraft"),
        ("a.3dcraftasm", "model/v2/rod.3dcraft"),
        ("work/x/y/b.3dcraftasm", "../../../model/v2/rod.3dcraft"),
        ("work//x/./y/c.3dcraftasm", "../../../model/v2/rod.3dcraft"),
    ] {
        run(&mut s, "file.save", json!({ "path": path }));
        placed(&mut s, want, path);
    }
    let e = error(&mut s, "file.save", json!({ "path": "/work/a.3dcraftasm" }));
    assert!(e.contains("absolute"), "{e}");
    placed(&mut s, "../../../model/v2/rod.3dcraft", "the refusal");
    let assembly = s.take_file("work/x/y/b.3dcraftasm").unwrap();
    let rod = s.take_file("model/v2/rod.3dcraft").unwrap();
    let mut other = Session::new(Workspace::memory());
    other.put_file("work/x/y/b.3dcraftasm", assembly).unwrap();
    other.put_file("model/v2/rod.3dcraft", rod).unwrap();
    run(
        &mut other,
        "file.open",
        json!({ "path": "work/x/y/b.3dcraftasm" }),
    );
    placed(&mut other, "../../../model/v2/rod.3dcraft", "file.open");
}

/// Paths a save must refuse leave the open assembly as it was; spellings
/// of a folder beneath the root (doubled slashes, `.`, the root's own
/// absolute path) write the paths of that folder.
#[test]
fn hostile_save_paths_are_refused_and_change_nothing() {
    let dir = workdir("asm-save-paths", "hostile");
    folders(&dir, &["model/v2", "work/rel"]);
    let root = text(&dir);
    let want = "../../model/v2/rod.3dcraft";
    let mut s = session_in(&dir);
    rod_assembly(&mut s);
    run(
        &mut s,
        "file.save",
        json!({ "path": "work/rel/a.3dcraftasm" }),
    );
    let deep = format!("{}/x.3dcraftasm", vec!["d"; 10_000].join("/"));
    for path in [
        String::new(),
        ".".to_string(),
        "/".to_string(),
        "//".to_string(),
        "work/rel/".to_string(),
        "../x.3dcraftasm".to_string(),
        "/x.3dcraftasm".to_string(),
        format!("{root}/../x.3dcraftasm"),
        format!("{root}/work/../../x.3dcraftasm"),
        deep.clone(),
        format!("{root}/{deep}"),
    ] {
        let shown: String = path.chars().take(60).collect();
        let e = error(&mut s, "file.save", json!({ "path": path }));
        assert!(!e.is_empty(), "{shown}");
        placed(&mut s, want, &shown);
        let status = run(&mut s, "file.list", json!({}));
        assert!(
            status.to_string().contains("work/rel/a.3dcraftasm"),
            "{shown}: {status}"
        );
    }
    // Ten thousand folders deep, the path back up is too long for a
    // source: refused saying so, before anything is written.
    let e = error(
        &mut s,
        "file.save",
        json!({ "path": format!("{root}/{deep}") }),
    );
    assert!(
        e.contains("cannot be written relative to") && e.contains("nothing was written"),
        "{}",
        e.chars().take(300).collect::<String>()
    );
    for (path, file, want) in [
        (
            "work//rel/./b.3dcraftasm".to_string(),
            "work/rel/b.3dcraftasm",
            want,
        ),
        (
            format!("{root}//work/./rel/c.3dcraftasm"),
            "work/rel/c.3dcraftasm",
            want,
        ),
        (
            format!("{root}/d.3dcraftasm"),
            "d.3dcraftasm",
            "model/v2/rod.3dcraft",
        ),
        (
            "work/rel/e.3dcraftasm".to_string(),
            "work/rel/e.3dcraftasm",
            want,
        ),
    ] {
        run(&mut s, "file.save", json!({ "path": path }));
        assert_eq!(stored(&dir.join(file)), want, "{path}");
        placed(&mut s, want, &path);
    }
}

/// A source that is not there says so in words; one whose path leaves
/// the root says that. The second is what a file the old save wrote
/// holds: opened again, its instance is pointed at its file with
/// `instance.edit`, and saved, it is whole again.
#[test]
fn a_missing_source_says_so_and_a_wrongly_saved_one_is_pointed_again() {
    let dir = workdir("asm-save-paths", "missing");
    folders(&dir, &["model/v2", "work/rel", "work/abs"]);
    let mut s = session_in(&dir);
    rod_assembly(&mut s);
    run(
        &mut s,
        "file.save",
        json!({ "path": "work/rel/a.3dcraftasm" }),
    );
    run(&mut s, "file.close", json!({}));
    // A file as the save before 2026-11-18 wrote it under an absolute
    // path: as many `..` as the folder is deep, then the path from the
    // root.
    let abs = dir.join("work/abs");
    let depth = abs
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    let wrong = format!("{}model/v2/rod.3dcraft", "../".repeat(depth));
    let mut doc =
        craft3d_format::from_bytes(&std::fs::read(dir.join("work/rel/a.3dcraftasm")).unwrap())
            .unwrap();
    match &mut doc.assembly.as_mut().unwrap().instances[0].source {
        craft3d_doc::Source::File { path, .. } => *path = wrong.clone(),
        craft3d_doc::Source::Packed { .. } => panic!("packed"),
    }
    let broken = abs.join("c.3dcraftasm");
    std::fs::write(&broken, craft3d_format::to_bytes(&doc).unwrap()).unwrap();

    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({ "path": text(&broken) }));
    let i = run(&mut s, "assembly.inspect", json!({}));
    let rod = &i["instances"][0];
    assert_eq!(rod["status"], "failed", "{i}");
    let why = rod["error"].as_str().unwrap();
    assert!(why.contains("outside the read root"), "{why}");
    // The way back: point the instance at its file again.
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "Rod", "source": "model/v2/rod.3dcraft" }),
    );
    placed(&mut s, "../../model/v2/rod.3dcraft", "instance.edit");
    run(&mut s, "file.save", json!({}));
    assert_eq!(stored(&broken), "../../model/v2/rod.3dcraft");
    opens(&dir, "work/abs/c.3dcraftasm");

    // The rod gone from beneath the root: no such file, not "outside".
    std::fs::rename(
        dir.join("model/v2/rod.3dcraft"),
        dir.join("model/v2/moved.3dcraft"),
    )
    .unwrap();
    let mut s = session_in(&dir);
    run(
        &mut s,
        "file.open",
        json!({ "path": "work/rel/a.3dcraftasm" }),
    );
    let i = run(&mut s, "assembly.inspect", json!({}));
    let rod = &i["instances"][0];
    assert_eq!(rod["status"], "failed", "{i}");
    let why = rod["error"].as_str().unwrap();
    assert!(
        why.contains("there is no such file") && !why.contains("outside"),
        "{why}"
    );
}

/// A source path that climbs above the file system's root from the
/// assembly's folder leads to no file: a save into another folder is
/// refused, naming the instance and the way out, and writes nothing.
#[test]
fn a_source_that_leads_nowhere_is_refused_by_a_save_elsewhere() {
    let dir = workdir("asm-save-paths", "nowhere");
    folders(&dir, &["model/v2", "work/rel", "work/abs", "out"]);
    let mut s = session_in(&dir);
    rod_assembly(&mut s);
    run(
        &mut s,
        "file.save",
        json!({ "path": "work/rel/a.3dcraftasm" }),
    );
    run(&mut s, "file.close", json!({}));
    let abs = dir.join("work/abs");
    let depth = abs
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    let mut doc =
        craft3d_format::from_bytes(&std::fs::read(dir.join("work/rel/a.3dcraftasm")).unwrap())
            .unwrap();
    let nowhere = format!("{}rod.3dcraft", "../".repeat(depth + 2));
    match &mut doc.assembly.as_mut().unwrap().instances[0].source {
        craft3d_doc::Source::File { path, .. } => *path = nowhere.clone(),
        craft3d_doc::Source::Packed { .. } => panic!("packed"),
    }
    let file = abs.join("n.3dcraftasm");
    std::fs::write(&file, craft3d_format::to_bytes(&doc).unwrap()).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({ "path": text(&file) }));
    let target = dir.join("out/n.3dcraftasm");
    for path in [text(&target), "out/n.3dcraftasm".to_string()] {
        let e = error(&mut s, "file.save", json!({ "path": path }));
        assert!(
            e.contains("instance `Rod`")
                && e.contains("leads to no file")
                && e.contains("nothing was written")
                && e.contains("instance.edit"),
            "{e}"
        );
        assert!(!target.exists(), "{path}");
    }
    // Into its own folder nothing is rewritten: saved as it is.
    run(&mut s, "file.save", json!({}));
    assert_eq!(stored(&file), nowhere);
}

/// An agent's workspace at the window: these folders, read and written.
fn agent(roots: &[PathBuf]) -> Workspace {
    Workspace {
        read: Access::from_roots(roots.to_vec()),
        write: Access::from_roots(roots.to_vec()),
    }
}

/// The person's Save As at the window (`apps/craft3d/src/files.rs`,
/// `grant`): writing in the folder of the file chosen, no reading at all.
fn person(folder: &Path) -> Workspace {
    Workspace {
        read: Access::None,
        write: Access::Root(folder.to_path_buf()),
    }
}

/// The person saves the active assembly as `target` (absolute), as the
/// window does: the save's own answer, then the agent's workspace back.
fn saved_by_the_person(s: &mut Session, target: &Path, back: &Workspace) -> Value {
    let folder = target.parent().unwrap();
    s.set_workspace(person(folder));
    let saved = run(s, "file.save", json!({ "path": text(target) }));
    s.set_workspace(back.clone());
    saved
}

/// The assembly file `file` opened by its absolute path in a fresh
/// session that reads everywhere (the command line) and in one rooted at
/// `root`: every instance placed; the volume of its first instance.
fn reopened(root: &Path, file: &Path) -> f64 {
    let mut volume = None;
    for workspace in [
        Workspace::unrestricted(),
        Workspace {
            read: Access::Root(root.to_path_buf()),
            write: Access::None,
        },
    ] {
        let mut s = Session::new(workspace);
        run(&mut s, "file.open", json!({ "path": text(file) }));
        let i = run(&mut s, "assembly.inspect", json!({ "measures": true }));
        for instance in i["instances"].as_array().unwrap() {
            assert_eq!(instance["status"], "ok", "{}: {i}", file.display());
        }
        assert_eq!(i["bodies"], 1, "{}: {i}", file.display());
        volume = i["instances"][0]["measured"]["volume"].as_f64();
    }
    volume.unwrap()
}

/// The case the bug was met by (2026-11-18): an agent at the window adds
/// a part by its path relative to the folder it was given, and the
/// person saves the assembly, never saved, through the window into a
/// folder of their own, outside the agent's. The paths lead to the
/// agent's files; the open assembly stays placed, with the person's
/// access (none to read) and the agent's back; the file opens elsewhere.
#[test]
fn the_person_saves_an_agents_assembly_into_a_folder_of_their_own() {
    let dir = workdir("asm-save-paths", "person");
    folders(&dir, &["agent/model/v2", "home/me"]);
    let roots = [dir.join("agent")];
    let mut s = Session::new(agent(&roots));
    rod_assembly(&mut s);
    placed(&mut s, "model/v2/rod.3dcraft", "instance.add");
    let target = dir.join("home/me/clamp.3dcraftasm");
    let want = "../../agent/model/v2/rod.3dcraft";
    s.set_workspace(person(&dir.join("home/me")));
    let saved = run(&mut s, "file.save", json!({ "path": text(&target) }));
    assert!(saved.get("warnings").is_none(), "{saved}");
    assert_eq!(stored(&target), want);
    placed(&mut s, want, "the save, with the person's access");
    s.set_workspace(agent(&roots));
    placed(&mut s, want, "the save, with the agent's access");
    // As the window does when it has the focus back.
    run(&mut s, "assembly.follow", json!({}));
    placed(&mut s, want, "assembly.follow");
    assert!((reopened(&dir, &target) - 640.0).abs() < 1e-6);
    // Saved again by the agent, beneath its folder, and by the person
    // over their file: still the same rod.
    folders(&dir, &["agent/work"]);
    run(
        &mut s,
        "file.save",
        json!({ "path": "work/clamp.3dcraftasm" }),
    );
    let theirs = dir.join("agent/work/clamp.3dcraftasm");
    assert_eq!(stored(&theirs), "../model/v2/rod.3dcraft");
    placed(&mut s, "../model/v2/rod.3dcraft", "the agent's save");
    saved_by_the_person(&mut s, &target, &agent(&roots));
    assert_eq!(stored(&target), want);
    placed(&mut s, want, "the person's second save");
}

/// The same with two read roots: a relative path is read beneath the
/// first, and the paths lead there, not to the rod of the same name
/// beneath the second.
#[test]
fn with_two_read_roots_the_paths_lead_beneath_the_first() {
    let dir = workdir("asm-save-paths", "two-roots");
    folders(&dir, &["a/model/v2", "b/model/v2", "home"]);
    let mut other = session_in(&dir.join("b"));
    rod_part(&mut other, 20.0);
    let roots = [dir.join("a"), dir.join("b")];
    let mut s = Session::new(agent(&roots));
    rod_assembly(&mut s);
    let target = dir.join("home/two.3dcraftasm");
    saved_by_the_person(&mut s, &target, &agent(&roots));
    assert_eq!(stored(&target), "../a/model/v2/rod.3dcraft");
    placed(&mut s, "../a/model/v2/rod.3dcraft", "the save");
    assert!((reopened(&dir, &target) - 640.0).abs() < 1e-6);
}

/// A part added by its absolute path beneath the agent's folder, and an
/// undo and a redo of the addition before the person's save.
#[test]
fn a_part_added_by_its_absolute_path_and_an_undone_addition_save_as_well() {
    let dir = workdir("asm-save-paths", "absolute-source");
    folders(&dir, &["agent/model/v2", "home"]);
    let roots = [dir.join("agent")];
    let mut s = Session::new(agent(&roots));
    rod_part(&mut s, 40.0);
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    let rod = dir.join("agent/model/v2/rod.3dcraft");
    run(
        &mut s,
        "instance.add",
        json!({ "source": text(&rod), "name": "Rod" }),
    );
    placed(
        &mut s,
        "model/v2/rod.3dcraft",
        "instance.add by its absolute path",
    );
    run(&mut s, "edit.undo", json!({}));
    let i = run(&mut s, "assembly.inspect", json!({}));
    assert_eq!(i["instances"].as_array().unwrap().len(), 0, "{i}");
    run(&mut s, "edit.redo", json!({}));
    placed(&mut s, "model/v2/rod.3dcraft", "edit.redo");
    let target = dir.join("home/abs.3dcraftasm");
    saved_by_the_person(&mut s, &target, &agent(&roots));
    assert_eq!(stored(&target), "../agent/model/v2/rod.3dcraft");
    placed(&mut s, "../agent/model/v2/rod.3dcraft", "the save");
    // Undo after the save keeps the rewritten path and the rod placed.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    placed(&mut s, "../agent/model/v2/rod.3dcraft", "undo and redo");
    assert!((reopened(&dir, &target) - 640.0).abs() < 1e-6);
}

/// Two assemblies without a file in two tabs, made while the agent had
/// two different folders, each saved by the person: each leads to the
/// rod of its own folder (40 and 20 long). (Their paths differ: a path
/// relative to the agent's folder names the file of that name beneath
/// whatever folder the agent has when it is read again, so two tabs with
/// one relative path would follow one file.)
#[test]
fn two_assemblies_in_two_tabs_keep_their_own_folders() {
    let dir = workdir("asm-save-paths", "tabs");
    folders(&dir, &["a/model/v2", "b/parts", "home"]);
    let (a, b) = ([dir.join("a")], [dir.join("b")]);
    let mut s = Session::new(agent(&a));
    rod_assembly_of(&mut s, 40.0);
    s.set_workspace(agent(&b));
    rod_part_at(&mut s, "parts/short.3dcraft", 20.0);
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "parts/short.3dcraft", "name": "Rod" }),
    );
    let statuses = s.document_statuses();
    assert_eq!(statuses.len(), 2, "{statuses:?}");
    for (index, roots, name, want, volume) in [
        (0, &a, "one", "../a/model/v2/rod.3dcraft", 640.0),
        (1, &b, "two", "../b/parts/short.3dcraft", 320.0),
    ] {
        s.set_workspace(agent(roots));
        run(&mut s, "file.activate", json!({ "document": index }));
        let target = dir.join(format!("home/{name}.3dcraftasm"));
        saved_by_the_person(&mut s, &target, &agent(roots));
        assert_eq!(stored(&target), want, "{name}");
        placed(&mut s, want, name);
        assert!((reopened(&dir, &target) - volume).abs() < 1e-6, "{name}");
    }
}

/// Where the folder the sources are relative to is truly not known, the
/// save is still refused in words, nothing written: an assembly without
/// a file whose sources were never read in this session (here its bytes
/// opened as the browser opens a file), saved where there is no read
/// root. Read once where they can be, the same save goes through.
#[test]
fn an_assembly_whose_sources_were_never_read_here_is_refused_without_a_read_root() {
    let dir = workdir("asm-save-paths", "unknown");
    folders(&dir, &["model/v2", "home"]);
    let mut s = session_in(&dir);
    rod_assembly(&mut s);
    run(&mut s, "file.save", json!({ "path": "a.3dcraftasm" }));
    let bytes = std::fs::read(dir.join("a.3dcraftasm")).unwrap();
    let mut s = Session::new(person(&dir.join("home")));
    s.open_bytes("a.3dcraftasm", &bytes).unwrap();
    let target = dir.join("home/a.3dcraftasm");
    let e = error(&mut s, "file.save", json!({ "path": text(&target) }));
    assert!(
        e.contains("not known here") && e.contains("assembly.reload"),
        "{e}"
    );
    assert!(!target.exists());
    s.set_workspace(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::None,
    });
    run(&mut s, "assembly.reload", json!({}));
    placed(&mut s, "model/v2/rod.3dcraft", "assembly.reload");
    saved_by_the_person(&mut s, &target, &agent(std::slice::from_ref(&dir)));
    assert_eq!(stored(&target), "../model/v2/rod.3dcraft");
}
