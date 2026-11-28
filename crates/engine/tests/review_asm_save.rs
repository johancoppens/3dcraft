//! The reviewer's tests of `check/asm-save-path` (review-asm-save,
//! 2026-11-18), apart from the author's: every kind of old folder (no file
//! with the folder its sources were read beneath, a relative path, an
//! absolute one, a file opened by either) saved into every kind of new one
//! (relative and absolute, one and three folders deep, a sibling tree, the
//! sources' own folders, the same folder, its own file, the root itself,
//! and the person's Save As outside the agent's folder), each file opened
//! again in fresh sessions; then undo after the save and a save elsewhere,
//! A then B then A, and a save of the save. The assembly holds a packed
//! source (its file removed), a part and a subassembly with a relative
//! source of its own. Also: in memory; what is shown after a save against
//! what a fresh session opens when the roots changed after the sources
//! were read, or two sources were read beneath two folders; a source that
//! changed on disk before a save; a read root that is not the write root;
//! the roots after such saves; the read errors' wording. Every name holds
//! `asm_paths`, so one filter runs them with `asm_paths.rs`'s unit tests.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{error, workdir};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::json;
use std::path::{Component, Path, PathBuf};

const ROD: &str = "model/v2/rod.3dcraft";
const SHORT: &str = "model/short.3dcraft";
const PACKED: &str = "model/p.3dcraft";
const SUB: &str = "sub/s.3dcraftasm";

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_string()
}

/// An agent at the window: its folder, read and written.
fn agent(root: &Path) -> Workspace {
    Workspace {
        read: Access::Root(root.to_path_buf()),
        write: Access::Root(root.to_path_buf()),
    }
}

/// The person's Save As at the window (`apps/craft3d/src/files.rs`,
/// `grant`): writing in the folder of the file chosen, no reading.
fn person(folder: &Path) -> Workspace {
    Workspace {
        read: Access::None,
        write: Access::Root(folder.to_path_buf()),
    }
}

fn reader(root: &Path) -> Workspace {
    Workspace {
        read: Access::Root(root.to_path_buf()),
        write: Access::None,
    }
}

/// A block `length` × 4 × 4 saved at `path`.
fn part(s: &mut Session, path: &str, length: f64) {
    run(s, "file.new", json!({ "units": "mm" }));
    run(
        s,
        "feature.block",
        json!({ "name": "Block", "size": [length, 4, 4] }),
    );
    run(s, "file.save", json!({ "path": path }));
    run(s, "file.close", json!({}));
}

/// The rod (40 long), the short part (20), the part to pack (10) and the
/// subassembly `sub/s.3dcraftasm` holding the short part by
/// `../model/short.3dcraft`.
fn sources(s: &mut Session) {
    part(s, ROD, 40.0);
    part(s, SHORT, 20.0);
    part(s, PACKED, 10.0);
    run(s, "assembly.new", json!({ "units": "mm" }));
    run(
        s,
        "instance.add",
        json!({ "source": SHORT, "name": "Inner" }),
    );
    run(s, "file.save", json!({ "path": SUB }));
    run(s, "file.close", json!({}));
}

/// The assembly under review, without a file: `P` packed, `Rod` and `Sub`
/// by their files, `Rod` moved last (an undo step).
fn top(s: &mut Session) {
    run(s, "assembly.new", json!({ "units": "mm" }));
    run(s, "instance.add", json!({ "source": PACKED, "name": "P" }));
    run(s, "assembly.pack", json!({}));
    run(s, "instance.add", json!({ "source": ROD, "name": "Rod" }));
    run(s, "instance.add", json!({ "source": SUB, "name": "Sub" }));
    run(
        s,
        "instance.edit",
        json!({ "name": "Rod", "position": [5, 0, 0] }),
    );
}

/// What the open assembly shows: per instance its status and volume, the
/// error of a failed one, and the bodies.
fn summary(s: &mut Session) -> String {
    let i = match s.execute("assembly.inspect", &json!({ "measures": true })) {
        Ok(i) => i,
        Err(e) => return format!("assembly.inspect refused: {e}"),
    };
    let mut out: Vec<String> = Vec::new();
    for instance in i["instances"].as_array().cloned().unwrap_or_default() {
        let name = instance["name"].as_str().unwrap_or("?").to_string();
        let status = instance["status"].as_str().unwrap_or("?").to_string();
        let volume = instance["measured"]["volume"].as_f64().unwrap_or(f64::NAN);
        let mut entry = format!("{name}:{status}:{volume:.3}");
        if !instance["held"].is_null() {
            entry.push_str(":held");
        }
        if let Some(why) = instance["error"].as_str() {
            entry.push_str(&format!(" ({why})"));
        }
        out.push(entry);
    }
    out.push(format!("bodies={}", i["bodies"]));
    out.join(" | ")
}

