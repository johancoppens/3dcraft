//! `cut.list` and `cut.export` (docs/design/cutting.md): plates recognised
//! from their faces (a box, a plate with a round hole and a slot, an L, an
//! L turned over, a plate standing on its side), a sheet-metal part by its
//! flat pattern, and bodies refused with the face that keeps them from
//! being a plate (a pocket, a chamfer, a sphere); thickness, length ×
//! width, area and cut length in closed form at three scales; identical
//! parts counted, grouped by thickness and material; the outlines written
//! to DXF and SVG and read back, with a kerf in closed form (a rectangle
//! grows by 2k a side, a hole's radius shrinks by k); bad parameters
//! refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

#[track_caller]
fn close(v: &Value, expected: f64, k: f64, what: &str) {
    let got = number(v);
    assert!(
        (got - expected).abs() <= 1e-9 * k.max(k * k).max(1e-9),
        "{what}: {got} != {expected}"
    );
}

/// `v` rounded to the decimal place of the document's tolerance `tol`
/// (the largest power of ten not above it): cut.list prints its lengths
/// so, to what the tolerance tells apart and no coarser (J-042).
fn at_place(v: f64, tol: f64) -> f64 {
    let place = tol.plog10().floor() as i32;
    let scale = 10.0_f64.ppowi(place.abs());
    if place < 0 {
        (v * scale).round() / scale
    } else {
        (v / scale).round() * scale
    }
}

/// The part named `name` of a cut list, with its group.
fn part<'a>(list: &'a Value, name: &str) -> (&'a Value, &'a Value) {
    for g in list["groups"].as_array().unwrap() {
        for p in g["parts"].as_array().unwrap() {
            if p["name"] == name {
                return (g, p);
            }
        }
    }
    panic!("no part {name} in {list}");
}

fn refused<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["notPlates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("{name} is not refused in {list}"))
}

/// A plate `w` × `h` with a round hole of radius 5 at (20, 25) and a slot
/// 30 long, of radius 4, from (50, 25), all times `k`, `t` thick.
fn holed_plate(s: &mut Session, name: &str, k: f64, t: f64) {
    let p = |x: f64, y: f64| json!([x * k, y * k]);
    let sketch = format!("{name} sketch");
    run(s, "sketch.create", json!({"plane": "xy", "name": sketch}));
    run(
        s,
        "sketch.rect",
        json!({"sketch": sketch, "from": p(0.0, 0.0), "to": p(100.0, 50.0)}),
    );
    run(
        s,
        "sketch.circle",
        json!({"sketch": sketch, "center": p(20.0, 25.0), "radius": 5.0 * k}),
    );
    run(
        s,
        "sketch.path",
        json!({"sketch": sketch, "start": p(50.0, 21.0), "segments": [
            {"line": p(80.0, 21.0)}, {"arc": p(80.0, 29.0), "tangent": true},
            {"line": p(50.0, 29.0), "tangent": true}, {"arc": "start", "tangent": true}]}),
    );
    run(
        s,
        "feature.extrude",
        json!({"sketch": sketch, "distance": t * k, "name": name}),
    );
}

/// An L 100 × 60 with arms 20 wide, `t` thick at height `z`, mirrored in
/// x when `mirrored`.
fn l_plate(s: &mut Session, name: &str, k: f64, z: f64, mirrored: bool) {
    let m = if mirrored { -1.0 } else { 1.0 };
    let p = |x: f64, y: f64| json!([m * x * k, y * k]);
    let sketch = format!("{name} sketch");
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": z * k, "name": sketch}),
    );
    run(
        s,
        "sketch.path",
        json!({"sketch": sketch, "start": p(0.0, 0.0), "closed": true, "segments": [
            {"line": p(60.0, 0.0)}, {"line": p(60.0, 20.0)}, {"line": p(20.0, 20.0)},
            {"line": p(20.0, 100.0)}, {"line": p(0.0, 100.0)}]}),
    );
    run(
        s,
        "feature.extrude",
        json!({"sketch": sketch, "distance": 9.0 * k, "name": name}),
    );
}

fn block(s: &mut Session, name: &str, size: [f64; 3], at: [f64; 3]) {
    run(
        s,
        "feature.block",
        json!({"size": size, "position": at, "name": name}),
    );
}

