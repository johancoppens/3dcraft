//! The scene a viewer draws: built by commands, checked against
//! `view.render` and against coordinates worked out by hand.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{
    AUTOMATIC_RELATIVE_TOLERANCE, Access, DISPLAY_RELATIVE_TOLERANCE, FeatureState, Scene,
    ScenePolyline, SceneQuality, SceneSketch, Session, SketchCurve, Workspace,
};
use craft3d_math::Vec3;
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::path::PathBuf;
use std::sync::Arc;

fn session() -> Session {
    Session::new(Workspace::none())
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn scene(session: &mut Session, tolerance: Option<f64>) -> Arc<Scene> {
    session.scene(tolerance).unwrap().expect("a document")
}

/// A line, a circle, an arc and a construction line, the same on every
/// plane:
///
/// ```text
/// line          (1, 2) -> (4, 6)
/// circle        about (10, 0), radius 4
/// arc           about (0, 0) from (3, 0) counter-clockwise to (0, 3)
/// construction  (0, 0) -> (0, 9)
/// ```
fn draw(session: &mut Session, name: &str, plane: &str, offset: &str) {
    run(
        session,
        "sketch.create",
        json!({"name": name, "plane": plane, "offset": offset}),
    );
    run(
        session,
        "sketch.line",
        json!({"sketch": name, "from": [1, 2], "to": [4, 6]}),
    );
    run(
        session,
        "sketch.circle",
        json!({"sketch": name, "center": [10, 0], "radius": 4}),
    );
    run(
        session,
        "sketch.arc",
        json!({"sketch": name, "center": [0, 0], "start": [3, 0], "end": [0, 3]}),
    );
    run(
        session,
        "sketch.line",
        json!({"sketch": name, "from": [0, 0], "to": [0, 9], "construction": true}),
    );
}

/// A block, a cylinder and a sketch on each world plane, off the origin.
fn part(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    run(
        session,
        "feature.block",
        json!({"name": "Plate", "size": ["60 mm", "40 mm", "8 mm"]}),
    );
    run(
        session,
        "feature.cylinder",
        json!({"name": "Post", "position": [15, 20, 8], "radius": "6 mm", "height": "30 mm"}),
    );
    draw(session, "OnXy", "xy", "5 mm");
    draw(session, "OnYz", "yz", "7 mm");
    draw(session, "OnZx", "zx", "-3 mm");
}

fn sketch<'a>(scene: &'a Scene, name: &str) -> &'a SceneSketch {
    scene
        .sketches
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no sketch {name}"))
}

fn curve(sketch: &SceneSketch, curve: SketchCurve, construction: bool) -> &ScenePolyline {
    sketch
        .polylines
        .iter()
        .find(|p| p.curve == curve && p.construction == construction)
        .unwrap_or_else(|| panic!("{}: no {curve:?}", sketch.name))
}

fn near(a: Vec3, b: Vec3) -> bool {
    a.distance(b) <= 1e-9
}