fn stored_bytes(bytes: &[u8]) -> Vec<(String, String)> {
    let doc = craft3d_format::from_bytes(bytes).unwrap();
    doc.assembly
        .unwrap()
        .instances
        .iter()
        .map(|i| {
            let path = match &i.source {
                craft3d_doc::Source::File { path, .. } => path.clone(),
                craft3d_doc::Source::Packed { .. } => "packed".to_string(),
            };
            (i.name.clone(), path)
        })
        .collect()
}

/// The source paths the assembly file at `file` holds, by instance.
fn stored(file: &Path) -> Vec<(String, String)> {
    stored_bytes(&std::fs::read(file).unwrap())
}

fn names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_str().unwrap().to_string()),
            _ => None,
        })
        .collect()
}

/// The path from the folder `from` to `to`, both absolute: the oracle.
fn rel(from: &Path, to: &Path) -> String {
    let (a, b) = (names(from), names(to));
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts = vec!["..".to_string(); a.len() - common];
    parts.extend(b[common..].iter().cloned());
    parts.join("/")
}

/// The source paths an assembly file at `file` must hold, `root` the
/// folder of the sources.
fn wanted(root: &Path, file: &Path) -> Vec<(String, String)> {
    let folder = file.parent().unwrap();
    vec![
        ("P".to_string(), "packed".to_string()),
        ("Rod".to_string(), rel(folder, &root.join(ROD))),
        ("Sub".to_string(), rel(folder, &root.join(SUB))),
    ]
}

/// The assembly at `path` opened in a fresh session with `workspace`.
fn reopened(workspace: Workspace, path: &str) -> String {
    let mut s = Session::new(workspace);
    match s.execute("file.open", &json!({ "path": path })) {
        Ok(_) => summary(&mut s),
        Err(e) => format!("file.open refused: {e}"),
    }
}

fn expect(findings: &mut Vec<String>, case: &str, what: &str, got: &str, want: &str) {
    if got != want {
        findings.push(format!("{case}: {what}: {got}  [want {want}]"));
    }
}

/// `c`, holding `root` (the agent's folder, with the sources) and
/// `home/me` (the person's).
struct Layout {
    c: PathBuf,
    root: PathBuf,
}