#[test]
fn plates_have_their_thickness_size_and_area_in_closed_form() {
    for k in SCALES {
        let dir = workdir("cut", &format!("plates-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        block(&mut s, "Shelf", [100.0 * k, 50.0 * k, 6.0 * k], [0.0; 3]);
        holed_plate(&mut s, "Holed", k, 6.0);
        l_plate(&mut s, "Ell", k, 40.0, false);
        // A D: a line and a cubic spline, whose side wall is freeform.
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "offset": 100.0 * k, "name": "D sketch"}),
        );
        run(
            &mut s,
            "sketch.line",
            json!({"sketch": "D sketch", "from": [0.0, 0.0], "to": [100.0 * k, 0.0]}),
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "D sketch", "control": [[100.0 * k, 0.0], [100.0 * k, 60.0 * k], [0.0, 60.0 * k], [0.0, 0.0]]}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "D sketch", "distance": 4.0 * k, "name": "D"}),
        );
        let list = run(&mut s, "cut.list", json!({}));
        assert_eq!(list["parts"], 4, "{list}");
        assert_eq!(list["notPlates"], json!([]), "{list}");
        let (group, d) = part(&list, "D");
        close(&group["thickness"], 4.0 * k, k, "D thickness");
        close(&d["length"], 100.0 * k, k, "D length");
        // The cubic's highest point: 3 · 60 · t (1 − t) at t = 1/2.
        close(&d["width"], 45.0 * k, k, "D width");
        close(&d["area"], 3600.0 * k * k, k, "D area");

        let (group, shelf) = part(&list, "Shelf");
        assert_eq!(shelf["kind"], "plate");
        close(&shelf["thickness"], 6.0 * k, k, "box thickness");
        close(&group["thickness"], 6.0 * k, k, "group thickness");
        close(&shelf["length"], 100.0 * k, k, "box length");
        close(&shelf["width"], 50.0 * k, k, "box width");
        close(&shelf["area"], 5000.0 * k * k, k, "box area");
        close(&shelf["cutLength"], 300.0 * k, k, "box cut length");
        assert_eq!(shelf["holes"], 0);
        // Seen from its top, the part's corner at the model's origin.
        assert_eq!(shelf["frame"]["normal"], json!([0.0, 0.0, 1.0]), "{shelf}");
        assert_eq!(shelf["frame"]["xAxis"], json!([1.0, 0.0, 0.0]), "{shelf}");
        for (i, want) in [0.0, 0.0, 6.0 * k].iter().enumerate() {
            close(&shelf["frame"]["origin"][i], *want, k, "box origin");
        }
        assert!(shelf["top"].as_str().unwrap().starts_with("F1:"), "{shelf}");

        let (_, holed) = part(&list, "Holed");
        assert_eq!(holed["holes"], 2, "{holed}");
        close(&holed["length"], 100.0 * k, k, "holed length");
        close(&holed["width"], 50.0 * k, k, "holed width");
        let area = 5000.0 - 25.0 * PI - (30.0 * 8.0 + 16.0 * PI);
        close(&holed["area"], area * k * k, k, "holed area");
        // Printed to the place of the tolerance (J-042): the closed form
        // rounded there.
        let cut = 300.0 + 10.0 * PI + 60.0 + 8.0 * PI;
        close(
            &holed["cutLength"],
            at_place(cut * k, 1e-6 * k),
            k,
            "holed cut length",
        );
        // Exact as well where nothing is rounded for printing: the length
        // of the paths cut.export draws of it.
        let drawn = run(
            &mut s,
            "cut.export",
            json!({"path": format!("holed-{k}.svg"), "bodies": ["Holed"]}),
        );
        close(&drawn["length"], cut * k, k, "holed drawn length");

        let (group, ell) = part(&list, "Ell");
        close(&group["thickness"], 9.0 * k, k, "L thickness");
        close(&ell["length"], 100.0 * k, k, "L length");
        close(&ell["width"], 60.0 * k, k, "L width");
        close(&ell["area"], 2800.0 * k * k, k, "L area");
        close(&ell["cutLength"], 320.0 * k, k, "L cut length");
        // Its own frame: the L lies in [0, 100] × [0, 60].
        let geometry = run(
            &mut s,
            "cut.list",
            json!({"geometry": true, "bodies": ["Ell"]}),
        );
        let (_, ell) = part(&geometry, "Ell");
        let outline = &ell["geometry"]["outline"];
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for seg in outline["segments"].as_array().unwrap() {
            let q = &seg["line"];
            for i in 0..2 {
                lo[i] = lo[i].min(number(&q[i]));
                hi[i] = hi[i].max(number(&q[i]));
            }
        }
        for (got, want) in [(lo[0], 0.0), (lo[1], 0.0), (hi[0], 100.0), (hi[1], 60.0)] {
            assert!((got - want * k).abs() <= 1e-9 * k, "{outline}");
        }
    }
}

