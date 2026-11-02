//! A window that follows the model frames it on a box that does not
//! depend on how finely the scene was sampled (`Scene::frame_box`): the
//! quick first picture after an edit and the display scene that follows
//! it are framed alike, so the camera does not move when the one turns
//! into the other, and a drawing of the view seen (`file.export` with
//! `view: "current"`) is the same whichever was there when it was asked.

#![allow(clippy::unwrap_used, clippy::panic)]

use craft3d_engine::{SceneQuality, Session, Workspace};
use craft3d_ui_egui::shell::Shell;
use serde_json::json;

fn drawing(session: &mut Session) -> Vec<u8> {
    session
        .execute(
            "file.export",
            &json!({"path": "seen.dxf", "view": "current", "projection": "orthographic", "hidden": "omit"}),
        )
        .unwrap();
    session.take_file("seen.dxf").unwrap()
}

#[test]
fn the_view_seen_is_the_same_before_and_after_the_display_scene() {
    for s in [1e-2, 1.0, 1e3] {
        let mm = |v: f64| format!("{} mm", v * s);
        let mut session = Session::new(Workspace::memory());
        session.set_stepwise(true);
        let mut shell = Shell::new();
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            (
                "feature.block",
                json!({"size": [mm(40.0), mm(20.0), mm(10.0)], "name": "Plate"}),
            ),
            (
                "feature.sphere",
                json!({"radius": mm(9.0), "position": [mm(70.0), "0 mm", "0 mm"], "name": "Ball"}),
            ),
            (
                "feature.cylinder",
                json!({"radius": mm(4.0), "height": mm(30.0), "position": [mm(0.0), mm(40.0), "0 mm"], "name": "Rod"}),
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
                &json!({"radius": mm(5.0), "position": [mm(0.0), mm(-40.0), mm(3.0)], "name": "Bead"}),
            )
            .unwrap();
        let mut steps = 0;
        while !session.scene_ready(SceneQuality::Preview) {
            session.prepare_some(display, || true);
            steps += 1;
            assert!(steps < 10_000, "no first picture at {s}");
        }
        let first = shell.sync(&mut session).unwrap();
        assert!(first.preview, "the quick first picture is shown at {s}");
        let camera = shell.state.camera;
        let early = drawing(&mut session);
        while session.prepare_some(display, || false).is_some() {}
        let shown = shell.sync(&mut session).unwrap();
        assert!(!shown.preview, "the display scene at {s}");
        // The meshes' boxes differ (a sphere is sampled more finely), the
        // frame does not.
        assert!(first.bbox != shown.bbox, "the samplings differ at {s}");
        assert!(first.frame_box() == shown.frame_box(), "at {s}");
        assert!(shell.state.camera == camera, "the camera moved at {s}");
        assert!(
            drawing(&mut session) == early,
            "the view seen changed at {s}"
        );
    }
}
