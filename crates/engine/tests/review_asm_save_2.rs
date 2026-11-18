//! The reviewer's second round on `check/asm-save-path` 43e60eaf
//! (review-asm-save, 2026-11-18): the last check before the write against
//! folders reached through a link, a root spelled two ways, a folder whose
//! name starts with two dots; per-source folders through undo, redo,
//! `instance.edit` to another source, an instance deleted and added again
//! by its name, and tab switches; whether the open assembly says it no
//! longer follows a source after a relative save with a read root that is
//! not the write root. Every name holds `asm_paths` (one filter).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::workdir;
use craft3d_engine::{Access, CommandError, Session, Workspace};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const ROD: &str = "model/v2/rod.3dcraft";

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_string()
}

fn agent(root: &Path) -> Workspace {
    Workspace {
        read: Access::Root(root.to_path_buf()),
        write: Access::Root(root.to_path_buf()),
    }
}

fn person(folder: &Path) -> Workspace {
    Workspace {
        read: Access::None,
        write: Access::Root(folder.to_path_buf()),
    }
}

fn split(read: &Path, write: &Path) -> Workspace {
    Workspace {
        read: Access::Root(read.to_path_buf()),
        write: Access::Root(write.to_path_buf()),
    }
}

fn reader(root: &Path) -> Workspace {
    Workspace {
        read: Access::Root(root.to_path_buf()),
        write: Access::None,
    }
}

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

fn outcome(result: Result<Value, CommandError>) -> Result<String, String> {
    result.map(|v| v.to_string()).map_err(|e| e.to_string())
}

/// Per instance its status and volume (`held` and errors left out: what
/// is shown, to compare).
fn shown(s: &mut Session) -> String {
    let i = match s.execute("assembly.inspect", &json!({ "measures": true })) {
        Ok(i) => i,
        Err(e) => return format!("assembly.inspect refused: {e}"),
    };
    let mut out: Vec<String> = Vec::new();
    for instance in i["instances"].as_array().cloned().unwrap_or_default() {
        let volume = instance["measured"]["volume"].as_f64().unwrap_or(f64::NAN);
        out.push(format!(
            "{}:{}:{volume:.3}",
            instance["name"].as_str().unwrap_or("?"),
            instance["status"].as_str().unwrap_or("?")
        ));
    }
    out.join(" | ")
}

/// [`shown`] with `held` and each failed instance's error.
fn told(s: &mut Session) -> String {
    let i = match s.execute("assembly.inspect", &json!({})) {
        Ok(i) => i,
        Err(e) => return format!("assembly.inspect refused: {e}"),
    };
    let mut out: Vec<String> = Vec::new();
    for instance in i["instances"].as_array().cloned().unwrap_or_default() {
        out.push(format!(
            "{}: held {} error {}",
            instance["name"], instance["held"], instance["error"]
        ));
    }
    out.join(" | ")
}

fn stored(file: &Path) -> String {
    let Ok(bytes) = std::fs::read(file) else {
        return "no file".to_string();
    };
    let doc = craft3d_format::from_bytes(&bytes).unwrap();
    let paths: Vec<String> = doc
        .assembly
        .unwrap()
        .instances
        .iter()
        .map(|i| match &i.source {
            craft3d_doc::Source::File { path, .. } => format!("{}={path}", i.name),
            craft3d_doc::Source::Packed { .. } => format!("{}=packed", i.name),
        })
        .collect();
    paths.join(", ")
}

fn reopened(workspace: Workspace, path: &str) -> String {
    let mut s = Session::new(workspace);
    match s.execute("file.open", &json!({ "path": path })) {
        Ok(_) => shown(&mut s),
        Err(e) => format!("file.open refused: {e}"),
    }
}