#[test]
fn identical_parts_are_counted_and_grouped_by_thickness_and_material() {
    for k in SCALES {
        let dir = workdir("cut", &format!("same-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        // Three 100 × 50 × 6 boxes: lying, standing on its side (its top is
        // the +x face), and one more of steel; a 100 × 50 × 9 box.
        block(&mut s, "Side", [100.0 * k, 50.0 * k, 6.0 * k], [0.0; 3]);
        block(
            &mut s,
            "Side 2",
            [6.0 * k, 100.0 * k, 50.0 * k],
            [300.0 * k, 0.0, 0.0],
        );
        block(
            &mut s,
            "Steel side",
            [100.0 * k, 50.0 * k, 6.0 * k],
            [0.0, 300.0 * k, 0.0],
        );
        block(
            &mut s,
            "Thick",
            [100.0 * k, 50.0 * k, 9.0 * k],
            [0.0, 600.0 * k, 0.0],
        );
        // Turned about z, and tilted out of every plane of the model.
        run(
            &mut s,
            "feature.block",
            json!({"size": [100.0 * k, 50.0 * k, 6.0 * k], "position": [0.0, 900.0 * k, 0.0],
                   "angle": "30 deg", "name": "Side 3"}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"size": [100.0 * k, 50.0 * k, 6.0 * k], "position": [0.0, 1200.0 * k, 0.0],
                   "axis": [1, 2, 3], "angle": "20 deg", "name": "Side 4"}),
        );
        // An L and its mirror image: the same part turned over.
        l_plate(&mut s, "Left", k, 100.0, false);
        l_plate(&mut s, "Right", k, 200.0, true);
        for body in ["Side", "Side 2", "Side 3", "Side 4"] {
            run(
                &mut s,
                "body.material",
                json!({"body": body, "name": "birch", "density": 700}),
            );
        }
        run(
            &mut s,
            "body.material",
            json!({"body": "Steel side", "name": "steel", "density": 7850}),
        );
        let list = run(&mut s, "cut.list", json!({}));
        assert_eq!(list["parts"], 8, "{list}");
        assert_eq!(list["distinct"], 4, "{list}");
        let (group, side) = part(&list, "Side");
        assert_eq!(side["count"], 4, "{side}");
        assert_eq!(
            side["bodies"],
            json!(["Side", "Side 2", "Side 3", "Side 4"])
        );
        assert_eq!(group["material"], "birch");
        close(&group["area"], 4.0 * 5000.0 * k * k, k, "birch area");
        let (group, steel) = part(&list, "Steel side");
        assert_eq!(steel["count"], 1);
        assert_eq!(group["material"], "steel");
        let (group, thick) = part(&list, "Thick");
        assert_eq!(thick["count"], 1);
        close(&group["thickness"], 9.0 * k, k, "thick");
        assert_eq!(group["material"], Value::Null);
        let (_, left) = part(&list, "Left");
        assert_eq!(left["count"], 2, "{left}");
        assert_eq!(left["bodies"], json!(["Left", "Right"]));
        // Groups by thickness, then material (none first).
        let groups: Vec<(f64, Value)> = list["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| (number(&g["thickness"]) / k, g["material"].clone()))
            .collect();
        // The Ls are 9 thick too, of no material: with Thick.
        assert_eq!(groups.len(), 3, "{groups:?}");
        assert!(
            groups.windows(2).all(|w| w[0].0 <= w[1].0 + 1e-9),
            "{groups:?}"
        );

        // A plate of the same size with a hole is another part.
        holed_plate(&mut s, "Holed", k, 9.0);
        let list = run(&mut s, "cut.list", json!({}));
        let (_, thick) = part(&list, "Thick");
        assert_eq!(thick["count"], 1, "{list}");
        assert_eq!(list["distinct"], 5, "{list}");

        // CSV: the header in the document's unit, a row per distinct part.
        let csv = run(&mut s, "cut.list", json!({"format": "csv"}));
        let text = csv["csv"].as_str().unwrap();
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(
                "material,thickness (mm),name,count,length (mm),width (mm),area (mm^2),cut length (mm),holes,kind,bodies,reason"
            )
        );
        let rows: Vec<&str> = lines.collect();
        assert_eq!(rows.len(), 5, "{text}");
        assert!(
            rows.iter().any(|r| r.starts_with("birch,")
                && r.contains(",Side,4,")
                && r.ends_with(",plate,Side; Side 2; Side 3; Side 4,")),
            "{text}"
        );
    }
}

#[test]
fn a_sheet_metal_part_is_listed_by_its_flat_pattern() {
    for k in SCALES {
        let dir = workdir("cut", &format!("sheet-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (w, d, l, r, t) = (40.0 * k, 30.0 * k, 25.0 * k, 3.0 * k, 2.0 * k);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        let made = run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [0.0, 0.0], "to": [w, d]}),
        );
        let bottom = made["lines"][0].as_str().unwrap().to_string();
        run(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Bracket", "thickness": format!("{t} mm"),
                   "radius": format!("{r} mm"), "kFactor": "0.4",
                   "flanges": [{"edge": bottom, "length": format!("{l} mm"), "angle": "90 deg"}]}),
        );
        let list = run(&mut s, "cut.list", json!({}));
        let (group, bracket) = part(&list, "Bracket");
        assert_eq!(bracket["kind"], "sheetMetal", "{bracket}");
        close(&group["thickness"], t, k, "sheet thickness");
        let ba = FRAC_PI_2 * (r + 0.4 * t);
        let long = d + ba + l;
        // Printed to the place of the tolerance (J-042): the closed form
        // rounded there; exact in the DXF below, whose outline is not
        // rounded for printing (its box against `long` to 1e-9 k).
        close(
            &bracket["length"],
            at_place(long, 1e-6 * k),
            k,
            "pattern length",
        );
        close(&bracket["width"], w, k, "pattern width");
        close(&bracket["area"], w * long, k, "pattern area");
        assert_eq!(bracket["top"], Value::Null);
        let geometry = run(&mut s, "cut.list", json!({"geometry": true}));
        let (_, bracket) = part(&geometry, "Bracket");
        let marks = bracket["geometry"]["marks"].as_array().unwrap();
        assert_eq!(marks.len(), 1, "{bracket}");
        assert_eq!(marks[0]["kind"], "bendUp");
        assert_eq!(bracket["holes"], 0, "{bracket}");
        // Exported: the bend line on its own layer, the outline on the
        // part's.
        let out = run(&mut s, "cut.export", json!({"path": "bracket.dxf"}));
        assert_eq!(out["parts"][0]["layer"], "Bracket", "{out}");
        let dxf = std::fs::read_to_string(dir.join("bracket.dxf")).unwrap();
        let lines = entities(&dxf);
        assert_eq!(
            lines.iter().filter(|e| e.layer == "bendUp").count(),
            1,
            "{dxf}"
        );
        let (lo, hi) = layer_box(&lines, "Bracket");
        assert!(((hi[0] - lo[0]) - long).abs() <= 1e-9 * k, "{lo:?} {hi:?}");
        assert!(((hi[1] - lo[1]) - w).abs() <= 1e-9 * k, "{lo:?} {hi:?}");
    }
}