#[test]
fn the_meshes_are_the_ones_view_render_uses() {
    let mut s = session();
    part(&mut s);
    // The automatic tolerance, and one given by hand.
    for tolerance in [None, Some(0.05), Some(2.0)] {
        let mut params = json!({"width": 64, "height": 48});
        if let Some(t) = tolerance {
            params["tolerance"] = t.into();
        }
        let rendered = run(&mut s, "view.render", params);
        let scene = scene(&mut s, tolerance);
        assert_eq!(
            scene.triangles() as u64,
            rendered["triangles"].as_u64().unwrap(),
            "tolerance {tolerance:?}"
        );
        // One body per solid feature, named and in history order, with the
        // face ids `view.render` reports.
        let bodies: Vec<(&str, &str, u64)> = scene
            .bodies
            .iter()
            .map(|b| {
                (
                    b.feature.as_str(),
                    b.name.as_str(),
                    u64::from(b.first_face_id),
                )
            })
            .collect();
        let legend = rendered["bodies"].as_array().unwrap();
        assert_eq!(bodies.len(), 2);
        assert_eq!((bodies[0].0, bodies[0].1), ("F1", "Plate"));
        assert_eq!((bodies[1].0, bodies[1].1), ("F2", "Post"));
        for (body, entry) in bodies.iter().zip(legend) {
            assert_eq!(entry["body"], body.1);
            assert_eq!(entry["firstFaceId"], body.2);
        }
        let per_body: usize = scene.bodies.iter().map(|b| b.mesh.triangles.len()).sum();
        assert_eq!(per_body, scene.triangles());
        // In the one mesh for the renderer every body keeps faces of its
        // own: a block has 6 and a cylinder 3 (two caps and the side), and
        // the cylinder's start where the legend says.
        let distinct = |ids: &[u32]| ids.iter().collect::<std::collections::BTreeSet<_>>().len();
        assert_eq!(distinct(&scene.bodies[0].mesh.face_ids), 6);
        assert_eq!(distinct(&scene.bodies[1].mesh.face_ids), 3);
        assert_eq!(distinct(&scene.mesh.face_ids), 9);
        let (plate, post) = scene
            .mesh
            .face_ids
            .split_at(scene.bodies[0].mesh.triangles.len());
        assert!(plate.iter().all(|&id| id < scene.bodies[1].first_face_id));
        assert!(post.iter().all(|&id| id >= scene.bodies[1].first_face_id));
        // And its triangles point at its own vertices.
        let vertices = scene.bodies[0].mesh.positions.len() as u32;
        let (plate, post) = scene
            .mesh
            .triangles
            .split_at(scene.bodies[0].mesh.triangles.len());
        assert!(plate.iter().flatten().all(|&v| v < vertices));
        assert!(post.iter().flatten().all(|&v| v >= vertices));
        assert_eq!(
            scene.mesh.positions.len(),
            scene
                .bodies
                .iter()
                .map(|b| b.mesh.positions.len())
                .sum::<usize>()
        );
        assert!(scene.problems.is_empty(), "{:?}", scene.problems);
        if let Some(t) = tolerance {
            assert_eq!(scene.tolerance, t);
        }
    }
    // A block is 12 triangles at any tolerance; a coarser cylinder has
    // fewer triangles than a fine one.
    let fine = scene(&mut s, Some(0.05));
    let coarse = scene(&mut s, Some(2.0));
    assert_eq!(fine.bodies[0].mesh.triangles.len(), 12);
    assert_eq!(coarse.bodies[0].mesh.triangles.len(), 12);
    assert!(coarse.bodies[1].mesh.triangles.len() < fine.bodies[1].mesh.triangles.len());
    // The box around everything: the plate (0..60, 0..40, 0..8), the post on
    // it (up to z = 38), and the sketches. The circle about (10, 0) with
    // radius 4 spans -4..4 along each sketch's own y: that is world y on
    // OnXy, world z on OnYz and world x on OnZx.
    let (lo, hi) = fine.bbox.corners().unwrap();
    assert!(near(lo, Vec3::new(-4.0, -4.0, -4.0)), "{lo:?}");
    assert!(near(hi, Vec3::new(60.0, 40.0, 38.0)), "{hi:?}");
}

