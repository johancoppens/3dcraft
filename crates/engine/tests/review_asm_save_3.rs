//! The reviewer's third round on `check/asm-save-path-2` 1f2fe236
//! (review-asm-save, 2026-11-18): folders spelled as the file system
//! spells them (`real_folder`, `real_kept`) against the roots: a link
//! inside the root to inside it, one inside the root to outside it, the
//! root itself given through a link, a link outside the roots on the way
//! to the person's source; `file.save` without a path, `assembly.follow`,
//! `file.list`, two tabs by two spellings of one file, a part (not an
//! assembly) through a link. Every name holds `asm_paths` (one filter).

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

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_string()
}

fn agent(root: &Path) -> Workspace {
    Workspace {
        read: Access::Root(root.to_path_buf()),
        write: Access::Root(root.to_path_buf()),
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

fn paths_listed(s: &mut Session) -> String {
    let list = run(s, "file.list", json!({}));
    list.to_string()
}

#[cfg(unix)]
#[test]
fn asm_paths_review3_links_on_the_way_and_the_roots() {
    use std::os::unix::fs::symlink;
    let c = workdir("review-asm-save-3", "links");
    let root = c.join("root");
    for folder in [
        root.join("model/v2"),
        root.join("a/b"),
        c.join("private/asm"),
        c.join("hidden/target"),
        c.join("elsewhere"),
    ] {
        std::fs::create_dir_all(folder).unwrap();
    }
    symlink(root.join("a/b"), root.join("lnk")).unwrap();
    symlink(c.join("elsewhere"), root.join("out")).unwrap();
    symlink(c.join("hidden/target"), c.join("private/lnkdir")).unwrap();
    symlink(&root, c.join("alias")).unwrap();
    part(&mut Session::new(agent(&root)), ROD, 40.0);
    let rod = "Rod:ok:640.000";

    // 1. Saved by its absolute path through `lnk` (inside the root, to
    // inside it, a folder deeper): the path written from the link's
    // spelling (`../model/v2/rod.3dcraft`) would not lead to the rod from
    // the folder on disk, and since asm-save-path-2's second hand-in no
    // resolved path is kept, so the save is refused in words, nothing
    // written. By the folder's own path it is saved, saved again without a
    // path, followed, and opened again by each spelling (by the link's, as
    // on main: the paths are joined with the text given).
    let mut s = Session::new(agent(&root));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": ROD, "name": "Rod" }),
    );
    let through = root.join("lnk/x.3dcraftasm");
    let real = root.join("a/b/x.3dcraftasm");
    let refused = outcome(s.execute("file.save", &json!({ "path": text(&through) })));
    println!("review3 lnk: save through the link {refused:?}");
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.contains("symbolic link") && e.contains("nothing was written")),
        "{refused:?}"
    );
    assert!(!real.exists());
    let saved = run(&mut s, "file.save", json!({ "path": text(&real) }));
    let listed = paths_listed(&mut s);
    let again = outcome(s.execute("file.save", &json!({})));
    let followed = run(&mut s, "assembly.follow", json!({}));
    let now = shown(&mut s);
    let by_link = reopened(agent(&root), &text(&through));
    let by_real = reopened(agent(&root), &text(&real));
    let by_relative_link = reopened(agent(&root), "lnk/x.3dcraftasm");
    let by_relative_real = reopened(agent(&root), "a/b/x.3dcraftasm");
    let everywhere = reopened(Workspace::unrestricted(), &text(&real));
    println!(
        "review3 lnk: save {saved}; stored {}; file.list {listed}",
        stored(&real)
    );
    println!("review3 lnk: save without a path {again:?}; follow {followed}; shown {now}");
    println!(
        "review3 lnk: opened by the link {by_link}; by the real folder {by_real}; relative by the link {by_relative_link}; relative by the real folder {by_relative_real}; with every file {everywhere}"
    );
    assert!(again.is_ok(), "{again:?}");
    assert_eq!(followed["failed"], 0, "{followed}");
    assert_eq!(now, rod);
    assert_eq!(by_real, rod);
    assert_eq!(by_relative_real, rod);
    assert_eq!(everywhere, rod);

    // 2. The root given through a link (`alias`): opened by the alias's
    // spelling, saved without a path, followed.
    let mut t = Session::new(agent(&c.join("alias")));
    let opened = run(
        &mut t,
        "file.open",
        json!({ "path": text(&c.join("alias/a/b/x.3dcraftasm")) }),
    );
    let listed = paths_listed(&mut t);
    let again = outcome(t.execute("file.save", &json!({})));
    let followed = run(&mut t, "assembly.follow", json!({}));
    let now = shown(&mut t);
    println!(
        "review3 alias root: open {opened}; file.list {listed}; save without a path {again:?}; follow {followed}; shown {now}"
    );
    assert!(again.is_ok(), "{again:?}");
    assert_eq!(followed["failed"], 0, "{followed}");
    assert_eq!(now, rod);

    // 3. Through a link inside the root that leads outside it: nothing
    // written, by a relative or an absolute path; nothing opened.
    for path in [
        "out/y.3dcraftasm".to_string(),
        text(&root.join("out/y.3dcraftasm")),
    ] {
        let e = outcome(s.execute("file.save", &json!({ "path": path })));
        println!("review3 out: save {path}: {e:?}");
        assert!(e.as_ref().is_err_and(|e| e.contains("write root")), "{e:?}");
    }
    assert!(!c.join("elsewhere/y.3dcraftasm").exists());
    std::fs::copy(&real, c.join("elsewhere/z.3dcraftasm")).unwrap();
    let e = outcome(s.execute("file.open", &json!({ "path": "out/z.3dcraftasm" })));
    println!("review3 out: open out/z.3dcraftasm: {e:?}");
    assert!(e.as_ref().is_err_and(|e| e.contains("read root")), "{e:?}");

    // 4. Two tabs by the two spellings of one file.
    let mut u = Session::new(agent(&root));
    run(&mut u, "file.open", json!({ "path": text(&through) }));
    run(
        &mut u,
        "file.open",
        json!({ "path": text(&root.join("a/b/x.3dcraftasm")) }),
    );
    run(&mut u, "file.open", json!({ "path": "lnk/x.3dcraftasm" }));
    println!(
        "review3 three tabs (link, real, relative by the link): {}",
        paths_listed(&mut u)
    );

    // 5. A part (not an assembly) opened through the link: kept as given?
    part(&mut Session::new(agent(&root)), "a/b/p.3dcraft", 10.0);
    let mut v = Session::new(agent(&root));
    let opened = run(
        &mut v,
        "file.open",
        json!({ "path": text(&root.join("lnk/p.3dcraft")) }),
    );
    println!(
        "review3 a part through the link: {opened}; {}",
        paths_listed(&mut v)
    );

    // 6. The person's assembly with a source in a folder reached through
    // a link outside the agent's root (`private/lnkdir` -> `hidden/target`),
    // open in the window, saved by the agent beneath its root: what the
    // written path says of that folder, and that the agent still cannot
    // read it.
    let mut me = Session::new(Workspace::unrestricted());
    let secret = c.join("private/lnkdir/part.3dcraft");
    part(&mut me, &text(&secret), 30.0);
    run(&mut me, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut me,
        "instance.add",
        json!({ "source": text(&secret), "name": "Secret" }),
    );
    let asm = c.join("private/asm/a.3dcraftasm");
    run(&mut me, "file.save", json!({ "path": text(&asm) }));
    let mut w = Session::new(Workspace {
        read: Access::Unrestricted,
        write: Access::None,
    });
    run(&mut w, "file.open", json!({ "path": text(&asm) }));
    w.set_workspace(agent(&root));
    let saved = outcome(w.execute("file.save", &json!({ "path": "o.3dcraftasm" })));
    let followed = run(&mut w, "assembly.follow", json!({}));
    let fresh = reopened(agent(&root), "o.3dcraftasm");
    println!(
        "review3 outside link: the person's file stores {}; the agent's save {saved:?} stores {}; its follow {followed}; its fresh session {fresh}",
        stored(&asm),
        stored(&root.join("o.3dcraftasm"))
    );
    assert!(saved.is_ok(), "{saved:?}");
    assert_eq!(followed["failed"], 1, "{followed}");
    assert!(
        fresh.contains("Secret:failed") && fresh.contains("read root"),
        "{fresh}"
    );
}