fn layout(name: &str) -> Layout {
    let c = workdir("review-asm-save", name);
    let root = c.join("root");
    for folder in ["model/v2", "sub", "o/r", "o/b", "n1/n2/n3", "s/t", "u/v"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    std::fs::create_dir_all(c.join("home/me/deep/er")).unwrap();
    Layout { c, root }
}

#[derive(Clone, Copy, Debug)]
enum Old {
    Unsaved,
    RelSaved,
    AbsSaved,
    RelOpened,
    AbsOpened,
}

#[derive(Clone, Copy, Debug)]
enum New {
    /// Beneath the agent's root, by a relative path.
    Rel(&'static str),
    /// The same, by its absolute path.
    Abs(&'static str),
    /// The person's Save As, this path beneath `c`.
    Person(&'static str),
}

const PLACES: [&str; 10] = [
    "x.3dcraftasm",
    "n1/x.3dcraftasm",
    "n1/n2/n3/x.3dcraftasm",
    "s/t/x.3dcraftasm",
    "sub/x.3dcraftasm",
    "model/v2/x.3dcraftasm",
    "o/r/x.3dcraftasm",
    "o/b/x.3dcraftasm",
    "o/r/a.3dcraftasm",
    "o/b/a.3dcraftasm",
];
const PERSON: [&str; 4] = [
    "home/me/x.3dcraftasm",
    "home/me/deep/er/x.3dcraftasm",
    "root/o/r/p.3dcraftasm",
    "root/x2.3dcraftasm",
];

fn news() -> Vec<New> {
    let mut out: Vec<New> = PLACES.iter().map(|&p| New::Rel(p)).collect();
    out.extend(PLACES.iter().map(|&p| New::Abs(p)));
    out.extend(PERSON.iter().map(|&p| New::Person(p)));
    out
}

/// The `path` a save of `new` is given, the file it writes and the
/// workspace it runs with.
fn target(l: &Layout, new: New) -> (String, PathBuf, Workspace) {
    match new {
        New::Rel(p) => (p.to_string(), l.root.join(p), agent(&l.root)),
        New::Abs(p) => (text(&l.root.join(p)), l.root.join(p), agent(&l.root)),
        New::Person(p) => {
            let file = l.c.join(p);
            let folder = file.parent().unwrap().to_path_buf();
            (text(&file), file, person(&folder))
        }
    }
}

/// The assembly in the state `old`, in a session with the agent's
/// workspace.
fn old_state(old: Old, l: &Layout) -> Session {
    let mut s = Session::new(agent(&l.root));
    sources(&mut s);
    top(&mut s);
    std::fs::remove_file(l.root.join(PACKED)).unwrap();
    let path = match old {
        Old::Unsaved => return s,
        Old::RelSaved | Old::RelOpened => "o/r/a.3dcraftasm".to_string(),
        Old::AbsSaved | Old::AbsOpened => text(&l.root.join("o/b/a.3dcraftasm")),
    };
    run(&mut s, "file.save", json!({ "path": path }));
    if matches!(old, Old::RelSaved | Old::AbsSaved) {
        return s;
    }
    let mut s = Session::new(agent(&l.root));
    run(&mut s, "file.open", json!({ "path": path }));
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "Rod", "position": [6, 0, 0] }),
    );
    s
}

/// The file a save wrote: its paths, and opened in fresh sessions (with
/// access to every file, beneath `c`, and the agent's by a relative path
/// where it is beneath the root).
fn written(
    findings: &mut Vec<String>,
    case: &str,
    l: &Layout,
    file: &Path,
    baseline: &str,
    what: &str,
) {
    if !file.exists() {
        findings.push(format!("{case}: {what}: no file at {}", file.display()));
        return;
    }
    let got = stored(file);
    let want = wanted(&l.root, file);
    if got != want {
        findings.push(format!("{case}: {what}: stored {got:?}, want {want:?}"));
    }
    let absolute = text(file);
    expect(
        findings,
        case,
        &format!("{what}, opened with access to every file"),
        &reopened(Workspace::unrestricted(), &absolute),
        baseline,
    );
    expect(
        findings,
        case,
        &format!("{what}, opened beneath the case folder"),
        &reopened(reader(&l.c), &absolute),
        baseline,
    );
    if let Ok(inside) = file.strip_prefix(&l.root) {
        let relative = text(inside);
        expect(
            findings,
            case,
            &format!("{what}, opened by the agent as {relative}"),
            &reopened(agent(&l.root), &relative),
            baseline,
        );
    }
}

fn one_case(
    findings: &mut Vec<String>,
    case: &str,
    l: &Layout,
    s: &mut Session,
    new: New,
    baseline: &str,
) {
    let (path, file, workspace) = target(l, new);
    s.set_workspace(workspace.clone());
    if let Err(e) = s.execute("file.save", &json!({ "path": path })) {
        findings.push(format!("{case}: the save was refused: {e}"));
        s.set_workspace(agent(&l.root));
        return;
    }
    expect(
        findings,
        case,
        "shown after the save, its access",
        &summary(s),
        baseline,
    );
    s.set_workspace(agent(&l.root));
    expect(
        findings,
        case,
        "shown after the save, the agent's access",
        &summary(s),
        baseline,
    );
    let followed = run(s, "assembly.follow", json!({}));
    if followed["failed"] != 0 {
        findings.push(format!(
            "{case}: assembly.follow after the save: {followed}"
        ));
    }
    expect(
        findings,
        case,
        "shown after assembly.follow",
        &summary(s),
        baseline,
    );
    written(findings, case, l, &file, baseline, "the save");
    // Undo after the save, then a save elsewhere (B).
    run(s, "edit.undo", json!({}));
    expect(
        findings,
        case,
        "shown after edit.undo",
        &summary(s),
        baseline,
    );
    let elsewhere = "u/v/y.3dcraftasm";
    match s.execute("file.save", &json!({ "path": elsewhere })) {
        Ok(_) => written(
            findings,
            case,
            l,
            &l.root.join(elsewhere),
            baseline,
            "a save after edit.undo",
        ),
        Err(e) => findings.push(format!("{case}: a save after edit.undo was refused: {e}")),
    }
    run(s, "edit.redo", json!({}));
    expect(
        findings,
        case,
        "shown after edit.redo",
        &summary(s),
        baseline,
    );
    // A again, then a save of the save.
    s.set_workspace(workspace.clone());
    for (what, params) in [
        ("A again", json!({ "path": path })),
        ("a save of the save", json!({})),
    ] {
        match s.execute("file.save", &params) {
            Ok(_) => written(findings, case, l, &file, baseline, what),
            Err(e) => findings.push(format!("{case}: {what} was refused: {e}")),
        }
    }
    s.set_workspace(agent(&l.root));
    expect(findings, case, "shown at the end", &summary(s), baseline);
}

fn matrix(old: Old) {
    let mut findings: Vec<String> = Vec::new();
    let mut cases = 0;
    for (k, new) in news().into_iter().enumerate() {
        let case = format!("{old:?} -> {new:?}");
        let l = layout(&format!("{old:?}-{k}"));
        let mut s = old_state(old, &l);
        let baseline = summary(&mut s);
        let whole = baseline.contains("P:ok:160.000")
            && baseline.contains("Rod:ok:640.000")
            && baseline.contains("Sub:ok:")
            && !baseline.contains("failed");
        if !whole {
            findings.push(format!("{case}: not whole before the save: {baseline}"));
            continue;
        }
        cases += 1;
        one_case(&mut findings, &case, &l, &mut s, new, &baseline);
    }
    println!(
        "review matrix {old:?}: {cases} cases, {} findings",
        findings.len()
    );
    for f in &findings {
        println!("FINDING {f}");
    }
    assert!(
        findings.is_empty(),
        "{old:?}: {} findings, first: {:?}",
        findings.len(),
        findings.first()
    );
}

#[test]
fn asm_paths_review_matrix_unsaved() {
    matrix(Old::Unsaved);
}

#[test]
fn asm_paths_review_matrix_relative_saved() {
    matrix(Old::RelSaved);
}

#[test]
fn asm_paths_review_matrix_absolute_saved() {
    matrix(Old::AbsSaved);
}

#[test]
fn asm_paths_review_matrix_relative_opened() {
    matrix(Old::RelOpened);
}

#[test]
fn asm_paths_review_matrix_absolute_opened() {
    matrix(Old::AbsOpened);
}

/// A file in the session's memory, left there.
fn copy(s: &mut Session, name: &str) -> Vec<u8> {
    let bytes = s.take_file(name).unwrap();
    s.put_file(name, bytes.clone()).unwrap();
    bytes
}

/// In memory, as in the browser: the same, relative paths only.
#[test]
fn asm_paths_review_matrix_in_memory() {
    let mut findings: Vec<String> = Vec::new();
    let fake = Path::new("/m");
    for saved_first in [false, true] {
        for place in PLACES {
            let case = format!("memory, saved first {saved_first} -> {place}");
            let mut s = Session::new(Workspace::memory());
            sources(&mut s);
            top(&mut s);
            s.take_file(PACKED).unwrap();
            if saved_first {
                run(&mut s, "file.save", json!({ "path": "o/r/a.3dcraftasm" }));
            }
            let baseline = summary(&mut s);
            if baseline.contains("failed") || !baseline.contains("Rod:ok:640.000") {
                findings.push(format!("{case}: not whole before the save: {baseline}"));
                continue;
            }
            let save = |s: &mut Session, path: &str, findings: &mut Vec<String>| {
                if let Err(e) = s.execute("file.save", &json!({ "path": path })) {
                    findings.push(format!("{case}: the save to {path} was refused: {e}"));
                }
            };
            save(&mut s, place, &mut findings);
            expect(
                &mut findings,
                &case,
                "shown after the save",
                &summary(&mut s),
                &baseline,
            );
            run(&mut s, "edit.undo", json!({}));
            save(&mut s, "u/v/y.3dcraftasm", &mut findings);
            run(&mut s, "edit.redo", json!({}));
            save(&mut s, place, &mut findings);
            expect(
                &mut findings,
                &case,
                "shown after A, B, A",
                &summary(&mut s),
                &baseline,
            );
            for file in [place, "u/v/y.3dcraftasm"] {
                let got = stored_bytes(&copy(&mut s, file));
                let want = wanted(fake, &fake.join(file));
                if got != want {
                    findings.push(format!("{case}: {file}: stored {got:?}, want {want:?}"));
                }
                let mut other = Session::new(Workspace::memory());
                for name in [ROD, SHORT, SUB, file] {
                    other.put_file(name, copy(&mut s, name)).unwrap();
                }
                let again = match other.execute("file.open", &json!({ "path": file })) {
                    Ok(_) => summary(&mut other),
                    Err(e) => format!("file.open refused: {e}"),
                };
                expect(
                    &mut findings,
                    &case,
                    &format!("{file} opened again"),
                    &again,
                    &baseline,
                );
            }
        }
    }
    for f in &findings {
        println!("FINDING {f}");
    }
    assert!(
        findings.is_empty(),
        "{} findings, first: {:?}",
        findings.len(),
        findings.first()
    );
}

/// The roots stay as strict: the person's assembly (sources outside the
/// agent's folder), open in the window, saved by the agent beneath its
/// folder, writes paths to the person's files, which the agent still
/// cannot read through them, nor through a fresh session; a save outside
/// the write root or through a link out of it is refused; a source through
/// a link out of the root is refused.
#[test]
fn asm_paths_review_the_roots_stay_as_strict() {
    let c = workdir("review-asm-save", "strict");
    let root = c.join("agent");
    for folder in [
        root.join("model"),
        root.join("out"),
        c.join("private/asm"),
        c.join("private/secret"),
        c.join("elsewhere"),
    ] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let secret = c.join("private/secret/part.3dcraft");
    let mine = root.join("model/mine.3dcraft");
    let mut me = Session::new(Workspace::unrestricted());
    part(&mut me, &text(&secret), 30.0);
    part(&mut me, &text(&mine), 10.0);
    part(&mut me, &text(&c.join("elsewhere/z.3dcraft")), 5.0);
    run(&mut me, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut me,
        "instance.add",
        json!({ "source": text(&secret), "name": "Secret" }),
    );
    run(
        &mut me,
        "instance.add",
        json!({ "source": text(&mine), "name": "Mine" }),
    );
    let asm = c.join("private/asm/a.3dcraftasm");
    run(&mut me, "file.save", json!({ "path": text(&asm) }));
    // The window: the person's file opened with the person's access, then
    // the agent's folder.
    let mut s = Session::new(Workspace {
        read: Access::Unrestricted,
        write: Access::None,
    });
    run(&mut s, "file.open", json!({ "path": text(&asm) }));
    let opened = summary(&mut s);
    assert!(
        !opened.contains("failed") && opened.contains("Secret:ok"),
        "{opened}"
    );
    s.set_workspace(agent(&root));
    let before = summary(&mut s);
    let saved = s
        .execute("file.save", &json!({ "path": "out/a.3dcraftasm" }))
        .map(|v| v.to_string())
        .map_err(|e| e.to_string());
    let after = summary(&mut s);
    let out = root.join("out/a.3dcraftasm");
    let stored_out = if out.exists() {
        stored(&out)
    } else {
        Vec::new()
    };
    let followed = run(&mut s, "assembly.follow", json!({}));
    let after_follow = summary(&mut s);
    let rooted = reopened(agent(&root), "out/a.3dcraftasm");
    let everywhere = reopened(Workspace::unrestricted(), &text(&out));
    println!("review strict: opened {opened}");
    println!("review strict: the agent before its save {before}");
    println!("review strict: save {saved:?}; stored {stored_out:?}");
    println!("review strict: after {after}");
    println!("review strict: follow {followed}; after it {after_follow}");
    println!("review strict: a fresh session at the agent's folder {rooted}");
    println!("review strict: a fresh session with access to every file {everywhere}");
    assert!(saved.is_ok(), "{saved:?}");
    assert_eq!(
        stored_out,
        vec![
            (
                "Secret".to_string(),
                "../../private/secret/part.3dcraft".to_string()
            ),
            ("Mine".to_string(), "../model/mine.3dcraft".to_string()),
        ]
    );
    // The agent's follow does not read the person's file: refused by the
    // root, in the root's words.
    assert_eq!(followed["failed"], 1, "{followed}");
    let why = followed["followed"][0]["failed"][0]["error"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        why.contains("read root") && !why.contains("no such file"),
        "{why}"
    );
    assert!(
        rooted.contains("Secret:failed")
            && rooted.contains("Mine:ok")
            && rooted.contains("read root"),
        "{rooted}"
    );
    assert!(!everywhere.contains("failed"), "{everywhere}");
    // Writing outside the write root, by its path or through a link: refused.
    let outside = c.join("elsewhere/x.3dcraftasm");
    let e = error(&mut s, "file.save", json!({ "path": text(&outside) }));
    assert!(e.contains("write root"), "{e}");
    assert!(!outside.exists());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(c.join("elsewhere"), root.join("lnk")).unwrap();
        let e = error(&mut s, "file.save", json!({ "path": "lnk/y.3dcraftasm" }));
        assert!(e.contains("write root"), "{e}");
        assert!(!c.join("elsewhere/y.3dcraftasm").exists());
        let e = error(
            &mut s,
            "instance.add",
            json!({ "source": "lnk/z.3dcraft", "name": "Z" }),
        );
        assert!(e.contains("read root"), "{e}");
        let e = error(
            &mut s,
            "instance.edit",
            json!({ "name": "Mine", "source": "../private/secret/part.3dcraft" }),
        );
        assert!(e.contains("read root"), "{e}");
    }
}