#[test]
fn sketch_polylines_lie_on_their_plane_where_the_frame_puts_them() {
    let mut s = session();
    part(&mut s);
    let scene = scene(&mut s, None);
    assert_eq!(scene.sketches.len(), 3);
    // (sketch, the world axis of its normal, offset, sketch x, sketch y):
    // the axes follow cyclically, so every frame is right-handed.
    let planes = [
        ("OnXy", Vec3::Z, 5.0, Vec3::X, Vec3::Y),
        ("OnYz", Vec3::X, 7.0, Vec3::Y, Vec3::Z),
        ("OnZx", Vec3::Y, -3.0, Vec3::Z, Vec3::X),
    ];
    for (name, normal, offset, x_axis, y_axis) in planes {
        let sketch = sketch(&scene, name);
        assert!(near(sketch.normal, normal), "{name}");
        assert!(near(x_axis.cross(y_axis), normal), "{name}: right-handed");
        let at = |x: f64, y: f64| normal * offset + x_axis * x + y_axis * y;
        assert_eq!(sketch.polylines.len(), 4, "{name}");
        for polyline in &sketch.polylines {
            for p in &polyline.points {
                assert!(
                    (p.dot(normal) - offset).abs() <= 1e-12,
                    "{name} {}: {p:?} is off the plane",
                    polyline.entity
                );
            }
        }
        // The line, end to end.
        let line = curve(sketch, SketchCurve::Line, false);
        assert_eq!(line.points.len(), 2);
        assert!(near(line.points[0], at(1.0, 2.0)), "{name}");
        assert!(near(line.points[1], at(4.0, 6.0)), "{name}");
        // The construction line is flagged and nothing else is.
        let guide = curve(sketch, SketchCurve::Line, true);
        assert!(near(guide.points[0], at(0.0, 0.0)) && near(guide.points[1], at(0.0, 9.0)));
        assert_eq!(
            sketch.polylines.iter().filter(|p| p.construction).count(),
            1
        );

        // The circle: every sample on it, closed, counter-clockwise from
        // the sketch's +x, and no chord further from the true circle than
        // the tolerance. A chord's middle is its furthest point.
        let circle = curve(sketch, SketchCurve::Circle, false);
        let centre = at(10.0, 0.0);
        let n = circle.points.len() - 1;
        assert!(n >= 24, "{name}: {n} segments");
        assert_eq!(circle.points[0], circle.points[n], "{name}: closed");
        assert!(near(circle.points[0], at(14.0, 0.0)), "{name}");
        let mut turned = 0.0;
        for pair in circle.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!((a.distance(centre) - 4.0).abs() <= 1e-9, "{name}");
            let sag = 4.0 - a.lerp(b, 0.5).distance(centre);
            assert!(
                (0.0..=scene.tolerance * (1.0 + 1e-9)).contains(&sag),
                "{name}: a chord is {sag} from the circle, tolerance {}",
                scene.tolerance
            );
            // The signed angle of this step, seen against the normal.
            turned += (a - centre)
                .cross(b - centre)
                .dot(normal)
                .atan2((a - centre).dot(b - centre));
        }
        assert!((turned - TAU).abs() <= 1e-9, "{name}: turned {turned}");

        // The arc: a quarter turn from (3, 0) to (0, 3), not the three
        // quarters the other way round.
        let arc = curve(sketch, SketchCurve::Arc, false);
        let origin = at(0.0, 0.0);
        assert!(near(arc.points[0], at(3.0, 0.0)), "{name}");
        assert!(near(*arc.points.last().unwrap(), at(0.0, 3.0)), "{name}");
        let mut turned = 0.0;
        for pair in arc.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!((b.distance(origin) - 3.0).abs() <= 1e-9, "{name}");
            let sag = 3.0 - a.lerp(b, 0.5).distance(origin);
            assert!(sag <= scene.tolerance * (1.0 + 1e-9), "{name}: {sag}");
            turned += (a - origin)
                .cross(b - origin)
                .dot(normal)
                .atan2((a - origin).dot(b - origin));
        }
        assert!(
            (turned - FRAC_PI_2).abs() <= 1e-9,
            "{name}: turned {turned}"
        );
        // Its point entities: the two ends of each line, the centres and
        // the ends of the arc.
        assert_eq!(sketch.points.len(), 8, "{name}");
    }
}

#[test]
fn an_arc_the_long_way_round_is_three_quarters_of_a_turn() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    run(
        &mut s,
        "sketch.arc",
        json!({"sketch": "S", "center": [0, 0], "start": [0, 3], "end": [3, 0]}),
    );
    let scene = scene(&mut s, None);
    let arc = &scene.sketches[0].polylines[0];
    let mut turned = 0.0;
    for pair in arc.points.windows(2) {
        turned += pair[0]
            .cross(pair[1])
            .dot(Vec3::Z)
            .atan2(pair[0].dot(pair[1]));
    }
    assert!((turned - 1.5 * PI).abs() <= 1e-9, "turned {turned}");
    // It passes through (-3, 0) and (0, -3).
    for through in [Vec3::new(-3.0, 0.0, 0.0), Vec3::new(0.0, -3.0, 0.0)] {
        let nearest = arc
            .points
            .iter()
            .map(|p| p.distance(through))
            .fold(f64::INFINITY, f64::min);
        assert!(nearest < 0.5, "{through:?}: {nearest}");
    }
}