#[test]
fn bodies_that_are_no_plates_are_refused_with_the_face() {
    for k in SCALES {
        let dir = workdir("cut", &format!("refused-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        block(&mut s, "Pocketed", [100.0 * k, 50.0 * k, 6.0 * k], [0.0; 3]);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "offset": 6.0 * k, "name": "Pocket"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Pocket", "center": [50.0 * k, 25.0 * k], "radius": 10.0 * k}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Pocket", "distance": 2.0 * k, "reversed": true, "cut": "Pocketed", "name": "Pocket cut"}),
        );
        block(
            &mut s,
            "Chamfered",
            [100.0 * k, 50.0 * k, 6.0 * k],
            [0.0, 200.0 * k, 0.0],
        );
        // Two top edges, along x and along y: a prism in no direction.
        run(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["Chamfered:face(1)", "Chamfered:face(2)"], ["Chamfered:face(1)", "Chamfered:face(4)"]],
                   "distance": 1.0 * k, "name": "Bevel"}),
        );
        // One long edge: a prism along it, 100 long, which the list says
        // is not a plate across its 6.
        block(
            &mut s,
            "Bar",
            [100.0 * k, 50.0 * k, 6.0 * k],
            [0.0, 1000.0 * k, 0.0],
        );
        run(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["Bar:face(1)", "Bar:face(2)"]], "distance": 1.0 * k, "name": "Bar edge"}),
        );
        run(
            &mut s,
            "feature.sphere",
            json!({"radius": 10.0 * k, "position": [0.0, 500.0 * k, 0.0], "name": "Ball"}),
        );
        block(
            &mut s,
            "Plain",
            [100.0 * k, 50.0 * k, 6.0 * k],
            [0.0, 800.0 * k, 0.0],
        );
        // A barrel: a loft through rounded squares of half-width 20, 25
        // and 20, its top and bottom flat and parallel, its corners only
        // in them, its freeform wall bulging between: no plate.
        for (i, (z, h)) in [(0.0, 20.0), (3.0, 25.0), (6.0, 20.0)].iter().enumerate() {
            let name = format!("Barrel {i}");
            run(
                &mut s,
                "sketch.create",
                json!({"plane": "xy", "offset": z * k, "name": name}),
            );
            run(
                &mut s,
                "sketch.rect",
                json!({"sketch": name, "from": [(500.0 - h) * k, -h * k], "to": [(500.0 + h) * k, h * k]}),
            );
        }
        run(
            &mut s,
            "feature.loft",
            json!({"sections": ["Barrel 0", "Barrel 1", "Barrel 2"], "name": "Barrel"}),
        );
        let list = run(&mut s, "cut.list", json!({}));
        assert_eq!(list["parts"], 2, "{list}");
        let barrel = refused(&list, "Barrel");
        assert!(
            barrel["reason"].as_str().unwrap().contains("freeform"),
            "{barrel}"
        );
        let (group, bar) = part(&list, "Bar edge");
        close(
            &group["thickness"],
            100.0 * k,
            k,
            "the bar along its length",
        );
        let note = bar["notes"][0].as_str().unwrap();
        assert!(
            note.contains("chamfer") && note.contains("thinnest"),
            "{note}"
        );
        // Its outline is the chamfered section: 50 × 6 less a corner.
        close(&bar["area"], (300.0 - 0.5) * k * k, k, "bar section");
        let pocket = refused(&list, "Pocket cut");
        let reason = pocket["reason"].as_str().unwrap();
        assert!(reason.contains("pocket's floor"), "{reason}");
        assert!(reason.contains("2"), "the depth: {reason}");
        assert_eq!(pocket["faces"].as_array().unwrap().len(), 1, "{pocket}");
        let bevel = refused(&list, "Bevel");
        let reason = bevel["reason"].as_str().unwrap();
        assert!(reason.contains("chamfer"), "{reason}");
        assert!(reason.contains("45"), "{reason}");
        let face = bevel["faces"][0].as_str().unwrap();
        assert!(reason.contains(face), "{reason} names {face}");
        let ball = refused(&list, "Ball");
        assert!(
            ball["reason"]
                .as_str()
                .unwrap()
                .contains("no face of it is flat"),
            "{ball}"
        );
        // The CSV lists them with their reasons.
        let csv = run(&mut s, "cut.list", json!({"format": "csv"}));
        let text = csv["csv"].as_str().unwrap();
        assert_eq!(text.lines().count(), 1 + 2 + 4, "{text}");
        assert!(text.contains(",not a plate,"), "{text}");
        // The export draws the plate and lists the rest.
        let out = run(&mut s, "cut.export", json!({"path": "parts.svg"}));
        assert_eq!(out["parts"].as_array().unwrap().len(), 2, "{out}");
        assert_eq!(out["notPlates"].as_array().unwrap().len(), 4, "{out}");
    }
}

/// A DXF entity: kind, layer and its numeric group codes.
struct Entity {
    kind: String,
    layer: String,
    codes: BTreeMap<i32, Vec<f64>>,
}

impl Entity {
    fn get(&self, code: i32) -> f64 {
        self.codes[&code][0]
    }
}

fn entities(dxf: &str) -> Vec<Entity> {
    let start = dxf.find("  2\nENTITIES\n").unwrap();
    let end = dxf.find("  2\nOBJECTS\n").unwrap();
    let lines: Vec<&str> = dxf[start..end].lines().collect();
    let mut out: Vec<Entity> = Vec::new();
    for pair in lines.chunks(2) {
        let [code, value] = pair else { continue };
        let code: i32 = code.trim().parse().unwrap();
        if code == 0 {
            out.push(Entity {
                kind: value.to_string(),
                layer: String::new(),
                codes: BTreeMap::new(),
            });
        } else if let Some(e) = out.last_mut() {
            if code == 8 {
                e.layer = value.to_string();
            } else if let Ok(v) = value.trim().parse::<f64>() {
                e.codes.entry(code).or_default().push(v);
            }
        }
    }
    out.retain(|e| ["LINE", "ARC", "CIRCLE"].contains(&e.kind.as_str()));
    out
}