/// A source changed on disk by someone else, then the assembly saved: the
/// save no longer reads the saved assembly's sources; `assembly.follow`
/// does. Printed: what is shown between the two.
#[test]
fn asm_paths_review_a_source_changed_on_disk_before_a_save() {
    let l = layout("changed");
    let mut s = Session::new(agent(&l.root));
    part(&mut s, ROD, 40.0);
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": ROD, "name": "Rod" }),
    );
    run(&mut s, "file.save", json!({ "path": "o/r/a.3dcraftasm" }));
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut other = Session::new(agent(&l.root));
    part(&mut other, ROD, 60.0);
    let before = summary(&mut s);
    run(&mut s, "file.save", json!({ "path": "o/b/a.3dcraftasm" }));
    let after_save = summary(&mut s);
    let inspect = run(&mut s, "assembly.inspect", json!({}));
    let changed = inspect["instances"][0]["changed"].clone();
    let followed = run(&mut s, "assembly.follow", json!({}));
    let after_follow = summary(&mut s);
    let again = reopened(agent(&l.root), "o/b/a.3dcraftasm");
    println!("review changed: before the save {before}");
    println!("review changed: after the save {after_save}; changed {changed}");
    println!("review changed: follow {followed}; after it {after_follow}");
    println!("review changed: opened again {again}");
    assert!(after_follow.contains("Rod:ok:960.000"), "{after_follow}");
    assert!(again.contains("Rod:ok:960.000"), "{again}");
}

