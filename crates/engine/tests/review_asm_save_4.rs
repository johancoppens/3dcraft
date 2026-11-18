//! The reviewer's fourth round on `check/asm-save-path-2` 2cd4f110
//! (review-asm-save, 2026-11-18): the root spelled two ways, both cases
//! held to an answer (the author's doubt about the second); a link in the
//! part of the path both folders share (`/home` a link to `/var/home`, as
//! some Linux systems have it) is no reason to refuse; an absolute save
//! keeps the path as given. Every name holds `asm_paths` (one filter).

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
use std::path::Path;

const ROD: &str = "model/v2/rod.3dcraft";
const ROD_SHOWN: &str = "Rod:ok:640.000";

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_string()
}

fn workspace(read: &Path, write: &Path) -> Workspace {
    Workspace {
        read: Access::Root(read.to_path_buf()),
        write: Access::Root(write.to_path_buf()),
    }
}

fn agent(root: &Path) -> Workspace {
    workspace(root, root)
}

fn person(folder: &Path) -> Workspace {
    Workspace {
        read: Access::None,
        write: Access::Root(folder.to_path_buf()),
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

fn shown(s: &mut Session) -> String {
    let i = match s.execute("assembly.inspect", &json!({ "measures": true })) {
        Ok(i) => i,
        Err(e) => return format!("assembly.inspect refused: {e}"),
    };
    let mut out: Vec<String> = Vec::new();
    for instance in i["instances"].as_array().cloned().unwrap_or_default() {
        let volume = instance["measured"]["volume"].as_f64().unwrap_or(f64::NAN);
        let mut entry = format!(
            "{}:{}:{volume:.3}",
            instance["name"].as_str().unwrap_or("?"),
            instance["status"].as_str().unwrap_or("?")
        );
        if let Some(why) = instance["error"].as_str() {
            entry.push_str(&format!(" ({why})"));
        }
        out.push(entry);
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
            craft3d_doc::Source::File { path, .. } => path.clone(),
            craft3d_doc::Source::Packed { .. } => "packed".to_string(),
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

fn rod_assembly(s: &mut Session) {
    run(s, "assembly.new", json!({ "units": "mm" }));
    run(s, "instance.add", json!({ "source": ROD, "name": "Rod" }));
}

/// `alias` a link to `root`. Case 1: read beneath `alias`, saved beneath
/// `root` by a relative path. Case 2: read root spelled `alias`, write
/// root `root`, a relative save (the honest save of round two). Both
/// saved, both written as `../model/v2/rod.3dcraft` or `model/…`, both
/// opened again by the agent at `root` and at `alias`, both followed.
#[cfg(unix)]
#[test]
fn asm_paths_review4_the_root_spelled_two_ways_both_cases() {
    let c = workdir("review-asm-save-4", "alias");
    let root = c.join("root");
    std::fs::create_dir_all(root.join("model/v2")).unwrap();
    std::fs::create_dir_all(root.join("n1")).unwrap();
    std::os::unix::fs::symlink(&root, c.join("alias")).unwrap();
    let alias = c.join("alias");
    part(&mut Session::new(agent(&root)), ROD, 40.0);
    let mut lines: Vec<String> = Vec::new();
    for (case, read_first, then, path, want) in [
        (
            "1: read beneath alias, saved beneath root",
            agent(&alias),
            agent(&root),
            "n1/x.3dcraftasm",
            "../model/v2/rod.3dcraft",
        ),
        (
            "2: read root alias, write root root",
            agent(&root),
            workspace(&alias, &root),
            "y.3dcraftasm",
            "model/v2/rod.3dcraft",
        ),
    ] {
        let mut s = Session::new(read_first);
        rod_assembly(&mut s);
        s.set_workspace(then);
        let saved = outcome(s.execute("file.save", &json!({ "path": path })));
        let file = root.join(path);
        let kept = stored(&file);
        let followed = run(&mut s, "assembly.follow", json!({}));
        let now = shown(&mut s);
        let at_root = reopened(agent(&root), path);
        let at_alias = reopened(agent(&alias), path);
        lines.push(format!(
            "{case}: save {saved:?}; stored {kept}; follow {followed}; shown {now}; opened at root {at_root}; at alias {at_alias}"
        ));
        println!("review4 {}", lines.last().unwrap());
        assert!(saved.is_ok(), "{case}: {saved:?}");
        assert_eq!(kept, want, "{case}");
        assert_eq!(followed["failed"], 0, "{case}: {followed}");
        assert_eq!(now, ROD_SHOWN, "{case}");
        assert_eq!(at_root, ROD_SHOWN, "{case}");
        assert_eq!(at_alias, ROD_SHOWN, "{case}");
    }
}

/// `home` a link to `var/home` (as `/home` is on some Linux systems), the
/// agent's folder and the person's both spelled through it: the person's
/// Save As climbs only below the link, so it is not refused, and the file
/// opens by either spelling. An absolute save keeps the path as given.
#[cfg(unix)]
#[test]
fn asm_paths_review4_a_link_both_folders_share_is_no_refusal() {
    let c = workdir("review-asm-save-4", "home-link");
    std::fs::create_dir_all(c.join("var/home/j/agent/model/v2")).unwrap();
    std::fs::create_dir_all(c.join("var/home/j/out")).unwrap();
    std::os::unix::fs::symlink(c.join("var/home"), c.join("home")).unwrap();
    let root = c.join("home/j/agent");
    let out = c.join("home/j/out");
    part(&mut Session::new(agent(&root)), ROD, 40.0);
    let mut s = Session::new(agent(&root));
    rod_assembly(&mut s);
    let file = out.join("x.3dcraftasm");
    s.set_workspace(person(&out));
    let saved = outcome(s.execute("file.save", &json!({ "path": text(&file) })));
    s.set_workspace(agent(&root));
    let kept = stored(&file);
    let by_link = reopened(Workspace::unrestricted(), &text(&file));
    let by_real = reopened(
        Workspace::unrestricted(),
        &text(&c.join("var/home/j/out/x.3dcraftasm")),
    );
    println!(
        "review4 home link: save {saved:?}; stored {kept}; opened by the link {by_link}; by the real path {by_real}"
    );
    assert!(saved.is_ok(), "{saved:?}");
    assert_eq!(kept, "../agent/model/v2/rod.3dcraft");
    assert_eq!(by_link, ROD_SHOWN);
    assert_eq!(by_real, ROD_SHOWN);
    // The agent's absolute save: its path is kept as given.
    let own = root.join("n.3dcraftasm");
    let saved = run(&mut s, "file.save", json!({ "path": text(&own) }));
    println!("review4 home link: the agent's absolute save {saved}");
    assert_eq!(saved["path"], text(&own), "{saved}");
}