/// The box of the lines, arcs and circles on `layer` (arcs by their
/// quarter points within their run).
fn layer_box(entities: &[Entity], layer: &str) -> ([f64; 2], [f64; 2]) {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut grow = |x: f64, y: f64| {
        lo = [lo[0].min(x), lo[1].min(y)];
        hi = [hi[0].max(x), hi[1].max(y)];
    };
    for e in entities.iter().filter(|e| e.layer == layer) {
        match e.kind.as_str() {
            "LINE" => {
                grow(e.get(10), e.get(20));
                grow(e.get(11), e.get(21));
            }
            "CIRCLE" | "ARC" => {
                let (cx, cy, r) = (e.get(10), e.get(20), e.get(40));
                let (a0, mut a1) = if e.kind == "ARC" {
                    (e.get(50), e.get(51))
                } else {
                    (0.0, 360.0)
                };
                while a1 <= a0 {
                    a1 += 360.0;
                }
                // The ends, and the quarter points the run passes.
                let mut angles = vec![a0, a1];
                let mut q = (a0 / 90.0).ceil() * 90.0;
                while q < a1 {
                    angles.push(q);
                    q += 90.0;
                }
                for a in angles {
                    let t = a.to_radians();
                    grow(cx + r * t.pcos(), cy + r * t.psin());
                }
            }
            _ => {}
        }
    }
    (lo, hi)
}

/// The length of the lines, arcs and circles on `layer`.
fn layer_length(entities: &[Entity], layer: &str) -> f64 {
    entities
        .iter()
        .filter(|e| e.layer == layer)
        .map(|e| match e.kind.as_str() {
            "LINE" => (e.get(11) - e.get(10)).phypot(e.get(21) - e.get(20)),
            "CIRCLE" => 2.0 * PI * e.get(40),
            "ARC" => {
                let mut sweep = e.get(51) - e.get(50);
                while sweep <= 0.0 {
                    sweep += 360.0;
                }
                e.get(40) * sweep.to_radians()
            }
            _ => 0.0,
        })
        .sum()
}

