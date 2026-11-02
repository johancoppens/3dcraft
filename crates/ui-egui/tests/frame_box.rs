//! A window that follows the model frames it on a box that does not
//! depend on how finely the scene was sampled (`Scene::frame_box`): the
//! quick first picture after an edit and the display scene that follows
//! it are framed alike, so the camera does not move when the one turns
//! into the other, and a drawing of the view seen (`file.export` with
//! `view: "current"`) is the same whichever was there when it was asked.
//! The frame holds everything drawn: a sketch circle larger than the
//! bodies whole, and a freeform body by its mesh, not its control net.

#![allow(clippy::unwrap_used, clippy::panic)]

use craft3d_engine::{Scene, SceneQuality, Session, Workspace};
use craft3d_math::{Bbox3, Vec3};
use craft3d_ui_egui::shell::Shell;
use serde_json::json;
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
