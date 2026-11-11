//! A window that follows the model frames it on a box that does not
//! depend on how finely the scene was sampled (`Scene::frame_box`): the
//! quick first picture after an edit and the display scene that follows
//! it are framed alike, so the camera does not move when the one turns
//! into the other, and a drawing of the view seen (`file.export` with
//! `view: "current"`) is the same whichever was there when it was asked.
//! The frame holds everything drawn: a sketch circle larger than the
//! bodies whole, and a freeform body by its mesh, not its control net.

#![allow(clippy::unwrap_used, clippy::panic)]

use craft3d_engine::{Access, Scene, SceneQuality, Session, Workspace};
use craft3d_math::{Bbox3, Vec3};
use craft3d_ui_egui::shell::Shell;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn drawing(session: &mut Session) -> Vec<u8> {
    session
        .execute(
            "file.export",
            &json!({"path": "seen.dxf", "view": "current", "projection": "orthographic", "hidden": "omit"}),
        )
        .unwrap();
    session.take_file("seen.dxf").unwrap()
}

/// `v` mm at scale `s`, `at` from the origin.
fn len(v: f64, s: f64, at: f64) -> String {
    format!("{} mm", v * s + at)
}

#[test]
fn the_view_seen_is_the_same_before_and_after_the_display_scene() {
    for (s, at) in [
        (1e-3, 0.0),
        (1e-2, 0.0),
        (1.0, 0.0),
        (1e3, 0.0),
        (1e6, 0.0),
        (1.0, 1e6),
    ] {
        let mm = |v: f64| len(v, s, at);
        let mut session = Session::new(Workspace::memory());
        session.set_stepwise(true);
        let mut shell = Shell::new();
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            (
                "feature.block",
                json!({"size": [len(40.0, s, 0.0), len(20.0, s, 0.0), len(10.0, s, 0.0)], "position": [mm(0.0), mm(0.0), mm(0.0)], "name": "Plate"}),
            ),
            (
                "feature.sphere",
                json!({"radius": len(9.0, s, 0.0), "position": [mm(70.0), mm(0.0), mm(0.0)], "name": "Ball"}),
            ),
            (
                "feature.cylinder",
                json!({"radius": len(4.0, s, 0.0), "height": len(30.0, s, 0.0), "position": [mm(0.0), mm(40.0), mm(0.0)], "name": "Rod"}),
            ),
        ] {
            session.execute(id, &params).unwrap();
        }
        let display = SceneQuality::display();
        while session.prepare_some(display, || false).is_some() {}
        shell.sync(&mut session);
        // An edit: the quick first picture first.
        session
            .execute(
                "feature.sphere",
                &json!({"radius": len(5.0, s, 0.0), "position": [mm(0.0), mm(-40.0), mm(3.0)], "name": "Bead"}),
            )
            .unwrap();
        let mut steps = 0;
        while !session.scene_ready(SceneQuality::Preview) {
            session.prepare_some(display, || true);
            steps += 1;
            assert!(steps < 10_000, "no first picture at {s}, {at}");
        }
        let first = shell.sync(&mut session).unwrap();
        assert!(
            first.preview,
            "the quick first picture is shown at {s}, {at}"
        );
        let camera = shell.state.camera;
        let early = drawing(&mut session);
        while session.prepare_some(display, || false).is_some() {}
        let shown = shell.sync(&mut session).unwrap();
        assert!(!shown.preview, "the display scene at {s}, {at}");
        // The meshes' boxes differ (a sphere is sampled more finely), the
        // frame does not.
        assert!(
            first.bbox != shown.bbox,
            "the samplings differ at {s}, {at}"
        );
        assert!(first.frame_box() == shown.frame_box(), "at {s}, {at}");
        assert!(
            shell.state.camera == camera,
            "the camera moved at {s}, {at}"
        );
        assert!(
            drawing(&mut session) == early,
            "the view seen changed at {s}, {at}"
        );
    }
}

fn scene(session: &mut Session) -> Arc<Scene> {
    session
        .scene_with(SceneQuality::display())
        .unwrap()
        .unwrap()
}

fn corners(b: &Bbox3) -> (Vec3, Vec3) {
    b.corners().unwrap()
}