/// Rule 3: no scale is special. A sketch-only document has no body area to
/// take a tolerance from, so it comes from the size of the sketch.
#[test]
fn circles_are_within_tolerance_at_every_scale() {
    for radius in [1e-3, 1.0, 250.0, 1e6] {
        let mut s = session();
        run(&mut s, "file.new", json!({}));
        run(&mut s, "sketch.create", json!({"name": "S"}));
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "S", "center": [0, 0], "radius": radius}),
        );
        let scene = scene(&mut s, None);
        // A thousandth of the diagonal of the circle's box.
        let expected = radius * 2.0 * 2.0_f64.sqrt() * 1e-3;
        assert!(
            (scene.tolerance - expected).abs() <= 1e-9 * expected,
            "radius {radius}: tolerance {}",
            scene.tolerance
        );
        let points = &scene.sketches[0].polylines[0].points;
        for pair in points.windows(2) {
            let sag = radius - pair[0].lerp(pair[1], 0.5).length();
            assert!(
                (0.0..=scene.tolerance * (1.0 + 1e-9)).contains(&sag),
                "radius {radius}: {sag}"
            );
        }
        // And with a body in the document the bodies decide, as in
        // view.render.
        run(&mut s, "feature.sphere", json!({"radius": radius}));
        let with_body = self::scene(&mut s, None);
        let area = 4.0 * PI * radius * radius;
        let expected = area.sqrt() * 1e-3;
        assert!(
            (with_body.tolerance - expected).abs() <= 1e-9 * expected,
            "radius {radius}: tolerance {}",
            with_body.tolerance
        );
    }
}

#[test]
fn the_revision_changes_exactly_when_the_document_does() {
    let mut s = session();
    let mut revision = s.scene_revision();
    assert_eq!(s.scene_revision(), revision, "asking does not change it");
    assert!(s.scene(None).unwrap().is_none(), "no document, no scene");

    // (command, params, should it succeed, does the document change)
    let steps: Vec<(&str, Value, bool, bool)> = vec![
        ("file.new", json!({}), true, true),
        (
            "variable.set",
            json!({"name": "w", "expr": "10 mm"}),
            true,
            true,
        ),
        ("document.measure", json!({}), true, false),
        ("document.inspect", json!({}), true, false),
        // Setting a variable to what it is already is no edit.
        (
            "variable.set",
            json!({"name": "w", "expr": "10 mm"}),
            true,
            false,
        ),
        (
            "feature.block",
            json!({"size": ["w", "w", "w"]}),
            true,
            true,
        ),
        (
            "view.render",
            json!({"width": 32, "height": 32}),
            true,
            false,
        ),
        // Failures leave the document alone.
        ("feature.block", json!({"size": "nonsense"}), false, false),
        ("feature.delete", json!({"feature": "Nobody"}), false, false),
        ("no.such.command", json!({}), false, false),
        ("sketch.create", json!({"name": "S"}), true, true),
        (
            "sketch.circle",
            json!({"sketch": "S", "center": [0, 0], "radius": 2}),
            true,
            true,
        ),
        ("sketch.inspect", json!({"sketch": "S"}), true, false),
        ("edit.undo", json!({}), true, true),
        ("edit.redo", json!({}), true, true),
        // Another document becomes the active one, and goes away again.
        ("file.new", json!({}), true, true),
        ("document.check", json!({}), true, false),
        ("file.close", json!({"discard": true}), true, true),
    ];
    for (index, (command, params, succeeds, changes)) in steps.iter().enumerate() {
        let before = scene_or_none(&mut s);
        let outcome = s.execute(command, params);
        assert_eq!(
            outcome.is_ok(),
            *succeeds,
            "step {index} {command}: {outcome:?}"
        );
        let now = s.scene_revision();
        assert_eq!(
            now != revision,
            *changes,
            "step {index} {command}: revision {revision} -> {now}"
        );
        // The scene follows: the same `Arc` while nothing changed, a new
        // one carrying the new revision when something did.
        let after = scene_or_none(&mut s);
        match (&before, &after) {
            (Some(before), Some(after)) => {
                assert_eq!(
                    Arc::ptr_eq(before, after),
                    !changes,
                    "step {index} {command}"
                );
                assert_eq!(after.revision, now);
            }
            (None, Some(after)) => assert_eq!(after.revision, now),
            other => panic!("step {index} {command}: {:?}", other.1.is_some()),
        }
        revision = now;
    }
    // Back on the first document: the block and the sketch are there.
    let scene = scene(&mut s, None);
    assert_eq!((scene.bodies.len(), scene.sketches.len()), (1, 1));
    // Closing the last document leaves nothing to draw, and that is a
    // change too.
    run(&mut s, "file.close", json!({"discard": true}));
    assert_ne!(s.scene_revision(), revision);
    assert!(s.scene(None).unwrap().is_none());
}