/// The person saves into a folder reached through a symbolic link
/// (`home/lnk` -> `real/deep/er`): the last check compares texts, so a
/// path that climbs out of the link's folder by `..` passes it; does the
/// file open where the command line opens it (by its path with its links
/// resolved, `workspace::kept_path`)? Two assemblies: the agent's without
/// a file (the branch's case), and one opened by its full path (the
/// rebase on main was as textual).
#[cfg(unix)]
#[test]
fn asm_paths_review2_a_save_into_a_folder_reached_through_a_link() {
    let c = workdir("review-asm-save-2", "link-folder");
    let root = c.join("root");
    std::fs::create_dir_all(root.join("model/v2")).unwrap();
    std::fs::create_dir_all(c.join("real/deep/er")).unwrap();
    std::fs::create_dir_all(c.join("home")).unwrap();
    std::os::unix::fs::symlink(c.join("real/deep/er"), c.join("home/lnk")).unwrap();
    part(&mut Session::new(agent(&root)), ROD, 40.0);
    let mut wrong: Vec<String> = Vec::new();
    for (k, kind) in ["the agent's, without a file", "opened by its full path"]
        .iter()
        .enumerate()
    {
        let mut s = Session::new(agent(&root));
        run(&mut s, "assembly.new", json!({ "units": "mm" }));
        run(
            &mut s,
            "instance.add",
            json!({ "source": ROD, "name": "Rod" }),
        );
        if k == 1 {
            let own = text(&root.join("a.3dcraftasm"));
            run(&mut s, "file.save", json!({ "path": own }));
            let mut p = Session::new(Workspace::unrestricted());
            run(&mut p, "file.open", json!({ "path": own }));
            s = p;
        }
        let before = shown(&mut s);
        let file = c.join(format!("home/lnk/{k}.3dcraftasm"));
        s.set_workspace(person(&c.join("home/lnk")));
        let saved = outcome(s.execute("file.save", &json!({ "path": text(&file) })));
        let through_link = reopened(Workspace::unrestricted(), &text(&file));
        let real = std::fs::canonicalize(&file)
            .map(|p| reopened(Workspace::unrestricted(), &text(&p)))
            .unwrap_or_else(|e| format!("no file: {e}"));
        let as_written = reopened(reader(&c), &text(&file));
        println!(
            "review2 link folder, {kind}: shown {before}; save {saved:?}; stored {}; opened with every file by the link {through_link}; by its real path {real}; beneath the case folder as written {as_written}",
            stored(&file)
        );
        if saved.is_ok() && (through_link != before || real != before) {
            wrong.push(format!(
                "{kind}: shown {before}, opened again {through_link}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The second check skips a path relative to the root that starts with
/// `..`, as climbing out of the root; a folder named `..x` does not climb.
/// A source read beneath `b`, the session then reading beneath `a` and
/// writing beneath `b`, a relative save: for `xx` the check refuses (the
/// session would read `a`'s file by it); for `..x` it is let through.
#[test]
fn asm_paths_review2_a_folder_whose_name_starts_with_two_dots() {
    let c = workdir("review-asm-save-2", "dots");
    let (a, b) = (c.join("a"), c.join("b"));
    let mut wrong: Vec<String> = Vec::new();
    for folder in ["xx", "..x"] {
        for root in [&a, &b] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        let source = format!("{folder}/rod.3dcraft");
        part(&mut Session::new(agent(&b)), &source, 40.0);
        part(&mut Session::new(agent(&a)), &source, 20.0);
        let mut s = Session::new(agent(&b));
        run(&mut s, "assembly.new", json!({ "units": "mm" }));
        run(
            &mut s,
            "instance.add",
            json!({ "source": source, "name": "Rod" }),
        );
        s.set_workspace(split(&a, &b));
        let before = shown(&mut s);
        let name = format!("{}.3dcraftasm", folder.replace('.', "d"));
        let saved = outcome(s.execute("file.save", &json!({ "path": name })));
        let after = shown(&mut s);
        let followed = run(&mut s, "assembly.follow", json!({}));
        let after_follow = shown(&mut s);
        let file = b.join(&name);
        let again = reopened(Workspace::unrestricted(), &text(&file));
        println!(
            "review2 folder {folder}: shown {before}; save {saved:?}; stored {}; after {after}; follow {followed}; after it {after_follow}; opened again {again}",
            stored(&file)
        );
        if saved.is_ok() && again != after_follow {
            wrong.push(format!(
                "{folder}: saved, the session now shows {after_follow}, the file opens with {again}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The limit of the author's point (2): a read root that is not the write
/// root, a relative save; the part then changed on disk. Is it said that
/// the open assembly no longer follows it, and where?
#[test]
fn asm_paths_review2_split_roots_after_a_relative_save() {
    let c = workdir("review-asm-save-2", "split");
    let (a, b) = (c.join("a"), c.join("b"));
    std::fs::create_dir_all(a.join("model/v2")).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    part(&mut Session::new(agent(&a)), ROD, 40.0);
    let mut s = Session::new(split(&a, &b));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": ROD, "name": "Rod" }),
    );
    let saved = run(&mut s, "file.save", json!({ "path": "x.3dcraftasm" }));
    // Said by the save (asm-save-path-2): a field, and a warning the status
    // bar counts.
    assert_eq!(saved["notFollowed"][0]["instance"], "Rod", "{saved}");
    assert_eq!(
        saved["notFollowed"][0]["path"], "../a/model/v2/rod.3dcraft",
        "{saved}"
    );
    let warning = saved["warnings"][0].as_str().unwrap_or_default();
    assert!(warning.contains("can no longer follow"), "{saved}");
    let after_save = told(&mut s);
    std::thread::sleep(std::time::Duration::from_millis(50));
    part(&mut Session::new(agent(&a)), ROD, 60.0);
    let followed = run(&mut s, "assembly.follow", json!({}));
    let after_follow = format!("{} / {}", shown(&mut s), told(&mut s));
    let file = b.join("x.3dcraftasm");
    let same_roots = reopened(split(&a, &b), &text(&file));
    let both = reopened(
        Workspace {
            read: Access::from_roots(vec![a.clone(), b.clone()]),
            write: Access::None,
        },
        &text(&file),
    );
    println!(
        "review2 split: save {saved}; stored {}; after the save {after_save}",
        stored(&file)
    );
    println!("review2 split: the rod made 60 long; follow {followed}; after it {after_follow}");
    println!(
        "review2 split: opened by its full path with the same roots: {same_roots}; reading a and b: {both}"
    );
    assert_eq!(followed["failed"], 1, "{followed}");
    assert!(after_follow.contains("Rod:ok:640.000"), "{after_follow}");
    assert!(both.contains("Rod:ok:960.000"), "{both}");
}

/// A root spelled two ways (`alias` -> `root`): read beneath `alias`, saved
/// with the root spelled `root`. And the read root spelled the other way
/// than the write root: an honest save refused?
#[cfg(unix)]
#[test]
fn asm_paths_review2_the_root_spelled_two_ways() {
    let c = workdir("review-asm-save-2", "alias");
    let root = c.join("root");
    std::fs::create_dir_all(root.join("model/v2")).unwrap();
    std::fs::create_dir_all(root.join("n1")).unwrap();
    std::os::unix::fs::symlink(&root, c.join("alias")).unwrap();
    let alias = c.join("alias");
    part(&mut Session::new(agent(&root)), ROD, 40.0);
    // Read beneath `alias`, saved beneath `root`.
    let mut s = Session::new(agent(&alias));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": ROD, "name": "Rod" }),
    );
    s.set_workspace(agent(&root));
    let saved = outcome(s.execute("file.save", &json!({ "path": "n1/x.3dcraftasm" })));
    let file = root.join("n1/x.3dcraftasm");
    let rooted = reopened(agent(&root), "n1/x.3dcraftasm");
    let everywhere = reopened(Workspace::unrestricted(), &text(&file));
    let follow = run(&mut s, "assembly.follow", json!({}));
    println!(
        "review2 alias: save {saved:?}; stored {}; the agent's fresh session {rooted}; every file {everywhere}; the open one's follow {follow}",
        stored(&file)
    );
    // Read beneath `root`, then read root `alias` and write root `root`.
    let mut s = Session::new(agent(&root));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": ROD, "name": "Rod" }),
    );
    s.set_workspace(split(&alias, &root));
    let refused = outcome(s.execute("file.save", &json!({ "path": "y.3dcraftasm" })));
    println!("review2 alias, read root spelled `alias`, write root `root`: save {refused:?}");
    assert!(rooted.contains("Rod:ok:640.000"), "{rooted}");
}

/// The person saves `home/me/<name>.3dcraftasm`: what is shown after the
/// save, and what a fresh session with every file opens.
fn person_saves(
    s: &mut Session,
    back: &Workspace,
    home: &Path,
    name: &str,
) -> (String, String, String) {
    let file = home.join(format!("{name}.3dcraftasm"));
    s.set_workspace(person(home));
    let saved = outcome(s.execute("file.save", &json!({ "path": text(&file) })));
    s.set_workspace(back.clone());
    let now = shown(s);
    let again = if saved.is_ok() {
        reopened(Workspace::unrestricted(), &text(&file))
    } else {
        format!("not saved: {saved:?}")
    };
    (format!("{saved:?}"), now, again)
}

/// Per-source folders: an instance read beneath `r1`, the agent given
/// `r2`, the instance pointed at a part of `r2`, undo and redo, deleted and
/// added again by its name (`r2`'s rod of that path), undone twice; a
/// second tab read beneath `r1`, the tabs switched. After each step the
/// person saves: what a fresh session opens is what was shown.
#[test]
fn asm_paths_review2_folders_per_source_through_undo_edit_delete_and_tabs() {
    let c = workdir("review-asm-save-2", "per-source");
    let (r1, r2, home) = (c.join("r1"), c.join("r2"), c.join("home/me"));
    for folder in [
        r1.join("model/v2"),
        r2.join("model/v2"),
        r2.join("parts"),
        home.clone(),
    ] {
        std::fs::create_dir_all(folder).unwrap();
    }
    part(&mut Session::new(agent(&r1)), ROD, 40.0);
    part(&mut Session::new(agent(&r2)), ROD, 20.0);
    part(&mut Session::new(agent(&r2)), "parts/b.3dcraft", 10.0);
    let mut s = Session::new(agent(&r1));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": ROD, "name": "A" }),
    );
    let w2 = agent(&r2);
    s.set_workspace(w2.clone());
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "A", "source": "parts/b.3dcraft" }),
    );
    let mut steps: Vec<(String, String, String, String)> = Vec::new();
    let mut step = |s: &mut Session, what: &str, back: &Workspace, n: usize| {
        let (saved, now, again) = person_saves(s, back, &home, &format!("s{n}"));
        steps.push((what.to_string(), saved, now, again));
    };
    step(&mut s, "A pointed at r2's part", &w2, 1);
    run(&mut s, "edit.undo", json!({}));
    step(&mut s, "undo: A back at r1's rod", &w2, 2);
    run(&mut s, "edit.redo", json!({}));
    step(&mut s, "redo: A at r2's part", &w2, 3);
    run(&mut s, "instance.delete", json!({ "name": "A" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": ROD, "name": "A" }),
    );
    step(&mut s, "A deleted and added again: r2's rod", &w2, 4);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    step(&mut s, "two undos: A at r2's part", &w2, 5);
    // A second tab, read beneath r1; then the first again, then the second.
    let w1 = agent(&r1);
    s.set_workspace(w1.clone());
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": ROD, "name": "C" }),
    );
    step(&mut s, "second tab, r1's rod", &w1, 6);
    s.set_workspace(w2.clone());
    run(&mut s, "file.activate", json!({ "document": 0 }));
    step(&mut s, "first tab again", &w2, 7);
    run(&mut s, "file.activate", json!({ "document": 1 }));
    step(&mut s, "second tab again, with r2", &w2, 8);
    let mut wrong: Vec<String> = Vec::new();
    for (what, saved, now, again) in &steps {
        println!("review2 per source, {what}: save {saved}; shown {now}; opened again {again}");
        if now != again {
            wrong.push(format!("{what}: shown {now}, opened again {again}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    let _: Option<PathBuf> = None;
}