#[test]
fn outlines_are_written_and_read_back_with_and_without_a_kerf() {
    for k in SCALES {
        let dir = workdir("cut", &format!("export-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        holed_plate(&mut s, "Holed", k, 6.0);
        block(
            &mut s,
            "Shelf",
            [100.0 * k, 50.0 * k, 6.0 * k],
            [0.0, 200.0 * k, 0.0],
        );
        // Without a kerf: the closed forms read back from the DXF.
        let out = run(
            &mut s,
            "cut.export",
            json!({"path": "sheet.dxf", "gap": 10.0 * k}),
        );
        assert_eq!(out["rows"], 1, "{out}");
        let dxf = std::fs::read_to_string(dir.join("sheet.dxf")).unwrap();
        let found = entities(&dxf);
        let cut = 300.0 + 10.0 * PI + 60.0 + 8.0 * PI;
        let got = layer_length(&found, "Holed");
        assert!((got - cut * k).abs() <= 1e-9 * k, "{got}");
        let (lo, hi) = layer_box(&found, "Holed");
        assert!((hi[0] - lo[0] - 100.0 * k).abs() <= 1e-9 * k);
        assert!((hi[1] - lo[1] - 50.0 * k).abs() <= 1e-9 * k);
        // The second part 10 to the right of the first.
        let (lo2, _) = layer_box(&found, "Shelf");
        assert!(
            (lo2[0] - hi[0] - 10.0 * k).abs() <= 1e-9 * k,
            "{lo2:?} {hi:?}"
        );
        // With a kerf k: the outline 2 k longer each way, the round hole k
        // smaller, the slot k narrower each side, arcs still arcs.
        let kerf = 0.2 * k;
        let out = run(
            &mut s,
            "cut.export",
            json!({"path": "kerf.dxf", "kerf": format!("{kerf} mm"), "bodies": ["Holed"]}),
        );
        close(&out["kerf"], kerf, k, "kerf");
        let dxf = std::fs::read_to_string(dir.join("kerf.dxf")).unwrap();
        let found = entities(&dxf);
        let (lo, hi) = layer_box(&found, "Holed");
        assert!(
            (hi[0] - lo[0] - (100.0 * k + 2.0 * kerf)).abs() <= 1e-9 * k,
            "{lo:?} {hi:?}"
        );
        assert!((hi[1] - lo[1] - (50.0 * k + 2.0 * kerf)).abs() <= 1e-9 * k);
        let circles: Vec<&Entity> = found.iter().filter(|e| e.kind == "CIRCLE").collect();
        assert_eq!(circles.len(), 1);
        assert!((circles[0].get(40) - (5.0 * k - kerf)).abs() <= 1e-9 * k);
        let arcs: Vec<f64> = found
            .iter()
            .filter(|e| e.kind == "ARC")
            .map(|e| e.get(40))
            .collect();
        // Four round corners of radius kerf, two slot ends of 4k - kerf.
        assert_eq!(
            arcs.iter()
                .filter(|r| (*r - kerf).abs() <= 1e-9 * k)
                .count(),
            4,
            "{arcs:?}"
        );
        assert_eq!(
            arcs.iter()
                .filter(|r| (*r - (4.0 * k - kerf)).abs() <= 1e-9 * k)
                .count(),
            2,
            "{arcs:?}"
        );
        // Length: the outline's Steiner perimeter, the holes' smaller.
        let length = (300.0 * k + 2.0 * PI * kerf)
            + 2.0 * PI * (5.0 * k - kerf)
            + (60.0 * k + 2.0 * PI * (4.0 * k - kerf));
        assert!((layer_length(&found, "Holed") - length).abs() <= 1e-9 * k);
        // SVG: a path per part, true size in the header.
        let out = run(
            &mut s,
            "cut.export",
            json!({"path": "sheet.svg", "gap": 10.0 * k}),
        );
        let svg = std::fs::read_to_string(dir.join("sheet.svg")).unwrap();
        assert!(svg.contains("id=\"layer-Holed\""), "{svg}");
        assert!(svg.contains("id=\"layer-Shelf\""), "{svg}");
        assert_eq!(out["parts"].as_array().unwrap().len(), 2);
        // The same document gives the same bytes.
        run(
            &mut s,
            "cut.export",
            json!({"path": "again.svg", "gap": 10.0 * k}),
        );
        assert_eq!(svg, std::fs::read_to_string(dir.join("again.svg")).unwrap());
        // One file per part.
        let out = run(
            &mut s,
            "cut.export",
            json!({"path": "part.dxf", "layout": "each"}),
        );
        assert_eq!(
            out["files"],
            json!(["part-Holed.dxf", "part-Shelf.dxf"]),
            "{out}"
        );
        for file in ["part-Holed.dxf", "part-Shelf.dxf"] {
            let dxf = std::fs::read_to_string(dir.join(file)).unwrap();
            let (lo, _) = layer_box(
                &entities(&dxf),
                file.trim_start_matches("part-").trim_end_matches(".dxf"),
            );
            assert!(
                lo[0].abs() <= 1e-9 * k && lo[1].abs() <= 1e-9 * k,
                "{file}: {lo:?}"
            );
        }
    }
}

#[test]
fn copies_go_on_rows_of_the_sheet_width() {
    let dir = workdir("cut", "rows");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    for i in 0..3 {
        block(
            &mut s,
            &format!("Panel {i}"),
            [100.0, 50.0, 6.0],
            [0.0, 100.0 * f64::from(i), 0.0],
        );
    }
    let out = run(
        &mut s,
        "cut.export",
        json!({"path": "rows.dxf", "copies": true, "width": "250 mm", "gap": "5 mm"}),
    );
    assert_eq!(out["rows"], 2, "{out}");
    let parts = out["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0]["layer"], "Panel_0");
    assert_eq!(parts[2]["bbox"], json!([0.0, 55.0, 100.0, 105.0]), "{out}");
    // Without copies, one panel to cut three times.
    let out = run(&mut s, "cut.export", json!({"path": "one.dxf"}));
    assert_eq!(out["parts"].as_array().unwrap().len(), 1);
    assert_eq!(out["parts"][0]["count"], 3);
}

#[test]
fn bad_parameters_and_impossible_kerfs_are_refused() {
    let dir = workdir("cut", "bad");
    let mut s = session_in(&dir);
    let no_doc = s.execute("cut.list", &json!({})).unwrap_err().to_string();
    assert!(!no_doc.contains("internal error"), "{no_doc}");
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "feature.sphere",
        json!({"radius": 10.0, "name": "Ball"}),
    );
    let none = s
        .execute("cut.export", &json!({"path": "x.dxf"}))
        .unwrap_err()
        .to_string();
    assert!(none.contains("no body is a part"), "{none}");
    holed_plate(&mut s, "Holed", 1.0, 6.0);
    for (id, params, words) in [
        ("cut.list", json!({"format": "xml"}), "json or csv"),
        ("cut.list", json!({"geometry": "yes"}), "geometry"),
        (
            "cut.list",
            json!({"format": "csv", "geometry": true}),
            "geometry",
        ),
        ("cut.list", json!({"bodies": []}), "empty"),
        ("cut.list", json!({"bodies": ["Nope"]}), "Nope"),
        ("cut.list", json!({"what": 1}), "unknown parameter"),
        ("cut.export", json!({}), "path"),
        ("cut.export", json!({"path": "x.txt"}), "format"),
        (
            "cut.export",
            json!({"path": "x.dxf", "layout": "nest"}),
            "sheet or each",
        ),
        (
            "cut.export",
            json!({"path": "x.dxf", "kerf": "-1 mm"}),
            "`kerf` must be a length of 0 or more, not -1 mm",
        ),
        (
            "cut.export",
            json!({"path": "x.dxf", "kerf": "4.5 mm"}),
            "closes a hole",
        ),
        (
            "cut.export",
            json!({"path": "x.dxf", "kerf": "4.5 mm"}),
            "largest distance that fits is 3.99",
        ),
        (
            "cut.export",
            json!({"path": "x.dxf", "stroke": 0.1}),
            "stroke",
        ),
        (
            "cut.export",
            json!({"path": "x.svg", "stroke": -1}),
            "`stroke` must be a line width in millimetres above 0 and at most 100, not -1",
        ),
        (
            "cut.export",
            json!({"path": "x.dxf", "layout": "each", "gap": 3}),
            "gap",
        ),
        (
            "cut.export",
            json!({"path": "x.dxf", "width": "50 mm"}),
            "mm long, more than the sheet's width 50 mm; give a larger `width`",
        ),
        (
            "cut.export",
            json!({"path": "x.dxf", "width": 0}),
            "`width` must be a length above 0, not 0 mm",
        ),
        (
            "cut.export",
            json!({"path": "x.dxf", "gap": "-1 mm"}),
            "`gap` must be a length of 0 or more, not -1 mm",
        ),
        (
            "cut.export",
            json!({"path": "x.dxf", "kerf": "1 deg"}),
            "kerf",
        ),
    ] {
        let e = s
            .execute(id, &params)
            .expect_err(&params.to_string())
            .to_string();
        assert!(!e.contains("internal error"), "{params}: {e}");
        assert!(e.contains(words), "{id} {params}: `{e}` lacks `{words}`");
    }
}

/// The shelf of the guide (docs/guide/12-files.md › *A cut list: a small
/// shelf*): its steps run as written, and its cut list is the one the
/// guide prints. The laid-out sheet is left in `target/evidence/cut/`.
#[test]
fn the_shelf_of_the_guide_gives_the_cut_list_it_prints() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let guide = std::fs::read_to_string(root.join("docs/guide/12-files.md")).unwrap();
    let at = guide.find("### A cut list: a small shelf").unwrap();
    let rest = &guide[at..];
    let script = &rest[rest.find("```json\n").unwrap() + 8..];
    let script = &script[..script.find("```").unwrap()];
    let printed = &rest[rest.find("```text\n").unwrap() + 8..];
    let printed = &printed[..printed.find("```").unwrap()];
    let dir = workdir("cut", "shelf");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let steps: Value = serde_json::from_str(script).unwrap();
    let mut last = Value::Null;
    for step in steps.as_array().unwrap() {
        let id = step[0].as_str().unwrap();
        last = run(&mut s, id, step[1].clone());
        if id == "cut.list" {
            assert_eq!(last["csv"].as_str().unwrap(), printed, "{last}");
        }
    }
    assert_eq!(last["parts"].as_array().unwrap().len(), 5, "{last}");
    assert_eq!(last["rows"], 2, "{last}");
    let list = run(&mut s, "cut.list", json!({}));
    let (_, back) = part(&list, "Back with hole");
    assert_eq!(back["material"], "hardboard");
    close(&back["area"], 180_000.0 - 400.0 * PI, 1.0, "back area");
    let svg = std::fs::read_to_string(dir.join("shelf.svg")).unwrap();
    let evidence = root.join("target/evidence/cut");
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(evidence.join("shelf.svg"), &svg).unwrap();
}

/// J-042: a 3 mm plate turned 40° about y is 3 thick, 100 long and 60
/// wide in the list, without rounding noise, and lands in one group, as
/// one part, with the same plate lying flat (the turned one first, so the
/// group's thickness is its).
#[test]
fn a_turned_plate_is_listed_without_noise_with_its_flat_twin() {
    let dir = workdir("cut", "turned");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": ["100 mm", "60 mm", "3 mm"], "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.transform",
        json!({"source": "Plate", "angle": "40 deg", "axis": [0, 1, 0], "pivot": ["0 mm", "0 mm", "300 mm"], "name": "Turned plate"}),
    );
    let list = run(&mut s, "cut.list", json!({}));
    let (group, turned) = part(&list, "Turned plate");
    for (value, want) in [
        (&group["thickness"], 3.0),
        (&turned["thickness"], 3.0),
        (&turned["length"], 100.0),
        (&turned["width"], 60.0),
        (&turned["cutLength"], 320.0),
    ] {
        assert_eq!(number(value), want, "{turned}");
    }
    block(&mut s, "Flat plate", [100.0, 60.0, 3.0], [0.0, 200.0, 0.0]);
    let list = run(&mut s, "cut.list", json!({}));
    assert_eq!(list["groups"].as_array().unwrap().len(), 1, "{list}");
    let (group, first) = part(&list, "Turned plate");
    assert_eq!(number(&group["thickness"]), 3.0, "{group}");
    assert_eq!(group["count"], 2, "{group}");
    assert_eq!(first["count"], 2, "{first}");
    assert_eq!(first["bodies"], json!(["Turned plate", "Flat plate"]));
    let csv = run(&mut s, "cut.list", json!({"format": "csv"}));
    let row = csv["csv"]
        .as_str()
        .unwrap()
        .lines()
        .nth(1)
        .unwrap()
        .to_string();
    assert!(
        row.starts_with(",3,Turned plate,2,100,60,"),
        "the sizes as a shop reads them: {row}"
    );
}

