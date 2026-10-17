//! Flat faces drawn for laser cutting (`file.export` of `face` and
//! `faces`): names followed to the newest body as features follow them,
//! the drawing frame (seen from outside, never mirrored: outer boundaries
//! counter-clockwise), and several faces laid out on one sheet. The
//! files are read back here: the SVG path data (lines and arcs) and the
//! DXF layers.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};
use std::path::PathBuf;

fn session(tag: &str) -> (Session, PathBuf) {
    let dir = std::env::temp_dir().join(format!("craft3d-sheet-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut session, "file.new", json!({}));
    (session, dir)
}

fn refused(session: &mut Session, params: Value, words: &[&str]) {
    let error = session
        .execute("file.export", &params)
        .expect_err(&params.to_string())
        .to_string();
    println!("refused: {error}");
    assert!(!error.contains("internal error"), "{params}: {error}");
    for word in words {
        assert!(error.contains(word), "{params}: `{error}` lacks `{word}`");
    }
}

fn numbers(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{value} is no list"))
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect()
}

fn near(a: f64, b: f64, scale: f64) -> bool {
    (a - b).abs() <= 1e-9 * scale.max(1.0)
}

// ------------------------------------------------------------ SVG reader

/// One closed loop of an SVG path: its signed area (counter-clockwise
/// positive, in the model's y-up axes), and its corners' box (exact for
/// lines; for arcs only the ends count).
#[derive(Debug, Clone, Copy)]
struct Loop {
    area: f64,
    bbox: [f64; 4],
}

fn tokens(d: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in d.chars() {
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            out.push(c.to_string());
        } else if c == ' ' || c == ',' {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The loops of an SVG `d` with M, L, A and Z only; y is turned back up.
fn loops(d: &str) -> Vec<Loop> {
    let t = tokens(d);
    let mut i = 0;
    let mut out = Vec::new();
    let mut twice = 0.0;
    let mut at = [0.0, 0.0];
    let mut start = [0.0, 0.0];
    let mut bbox = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let num = |i: &mut usize| -> f64 {
        let v: f64 = t[*i].parse().unwrap();
        *i += 1;
        v
    };
    let grow = |b: &mut [f64; 4], p: [f64; 2]| {
        *b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
    };
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    while i < t.len() {
        let command = t[i].clone();
        i += 1;
        match command.as_str() {
            "M" => {
                let p = [num(&mut i), -num(&mut i)];
                (at, start) = (p, p);
                twice = 0.0;
                bbox = [p[0], p[1], p[0], p[1]];
            }
            "L" => {
                let p = [num(&mut i), -num(&mut i)];
                twice += cross(at, p);
                grow(&mut bbox, p);
                at = p;
            }
            "A" => {
                let (rx, ry, _rot) = (num(&mut i), num(&mut i), num(&mut i));
                assert!((rx - ry).abs() <= 1e-9 * rx, "circular arcs only");
                let large = num(&mut i) != 0.0;
                // SVG's sweep flag 1 runs clockwise once y is turned up.
                let cw = num(&mut i) != 0.0;
                let p = [num(&mut i), -num(&mut i)];
                let chord = ((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2)).sqrt();
                let half = (chord / 2.0 / rx).clamp(-1.0, 1.0).asin();
                let theta = if large { TAU - 2.0 * half } else { 2.0 * half };
                let segment = rx * rx / 2.0 * (theta - theta.sin());
                twice += cross(at, p) + 2.0 * if cw { -segment } else { segment };
                grow(&mut bbox, p);
                at = p;
            }
            "Z" => {
                twice += cross(at, start);
                out.push(Loop {
                    area: twice / 2.0,
                    bbox,
                });
                at = start;
            }
            other => panic!("unexpected SVG command {other} in {d}"),
        }
    }
    out
}

/// The `<path>` elements of an SVG file: layer name → loops.
fn svg_layers(svg: &str) -> BTreeMap<String, Vec<Loop>> {
    let mut out = BTreeMap::new();
    for element in svg.split("<path").skip(1) {
        let attribute = |name: &str| {
            let key = format!(" {name}=\"");
            let start = element.find(&key).unwrap() + key.len();
            element[start..start + element[start..].find('"').unwrap()].to_string()
        };
        let id = attribute("id");
        let name = id.strip_prefix("layer-").unwrap().to_string();
        out.insert(name, loops(&attribute("d")));
    }
    out
}

/// The layer of every entity of a DXF file, and the layer table.
fn dxf_layers(dxf: &str) -> (Vec<String>, Vec<String>) {
    let lines: Vec<&str> = dxf.lines().collect();
    let pairs: Vec<(&str, &str)> = lines.chunks(2).map(|p| (p[0].trim(), p[1])).collect();
    let entities = pairs.iter().position(|p| *p == ("2", "ENTITIES")).unwrap();
    let mut on = Vec::new();
    for (code, value) in &pairs[entities..] {
        if *code == "8" {
            on.push(value.to_string());
        }
        if *value == "ENDSEC" {
            break;
        }
    }
    let mut table = Vec::new();
    let mut in_layer = false;
    for (code, value) in &pairs[..entities] {
        if *code == "0" {
            in_layer = *value == "LAYER";
        }
        if in_layer && *code == "2" && *value != "LAYER" {
            table.push(value.to_string());
        }
    }
    (on, table)
}

fn svg_of(dir: &std::path::Path, file: &str) -> BTreeMap<String, Vec<Loop>> {
    svg_layers(&std::fs::read_to_string(dir.join(file)).unwrap())
}

/// The one layer of a single face's SVG.
fn outline(dir: &std::path::Path, file: &str) -> Vec<Loop> {
    let layers = svg_of(dir, file);
    assert_eq!(layers.len(), 1, "{layers:?}");
    layers["outline"].clone()
}

/// Whether box `a` lies inside box `b`.
fn inside(a: [f64; 4], b: [f64; 4]) -> bool {
    a != b && a[0] >= b[0] && a[1] >= b[1] && a[2] <= b[2] && a[3] <= b[3]
}

/// The loops that lie inside no other (outer boundaries) must run
/// counter-clockwise; the area inside them less the holes. A hole that
/// is a chain runs clockwise; a hole that is one whole circle is a circle,
/// which has no direction of its own, so only its size counts.
fn oriented_area(loops: &[Loop]) -> f64 {
    let mut total = 0.0;
    for l in loops {
        if loops.iter().any(|other| inside(l.bbox, other.bbox)) {
            total -= l.area.abs();
        } else {
            assert!(l.area > 0.0, "an outer boundary runs clockwise: {loops:?}");
            total += l.area;
        }
    }
    total
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn arr3(v: &Value) -> [f64; 3] {
    let n = numbers(v);
    [n[0], n[1], n[2]]
}

/// A plate with a notch cut out of one edge: the laser box's panel.
/// `Plate` is a w × h × 3 block at `at`, the notch 10 wide and 3 deep.
fn panel(s: &mut Session, name: &str, w: f64, h: f64, at: [f64; 3]) {
    run(
        s,
        "feature.block",
        json!({"name": format!("{name}Blank"), "size": [w, h, 3], "position": at}),
    );
    run(
        s,
        "feature.block",
        json!({"name": format!("{name}Notch"), "size": [10, 6, 9],
               "position": [at[0] + w / 2.0 - 5.0, at[1] - 3.0, at[2] - 3.0]}),
    );
    run(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": format!("{name}Blank"), "tool": format!("{name}Notch"), "name": name}),
    );
}

// ------------------------------------------------------- following names

#[test]
fn a_face_is_exported_by_the_name_it_had_in_an_earlier_body() {
    let (mut s, dir) = session("follow");
    panel(&mut s, "Panel", 60.0, 40.0, [0.0, 0.0, 0.0]);
    let full = run(
        &mut s,
        "file.export",
        json!({"path": "full.svg", "face": "F3:copy(0)/F1:face(1)"}),
    );
    // The blank's own name, with and without the body, and the blank's
    // feature as the body: all followed to the panel.
    for (file, params) in [
        ("short.svg", json!({"face": "F1:face(1)"})),
        (
            "body.svg",
            json!({"face": "F1:face(1)", "feature": "Panel"}),
        ),
        (
            "blank.svg",
            json!({"face": "F1:face(1)", "feature": "PanelBlank"}),
        ),
    ] {
        let mut params = params;
        params["path"] = json!(file);
        let result = run(&mut s, "file.export", params);
        println!("{result}");
        assert_eq!(result["followed"], "F3:copy(0)/F1:face(1)");
        assert_eq!(result["face"], "F1:face(1)");
        assert_eq!(result["feature"], "F3");
        assert_eq!(result["name"], "Panel");
        assert_eq!(
            std::fs::read(dir.join(file)).unwrap(),
            std::fs::read(dir.join("full.svg")).unwrap(),
            "the short name draws what the full one does"
        );
    }
    assert!(full.get("followed").is_none(), "{full}");
    // 60 × 40 less the notch's 10 × 3.
    let area = oriented_area(&outline(&dir, "full.svg"));
    assert!(near(area, 2370.0, 2370.0), "{area}");

    // A face cut in two is one copy that names both pieces: both drawn.
    run(
        &mut s,
        "feature.block",
        json!({"name": "Slot", "size": [4, 50, 9], "position": [10, -5, -3]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Panel", "tool": "Slot", "name": "Slotted"}),
    );
    let pieces = run(
        &mut s,
        "file.export",
        json!({"path": "pieces.svg", "face": "F1:face(1)"}),
    );
    assert_eq!(pieces["faces"], 2, "{pieces}");
    assert_eq!(pieces["followed"], "F5:copy(0)/F1:face(1)");
    let area = oriented_area(&outline(&dir, "pieces.svg"));
    assert!(near(area, 2370.0 - 4.0 * 40.0, 2370.0), "{area}");

    // A face a boolean took away is gone, and says so.
    run(
        &mut s,
        "feature.block",
        json!({"name": "Lid", "size": [100, 100, 10], "position": [-20, -20, 1]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Slotted", "tool": "Lid", "name": "Thin"}),
    );
    refused(
        &mut s,
        json!({"path": "gone.svg", "face": "F1:face(1)"}),
        &["F1:face(1)", "is gone", "Thin"],
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_face_that_became_several_lists_the_names_to_choose_from() {
    let (mut s, dir) = session("several");
    run(
        &mut s,
        "feature.block",
        json!({"name": "Tile", "size": [20, 10, 3]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Tile", "count": 3, "step": [30, 0, 0], "name": "Row"}),
    );
    refused(
        &mut s,
        json!({"path": "a.svg", "face": "F1:face(1)"}),
        &[
            "more than one face",
            "Row",
            "F2:copy(0)/F1:face(1)",
            "F2:copy(1)/F1:face(1)",
            "F2:copy(2)/F1:face(1)",
        ],
    );
    // One of them, by its full name.
    let one = run(
        &mut s,
        "file.export",
        json!({"path": "b.svg", "face": "F2:copy(2)/F1:face(1)"}),
    );
    let bbox = numbers(&one["bbox"]);
    assert_eq!(bbox, [60.0, 0.0, 80.0, 10.0], "{one}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_kept_copy_is_a_body_of_its_own_and_is_not_followed_into() {
    let (mut s, dir) = session("kept");
    run(
        &mut s,
        "feature.block",
        json!({"name": "Blank", "size": [60, 40, 3]}),
    );
    run(
        &mut s,
        "feature.mirror",
        json!({"source": "Blank", "normal": [0, 1, 0], "point": [0, 50, 0], "keep": true, "name": "Back"}),
    );
    panel_cut(&mut s, "Blank", "Front");
    // The blank's name goes where the blank's body went: the front.
    let front = run(
        &mut s,
        "file.export",
        json!({"path": "front.svg", "face": "F1:face(1)"}),
    );
    assert_eq!(front["name"], "Front");
    refused(
        &mut s,
        json!({"path": "back.svg", "face": "F1:face(1)", "feature": "Back"}),
        &["`Back` has no face F1:face(1)", "`Front`", "keep"],
    );
    // The copy by its own name.
    let back = run(
        &mut s,
        "file.export",
        json!({"path": "back.svg", "face": "F2:copy(0)/F1:face(1)", "feature": "Back"}),
    );
    assert!(
        near(oriented_area(&outline(&dir, "back.svg")), 2400.0, 2400.0),
        "{back}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Cuts the notch out of the edge of `target` as `panel` does.
fn panel_cut(s: &mut Session, target: &str, name: &str) {
    run(
        s,
        "feature.block",
        json!({"name": format!("{name}Notch"), "size": [10, 6, 9], "position": [25, -3, -3]}),
    );
    run(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": target, "tool": format!("{name}Notch"), "name": name}),
    );
}

// ------------------------------------------------------------- the frame

/// Checks the frame of a face's drawing: seen from outside (z = x × y
/// points out of the body), right-handed, y up the face or x along the
/// model's x on a face that lies flat; the outline counter-clockwise.
fn check_frame(result: &Value, loops: &[Loop], centre: [f64; 3], area: f64) {
    // No negative zero in the result.
    for key in ["origin", "xAxis", "yAxis"] {
        for v in numbers(&result[key]) {
            assert!(!(v == 0.0 && v.is_sign_negative()), "-0 in {key}: {result}");
        }
    }
    let (x, y) = (arr3(&result["xAxis"]), arr3(&result["yAxis"]));
    let origin = arr3(&result["origin"]);
    let z = cross3(x, y);
    assert!(near(dot3(x, y), 0.0, 1.0) && near(dot3(z, z), 1.0, 1.0));
    // The middle of the outline in the model lies on the far side of the
    // face from the body's centre, along z.
    let b = loops[0].bbox;
    let (u, v) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    let middle = [0, 1, 2].map(|i| origin[i] + x[i] * u + y[i] * v);
    let out = [0, 1, 2].map(|i| middle[i] - centre[i]);
    assert!(dot3(out, z) > 0.0, "the face is seen from inside: {result}");
    let flat = cross3(z, [0.0, 0.0, 1.0]);
    if dot3(flat, flat).sqrt() <= 1e-9 {
        assert!(near(x[0], 1.0, 1.0), "x is the model's x: {result}");
    } else {
        // y runs up the face, and x is level.
        assert!(y[2] > 0.0 && near(x[2], 0.0, 1.0), "{result}");
        let up = [0.0, 0.0, 1.0];
        let along = [0, 1, 2].map(|i| up[i] - z[i] * dot3(up, z));
        let n = dot3(along, along).sqrt();
        assert!(near(dot3(y, along) / n, 1.0, 1.0), "{result}");
    }
    let got = oriented_area(loops);
    assert!(near(got, area, area), "{got} {area}");
}

#[test]
fn every_face_of_a_block_is_drawn_from_outside_counter_clockwise() {
    let (mut s, dir) = session("frame");
    let (a, b, c) = (30.0, 20.0, 10.0);
    // Upright, and leaning about two axes at a place away from the
    // origin.
    let lean = [0.3, -0.5, 0.8];
    run(
        &mut s,
        "feature.block",
        json!({"name": "Upright", "size": [a, b, c], "position": [5, -7, 2]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Leaning", "size": [a, b, c], "position": [100, 40, -30], "axis": lean, "angle": "20 deg"}),
    );
    let measure = run(&mut s, "document.measure", json!({}));
    println!("{measure}");
    let centres: Vec<[f64; 3]> = measure["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| arr3(&b["centroid"]))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    for (feature, centre) in ["F1", "F2"].iter().zip(&centres) {
        for k in 0..6 {
            let face = format!("{feature}:face({k})");
            let file = format!("{feature}-{k}.svg");
            let result = run(&mut s, "file.export", json!({"path": file, "face": face}));
            let loops = outline(&dir, &file);
            assert_eq!(loops.len(), 1);
            let area = loops[0].area;
            // Each face is one of the block's three rectangles.
            let expected = [a * b, b * c, a * c]
                .into_iter()
                .find(|e| near(*e, area, 1e3 * e))
                .unwrap_or_else(|| panic!("{face}: {result}"));
            check_frame(&result, &loops, *centre, expected);
            let z = cross3(arr3(&result["xAxis"]), arr3(&result["yAxis"]));
            seen.insert((*feature, z.map(|v| (v * 1e6).round() as i64)));
        }
    }
    // Six directions each.
    assert_eq!(seen.len(), 12);
    // The bottom and the top of the upright block: x along x both, the
    // bottom seen from below.
    let bottom = run(
        &mut s,
        "file.export",
        json!({"path": "bottom.svg", "face": "F1:face(0)"}),
    );
    assert_eq!(arr3(&bottom["xAxis"]), [1.0, 0.0, 0.0]);
    assert_eq!(arr3(&bottom["yAxis"]), [0.0, -1.0, 0.0]);
    assert_eq!(numbers(&bottom["bbox"]), [5.0, 7.0 - b, 5.0 + a, 7.0]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn holes_run_clockwise_and_the_axes_and_origin_can_be_chosen() {
    let (mut s, dir) = session("axes");
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 20, 4], "position": [10, 10, 0]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": 3, "height": 10, "position": [20, 20, -2]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Holed"}),
    );
    // And a square hole, which is a chain.
    run(
        &mut s,
        "feature.block",
        json!({"name": "Punch", "size": [5, 5, 10], "position": [35, 15, -2]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Holed", "tool": "Punch", "name": "Punched"}),
    );
    let area = 40.0 * 20.0 - PI * 9.0 - 25.0;
    for (k, file) in [(0, "bottom.svg"), (1, "top.svg")] {
        let result = run(
            &mut s,
            "file.export",
            json!({"path": file, "face": format!("F1:face({k})")}),
        );
        let loops = outline(&dir, file);
        assert_eq!(loops.len(), 3, "{result}");
        let square = loops
            .iter()
            .find(|l| near(l.area.abs(), 25.0, 25.0))
            .unwrap_or_else(|| panic!("{loops:?}"));
        assert!(
            square.area < 0.0,
            "the square hole runs clockwise: {loops:?}"
        );
        let got = oriented_area(&loops);
        assert!(near(got, area, area), "{file}: {got} {area}");
    }
    // x along the model's y: the top turned a quarter, still seen from
    // above.
    let turned = run(
        &mut s,
        "file.export",
        json!({"path": "turned.svg", "face": "F1:face(1)", "xAxis": [0, 2, 0.5]}),
    );
    assert_eq!(arr3(&turned["xAxis"]), [0.0, 1.0, 0.0]);
    assert_eq!(arr3(&turned["yAxis"]), [-1.0, 0.0, 0.0]);
    assert_eq!(numbers(&turned["bbox"]), [10.0, -50.0, 30.0, -10.0]);
    assert!(near(
        oriented_area(&outline(&dir, "turned.svg")),
        area,
        area
    ));
    // The corner of the outline at the origin.
    let corner = run(
        &mut s,
        "file.export",
        json!({"path": "corner.svg", "face": "F1:face(1)", "origin": "corner"}),
    );
    assert_eq!(numbers(&corner["bbox"]), [0.0, 0.0, 40.0, 20.0]);
    assert_eq!(arr3(&corner["origin"]), [10.0, 10.0, 4.0]);
    let side = run(
        &mut s,
        "file.export",
        json!({"path": "side.svg", "face": "F1:face(2)", "origin": "corner"}),
    );
    let bbox = numbers(&side["bbox"]);
    assert_eq!((bbox[0], bbox[1]), (0.0, 0.0), "{side}");
    // What a frame cannot be.
    for (x_axis, word) in [
        (json!([0, 0, 1]), "along the face's normal"),
        (json!([0, 0, 0]), "no direction"),
        (json!([1, 0]), "three finite numbers"),
        (json!("x"), "three finite numbers"),
        (json!([1, "a", 0]), "three finite numbers"),
    ] {
        refused(
            &mut s,
            json!({"path": "bad.svg", "face": "F1:face(1)", "xAxis": x_axis}),
            &[word],
        );
    }
    // A direction is a direction, however long.
    let long = run(
        &mut s,
        "file.export",
        json!({"path": "long.svg", "face": "F1:face(1)", "xAxis": [1e308, -1e308, 0]}),
    );
    let x = arr3(&long["xAxis"]);
    assert!(near(x[0], 0.5_f64.sqrt(), 1.0) && near(x[1], -(0.5_f64.sqrt()), 1.0));
    refused(
        &mut s,
        json!({"path": "bad.svg", "face": "F1:face(1)", "origin": "middle"}),
        &["`origin`", "corner"],
    );
    refused(
        &mut s,
        json!({"path": "bad.svg", "face": "F1:face(1)", "origin": 3}),
        &["`origin`"],
    );
    refused(
        &mut s,
        json!({"path": "bad.svg", "sketch": "x", "xAxis": [1, 0, 0]}),
        &["`xAxis`", "sketch"],
    );
    refused(
        &mut s,
        json!({"path": "bad.stl", "xAxis": [1, 0, 0]}),
        &["`xAxis`"],
    );
    assert!(!dir.join("bad.svg").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

// ------------------------------------------------------------- the sheet

/// The laser box's panels: a bottom lying flat, two sides standing, all
/// notched, exported by their blanks' names.
fn laser_box(s: &mut Session) -> Vec<(&'static str, f64, f64)> {
    panel(s, "Bottom", 60.0, 40.0, [0.0, 0.0, 0.0]);
    panel(s, "Side", 40.0, 25.0, [0.0, 80.0, 0.0]);
    panel(s, "End", 30.0, 25.0, [0.0, 160.0, 0.0]);
    // Stand the side and the end up: a turn about x.
    run(
        s,
        "feature.transform",
        json!({"source": "Side", "angle": "90 deg", "axis": [1, 0, 0], "pivot": [0, 80, 0], "name": "SideUp"}),
    );
    run(
        s,
        "feature.transform",
        json!({"source": "End", "angle": "-90 deg", "axis": [0, 1, 0], "pivot": [0, 160, 0], "name": "EndUp"}),
    );
    vec![
        ("F1:face(1)", 60.0, 40.0),
        ("F4:face(1)", 40.0, 25.0),
        ("F7:face(1)", 30.0, 25.0),
    ]
}

fn boxes_apart(boxes: &[[f64; 4]], gap: f64) {
    for (i, a) in boxes.iter().enumerate() {
        for b in boxes.iter().skip(i + 1) {
            let apart = a[2] + gap <= b[0] + 1e-9
                || b[2] + gap <= a[0] + 1e-9
                || a[3] + gap <= b[1] + 1e-9
                || b[3] + gap <= a[1] + 1e-9;
            assert!(apart, "{a:?} and {b:?} are closer than {gap}");
        }
    }
}

#[test]
fn panels_named_by_their_blanks_are_laid_out_on_one_sheet() {
    let (mut s, dir) = session("sheet");
    let panels = laser_box(&mut s);
    let names: Vec<&str> = panels.iter().map(|p| p.0).collect();
    for format in ["svg", "dxf"] {
        let file = format!("sheet.{format}");
        let result = run(
            &mut s,
            "file.export",
            json!({"path": file, "faces": names, "gap": 4}),
        );
        println!("{result}");
        assert_eq!(result["rows"], 1);
        assert_eq!(result["closed"], 3);
        let sheet = result["sheet"].as_array().unwrap();
        let layers: Vec<&str> = sheet.iter().map(|e| e["layer"].as_str().unwrap()).collect();
        assert_eq!(layers, ["F1_face_1", "F4_face_1", "F7_face_1"]);
        let followed: Vec<&str> = sheet
            .iter()
            .map(|e| e["followed"].as_str().unwrap())
            .collect();
        assert_eq!(
            followed,
            [
                "F3:copy(0)/F1:face(1)",
                "F10:copy(0)/F4:face(1)",
                "F11:copy(0)/F7:face(1)"
            ]
        );
        let boxes: Vec<[f64; 4]> = sheet
            .iter()
            .map(|e| {
                let b = numbers(&e["bbox"]);
                [b[0], b[1], b[2], b[3]]
            })
            .collect();
        // Left to right from the origin, bottoms on one line, 4 apart.
        let mut x = 0.0;
        for ((b, (_, w, h)), entry) in boxes.iter().zip(&panels).zip(sheet) {
            let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
            let (w, h) = if near(bw, *w, 60.0) {
                (*w, *h)
            } else {
                (*h, *w)
            };
            assert!(near(b[0], x, 60.0) && near(b[1], 0.0, 1.0), "{entry}");
            assert!(near(bw, w, 60.0) && near(bh, h, 60.0), "{entry}");
            x += w + 4.0;
        }
        boxes_apart(&boxes, 4.0);
        let total = numbers(&result["bbox"]);
        assert!(near(total[2], x - 4.0, 200.0), "{total:?}");
        if format == "svg" {
            let read = svg_of(&dir, &file);
            assert_eq!(
                read.keys().collect::<Vec<_>>(),
                layers.iter().collect::<Vec<_>>()
            );
            for ((layer, (_, w, h)), b) in layers.iter().zip(&panels).zip(&boxes) {
                let loops = &read[*layer];
                let area = oriented_area(loops);
                assert!(near(area, w * h - 30.0, w * h), "{layer}: {area}");
                // The file has the outline where the result says.
                for (got, want) in loops[0].bbox.iter().zip(b) {
                    assert!((got - want).abs() < 1e-9, "{layer}: {loops:?} {b:?}");
                }
            }
        } else {
            let text = std::fs::read_to_string(dir.join(&file)).unwrap();
            let (on, table) = dxf_layers(&text);
            for layer in &layers {
                assert!(table.contains(&layer.to_string()), "{table:?}");
                // A notched rectangle is eight lines.
                assert_eq!(on.iter().filter(|l| l == layer).count(), 8, "{layer}");
            }
        }
        // The same sheet twice is the same file.
        let again = format!("again.{format}");
        run(
            &mut s,
            "file.export",
            json!({"path": again, "faces": names, "gap": 4}),
        );
        assert_eq!(
            std::fs::read(dir.join(&file)).unwrap(),
            std::fs::read(dir.join(&again)).unwrap()
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_sheet_of_limited_width_takes_rows_and_names_each_face_once() {
    let (mut s, dir) = session("rows");
    laser_box(&mut s);
    let faces = json!([
        "F1:face(1)",
        {"face": "F1:face(1)", "xAxis": [0, 1, 0]},
        {"face": "F4:face(1)", "feature": "SideUp", "layer": "side"},
        "F7:face(1)",
        "F7:face(1)",
        {"face": "F3:copy(0)/F1:face(0)", "layer": "F1_face_1-2"},
    ]);
    let result = run(
        &mut s,
        "file.export",
        json!({"path": "rows.svg", "faces": faces, "gap": 2.5, "width": 100}),
    );
    println!("{result}");
    let sheet = result["sheet"].as_array().unwrap();
    let layers: Vec<&str> = sheet.iter().map(|e| e["layer"].as_str().unwrap()).collect();
    // Given names are kept, and the others keep clear of them.
    assert_eq!(
        layers,
        [
            "F1_face_1",
            "F1_face_1-3",
            "side",
            "F7_face_1",
            "F7_face_1-2",
            "F1_face_1-2"
        ]
    );
    let boxes: Vec<[f64; 4]> = sheet
        .iter()
        .map(|e| {
            let b = numbers(&e["bbox"]);
            [b[0], b[1], b[2], b[3]]
        })
        .collect();
    boxes_apart(&boxes, 2.5);
    for b in &boxes {
        assert!(b[0] >= -1e-9 && b[2] <= 100.0 + 1e-9, "{b:?}");
    }
    // 60 then 40 (turned) fits in 100 only without the gap: two rows at
    // least, the second above the first.
    let rows = result["rows"].as_u64().unwrap();
    assert!(rows >= 3, "{result}");
    assert!(boxes[1][1] >= boxes[0][3] + 2.5 - 1e-9, "{boxes:?}");
    // The second is the first turned a quarter: 40 wide, 60 high.
    assert!(near(boxes[1][2] - boxes[1][0], 40.0, 60.0), "{boxes:?}");
    let read = svg_of(&dir, "rows.svg");
    assert_eq!(read.len(), 6);
    for layer in &layers {
        oriented_area(&read[*layer]);
    }
    // The bottom face, seen from below: the notch on the other side.
    let below = &read["F1_face_1-2"];
    assert!(near(oriented_area(below), 2370.0, 2370.0));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_sheet_that_cannot_be_drawn_is_refused_with_the_reason() {
    let (mut s, dir) = session("refused");
    laser_box(&mut s);
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Pin", "radius": 2, "height": 9, "position": [200, 0, 0]}),
    );
    let ok = || json!(["F1:face(1)", "F4:face(1)"]);
    let cases: Vec<(Value, Vec<&str>)> = vec![
        (json!({"faces": []}), vec!["`faces` is empty"]),
        (
            json!({"faces": "F1:face(1)"}),
            vec!["`faces` must be a list"],
        ),
        (json!({"faces": [3]}), vec!["`faces.0`", "face name"]),
        (
            json!({"faces": ["F1:face(1)", "nonsense"]}),
            vec!["`faces.1`"],
        ),
        (
            json!({"faces": ["F1:face(1)", "F1:side(e1)"]}),
            vec!["`faces.1`", "no face"],
        ),
        (json!({"faces": ["F1:startEdge(e1)"]}), vec!["not a face"]),
        (
            json!({"faces": ["F1:face(1)", "F12:face(0)"]}),
            vec!["`faces.1`", "cylinder face", "flat"],
        ),
        (
            json!({"faces": [{"feature": "Pin"}]}),
            vec!["has no `face`"],
        ),
        (
            json!({"faces": [{"face": "F1:face(1)", "colour": 1}]}),
            vec!["unknown parameter `colour`"],
        ),
        (
            json!({"faces": [{"face": "F1:face(1)", "feature": "Nope"}]}),
            vec!["Nope"],
        ),
        (
            json!({"faces": [{"face": "F1:face(1)", "feature": "Pin"}]}),
            vec!["`Pin` has no face"],
        ),
        (
            json!({"faces": [{"face": "F1:face(1)", "layer": "a b"}]}),
            vec!["layer name"],
        ),
        (
            json!({"faces": [{"face": "F1:face(1)", "layer": 7}]}),
            vec!["`layer`"],
        ),
        (
            json!({"faces": [{"face": "F1:face(1)", "layer": "x"}, {"face": "F4:face(1)", "layer": "X"}]}),
            vec!["layer name"],
        ),
        (json!({"faces": ok(), "gap": -1}), vec!["`gap`", "negative"]),
        (json!({"faces": ok(), "gap": "wide"}), vec!["`gap`"]),
        (
            json!({"faces": ok(), "gap": 1e308}),
            vec!["tolerance", "smaller `gap`"],
        ),
        (
            json!({"faces": ok(), "gap": 1e12}),
            vec!["tolerance", "smaller `gap`"],
        ),
        (
            json!({"faces": ok(), "width": 0}),
            vec!["`width`", "positive"],
        ),
        (
            json!({"faces": ok(), "width": 30}),
            vec!["wide", "width 30"],
        ),
        (
            json!({"faces": ok(), "feature": "Bottom"}),
            vec!["`feature`", "one `face`"],
        ),
        (json!({"faces": ok(), "origin": "corner"}), vec!["`origin`"]),
        (
            json!({"faces": ok(), "face": "F1:face(1)"}),
            vec!["not `face` and `faces`"],
        ),
        (
            json!({"faces": ok(), "construction": true}),
            vec!["`construction`"],
        ),
        (
            json!({"face": "F1:face(1)", "gap": 3}),
            vec!["`gap`", "sheet"],
        ),
        (
            json!({"faces": ok(), "xAxis": [0, 0, 0]}),
            vec!["no direction"],
        ),
        (
            json!({"faces": ok(), "xAxis": [0, 0, 1]}),
            vec!["`faces.0`", "along the face's normal"],
        ),
    ];
    for (mut params, words) in cases {
        params["path"] = json!("bad.svg");
        refused(&mut s, params.clone(), &words);
        params["path"] = json!("bad.dxf");
        refused(&mut s, params, &words);
    }
    let many: Vec<&str> = vec!["F1:face(1)"; 10_001];
    refused(
        &mut s,
        json!({"path": "bad.dxf", "faces": many}),
        &["at most 10000"],
    );
    refused(
        &mut s,
        json!({"path": "bad.step", "faces": ok()}),
        &["`faces`"],
    );
    assert!(!dir.join("bad.svg").exists() && !dir.join("bad.dxf").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_face_tilted_just_past_the_angular_tolerance_is_drawn_by_the_slope_rule() {
    // The document's angular tolerance is 1e-9 rad by default: a block
    // whose axis leans by half of it lies flat, one leaning by ten times
    // it has a slope, and xAxis pins either.
    let (mut s, dir) = session("threshold");
    for (name, tilt) in [("Flat", 0.5e-9), ("Sloped", 1e-8)] {
        run(
            &mut s,
            "feature.block",
            json!({"name": name, "size": [30, 20, 3], "axis": [0, -tilt, 1]}),
        );
    }
    let flat = run(
        &mut s,
        "file.export",
        json!({"path": "f.svg", "face": "F1:face(1)"}),
    );
    let sloped = run(
        &mut s,
        "file.export",
        json!({"path": "s.svg", "face": "F2:face(1)"}),
    );
    println!("{flat}\n{sloped}");
    let (fx, sx) = (arr3(&flat["xAxis"]), arr3(&sloped["xAxis"]));
    assert!(near(fx[0], 1.0, 1.0), "flat: x along x: {flat}");
    // The sloped one: y runs up its slope, along ±y, and x is level
    // across it, not the model's x.
    let sy = arr3(&sloped["yAxis"]);
    assert!(
        near(sy[1].abs(), 1.0, 1.0) && near(sx[1], 0.0, 1.0),
        "sloped: y up the slope: {sloped}"
    );
    for (file, face) in [("fp.svg", "F1:face(1)"), ("sp.svg", "F2:face(1)")] {
        let pinned = run(
            &mut s,
            "file.export",
            json!({"path": file, "face": face, "xAxis": [1, 0, 0]}),
        );
        let x = arr3(&pinned["xAxis"]);
        assert!(near(x[0], 1.0, 1.0) && near(x[1], 0.0, 1.0), "{pinned}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