/// A sketch circle larger than the body is framed whole (each of its
/// points, not one), and a freeform body by its mesh, not its control
/// net, which reaches far past it.
#[test]
fn the_frame_holds_a_large_circle_and_a_lofted_body_tightly() {
    for s in [1e-3, 1.0, 1e6] {
        let mm = |v: f64| format!("{} mm", v * s);
        let mut session = Session::new(Workspace::memory());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            (
                "feature.block",
                json!({"size": [mm(10.0), mm(10.0), mm(10.0)], "name": "Box"}),
            ),
            ("sketch.create", json!({"plane": "xy", "name": "Ring"})),
            (
                "sketch.circle",
                json!({"sketch": "Ring", "center": [mm(5.0), mm(5.0)], "radius": mm(50.0)}),
            ),
        ] {
            session.execute(id, &params).unwrap();
        }
        let (low, high) = corners(&scene(&mut session).frame_box());
        let slack = 1e-2 * 50.0 * s;
        for (got, want) in [
            (low.x, -45.0),
            (low.y, -45.0),
            (high.x, 55.0),
            (high.y, 55.0),
        ] {
            assert!(
                (got - want * s).abs() <= slack,
                "circle {got} vs {} at {s}",
                want * s
            );
        }

        // A loft whose middle section is the widest: interpolated, its
        // surface's control net bulges past the section.
        let mut session = Session::new(Workspace::memory());
        session
            .execute("file.new", &json!({"units": "mm"}))
            .unwrap();
        for (name, offset, half) in [("A", 0.0, 10.0), ("B", 15.0, 30.0), ("C", 30.0, 10.0)] {
            session
                .execute(
                    "sketch.create",
                    &json!({"plane": "xy", "name": name, "offset": mm(offset)}),
                )
                .unwrap();
            session
                .execute(
                    "sketch.rect",
                    &json!({"sketch": name, "from": [mm(-half), mm(-half)], "to": [mm(half), mm(half)]}),
                )
                .unwrap();
        }
        session
            .execute(
                "feature.loft",
                &json!({"sections": ["A", "B", "C"], "name": "Vase"}),
            )
            .unwrap();
        let drawn = scene(&mut session);
        let mesh = drawn
            .bodies
            .iter()
            .find(|b| b.name == "Vase")
            .unwrap()
            .mesh
            .bbox();
        let (mesh_low, mesh_high) = corners(&mesh);
        let (low, high) = corners(&drawn.frame_box());
        let pad = 2.0 * drawn.tolerance + 1e-9 * 60.0 * s;
        assert!(
            low.x >= mesh_low.x - pad && high.x <= mesh_high.x + pad,
            "frame {low:?}..{high:?} past the mesh {mesh_low:?}..{mesh_high:?} at {s}"
        );
        assert!(
            low.x <= mesh_low.x + pad && high.x >= mesh_high.x - pad,
            "at {s}"
        );
        // The section is 30 wide at its middle: the frame holds it.
        assert!(high.x >= 30.0 * s - pad, "at {s}");
    }
}