/// The agent's folder changed after the sources were read (the person
/// gives the agent another folder at the window), each folder with a rod of
/// that name (40 and 20 long). Saved into the second folder by a relative
/// path, by its absolute path, and by the person into a folder of their
/// own: is what a fresh session opens what the session shows?
#[test]
fn asm_paths_review_the_roots_changed_after_the_sources_were_read() {
    let c = workdir("review-asm-save", "roots-changed");
    let (r1, r2) = (c.join("r1"), c.join("r2"));
    for folder in [r1.join("model/v2"), r2.join("model/v2"), c.join("home/me")] {
        std::fs::create_dir_all(folder).unwrap();
    }
    part(&mut Session::new(agent(&r1)), ROD, 40.0);
    part(&mut Session::new(agent(&r2)), ROD, 20.0);
    let mut differ: Vec<String> = Vec::new();
    for how in ["relative", "absolute", "person"] {
        let mut s = Session::new(agent(&r1));
        run(&mut s, "assembly.new", json!({ "units": "mm" }));
        run(
            &mut s,
            "instance.add",
            json!({ "source": ROD, "name": "Rod" }),
        );
        s.set_workspace(agent(&r2));
        let shown = summary(&mut s);
        let (path, file, workspace) = match how {
            "relative" => (
                "a.3dcraftasm".to_string(),
                r2.join("a.3dcraftasm"),
                agent(&r2),
            ),
            "absolute" => (
                text(&r2.join("b.3dcraftasm")),
                r2.join("b.3dcraftasm"),
                agent(&r2),
            ),
            _ => {
                let file = c.join("home/me/c.3dcraftasm");
                (text(&file), file, person(&c.join("home/me")))
            }
        };
        s.set_workspace(workspace);
        let saved = s
            .execute("file.save", &json!({ "path": path }))
            .map(|v| v.to_string())
            .map_err(|e| e.to_string());
        s.set_workspace(agent(&r2));
        let after = summary(&mut s);
        let again = reopened(Workspace::unrestricted(), &text(&file));
        let kept = if file.exists() {
            format!("{:?}", stored(&file))
        } else {
            "no file".to_string()
        };
        println!(
            "review roots changed, {how}: shown {shown}; save {saved:?}; stored {kept}; shown after {after}; opened again {again}"
        );
        if saved.is_ok() && again != after {
            differ.push(how.to_string());
        }
    }
    println!("review roots changed: opened again differs from shown for {differ:?}");
    // The relative save is as on main (a path relative to the agent's
    // folder names the file beneath its folder now); the branch's own
    // paths are the absolute save and the person's.
    assert!(!differ.iter().any(|h| h != "relative"), "{differ:?}");
}