fn scene_or_none(session: &mut Session) -> Option<Arc<Scene>> {
    session.scene(None).unwrap()
}

#[test]
fn the_cache_is_per_revision_and_per_tolerance() {
    let mut s = session();
    part(&mut s);
    let automatic = scene(&mut s, None);
    assert!(Arc::ptr_eq(&automatic, &scene(&mut s, None)));
    let coarse = scene(&mut s, Some(2.0));
    let fine = scene(&mut s, Some(0.01));
    // Three scenes, each meshed and sampled at its own tolerance.
    assert!(!Arc::ptr_eq(&automatic, &coarse) && !Arc::ptr_eq(&coarse, &fine));
    assert_eq!((coarse.tolerance, fine.tolerance), (2.0, 0.01));
    assert!(coarse.triangles() < automatic.triangles());
    assert!(automatic.triangles() < fine.triangles());
    let samples = |scene: &Scene| {
        curve(sketch(scene, "OnXy"), SketchCurve::Circle, false)
            .points
            .len()
    };
    assert!(samples(&coarse) < samples(&fine));
    // All three are still the cached ones, in any order.
    assert!(Arc::ptr_eq(&fine, &scene(&mut s, Some(0.01))));
    assert!(Arc::ptr_eq(&automatic, &scene(&mut s, None)));
    assert!(Arc::ptr_eq(&coarse, &scene(&mut s, Some(2.0))));
    // An edit drops them all.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Post", "height": "31 mm"}),
    );
    assert!(!Arc::ptr_eq(&coarse, &scene(&mut s, Some(2.0))));
    assert!(!Arc::ptr_eq(&automatic, &scene(&mut s, None)));
    // A tolerance that is not a positive number is refused, not meshed.
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let error = s.scene(Some(bad)).unwrap_err();
        assert!(error.message().contains("tolerance"), "{error}");
    }
    // Asking for many tolerances keeps a bounded number of scenes and
    // still answers.
    for k in 1..20 {
        let t = f64::from(k) * 0.1;
        assert_eq!(scene(&mut s, Some(t)).tolerance, t);
    }
}

#[test]
fn feature_rows_carry_status_error_and_degrees_of_freedom() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "10 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Good", "size": ["w", "w", "w"]}),
    );
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Off", "radius": 3}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Bad", "radius": "w / 2", "height": "w"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Free"}));
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Free", "from": [0, 0], "to": [5, 0]}),
    );
    run(&mut s, "feature.suppress", json!({"feature": "Off"}));
    // Good and Bad both use w; without it they fail, and say why.
    run(&mut s, "variable.delete", json!({"name": "w"}));
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Good", "size": [1, 2, 3]}),
    );

    let scene = scene(&mut s, None);
    let rows: Vec<(&str, &str, &str, FeatureState, Option<usize>)> = scene
        .features
        .iter()
        .map(|f| (f.id.as_str(), f.name.as_str(), f.kind, f.state, f.dof))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("F1", "Good", "block", FeatureState::Ok, None),
            ("F2", "Off", "sphere", FeatureState::Suppressed, None),
            ("F3", "Bad", "cylinder", FeatureState::Failed, None),
            // A free line: two points, two coordinates each.
            ("F4", "Free", "sketch", FeatureState::Ok, Some(4)),
        ]
    );
    let error = scene.features[2].error.as_deref().unwrap();
    assert!(error.contains('w'), "{error}");
    assert!(scene.features[0].error.is_none());
    assert!(scene.features[3].is_sketch && !scene.features[0].is_sketch);
    // The same words and the same error as document.inspect.
    let inspected = run(&mut s, "document.inspect", json!({}));
    for (row, entry) in scene
        .features
        .iter()
        .zip(inspected["features"].as_array().unwrap())
    {
        assert_eq!(entry["status"], row.state.name());
        assert_eq!(entry["id"], row.id.as_str());
        assert_eq!(entry["error"].as_str(), row.error.as_deref());
    }
    // Only what regenerated is drawn.
    assert_eq!(scene.bodies.len(), 1);
    assert_eq!(scene.bodies[0].name, "Good");
    assert_eq!(scene.sketches.len(), 1);

    // After the rollback marker nothing is drawn either.
    run(&mut s, "history.rollback", json!({"feature": "Good"}));
    let rolled = self::scene(&mut s, None);
    assert_eq!(rolled.features[3].state, FeatureState::RolledBack);
    assert!(rolled.sketches.is_empty());
}