/// Fit, and the framing of a document as it opens, frame what is drawn:
/// a body hidden in the view does not count, nor a sketch while the
/// sketches are not shown (the ship propeller's expanded blade outline,
/// 670 mm below it, left the propeller small at the top of the view,
/// 2026-11-10). Shown again, Fit frames them too; with every body hidden
/// the frame is the scene's, as before.
#[test]
fn fit_frames_what_is_drawn() {
    let mut session = Session::new(Workspace::memory());
    let run = |session: &mut Session, id: &str, params: serde_json::Value| {
        session
            .execute(id, &params)
            .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    };
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [20, 20, 20], "position": [0, 0, 0], "name": "Near"}),
    );
    run(
        &mut session,
        "feature.block",
        json!({"size": [20, 20, 20], "position": [0, 0, -700], "name": "Far"}),
    );
    run(
        &mut session,
        "sketch.create",
        json!({"plane": "xy", "offset": "-1400 mm", "name": "Outline"}),
    );
    run(
        &mut session,
        "sketch.rect",
        json!({"sketch": "Outline", "from": [0, 0], "to": [100, 60]}),
    );
    run(
        &mut session,
        "feature.visible",
        json!({"feature": "Far", "visible": false}),
    );
    let box_of = |session: &mut Session, names: &[&str]| {
        let drawn = scene(session);
        let mut b = Bbox3::EMPTY;
        for body in drawn
            .bodies
            .iter()
            .filter(|b| names.contains(&b.name.as_str()))
        {
            let (low, high) = corners(&body.mesh.bbox());
            b.add(low);
            b.add(high);
        }
        b
    };
    let near = box_of(&mut session, &["Near"]);
    let at = |shell: &Shell| {
        let t = shell.state.camera.target;
        Vec3::new(t[0], t[1], t[2])
    };
    let centred = |shell: &Shell, b: &Bbox3, what: &str| {
        let want = b.center().unwrap();
        assert!(
            at(shell).distance(want) < 1e-6,
            "{what}: the camera looks at {:?}, not at {want:?}",
            at(shell)
        );
    };
    // As the document opens, with the sketches not shown: the near block.
    let mut shell = Shell::new();
    shell.state.show.sketches = false;
    shell.sync(&mut session);
    centred(&shell, &near, "opened");
    assert!(shell.fit(&mut session));
    centred(&shell, &near, "fit");
    // The far block shown: both.
    run(
        &mut session,
        "feature.visible",
        json!({"feature": "Far", "visible": true}),
    );
    assert!(shell.fit(&mut session));
    let both = box_of(&mut session, &["Near", "Far"]);
    centred(&shell, &both, "fit, both shown");
    // The sketches shown: the outline too, far below.
    shell.state.show.sketches = true;
    assert!(shell.fit(&mut session));
    assert!(
        at(&shell).z < both.center().unwrap().z - 100.0,
        "the outline is framed: {:?}",
        at(&shell)
    );
    // Every body hidden: the scene's own frame, the outline's.
    run(
        &mut session,
        "feature.visible",
        json!({"feature": ["Near", "Far"], "visible": false}),
    );
    shell.state.show.sketches = false;
    assert!(shell.fit(&mut session));
    let frame = scene(&mut session).frame_box();
    centred(&shell, &frame, "every body hidden");
    assert!((at(&shell).z + 1400.0).abs() < 1e-6, "{:?}", at(&shell));
}

/// Datums are hidden in a fresh window (decided 2026-11-11: published
/// Jigs opened on their datum planes, drawn as large as the model, over
/// it), and the frame never counts them: a document with a small block, a
/// datum plane far above it and a datum point far beside it opens framed
/// on the block, the camera the one of the block alone; with Datums
/// switched on, Fit frames the block alone still. Flying to the plane
/// (`ui.fit` with its name, what a double click on its row does) shows
/// the datums and frames the plane.
#[test]
fn the_opening_frame_and_fit_leave_out_the_datums() {
    let run = |session: &mut Session, id: &str, params: serde_json::Value| {
        session
            .execute(id, &params)
            .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    };
    let block = || {
        let mut session = Session::new(Workspace::memory());
        run(&mut session, "file.new", json!({"units": "mm"}));
        run(
            &mut session,
            "feature.block",
            json!({"size": [10, 10, 10], "position": [0, 0, 0], "name": "Block"}),
        );
        session
    };
    let mut bare = block();
    let mut session = block();
    run(
        &mut session,
        "datum.plane",
        json!({"from": "xy", "distance": 400, "name": "Lid"}),
    );
    run(
        &mut session,
        "datum.point",
        json!({"at": [600, 0, 0], "name": "Mark"}),
    );
    let drawn = scene(&mut session);
    assert_eq!(drawn.datums.len(), 2, "the plane and the point are placed");
    // A fresh window: the datums hidden, the sketches shown.
    let mut shell = Shell::new();
    assert!(!shell.state.show.datums);
    assert!(shell.state.show.sketches);
    let mut alone = Shell::new();
    alone.sync(&mut bare);
    // As the document opens: framed on the block, as the block alone is.
    shell.sync(&mut session);
    let opened = shell.state.camera;
    assert!(opened.auto_fit);
    assert_eq!(opened, alone.state.camera, "opened");
    assert_eq!(opened.target, [5.0, 5.0, 5.0]);
    // Datums switched on, the camera put elsewhere, then Fit: the block.
    let call =
        |shell: &mut Shell, session: &mut Session, method: &str, params: serde_json::Value| {
            shell
                .handle_method(session, method, Some(&params))
                .unwrap_or_else(|| panic!("{method} is not a ui method"))
                .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
        };
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"datums": true}, "camera": {"target": [300, 0, 200], "distance": 5000}}),
    );
    assert!(shell.state.show.datums);
    assert!(shell.fit(&mut session));
    assert_eq!(shell.state.camera, opened, "fit with the datums shown");
    assert!(shell.state.show.datums, "fit leaves them shown");
    // Hidden again, a flight to the plane shows them and frames it.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"datums": false}}),
    );
    call(
        &mut shell,
        &mut session,
        "ui.fit",
        json!({"feature": "Lid"}),
    );
    assert!(shell.state.show.datums, "the flight shows the datums");
    assert!(
        shell.state.camera.target[2] > 300.0,
        "the plane is framed: {:?}",
        shell.state.camera.target
    );
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["show"]["datums"], json!(true));
}