/// Two sources read beneath two folders: the rod beneath the first, then
/// the agent given a second folder, and a part added from there. The
/// person saves: does each path lead to the file the session shows?
#[test]
fn asm_paths_review_two_sources_read_beneath_two_folders() {
    let mut differ: Vec<String> = Vec::new();
    for second_has_rod in [false, true] {
        let c = workdir("review-asm-save", &format!("two-folders-{second_has_rod}"));
        let (r1, r2) = (c.join("r1"), c.join("r2"));
        for folder in [
            r1.join("model/v2"),
            r2.join("model/v2"),
            r2.join("parts"),
            c.join("home/me"),
        ] {
            std::fs::create_dir_all(folder).unwrap();
        }
        part(&mut Session::new(agent(&r1)), ROD, 40.0);
        part(&mut Session::new(agent(&r2)), "parts/b.3dcraft", 10.0);
        if second_has_rod {
            part(&mut Session::new(agent(&r2)), ROD, 20.0);
        }
        let mut s = Session::new(agent(&r1));
        run(&mut s, "assembly.new", json!({ "units": "mm" }));
        run(
            &mut s,
            "instance.add",
            json!({ "source": ROD, "name": "A" }),
        );
        s.set_workspace(agent(&r2));
        run(
            &mut s,
            "instance.add",
            json!({ "source": "parts/b.3dcraft", "name": "B" }),
        );
        let shown = summary(&mut s);
        let file = c.join("home/me/two.3dcraftasm");
        s.set_workspace(person(&c.join("home/me")));
        let saved = s
            .execute("file.save", &json!({ "path": text(&file) }))
            .map(|v| v.to_string())
            .map_err(|e| e.to_string());
        s.set_workspace(agent(&r2));
        let after = summary(&mut s);
        let again = reopened(Workspace::unrestricted(), &text(&file));
        let kept = if file.exists() {
            format!("{:?}", stored(&file))
        } else {
            "no file".to_string()
        };
        run(&mut s, "assembly.follow", json!({}));
        let after_follow = summary(&mut s);
        println!(
            "review two folders, second has a rod {second_has_rod}: shown {shown}; save {saved:?}; stored {kept}; shown after {after}; opened again {again}; after assembly.follow {after_follow}"
        );
        if saved.is_ok() && again != after {
            differ.push(format!(
                "second has a rod {second_has_rod}: shown {after}, opened again {again}"
            ));
        }
    }
    assert!(differ.is_empty(), "{differ:#?}");
}

