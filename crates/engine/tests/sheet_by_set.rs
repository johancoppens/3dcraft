//! A cut sheet by named set (`file.export {set}`, Jigs gap G7): every
//! flat face a set holds now laid out on one sheet, a layer per body.
//! The oracle is the same sheet given face by face (`faces`), which
//! `flat_sheet.rs` checks against the panels' closed forms; and a set
//! that follows the design (a count from a variable) draws as many
//! panels as there are.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{refused, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// Three notched panels lying flat, `count` of them in a pattern for the
/// last; the set `Cut sheet` holds every top face (`+z`) of the panels.
fn panels(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(s, "variable.set", json!({"name": "count", "expr": "2"}));
    for (name, w, h, y) in [("Bottom", 60, 40, 0), ("Side", 40, 25, 80)] {
        run(
            s,
            "feature.block",
            json!({"name": format!("{name}Blank"), "size": [w, h, 3], "position": [0, y, 0]}),
        );
        run(
            s,
            "feature.block",
            json!({"name": format!("{name}Notch"), "size": [10, 6, 9],
                   "position": [w / 2 - 5, y - 3, -3]}),
        );
        run(
            s,
            "feature.boolean",
            json!({"op": "subtract", "target": format!("{name}Blank"), "tool": format!("{name}Notch"), "name": name}),
        );
    }
    run(
        s,
        "feature.pattern",
        json!({"source": "Side", "count": "count", "step": [0, 40, 0], "name": "Sides"}),
    );
    run(
        s,
        "set.define",
        json!({"name": "Cut sheet", "kind": "face", "where": {"normal": {"along": "z"}}}),
    );
}

fn layers(result: &Value) -> Vec<String> {
    result["sheet"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["layer"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_set_puts_every_flat_face_it_holds_on_one_sheet() {
    let dir = workdir("sheet-by-set", "all");
    let mut s = session_in(&dir);
    panels(&mut s);
    for format in ["svg", "dxf"] {
        let by_set = run(
            &mut s,
            "file.export",
            json!({"path": format!("set.{format}"), "set": "Cut sheet", "gap": 4}),
        );
        println!("{by_set}");
        assert_eq!(by_set["set"], "Cut sheet");
        assert_eq!(by_set["notFlat"], json!([]));
        assert_eq!(by_set["others"], json!({}));
        // The bottom and the pattern of sides: the pattern is one body
        // with two lumps, so two top faces of one body, on its layer.
        let sheet = by_set["sheet"].as_array().unwrap();
        assert_eq!(sheet.len(), 3, "{by_set}");
        assert_eq!(by_set["closed"], 3);
        assert_eq!(layers(&by_set), ["Bottom", "Sides", "Sides"]);
        // The same faces given one by one draw the same sheet.
        let faces: Vec<Value> = sheet
            .iter()
            .map(|e| json!({"face": e["face"], "feature": e["feature"], "layer": e["layer"]}))
            .collect();
        let by_faces = run(
            &mut s,
            "file.export",
            json!({"path": format!("faces.{format}"), "faces": faces, "gap": 4}),
        );
        assert_eq!(by_set["bbox"], by_faces["bbox"]);
        assert_eq!(by_set["length"], by_faces["length"]);
        assert_eq!(
            std::fs::read(dir.join(format!("set.{format}"))).unwrap(),
            std::fs::read(dir.join(format!("faces.{format}"))).unwrap()
        );
        if format == "svg" {
            let text = std::fs::read_to_string(dir.join("set.svg")).unwrap();
            // One path per layer: the bottom, and both sides together.
            assert_eq!(text.matches("<path").count(), 2, "{text}");
            assert!(text.contains("id=\"layer-Sides\""), "{text}");
        }
    }
    // The set follows the design: four sides, five panels.
    run(
        &mut s,
        "variable.set",
        json!({"name": "count", "expr": "4"}),
    );
    let more = run(
        &mut s,
        "file.export",
        json!({"path": "more.dxf", "set": "Cut sheet", "width": 150}),
    );
    assert_eq!(more["sheet"].as_array().unwrap().len(), 5, "{more}");
    assert_eq!(more["closed"], 5);
    assert!(more["rows"].as_u64().unwrap() >= 2, "{more}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn faces_that_are_not_flat_and_other_items_are_said_and_left_off() {
    let dir = workdir("sheet-by-set", "mixed");
    let mut s = session_in(&dir);
    panels(&mut s);
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Pin", "radius": 2, "height": 9, "position": [200, 0, 0]}),
    );
    run(
        &mut s,
        "set.define",
        json!({"name": "Mixed", "where": {"any": [{"in": "Pin"}, {"in": "Bottom", "normal": {"along": "z"}}]}}),
    );
    let result = run(
        &mut s,
        "file.export",
        json!({"path": "mixed.svg", "set": "Mixed"}),
    );
    println!("{result}");
    // The pin's two caps and the bottom's top are flat; its side is not;
    // the pin body, its edges and its vertices are other kinds of item,
    // counted kind by kind.
    assert_eq!(result["sheet"].as_array().unwrap().len(), 3, "{result}");
    assert_eq!(result["notFlat"].as_array().unwrap().len(), 1, "{result}");
    assert_eq!(result["others"]["body"], 1, "{result}");
    assert!(result["others"]["edge"].as_u64().unwrap() >= 2, "{result}");
    run(
        &mut s,
        "set.define",
        json!({"name": "Round", "kind": "face", "where": {"surface": "cylinder"}}),
    );
    let error = refused(
        &mut s,
        "file.export",
        json!({"path": "round.svg", "set": "Round"}),
    );
    assert!(error.contains("no flat face"), "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_set_that_cannot_be_drawn_is_refused_with_the_reason() {
    let dir = workdir("sheet-by-set", "refused");
    let mut s = session_in(&dir);
    panels(&mut s);
    for (params, word) in [
        (
            json!({"path": "x.svg", "set": "Nope"}),
            "the sets are `Cut sheet`",
        ),
        (json!({"path": "x.svg", "set": 3}), "`set` must be the name"),
        (
            json!({"path": "x.svg", "set": "Cut sheet", "faces": ["F1:face(1)"]}),
            "not `faces` and `set` at once",
        ),
        (
            json!({"path": "x.svg", "set": "Cut sheet", "feature": "Bottom"}),
            "`feature`",
        ),
        (
            json!({"path": "x.stl", "set": "Cut sheet"}),
            "`set` is for a drawing",
        ),
        (
            json!({"path": "x.svg", "faces": "Cut sheet"}),
            "given as `set`",
        ),
    ] {
        let error = refused(&mut s, "file.export", params.clone());
        assert!(error.contains(word), "{params}: {error}");
    }
    run(&mut s, "file.new", json!({}));
    run(&mut s, "feature.block", json!({"size": [1, 1, 1]}));
    let error = refused(&mut s, "file.export", json!({"path": "x.svg", "set": "A"}));
    assert!(error.contains("no named sets"), "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}