#[test]
fn the_document_status_follows_saving() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("scene-status");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    assert!(s.document_status().is_none());
    run(&mut s, "file.new", json!({"units": "in"}));
    let fresh = s.document_status().unwrap();
    assert_eq!((fresh.index, fresh.open), (0, 1));
    assert_eq!((fresh.length_unit, fresh.angle_unit), ("in", "deg"));
    assert_eq!(fresh.path, None);
    run(&mut s, "feature.sphere", json!({"radius": 1}));
    assert!(s.document_status().unwrap().dirty);
    // Saving does not change the document: same revision, same scene.
    let revision = s.scene_revision();
    run(&mut s, "file.save", json!({"path": "ball.3dcraft"}));
    let saved = s.document_status().unwrap();
    assert_eq!(saved.path.as_deref(), Some("ball.3dcraft"));
    assert!(!saved.dirty);
    assert_eq!(s.scene_revision(), revision);
    run(&mut s, "feature.sphere", json!({"radius": 2}));
    assert!(s.document_status().unwrap().dirty);

    // A shell's own output goes through the write root like a command's.
    s.write_output("shot.bin", b"abc").unwrap();
    assert_eq!(std::fs::read(dir.join("shot.bin")).unwrap(), b"abc");
    assert!(s.write_output("../escape.bin", b"abc").is_err());
    assert!(!dir.parent().unwrap().join("escape.bin").exists());
    let none = Session::new(Workspace::none());
    assert!(none.write_output("shot.bin", b"abc").is_err());
    assert_eq!(*none.workspace(), Workspace::none());
    assert_eq!(craft3d_engine::encode_base64(b"foobar"), "Zm9vYmFy");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bodies_carry_their_edges_as_polylines_on_the_mesh() {
    let mut s = session();
    part(&mut s);
    let coarse = scene(&mut s, Some(0.01));
    assert!(coarse.problems.is_empty(), "{:?}", coarse.problems);
    let [plate, post] = coarse.bodies.as_slice() else {
        panic!("two bodies expected");
    };

    // The plate: twelve straight edges, the outline of a 60 × 40 × 8 box.
    assert_eq!(plate.edges.len(), 12);
    assert!(plate.edges.iter().all(|edge| edge.len() == 2));
    let length = |edge: &[Vec3]| -> f64 { edge.windows(2).map(|w| w[0].distance(w[1])).sum() };
    let total: f64 = plate.edges.iter().map(|e| length(e)).sum();
    assert!((total - 4.0 * (60.0 + 40.0 + 8.0)).abs() < 1e-9, "{total}");
    for point in plate.edges.iter().flatten() {
        // Every end is a corner of the box.
        assert!([0.0, 60.0].contains(&point.x), "{point:?}");
        assert!([0.0, 40.0].contains(&point.y), "{point:?}");
        assert!([0.0, 8.0].contains(&point.z), "{point:?}");
    }

    // The post: two circles of radius 6 about (15, 20), at z = 8 and 38.
    assert_eq!(post.edges.len(), 2);
    let mut heights = Vec::new();
    for circle in &post.edges {
        let z = circle[0].z;
        heights.push(z);
        assert!(near(circle[0], *circle.last().unwrap()), "closed");
        for point in circle {
            assert_eq!(point.z, z);
            let r = (point.x - 15.0).hypot(point.y - 20.0);
            assert!((r - 6.0).abs() < 1e-9, "{r}");
        }
        // Within the tolerance of the circle: the polygon is shorter than
        // it by less than a polygon with a sag of 0.01 can be.
        let around = length(circle);
        assert!(
            around < TAU * 6.0 && around > TAU * 6.0 * (1.0 - 2.0 * 0.01 / 6.0),
            "{around}"
        );
    }
    heights.sort_by(f64::total_cmp);
    assert_eq!(heights, vec![8.0, 38.0]);

    // The edge points are vertices of the body's own mesh, bit for bit:
    // a line through them lies on the border of the shaded faces.
    let bits = |p: &Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    for body in &coarse.bodies {
        let vertices: std::collections::BTreeSet<_> =
            body.mesh.positions.iter().map(bits).collect();
        for point in body.edges.iter().flatten() {
            assert!(vertices.contains(&bits(point)), "{}: {point:?}", body.name);
        }
    }

    // A finer tolerance gives finer circles and the same straight edges;
    // asking twice gives the same scene.
    let fine = scene(&mut s, Some(0.0005));
    assert!(fine.bodies[1].edges[0].len() > post.edges[0].len());
    assert_eq!(fine.bodies[0].edges, plate.edges);
    assert_eq!(scene(&mut s, Some(0.0005)).bodies, fine.bodies);
}