/// A read root that is not the write root (`serve --read-root a
/// --write-root b`): a relative save and an absolute one both lead to the
/// files read (the relative one wrote `model/v2/rod.3dcraft` beneath `b`
/// before the second round of asm-save-path, 2026-11-18), and the open
/// assembly goes on showing the rod.
#[test]
fn asm_paths_review_a_read_root_that_is_not_the_write_root() {
    let c = workdir("review-asm-save", "read-write");
    let (a, b) = (c.join("a"), c.join("b"));
    for folder in [a.join("model/v2"), b.clone()] {
        std::fs::create_dir_all(folder).unwrap();
    }
    part(&mut Session::new(agent(&a)), ROD, 40.0);
    let mut s = Session::new(Workspace {
        read: Access::Root(a.clone()),
        write: Access::Root(b.clone()),
    });
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": ROD, "name": "Rod" }),
    );
    let shown = summary(&mut s);
    run(&mut s, "file.save", json!({ "path": "x.3dcraftasm" }));
    let relative = reopened(Workspace::unrestricted(), &text(&b.join("x.3dcraftasm")));
    let after_relative = summary(&mut s);
    run(
        &mut s,
        "file.save",
        json!({ "path": text(&b.join("y.3dcraftasm")) }),
    );
    let absolute = reopened(Workspace::unrestricted(), &text(&b.join("y.3dcraftasm")));
    println!("review read root a, write root b: shown {shown}");
    println!(
        "review: x (relative) stored {:?}, opened again {relative}; shown after {after_relative}",
        stored(&b.join("x.3dcraftasm"))
    );
    println!(
        "review: y (absolute) stored {:?}, opened again {absolute}",
        stored(&b.join("y.3dcraftasm"))
    );
    assert!(absolute.contains("Rod:ok:640.000"), "{absolute}");
    assert_eq!(
        stored(&b.join("x.3dcraftasm"))[0].1,
        "../a/model/v2/rod.3dcraft"
    );
    assert!(relative.contains("Rod:ok:640.000"), "{relative}");
    assert!(
        after_relative.contains("Rod:ok:640.000"),
        "{after_relative}"
    );
}