/// J-003: a body switched off (its condition is 0) named in `bodies` is
/// left out, so one list holds for every option; a list of only such
/// bodies lists nothing; a name of no feature is still refused.
#[test]
fn a_switched_off_body_is_left_out_of_the_cut_list() {
    let dir = workdir("cut", "switched-off");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    block(&mut s, "Board", [100.0, 50.0, 18.0], [0.0; 3]);
    run(
        &mut s,
        "feature.block",
        json!({"name": "Optional shelf", "size": [100, 50, 18], "position": [0, 0, 40], "when": "0"}),
    );
    let both = json!(["Board", "Optional shelf"]);
    // What was left out is said (review, S4).
    let left = json!({"count": 1, "bodies": ["Optional shelf"]});
    let list = run(&mut s, "cut.list", json!({"bodies": both}));
    assert_eq!(list["parts"], 1, "{list}");
    part(&list, "Board");
    assert_eq!(list["switchedOffLeftOut"], left, "{list}");
    let csv = run(&mut s, "cut.list", json!({"bodies": both, "format": "csv"}));
    assert_eq!(csv["parts"], 1, "{csv}");
    assert_eq!(csv["switchedOffLeftOut"], left, "{csv}");
    let none = run(&mut s, "cut.list", json!({"bodies": ["Optional shelf"]}));
    assert_eq!(none["parts"], 0, "{none}");
    assert_eq!(none["groups"], json!([]), "{none}");
    assert_eq!(none["switchedOffLeftOut"], left, "{none}");
    let out = run(
        &mut s,
        "cut.export",
        json!({"path": "board.dxf", "bodies": both}),
    );
    assert_eq!(out["parts"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(out["switchedOffLeftOut"], left, "{out}");
    let e = common::error(&mut s, "cut.list", json!({"bodies": ["Board", "Nobody"]}));
    assert!(e.contains("there is no feature `Nobody`"), "{e}");
    // A sketch switched off never has a body: still an error that says so,
    // not left out.
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Drawing", "plane": "xy"}),
    );
    run(
        &mut s,
        "feature.when",
        json!({"feature": "Drawing", "expr": "0"}),
    );
    let e = common::error(&mut s, "cut.list", json!({"bodies": ["Board", "Drawing"]}));
    assert!(e.contains("Drawing") && e.contains("no body"), "{e}");
}

/// J-046: without `bodies`, a body hidden by feature.visible is left out
/// of the cut list and its export, which say which; named, it is listed;
/// every body hidden lists nothing and the export is refused.
#[test]
fn a_hidden_body_is_left_out_of_the_cut_list_unless_named() {
    let dir = workdir("cut", "hidden");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    block(&mut s, "Board", [100.0, 60.0, 18.0], [0.0; 3]);
    block(&mut s, "Helper", [80.0, 40.0, 18.0], [0.0, 0.0, 40.0]);
    run(
        &mut s,
        "feature.visible",
        json!({"feature": "Helper", "visible": false}),
    );
    let left = json!({"count": 1, "bodies": ["Helper"]});
    let list = run(&mut s, "cut.list", json!({}));
    assert_eq!(list["parts"], 1, "{list}");
    part(&list, "Board");
    assert_eq!(list["hiddenLeftOut"], left, "{list}");
    let csv = run(&mut s, "cut.list", json!({"format": "csv"}));
    assert_eq!(csv["hiddenLeftOut"], left, "{csv}");
    assert!(!csv["csv"].as_str().unwrap().contains("Helper"), "{csv}");
    let out = run(&mut s, "cut.export", json!({"path": "board.svg"}));
    assert_eq!(out["parts"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(out["hiddenLeftOut"], left, "{out}");
    let named = run(&mut s, "cut.list", json!({"bodies": ["Helper"]}));
    assert_eq!(named["parts"], 1, "{named}");
    assert!(named.get("hiddenLeftOut").is_none(), "{named}");
    run(
        &mut s,
        "feature.visible",
        json!({"feature": "Board", "visible": false}),
    );
    let none = run(&mut s, "cut.list", json!({}));
    assert_eq!(none["parts"], 0, "{none}");
    assert_eq!(none["hiddenLeftOut"]["count"], 2, "{none}");
    let e = common::error(&mut s, "cut.export", json!({"path": "none.svg"}));
    assert!(e.contains("every body is hidden (Board, Helper)"), "{e}");
}

#[test]
fn a_name_like_a_formula_is_not_a_formula_in_the_csv() {
    let dir = workdir("cut", "formula");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    block(&mut s, "=1+1", [100.0, 50.0, 6.0], [0.0; 3]);
    let csv = run(&mut s, "cut.list", json!({"format": "csv"}));
    let row = csv["csv"]
        .as_str()
        .unwrap()
        .lines()
        .nth(1)
        .unwrap()
        .to_string();
    assert!(row.contains(",'=1+1,1,"), "{row}");
    assert!(row.ends_with(",plate,'=1+1,"), "{row}");
}

/// gap and width are read alike by every command that lays parts out in
/// rows (cut.export, file.export of faces and of flat patterns): an
/// infinite or not-a-number length is refused, a huge one is given short
/// (not 309 digits), and null is not given (also for layout each, which
/// takes no gap).
#[test]
fn layout_lengths_are_finite_short_and_null_is_not_given() {
    let dir = workdir("cut", "layout-lengths");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    holed_plate(&mut s, "Holed", 1.0, 6.0);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Base"}),
    );
    let made = run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [200.0, 0.0], "to": [240.0, 30.0]}),
    );
    let edge = made["lines"][0].as_str().unwrap().to_string();
    run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Bracket", "thickness": "2 mm", "radius": "3 mm",
               "kFactor": "0.4", "flanges": [{"edge": edge, "length": "25 mm", "angle": "90 deg"}]}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "huge", "expr": "1e300 mm"}),
    );
    let inspect = run(&mut s, "document.inspect", json!({}));
    let top = inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "Holed")
        .and_then(|b| b["faceNames"].as_array())
        .and_then(|f| f.iter().find(|n| n.as_str().unwrap().contains("endCap")))
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let commands = [
        ("cut.export", json!({"path": "a.dxf"})),
        ("file.export", json!({"path": "b.dxf", "faces": [top]})),
        (
            "file.export",
            json!({"path": "c.dxf", "sheetMetal": ["Bracket"]}),
        ),
    ];
    for (id, base) in &commands {
        let with = |key: &str, value: Value| {
            let mut p = base.clone();
            p[key] = value;
            p
        };
        for (key, value) in [
            ("gap", json!("huge * 1e300")),
            ("width", json!("huge * 1e300")),
            ("gap", json!("-huge")),
        ] {
            let params = with(key, value);
            let e = s
                .execute(id, &params)
                .expect_err(&params.to_string())
                .to_string();
            assert!(!e.contains("internal error"), "{id} {params}: {e}");
            assert!(e.contains(&format!("`{key}`")), "{id} {params}: {e}");
            assert!(e.len() < 400, "{id} {params}: a short message: {e}");
            assert!(!e.contains("000000000000"), "{id} {params}: {e}");
        }
        let minus = s
            .execute(id, &with("gap", json!("-huge")))
            .unwrap_err()
            .to_string();
        assert!(minus.contains("not -1e300 mm"), "{id}: {minus}");
        // null is not given: the default gap, no limit of width.
        let null = with("gap", Value::Null);
        let null = {
            let mut p = null;
            p["width"] = Value::Null;
            p
        };
        run(&mut s, id, null);
    }
    // layout each takes no gap or width; null is not giving one.
    run(
        &mut s,
        "cut.export",
        json!({"path": "each.dxf", "layout": "each", "gap": null, "width": null}),
    );
    let e = s
        .execute(
            "cut.export",
            &json!({"path": "each.dxf", "layout": "each", "gap": 3}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("`gap` and `width` lay parts out on one sheet"),
        "{e}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
