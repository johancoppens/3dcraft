//! The drawing of the model as seen (`file.export` to DXF or SVG of a
//! named view) is a function of the document alone: the same bytes
//! whatever the session did before it (golden rule 5), with the automatic
//! tolerance (from the bodies' areas, which several paths measure into one
//! store) and with a given one. Checked 2026-11-02 after the web build's
//! export tests saw a drawing of `view: "current"` change with what the
//! page did before it: that was the window's camera (`Shell::sync` gives
//! it to the session, and it follows the model once the scene is there),
//! not the drawing; a named view gave the same bytes in every case.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_engine::{SceneQuality, Session, Workspace};
use serde_json::{Value, json};

/// A plate with a colour and a sheet-metal part, at scale `s`.
fn model(s: f64) -> Session {
    let mm = |v: f64| format!("{} mm", v * s);
    let mut session = Session::new(Workspace::memory());
    for (id, params) in [
        ("file.new", json!({"units": "mm"})),
        (
            "feature.block",
            json!({"size": [mm(40.0), mm(20.0), mm(10.0)], "name": "Plate"}),
        ),
        ("sketch.create", json!({"plane": "xy", "name": "S"})),
        (
            "sketch.rect",
            json!({"sketch": "S", "from": [mm(60.0), "0 mm"], "to": [mm(110.0), mm(40.0)]}),
        ),
        (
            "feature.sheetMetal",
            json!({"sketch": "S", "name": "Part", "thickness": mm(1.0), "radius": mm(1.0),
                "kFactor": 0.4, "flanges": [{"edge": "e6", "length": mm(15.0), "angle": "90 deg"}]}),
        ),
        (
            "feature.sphere",
            json!({"radius": mm(6.0), "position": ["0 mm", mm(50.0), "0 mm"], "name": "Ball"}),
        ),
        (
            "body.appearance",
            json!({"body": "Plate", "color": "#2060c0"}),
        ),
        (
            "set.define",
            json!({"name": "Tops", "kind": "face", "where": {"normal": {"along": "z"}, "in": "Plate"}}),
        ),
    ] {
        session
            .execute(id, &params)
            .unwrap_or_else(|e| panic!("{id}: {}", e.message()));
    }
    // Opened from its file, as the page opens one.
    session
        .execute("file.save", &json!({"path": "model.3dcraft"}))
        .unwrap();
    let bytes = session.take_file("model.3dcraft").unwrap();
    let mut opened = Session::new(Workspace::memory());
    opened.open_bytes("model.3dcraft", &bytes).unwrap();
    opened
}

fn drawing(session: &mut Session, name: &str, extra: &Value) -> Vec<u8> {
    let mut params = json!({"path": name, "view": "iso"});
    if let (Some(map), Some(more)) = (params.as_object_mut(), extra.as_object()) {
        for (k, v) in more {
            map.insert(k.clone(), v.clone());
        }
    }
    session
        .execute("file.export", &params)
        .unwrap_or_else(|e| panic!("{name}: {}", e.message()));
    session.take_file(name).unwrap()
}

/// What a session may have done before the drawing.
fn before(session: &mut Session, what: &str) {
    match what {
        "nothing" => {}
        "first picture" => {
            session.scene_with(SceneQuality::Preview).unwrap();
        }
        "display scene" => {
            session.scene_with(SceneQuality::display()).unwrap();
        }
        "meshes exported" => {
            for name in ["m.step", "m.stl", "m.3mf", "m.obj"] {
                let mut params = json!({"path": name});
                if !name.ends_with("step") {
                    params["quality"] = json!("normal");
                }
                session.execute("file.export", &params).unwrap();
            }
            for name in session.file_names() {
                session.take_file(&name);
            }
        }
        "measured" => {
            session.execute("document.measure", &json!({})).unwrap();
        }
        "inertia" => {
            session
                .execute(
                    "body.material",
                    &json!({"body": "Plate", "material": "steel"}),
                )
                .ok();
            session.execute("document.inspect", &json!({})).ok();
        }
        "step exported" => {
            session
                .execute("file.export", &json!({"path": "m.step"}))
                .unwrap();
            session.take_file("m.step");
        }
        "one body measured" => {
            session
                .execute(
                    "select.query",
                    &json!({"kind": "body", "where": {"name": "Part", "volume": {"min": 0}}}),
                )
                .unwrap();
        }
        "pieces" => {
            session.set_stepwise(true);
            let quality = SceneQuality::display();
            while session.prepare_some(quality, || false).is_some() {}
        }
        "export choices" => {
            for (id, params) in [
                ("select.get", json!({"context": false})),
                (
                    "select.query",
                    json!({"kind": "feature", "where": {"type": "sheetMetal", "status": "ok"}, "limit": 256}),
                ),
                ("set.list", json!({})),
                ("select.query", json!({"kind": "body"})),
            ] {
                session.execute(id, &params).unwrap();
            }
        }
        other => panic!("{other}"),
    }
}

#[test]
fn the_drawing_of_the_model_is_the_same_whatever_came_before() {
    for s in [1e-2, 1.0, 1e3] {
        for extension in ["dxf", "svg"] {
            let name = format!("view.{extension}");
            for extra in [
                json!({"projection": "orthographic", "hidden": "omit"}),
                json!({"projection": "orthographic", "hidden": "omit", "tolerance": 0.01 * s}),
            ] {
                let mut fresh = model(s);
                let want = drawing(&mut fresh, &name, &extra);
                for what in [
                    "step exported",
                    "one body measured",
                    "pieces",
                    "export choices",
                    "first picture",
                    "display scene",
                    "meshes exported",
                    "measured",
                    "inertia",
                ] {
                    let mut session = model(s);
                    before(&mut session, what);
                    let got = drawing(&mut session, &name, &extra);
                    assert!(
                        got == want,
                        "{name} {extra} at {s} after {what}: the drawing differs"
                    );
                }
            }
        }
    }
}