fn display(session: &mut Session, relative: f64, max_triangles: usize) -> Arc<Scene> {
    session
        .scene_with(SceneQuality::Display {
            relative,
            max_triangles,
        })
        .unwrap()
        .expect("a document")
}

/// The display rule: a fraction of the model's size, the same size the
/// automatic tolerance is a fraction of.
#[test]
fn the_display_quality_is_a_finer_fraction_of_the_same_size() {
    let mut s = session();
    part(&mut s);
    let automatic = scene(&mut s, None);
    let size = automatic.tolerance / AUTOMATIC_RELATIVE_TOLERANCE;
    assert!(!automatic.coarsened);
    assert!(!scene(&mut s, Some(0.05)).coarsened);
    let shown = s.scene_with(SceneQuality::display()).unwrap().unwrap();
    assert!(
        (shown.tolerance - size * DISPLAY_RELATIVE_TOLERANCE).abs() <= 1e-12 * size,
        "{} for a size of {size}",
        shown.tolerance
    );
    assert!(!shown.coarsened);
    assert!(shown.problems.is_empty(), "{:?}", shown.problems);
    assert!(shown.triangles() > automatic.triangles());
    // The post's circles are rounder: more points, each on the circle,
    // and the polygon within the finer tolerance of the circle's length.
    let circle = |scene: &Scene| scene.bodies[1].edges[0].clone();
    let (coarse, fine) = (circle(&automatic), circle(&shown));
    assert!(
        fine.len() > 2 * coarse.len(),
        "{} vs {}",
        fine.len(),
        coarse.len()
    );
    let around: f64 = fine.windows(2).map(|w| w[0].distance(w[1])).sum();
    assert!(around < TAU * 6.0 && around > TAU * 6.0 * (1.0 - 2.0 * shown.tolerance / 6.0));
    for point in &fine {
        assert!(((point.x - 15.0).hypot(point.y - 20.0) - 6.0).abs() < 1e-9);
    }
    // The sketch circles follow the same tolerance.
    let samples = |scene: &Scene| {
        curve(sketch(scene, "OnXy"), SketchCurve::Circle, false)
            .points
            .len()
    };
    assert!(samples(&shown) > samples(&automatic));

    // Cached per quality, and dropped by an edit like the others.
    assert!(Arc::ptr_eq(
        &shown,
        &s.scene_with(SceneQuality::display()).unwrap().unwrap()
    ));
    assert!(Arc::ptr_eq(&automatic, &scene(&mut s, None)));
    let other = display(&mut s, 2.0 * DISPLAY_RELATIVE_TOLERANCE, usize::MAX);
    assert!(!Arc::ptr_eq(&other, &shown));
    assert!(other.tolerance > shown.tolerance);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Post", "height": "31 mm"}),
    );
    let after = s.scene_with(SceneQuality::display()).unwrap().unwrap();
    assert!(!Arc::ptr_eq(&after, &shown));
    assert_eq!(after.revision, shown.revision + 1);

    // A request coarser than the automatic tolerance is what it asks for:
    // no budget applies there, and nothing was coarsened.
    let coarse = display(&mut s, 5.0 * AUTOMATIC_RELATIVE_TOLERANCE, 10);
    let automatic = scene(&mut s, None);
    let size = automatic.tolerance / AUTOMATIC_RELATIVE_TOLERANCE;
    assert!((coarse.tolerance - 5.0 * AUTOMATIC_RELATIVE_TOLERANCE * size).abs() <= 1e-12 * size);
    assert!(!coarse.coarsened);
    assert!(coarse.triangles() > 10);
    assert!(coarse.triangles() < automatic.triangles());
}