/// A folder of its own for the files of `name`, emptied.
fn folder(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("frame-box-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A session that reads and writes files in `dir`, on disk.
fn session_in(dir: &Path) -> Session {
    Session::new(Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    })
}

fn exec(session: &mut Session, id: &str, params: serde_json::Value) {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
}

/// The commands that add a 10 mm block, and a sketch with one point 2 m
/// away (a sketch of the sundial's kind: points far from the part).
fn block() -> (&'static str, serde_json::Value) {
    (
        "feature.block",
        json!({"size": [10, 10, 10], "position": [0, 0, 0], "name": "Block"}),
    )
}

fn far_point() -> [(&'static str, serde_json::Value); 2] {
    [
        ("sketch.create", json!({"plane": "xy", "name": "Far"})),
        ("sketch.point", json!({"sketch": "Far", "at": [2000, 0]})),
    ]
}

/// The camera `shell` would have fitted to `bbox` from where it looks now.
fn fitted(shell: &Shell, bbox: &Bbox3) -> craft3d_ui_egui::state::CameraState {
    let [w, h] = shell.viewport_size();
    let mut camera = shell.state.camera;
    camera.fit(bbox, f64::from(w) / f64::from(h));
    camera
}

/// Writes into `dir`, as a person or an agent saves them: `block.3dcraft`
/// (the block), `block-and-point.3dcraft` (the block and the far point)
/// and `hidden.3dcraft` (the same with the block hidden by its eye).
fn write_files(dir: &Path) {
    let mut maker = session_in(dir);
    exec(&mut maker, "file.new", json!({"units": "mm"}));
    let (id, params) = block();
    exec(&mut maker, id, params);
    exec(&mut maker, "file.save", json!({"path": "block.3dcraft"}));
    for (id, params) in far_point() {
        exec(&mut maker, id, params);
    }
    exec(
        &mut maker,
        "file.save",
        json!({"path": "block-and-point.3dcraft"}),
    );
    exec(
        &mut maker,
        "feature.visible",
        json!({"feature": "Block", "visible": false}),
    );
    exec(&mut maker, "file.save", json!({"path": "hidden.3dcraft"}));
}

/// A window showing `file` of `dir`, opened as File › Open opens it, and
/// its session.
fn opened(dir: &Path, file: &str) -> (Shell, Session) {
    let mut session = session_in(dir);
    exec(&mut session, "file.open", json!({ "path": file }));
    let mut shell = Shell::new();
    shell.sync(&mut session);
    (shell, session)
}

/// A new document, as File › New makes it in the window, follows
/// everything drawn as it is built, exactly as every document did before
/// 2026-11-11: the window shows it empty, then a 10 mm block, then a
/// sketch point 2 m away, and frames block and point without Fit being
/// asked; Fit gives that same camera.
#[test]
fn a_new_document_follows_everything_drawn_as_it_is_built() {
    let mut session = session_in(&folder("new"));
    let mut shell = Shell::new();
    exec(&mut session, "file.new", json!({"units": "mm"}));
    shell.sync(&mut session);
    let (id, params) = block();
    exec(&mut session, id, params);
    shell.sync(&mut session);
    assert_eq!(shell.state.camera.target, [5.0, 5.0, 5.0], "the block");
    for (id, params) in far_point() {
        exec(&mut session, id, params);
        shell.sync(&mut session);
    }
    let followed = shell.state.camera;
    let everything = scene(&mut session).frame_box();
    let (_, high) = corners(&everything);
    assert!(high.x >= 2000.0, "the scene's frame holds the point");
    assert_eq!(followed, fitted(&shell, &everything), "everything drawn");
    assert!(followed.target[0] > 900.0, "the point is in view");
    assert!(followed.auto_fit);
    assert!(shell.fit(&mut session));
    assert_eq!(shell.state.camera, followed, "what Fit gives");
}

/// A document opened from a file with a body is framed on the bodies
/// drawn (2026-11-11: the sundial opened with its dial a speck beside
/// rows of sketch points far away): the block and the point, saved and
/// opened again, open with the camera of the block alone opened, and the
/// camera keeps following the bodies as the model changes. Fit frames
/// everything drawn, the far point too, as before, and follows that from
/// then on. A new document in the same window follows everything drawn.
#[test]
fn a_document_opened_with_a_body_is_framed_on_its_bodies_until_fit() {
    let dir = folder("opened");
    write_files(&dir);
    let (alone, _) = opened(&dir, "block.3dcraft");
    let (mut shell, mut session) = opened(&dir, "block-and-point.3dcraft");
    let drawn = scene(&mut session);
    assert_eq!(drawn.sketches.len(), 1, "the point is drawn");
    let (_, high) = corners(&drawn.frame_box());
    assert!(high.x >= 2000.0, "the scene's frame holds the point");
    assert_eq!(
        shell.state.camera, alone.state.camera,
        "opened on the block"
    );
    assert_eq!(shell.state.camera.target, [5.0, 5.0, 5.0]);
    // A second block beside it: the camera follows the bodies, not the
    // point.
    exec(
        &mut session,
        "feature.block",
        json!({"size": [10, 10, 10], "position": [20, 0, 0], "name": "Beside"}),
    );
    shell.sync(&mut session);
    assert_eq!(shell.state.camera.target, [15.0, 5.0, 5.0], "the bodies");
    // Fit: everything drawn, the point too, as Fit framed it before.
    assert!(shell.fit(&mut session));
    let everything = scene(&mut session).frame_box();
    assert_eq!(shell.state.camera, fitted(&shell, &everything), "fit");
    assert!(shell.state.camera.target[0] > 900.0, "the point is framed");
    assert!(shell.state.camera.auto_fit);
    // From then on the camera follows everything drawn.
    exec(
        &mut session,
        "feature.block",
        json!({"size": [10, 10, 10], "position": [0, 0, 300], "name": "Above"}),
    );
    shell.sync(&mut session);
    let everything = scene(&mut session).frame_box();
    assert_eq!(shell.state.camera, fitted(&shell, &everything), "after fit");
    // File › New in the same window: the new document follows everything
    // drawn as it is built.
    exec(&mut session, "file.new", json!({"units": "mm"}));
    shell.sync(&mut session);
    let (id, params) = block();
    exec(&mut session, id, params);
    for (id, params) in far_point() {
        exec(&mut session, id, params);
    }
    shell.sync(&mut session);
    let everything = scene(&mut session).frame_box();
    assert_eq!(shell.state.camera, fitted(&shell, &everything), "a new one");
    assert!(shell.state.camera.target[0] > 900.0, "the point is in view");
}

/// Without a body drawn a document opens framed on everything drawn, as
/// before: a sketch alone opens on the sketch. A body hidden by its eye is
/// not drawn: the block and the point, saved with the block hidden and
/// opened, frame the far point, and with the block shown again the block
/// alone, as the block alone opens.
#[test]
fn without_a_body_drawn_a_document_opens_on_what_is_drawn() {
    let dir = folder("bodiless");
    write_files(&dir);
    let mut maker = session_in(&dir);
    for (id, params) in [
        ("file.new", json!({"units": "mm"})),
        ("sketch.create", json!({"plane": "xy", "name": "Outline"})),
        (
            "sketch.rect",
            json!({"sketch": "Outline", "from": [0, 0], "to": [100, 60]}),
        ),
        ("file.save", json!({"path": "outline.3dcraft"})),
    ] {
        exec(&mut maker, id, params);
    }
    let (shell, mut session) = opened(&dir, "outline.3dcraft");
    let sketch = scene(&mut session).frame_box();
    let (low, high) = corners(&sketch);
    assert!(
        high.x - low.x >= 100.0 && high.y - low.y >= 60.0,
        "{sketch:?}"
    );
    assert_eq!(
        shell.state.camera,
        fitted(&shell, &sketch),
        "a sketch alone"
    );

    let (alone, _) = opened(&dir, "block.3dcraft");
    let (mut shell, mut session) = opened(&dir, "hidden.3dcraft");
    assert_eq!(
        shell.state.camera.target,
        [2000.0, 0.0, 0.0],
        "the block hidden: the point"
    );
    exec(
        &mut session,
        "feature.visible",
        json!({"feature": "Block", "visible": true}),
    );
    shell.sync(&mut session);
    assert_eq!(
        shell.state.camera, alone.state.camera,
        "shown again: the block"
    );
}
