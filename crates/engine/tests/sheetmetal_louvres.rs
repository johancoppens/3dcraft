//! A flange on a cut-out's edge by command (docs/design/sheetmetal.md ›
//! *Flanges on a cut-out's edge*): a base 60 × 40 with a wall 30 high on
//! its front edge, a slot 20 × 8 in the wall drawn in a `cutouts` sketch,
//! and a louvre bent out of the slot's edge nearer the bend (`cutout:
//! {sketch, edge}`), 16 wide, 4 long, at 35 degrees, with rect bend
//! reliefs. Held at 1e-3, 1 and 1e6: the flat area in closed form (base,
//! wall less the slot, the strips, the louvre, less the notches), the
//! volume `A T` at K = ½, the bend table's `on`, the names of the part
//! without the louvre kept but those along the slot's edge, the part saved
//! and opened again; the flat measures `flatExtent` and `flatArea`; bad
//! cut-out edges are errors or failed parts with the reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::FRAC_PI_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(v: f64) -> String {
    format!("{v} mm")
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn lines(made: &Value) -> Vec<String> {
    made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect()
}

fn names(s: &mut Session) -> Vec<String> {
    body(s, "Panel")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

/// The base's front edge, the slot's lines (bottom, right, top, left as
/// `sketch.rect` makes them) and the bend allowance of the wall.
fn sketches(s: &mut Session, k: f64) -> (String, Vec<String>, f64) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let base = lines(&run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [60.0 * k, 40.0 * k]}),
    ));
    let ba = FRAC_PI_2 * 1.5 * k;
    run(s, "sketch.create", json!({"plane": "xy", "name": "Vents"}));
    // The slot 12 to 20 below the wall's tangent line on its flat.
    let top = -ba - 12.0 * k;
    let slot = lines(&run(
        s,
        "sketch.rect",
        json!({"sketch": "Vents", "from": [20.0 * k, top - 8.0 * k], "to": [40.0 * k, top]}),
    ));
    (base[0].clone(), slot, ba)
}

fn panel(s: &mut Session, k: f64, flanges: Vec<Value>) -> Value {
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Panel", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5, "flanges": flanges, "cutouts": ["Vents"],
               "bendRelief": {"shape": "rect", "size": mm(0.5 * k)}}),
    )
}