/// The triangle budget coarsens the display tolerance until the scene
/// fits, and the automatic tolerance is the floor.
#[test]
fn the_triangle_budget_coarsens_the_display_down_to_the_automatic_tolerance() {
    let mut s = session();
    part(&mut s);
    let automatic = scene(&mut s, None);
    let fine = display(&mut s, DISPLAY_RELATIVE_TOLERANCE, usize::MAX);
    assert!(!fine.coarsened);
    assert!(fine.triangles() > automatic.triangles() + 100);

    // A budget between the two: a scene that fits it, coarser than asked,
    // finer than automatic, and complete.
    let budget = (automatic.triangles() + fine.triangles()) / 2;
    let fitted = display(&mut s, DISPLAY_RELATIVE_TOLERANCE, budget);
    assert!(fitted.coarsened);
    assert!(
        fitted.triangles() <= budget,
        "{} > {budget}",
        fitted.triangles()
    );
    assert!(fitted.tolerance > fine.tolerance && fitted.tolerance <= automatic.tolerance);
    assert_eq!(fitted.bodies.len(), 2);
    assert_eq!(fitted.sketches.len(), 3);
    assert!(fitted.problems.is_empty(), "{:?}", fitted.problems);
    // The budget is the exact count: one more triangle and the fit is
    // the same scene; the fitted count itself is not refused.
    let again = display(&mut s, DISPLAY_RELATIVE_TOLERANCE, fitted.triangles());
    assert!(again.triangles() <= fitted.triangles());

    // A budget no scene can meet: the automatic scene, bit for bit, over
    // budget and said to be coarsened.
    let floor = display(&mut s, DISPLAY_RELATIVE_TOLERANCE, 1);
    assert!(floor.coarsened);
    assert_eq!(floor.tolerance, automatic.tolerance);
    assert_eq!(floor.mesh, automatic.mesh);
    assert_eq!(floor.bodies, automatic.bodies);
    assert!(floor.triangles() > 1);
    assert!(floor.problems.is_empty());
}

/// A tolerance the tessellator cannot meet on a body (more segments on a
/// patch than it allows) is coarsened until it can, where asking for that
/// tolerance outright leaves the body out.
#[test]
fn a_display_tolerance_the_tessellator_cannot_meet_is_coarsened_until_it_can() {
    let mut s = session();
    part(&mut s);
    let automatic = scene(&mut s, None);
    let size = automatic.tolerance / AUTOMATIC_RELATIVE_TOLERANCE;
    // The post's circles of radius 6 would need some twenty thousand
    // segments, more than the tessellator allows on an edge.
    let relative = 1e-9;
    let asked = scene(&mut s, Some(size * relative));
    assert!(
        asked.problems.iter().any(|p| p.contains("Post")),
        "{:?}",
        asked.problems
    );
    assert_eq!(asked.bodies.len(), 1);
    let shown = display(&mut s, relative, usize::MAX);
    assert!(shown.problems.is_empty(), "{:?}", shown.problems);
    assert!(shown.coarsened);
    assert_eq!(shown.bodies.len(), 2);
    assert!(shown.tolerance > size * relative && shown.tolerance <= automatic.tolerance);
    // Still much finer than automatic: the coarsening stops where the
    // tessellator can go on, not at the floor.
    assert!(
        shown.tolerance < automatic.tolerance / 4.0,
        "{}",
        shown.tolerance
    );
}

#[test]
fn a_display_quality_that_is_not_a_request_is_refused() {
    let mut s = session();
    part(&mut s);
    for relative in [0.0, -1e-5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = s
            .scene_with(SceneQuality::Display {
                relative,
                max_triangles: 1000,
            })
            .unwrap_err();
        assert!(error.message().contains("chordal"), "{relative}: {error}");
    }
    let error = s
        .scene_with(SceneQuality::Display {
            relative: 1e-4,
            max_triangles: 0,
        })
        .unwrap_err();
    assert!(error.message().contains("budget"), "{error}");
    // Refusals are not cached as scenes: the good request still answers.
    assert!(s.scene_with(SceneQuality::display()).unwrap().is_some());
    // Without a document there is no scene, whatever the quality.
    let mut empty = session();
    assert!(empty.scene_with(SceneQuality::display()).unwrap().is_none());
    assert!(empty.scene_with(SceneQuality::Automatic).unwrap().is_none());
}