/// An assembly opened from its bytes where nothing can be read (its
/// sources never read here), then given the agent's folder (its first read
/// root) and saved by the agent under an absolute path: the root stands
/// for its folder; the instances, failed until read, follow after
/// `assembly.follow`.
#[test]
fn asm_paths_review_an_assembly_never_read_saved_by_the_agent() {
    let l = layout("bytes");
    let mut s = Session::new(agent(&l.root));
    sources(&mut s);
    top(&mut s);
    run(&mut s, "file.save", json!({ "path": "a.3dcraftasm" }));
    let baseline = summary(&mut s);
    let bytes = std::fs::read(l.root.join("a.3dcraftasm")).unwrap();
    let mut s = Session::new(Workspace::none());
    s.open_bytes("a.3dcraftasm", &bytes).unwrap();
    let unread = summary(&mut s);
    s.set_workspace(agent(&l.root));
    let file = l.root.join("n1/n2/n3/x.3dcraftasm");
    let saved = s
        .execute("file.save", &json!({ "path": text(&file) }))
        .map(|v| v.to_string())
        .map_err(|e| e.to_string());
    let after = summary(&mut s);
    run(&mut s, "assembly.follow", json!({}));
    let after_follow = summary(&mut s);
    println!(
        "review never read: unread {unread}; save {saved:?}; after {after}; after follow {after_follow}"
    );
    assert!(saved.is_ok(), "{saved:?}");
    assert_eq!(stored(&file), wanted(&l.root, &file));
    assert_eq!(reopened(agent(&l.root), "n1/n2/n3/x.3dcraftasm"), baseline);
    assert_eq!(after_follow, baseline);
}

/// "there is no such file" only where there is none: not for a folder,
/// not for a file this user may not read, not for a path outside the
/// root; for a path through a file only where the system says that it
/// names no file (Windows).
#[test]
fn asm_paths_review_read_errors_say_what_is_so() {
    let l = layout("errors");
    let mut s = Session::new(agent(&l.root));
    part(&mut s, ROD, 40.0);
    std::fs::create_dir_all(l.root.join("model/dir.3dcraft")).unwrap();
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    // A path through a file: Unix says that a part of it is not a folder
    // (ENOTDIR); Windows says that the path is not found
    // (ERROR_PATH_NOT_FOUND, `NotFound`), as for a folder that is not
    // there. Both are so: there is no file by that path, and its name
    // and its folder are what to check. Pinned by the system itself, not
    // by the engine's words (public CI on Windows, 2026-11-23 and
    // 2026-11-27, expected "not no such file" as on Unix).
    let through = "model/v2/rod.3dcraft/x.3dcraft";
    let kind = std::fs::read(l.root.join(through)).unwrap_err().kind();
    assert_eq!(
        kind == std::io::ErrorKind::NotFound,
        cfg!(windows),
        "{kind:?}"
    );
    for (source, missing) in [
        ("model/none.3dcraft", true),
        ("model/v3/rod.3dcraft", true),
        (through, cfg!(windows)),
        ("model/dir.3dcraft", false),
        ("../outside.3dcraft", false),
    ] {
        let e = error(
            &mut s,
            "instance.add",
            json!({ "source": source, "name": "X" }),
        );
        println!("review read error, {source}: {e}");
        assert_eq!(
            e.contains("there is no such file"),
            missing,
            "{source}: {e}"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = l.root.join("model/locked.3dcraft");
        std::fs::copy(l.root.join(ROD), &locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&locked).is_err() {
            let e = error(
                &mut s,
                "instance.add",
                json!({ "source": "model/locked.3dcraft", "name": "X" }),
            );
            println!("review read error, locked: {e}");
            assert!(
                !e.contains("no such file") && e.contains("permission denied"),
                "{e}"
            );
        } else {
            println!("review read error, locked: this user reads any file, not tried");
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
}