#[test]
fn a_louvre_bent_out_of_a_slot_in_a_wall() {
    for k in SCALES {
        let dir = workdir("sheetmetal-louvres", &format!("panel-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (edge, slot, ba) = sketches(&mut s, k);
        let wall = json!({"edge": edge, "length": mm(30.0 * k), "angle": "90 deg"});
        panel(&mut s, k, vec![wall.clone()]);
        let before = names(&mut s);
        // The slot's top line, nearer the bend: in the hole's loop it runs
        // with the wall on its left, so the louvre grows into the slot.
        let louvre = json!({"cutout": {"sketch": "Vents", "edge": slot[2]}, "length": mm(4.0 * k),
                            "angle": "35 deg", "direction": "down",
                            "insetStart": mm(2.0 * k), "insetEnd": mm(2.0 * k)});
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": [wall.clone(), louvre.clone()]}),
        );
        let found = body(&mut s, "Panel");
        assert_eq!(found["valid"], true, "scale {k}: {found}");
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Panel"}));
        let theta = 35f64.to_radians();
        let bl = theta * 1.5 * k;
        let notch = 0.25 * k * k;
        let a = number(&table["flatArea"]);
        let want = 2400.0 * k * k + 60.0 * k * (ba + 30.0 * k) - 160.0 * k * k
            + 16.0 * k * (bl + 4.0 * k)
            - 2.0 * notch;
        assert!(
            (a - want).abs() <= 1e-9 * want,
            "scale {k}: flat area {a} against {want}"
        );
        let v = number(&found["volume"]);
        assert!(
            (v - a * k).abs() <= 1e-9 * v,
            "scale {k}: volume {v} against {}",
            a * k
        );
        let bends = table["bends"].as_array().unwrap();
        let vents = run(&mut s, "document.inspect", json!({}))["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "Vents")
            .unwrap()["id"]
            .clone();
        assert_eq!(
            bends[1]["on"],
            format!(
                "{} of the cut-out of sketch {}",
                slot[2],
                vents.as_str().unwrap()
            ),
            "{table}"
        );
        assert_eq!(bends[1]["ends"].as_array().unwrap().len(), 2, "{table}");
        // The slot carries the louvre: it is no hole beside a bend.
        assert_eq!(table["holes"].as_array().unwrap().len(), 0, "{table}");
        // Names: those of the wall without the louvre are kept but the
        // rim along the slot's top line.
        let after = names(&mut s);
        let lost: Vec<&String> = before.iter().filter(|n| !after.contains(n)).collect();
        assert!(
            lost.len() <= 1 && lost.iter().all(|n| n.contains("rim(")),
            "scale {k}: lost {lost:?}"
        );
        // The flat measures: the pattern's box and area.
        for (axis, size) in [("x", 60.0 * k), ("y", 40.0 * k + ba + 30.0 * k)] {
            let check = run(
                &mut s,
                "assert.add",
                json!({"name": format!("flat {axis}"), "measure": "flatExtent", "of": "Panel",
                       "axis": axis, "eq": mm(size)}),
            );
            assert_eq!(check["status"], "passed", "scale {k}: {check}");
        }
        let check = run(
            &mut s,
            "assert.add",
            json!({"name": "flat area", "measure": "flatArea", "of": "Panel",
                   "eq": format!("{a} * (1 mm)^2")}),
        );
        assert_eq!(check["status"], "passed", "scale {k}: {check}");
        // Saved and opened again: the same part.
        run(
            &mut s,
            "file.save",
            json!({"path": "panel.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "panel.3dcraft"}));
        let again = body(&mut s, "Panel");
        assert_eq!(again["volume"], found["volume"], "scale {k}");
        assert_eq!(again["faceNames"], found["faceNames"], "scale {k}");
        // Too long for its slot: the part fails with the reason.
        let long = json!({"cutout": {"sketch": "Vents", "edge": slot[2]}, "length": mm(8.0 * k),
                          "angle": "35 deg", "direction": "down",
                          "insetStart": mm(2.0 * k), "insetEnd": mm(2.0 * k)});
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": [wall.clone(), long]}),
        );
        assert_eq!(out["status"], "failed", "{out}");
        assert!(
            out["error"].as_str().unwrap().contains("overlaps itself"),
            "{out}"
        );
        run(&mut s, "edit.undo", json!({}));
        // Listed before the wall that holds the slot.
        let out = run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": [louvre.clone(), wall.clone()]}),
        );
        assert_eq!(out["status"], "failed", "{out}");
        assert!(
            out["error"]
                .as_str()
                .unwrap()
                .contains("lies in no flat laid out before it"),
            "{out}"
        );
        run(&mut s, "edit.undo", json!({}));
        // Edges that name nothing a flange can sit on.
        for (on, why) in [
            (json!({"sketch": "Vents", "edge": "e99"}), "has no e99"),
            (json!({"sketch": "Nowhere", "edge": slot[2]}), "cutout is"),
            (json!({"sketch": "Vents"}), "cutout is"),
            (json!("Vents"), "cutout is"),
            (
                json!({"sketch": "Vents", "edge": slot[2], "more": 1}),
                "cutout is",
            ),
        ] {
            let bad = json!({"cutout": on, "length": mm(4.0 * k), "angle": "35 deg"});
            let got = error(
                &mut s,
                "sheetMetal.edit",
                json!({"feature": "Panel", "flanges": [wall.clone(), bad]}),
            );
            assert!(got.contains(why), "{on}: {got}");
        }
        // A sketch that is not one of the part's cutouts.
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Other"}),
        );
        let other = lines(&run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Other", "from": [20.0 * k, -20.0 * k], "to": [40.0 * k, -15.0 * k]}),
        ));
        let bad = json!({"cutout": {"sketch": "Other", "edge": other[2]}, "length": mm(2.0 * k),
                         "angle": "35 deg"});
        let got = error(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": [wall.clone(), bad]}),
        );
        assert!(got.contains("list it in `cutouts`"), "{got}");
        // With an edge too: one place to sit on.
        let bad = json!({"cutout": {"sketch": "Vents", "edge": slot[2]}, "edge": edge,
                         "length": mm(2.0 * k), "angle": "35 deg"});
        let got = error(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": [wall.clone(), bad]}),
        );
        assert!(got.contains("sits on one edge"), "{got}");
        // A flat measure with a bad axis.
        let got = run(
            &mut s,
            "assert.add",
            json!({"name": "bad axis", "measure": "flatExtent", "of": "Panel", "axis": "z",
                   "eq": "1 mm"}),
        );
        assert_eq!(got["status"], "error", "{got}");
    }
}
