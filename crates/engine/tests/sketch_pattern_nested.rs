//! Nested sketch patterns (`sketch.pattern` with `patterns`): a pattern
//! whose seed holds whole patterns, a tree at most three levels deep. A
//! rib's top edge is a chained run of cells, each a run of tabs and a
//! spacer; a plate holds a grid of bolt circles; a ring of rows of slots.
//! Each is measured against its closed form at three scales while the
//! counts of both levels change; names hold an index per level, the
//! innermost first (`e3[2][1]`, `e3[2]` in cell 0), so wrapping a pattern
//! renames nothing; faces keep their names and go exactly where their
//! names say; skipped instances are left out and named by origin.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::f64::consts::PI;

fn session() -> Session {
    Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    })
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    let result = session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

fn error(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should fail, returned {value}"),
        Err(e) => {
            assert!(!e.message().contains("internal error"), "{id}: {e}");
            e.to_string()
        }
    }
}

fn str_of(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("not a string: {value}"))
        .to_string()
}

fn strings(value: &Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(str_of).collect()
}

fn document(session: &mut Session) -> Value {
    run(session, "document.inspect", json!({}))
}

fn body(session: &mut Session, name: &str) -> Value {
    let document = document(session);
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name} in {document}"))
        .clone()
}

fn status(session: &mut Session, name: &str) -> String {
    let document = document(session);
    let feature = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
        .clone();
    match (&feature["status"], &feature["error"]) {
        (Value::String(s), Value::String(e)) => format!("{s}: {e}"),
        (Value::String(s), _) => s.clone(),
        (other, _) => other.to_string(),
    }
}

fn close(got: f64, want: f64, rel: f64, what: &str) {
    let scale = want.abs().max(1e-300);
    assert!(
        ((got - want) / scale).abs() <= rel,
        "{what}: {got} instead of {want}"
    );
}

fn volume(session: &mut Session, name: &str) -> f64 {
    body(session, name)["volume"].as_f64().unwrap()
}

/// The box of every face of a feature's body, from the end points of its
/// edges: [x0, x1, y0, y1, z0, z1].
fn face_boxes(session: &mut Session, feature: &str) -> BTreeMap<String, [f64; 6]> {
    let edges = run(session, "document.edges", json!({"feature": feature}));
    let mut out: BTreeMap<String, [f64; 6]> = BTreeMap::new();
    for edge in edges["edges"].as_array().unwrap() {
        for face in strings(&edge["faces"]) {
            let b = out.entry(face).or_insert([
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ]);
            for end in ["start", "end"] {
                let p = edge[end].as_array().unwrap();
                for axis in 0..3 {
                    let v = p[axis].as_f64().unwrap();
                    b[2 * axis] = b[2 * axis].min(v);
                    b[2 * axis + 1] = b[2 * axis + 1].max(v);
                }
            }
        }
    }
    out
}

fn same_box(got: &[f64; 6], want: &[f64; 6], size: f64, what: &str) {
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g - w).abs() <= 1e-9 * size,
            "{what}: {got:?} instead of {want:?}"
        );
    }
}

/// The entities of the rib edge, as the commands made them.
struct Rib {
    /// The tab's flank up, its top, its flank down, the flat after it.
    tab: [String; 4],
    spacer: String,
    closing: [String; 3],
    /// The run's start, the end of one tab period, the end of a cell.
    a: String,
    b: String,
    cell_end: String,
}

/// A rib in sketch `Edge`, extruded `t` as `Rib`: the top edge is `n`
/// cells (variable `n`), each `k` tabs (variable `k`) of width `w` and
/// height `c` on a period `p`, then a spacer `g`; the body below is `h`
/// deep. All in the units given; the cell is a chained run of the tab run
/// (`p1`) and the spacer (`p2`), and the closing outline starts on the
/// end of the last cell. Area `L h + n k w c` with `L = n (k p + g)`.
fn rib(s: &mut Session, [w, c, p, g, h, t]: [f64; 6], k: u32, n: u32) -> Rib {
    run(
        s,
        "variable.set",
        json!({"name": "k", "expr": k.to_string()}),
    );
    run(
        s,
        "variable.set",
        json!({"name": "n", "expr": n.to_string()}),
    );
    run(s, "sketch.create", json!({"name": "Edge"}));
    let made = run(
        s,
        "sketch.polyline",
        json!({"sketch": "Edge", "points": [[0, 0], [0, c], [w, c], [w, 0], [p, 0]]}),
    );
    let points = strings(&made["points"]);
    let lines = strings(&made["lines"]);
    let (a, b) = (points[0].clone(), points[4].clone());
    let constrain = |s: &mut Session, kind: &str, entities: Value, value: Option<f64>| {
        let mut params = json!({"sketch": "Edge", "type": kind, "entities": entities});
        if let Some(value) = value {
            params["value"] = json!(value);
        }
        run(s, "sketch.constrain", params)
    };
    constrain(s, "fixed", json!([a]), None);
    constrain(s, "vertical", json!([lines[0]]), None);
    constrain(s, "distance", json!([lines[0]]), Some(c));
    constrain(s, "horizontal", json!([lines[1]]), None);
    constrain(s, "distance", json!([lines[1]]), Some(w));
    constrain(s, "vertical", json!([lines[2]]), None);
    constrain(s, "horizontal", json!([lines[3]]), None);
    constrain(s, "distance", json!([lines[3]]), Some(p - w));
    constrain(s, "horizontal", json!([a, b]), None);
    let inner = run(
        s,
        "sketch.pattern",
        json!({"sketch": "Edge", "entities": lines, "count": "k", "chain": [a, b]}),
    );
    assert_eq!(str_of(&inner["pattern"]), "p1");
    let kp = f64::from(k) * p;
    let made = run(
        s,
        "sketch.polyline",
        json!({"sketch": "Edge", "points": [format!("{b}[last]"), [kp + g, 0]]}),
    );
    let spacer = strings(&made["lines"])[0].clone();
    let cell_end = strings(&made["points"])[1].clone();
    constrain(s, "horizontal", json!([spacer]), None);
    constrain(s, "distance", json!([spacer]), Some(g));
    let outer = run(
        s,
        "sketch.pattern",
        json!({"sketch": "Edge", "entities": [spacer], "patterns": ["p1"], "count": "n", "chain": [a, cell_end]}),
    );
    assert_eq!(str_of(&outer["pattern"]), "p2", "{outer}");
    let length = f64::from(n) * (kp + g);
    let made = run(
        s,
        "sketch.polyline",
        json!({"sketch": "Edge", "points": [format!("{cell_end}[last]"), [length, -h], [0, -h], a]}),
    );
    let closing = strings(&made["lines"]);
    constrain(s, "vertical", json!([closing[0]]), None);
    constrain(s, "horizontal", json!([closing[1]]), None);
    constrain(s, "vertical", json!([closing[2]]), None);
    let last = constrain(s, "distance", json!([closing[2]]), Some(h));
    assert_eq!(last["status"], "ok", "{last}");
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Edge", "distance": t, "name": "Rib"}),
    );
    Rib {
        tab: [
            lines[0].clone(),
            lines[1].clone(),
            lines[2].clone(),
            lines[3].clone(),
        ],
        spacer,
        closing: [closing[0].clone(), closing[1].clone(), closing[2].clone()],
        a,
        b,
        cell_end,
    }
}

fn set(s: &mut Session, name: &str, value: u32) {
    run(
        s,
        "variable.set",
        json!({"name": name, "expr": value.to_string()}),
    );
}

/// The name of instance `k` of seed curve `e` in cell `i`, as names write
/// it: trailing instance-0 levels left out.
fn instance(e: &str, k: u32, i: u32) -> String {
    match (k, i) {
        (0, 0) => e.to_string(),
        (k, 0) => format!("{e}[{k}]"),
        (k, i) => format!("{e}[{k}][{i}]"),
    }
}

#[test]
fn a_rib_edge_of_cells_of_tabs_is_its_closed_form_and_names_follow_both_counts() {
    for scale in [1e-3, 1.0, 1e6] {
        let [w, c, p, g, h, t] = [1.0, 0.25, 3.0, 2.0, 6.0, 0.5].map(|v| v * scale);
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        let r = rib(&mut s, [w, c, p, g, h, t], 2, 5);
        let mut cap: Option<String> = None;
        let mut first: Option<BTreeMap<String, [f64; 6]>> = None;
        for (k, n) in [(2_u32, 5_u32), (3, 5), (1, 5), (2, 5), (2, 7), (2, 5)] {
            set(&mut s, "k", k);
            set(&mut s, "n", n);
            let what = format!("k {k} n {n} at {scale}");
            assert_eq!(status(&mut s, "Edge"), "ok", "{what}");
            assert_eq!(status(&mut s, "Rib"), "ok", "{what}");
            let cell = f64::from(k) * p + g;
            let length = f64::from(n) * cell;
            let area = length * h + f64::from(n * k) * w * c;
            close(volume(&mut s, "Rib"), area * t, 1e-9, &what);
            // Every tab top is where its name says: tab j of cell i at
            // x = i (k p + g) + j p, from y = c, the whole depth t.
            let boxes = face_boxes(&mut s, "Rib");
            assert_eq!(boxes.len(), (n * (4 * k + 1) + 5) as usize, "{what}");
            for i in 0..n + 1 {
                for j in 0..k + 1 {
                    let name = format!("F2:side({})", instance(&r.tab[1], j, i));
                    if i < n && j < k {
                        let x = f64::from(i) * cell + f64::from(j) * p;
                        let got = boxes
                            .get(&name)
                            .unwrap_or_else(|| panic!("{what}: no {name}"));
                        same_box(got, &[x, x + w, c, c, 0.0, t], scale, &name);
                    } else {
                        assert!(!boxes.contains_key(&name), "{what}: {name} is there");
                    }
                }
                let spacer = format!("F2:side({})", instance(&r.spacer, i, 0));
                if i < n {
                    let x = f64::from(i) * cell + f64::from(k) * p;
                    same_box(
                        &boxes[&spacer],
                        &[x, x + g, 0.0, 0.0, 0.0, t],
                        scale,
                        &spacer,
                    );
                } else {
                    assert!(!boxes.contains_key(&spacer), "{what}: {spacer}");
                }
            }
            // The caps name every run once, whatever the counts.
            let caps: Vec<&String> = boxes.keys().filter(|n| n.contains("endCap")).collect();
            assert_eq!(caps.len(), 1, "{what}: {caps:?}");
            for curve in r.tab.iter().chain([&r.spacer]) {
                assert!(caps[0].contains(&format!("{curve}[*]")), "{caps:?}");
            }
            match &cap {
                None => cap = Some(caps[0].clone()),
                Some(cap) => assert_eq!(cap, caps[0], "{what}: the cap keeps its name"),
            }
            if (k, n) == (2, 5) {
                match &first {
                    None => first = Some(boxes),
                    Some(first) => {
                        assert_eq!(
                            first.keys().collect::<Vec<_>>(),
                            boxes.keys().collect::<Vec<_>>()
                        );
                        for (name, b) in first {
                            same_box(&boxes[name], b, scale * 1e-3, name);
                        }
                    }
                }
            }
        }
        // The inspection shows the tree.
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Edge"}));
        let p2 = &inspected["patterns"][1];
        assert_eq!(p2["patterns"], json!(["p1"]), "{p2}");
        assert_eq!(p2["levels"], json!(["p2"]), "{p2}");
        let p1 = &inspected["patterns"][0];
        assert_eq!(p1["in"], "p2", "{p1}");
        assert_eq!(p1["levels"], json!(["p1", "p2"]), "{p1}");
        assert_eq!(p1["allInstances"], 10, "{p1}");
        assert_eq!(
            p1["curves"][1]["last"],
            format!("{}[1][4]", r.tab[1]),
            "{p1}"
        );
        let _ = (&r.closing, &r.a, &r.b, &r.cell_end);
    }
}

/// The tab run of [`rib`] alone, closed by a point `C` held on its end
/// (`e5[last]`): the outline before it is wrapped in a pattern of cells.
/// Returns the tab lines, the run's ends and C and the constraint that
/// holds C there.
fn tab_run(
    s: &mut Session,
    [w, c, p, h, t]: [f64; 5],
    k: u32,
) -> (Vec<String>, [String; 3], String) {
    run(
        s,
        "variable.set",
        json!({"name": "k", "expr": k.to_string()}),
    );
    run(s, "variable.set", json!({"name": "n", "expr": "1"}));
    run(s, "sketch.create", json!({"name": "Edge"}));
    let made = run(
        s,
        "sketch.polyline",
        json!({"sketch": "Edge", "points": [[0, 0], [0, c], [w, c], [w, 0], [p, 0]]}),
    );
    let points = strings(&made["points"]);
    let lines = strings(&made["lines"]);
    let (a, b) = (points[0].clone(), points[4].clone());
    let constrain = |s: &mut Session, kind: &str, entities: Value, value: Option<f64>| {
        let mut params = json!({"sketch": "Edge", "type": kind, "entities": entities});
        if let Some(value) = value {
            params["value"] = json!(value);
        }
        run(s, "sketch.constrain", params)
    };
    constrain(s, "fixed", json!([a]), None);
    constrain(s, "vertical", json!([lines[0]]), None);
    constrain(s, "distance", json!([lines[0]]), Some(c));
    constrain(s, "horizontal", json!([lines[1]]), None);
    constrain(s, "distance", json!([lines[1]]), Some(w));
    constrain(s, "vertical", json!([lines[2]]), None);
    constrain(s, "horizontal", json!([lines[3]]), None);
    constrain(s, "distance", json!([lines[3]]), Some(p - w));
    constrain(s, "horizontal", json!([a, b]), None);
    run(
        s,
        "sketch.pattern",
        json!({"sketch": "Edge", "entities": lines, "count": "k", "chain": [a, b]}),
    );
    let length = f64::from(k) * p;
    let made = run(
        s,
        "sketch.polyline",
        json!({"sketch": "Edge", "points": [[length, 0], [length, -h], [0, -h], a]}),
    );
    let corner = strings(&made["points"])[0].clone();
    let closing = strings(&made["lines"]);
    let held = constrain(s, "coincident", json!([corner, format!("{b}[last]")]), None);
    let held = str_of(&held["constraint"]);
    constrain(s, "vertical", json!([closing[0]]), None);
    constrain(s, "horizontal", json!([closing[1]]), None);
    constrain(s, "vertical", json!([closing[2]]), None);
    constrain(s, "distance", json!([closing[2]]), Some(h));
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Edge", "distance": t, "name": "Rib"}),
    );
    (lines, [a, b, corner], held)
}

/// Wrapping a run in a pattern of cells renames nothing; cells added later
/// get names of their own; unwrapping keeps the names of cell 0 and makes
/// a reference to another cell fail; wrapping again brings it back.
#[test]
fn wrapping_a_pattern_renames_nothing_and_unwrapping_keeps_cell_0() {
    for scale in [1e-3, 1.0, 1e6] {
        let [w, c, p, h, t] = [1.0, 0.25, 3.0, 6.0, 0.5].map(|v| v * scale);
        let k = 3;
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        let (lines, [a, b, corner], held) = tab_run(&mut s, [w, c, p, h, t], k);
        assert_eq!(status(&mut s, "Rib"), "ok");
        let before = face_boxes(&mut s, "Rib");
        let area = |n: u32| f64::from(n * k) * p * h + f64::from(n * k) * w * c;
        close(volume(&mut s, "Rib"), area(1) * t, 1e-9, "the run");
        // Wrapped, the period of a cell exactly the run: e5[last] is the
        // chain's end.
        let made = run(
            &mut s,
            "sketch.pattern",
            json!({"sketch": "Edge", "entities": [], "patterns": ["p1"], "count": "n", "chain": [a, format!("{b}[last]")]}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        let wrapped = face_boxes(&mut s, "Rib");
        assert_eq!(
            wrapped.keys().collect::<Vec<_>>(),
            before.keys().collect::<Vec<_>>(),
            "wrapping renames nothing"
        );
        for (name, b) in &before {
            same_box(&wrapped[name], b, scale * 1e-3, name);
        }
        // The corner on the end of the whole edge, three cells.
        run(
            &mut s,
            "sketch.delete",
            json!({"sketch": "Edge", "constraint": held}),
        );
        run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Edge", "type": "coincident", "entities": [corner, format!("{b}[last][last]")]}),
        );
        set(&mut s, "n", 3);
        assert_eq!(status(&mut s, "Rib"), "ok");
        close(volume(&mut s, "Rib"), area(3) * t, 1e-9, "three cells");
        let cells = face_boxes(&mut s, "Rib");
        for line in &lines {
            for j in 0..k {
                let name = format!("F2:side({})", instance(line, j, 0));
                same_box(&cells[&name], &before[&name], scale * 1e-3, &name);
                for i in 1..3 {
                    let name = format!("F2:side({})", instance(line, j, i));
                    assert!(cells.contains_key(&name), "{name}");
                }
            }
        }
        // A sketch on the top of tab 1 of cell 1.
        let top = format!("F2:side({})", instance(&lines[1], 1, 1));
        run(
            &mut s,
            "sketch.create",
            json!({"face": top, "name": "Mark"}),
        );
        assert_eq!(status(&mut s, "Mark"), "ok");
        // Unwrapped: cell 0 keeps its names, the mark fails naming its face.
        let removal = run(
            &mut s,
            "sketch.delete",
            json!({"sketch": "Edge", "pattern": "p2"}),
        );
        assert!(
            removal["imagesDeleted"]
                .as_array()
                .is_some_and(|i| !i.is_empty()),
            "{removal}"
        );
        run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Edge", "type": "coincident", "entities": [corner, format!("{b}[last]")]}),
        );
        assert_eq!(status(&mut s, "Rib"), "ok");
        let mark = status(&mut s, "Mark");
        assert!(mark.contains(&top) && mark.starts_with("failed"), "{mark}");
        let unwrapped = face_boxes(&mut s, "Rib");
        assert_eq!(
            unwrapped.keys().collect::<Vec<_>>(),
            before.keys().collect::<Vec<_>>()
        );
        for (name, b) in &before {
            same_box(&unwrapped[name], b, scale * 1e-3, name);
        }
    }
}

/// A fillet on the edge of a tab in a cell fails when the count no longer
/// has that tab, naming it, and is back when it does.
#[test]
fn a_fillet_on_a_tab_of_a_cell_fails_when_the_tab_is_gone_and_is_back() {
    let [w, c, p, g, h, t] = [1.0, 0.25, 3.0, 2.0, 6.0, 0.5];
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let r = rib(&mut s, [w, c, p, g, h, t], 3, 2);
    let flank = format!("F2:side({})", instance(&r.tab[0], 2, 1));
    let top = format!("F2:side({})", instance(&r.tab[1], 2, 1));
    run(
        &mut s,
        "feature.fillet",
        json!({"edges": [[flank, top]], "radius": 0.1, "name": "Round"}),
    );
    assert_eq!(status(&mut s, "Round"), "ok");
    let rounded = volume(&mut s, "Round");
    // The quarter of the square the ball takes away, along the depth.
    let area = 2.0 * (3.0 * p + g) * h + 6.0 * w * c;
    close(
        rounded,
        (area - (1.0 - PI / 4.0) * 0.01) * t,
        1e-9,
        "rounded",
    );
    set(&mut s, "k", 2);
    let failed = status(&mut s, "Round");
    assert!(
        failed.starts_with("failed") && failed.contains(&top),
        "{failed}"
    );
    set(&mut s, "k", 3);
    assert_eq!(status(&mut s, "Round"), "ok");
    close(volume(&mut s, "Round"), rounded, 1e-12, "back");
}

/// A plate with a grid of bolt circles: a ring of `m` holes, repeated
/// `nx` by `ny` times. Volume `(W D - nx ny m pi r^2) t`.
#[test]
fn a_grid_of_bolt_circles_is_its_closed_form() {
    for scale in [1e-3, 1.0, 1e6] {
        let [width, depth, r, ring, pitch_x, pitch_y, t] =
            [100.0, 60.0, 1.5, 6.0, 25.0, 20.0, 2.0].map(|v| v * scale);
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "variable.set",
            json!({"name": "holes", "expr": "6"}),
        );
        run(&mut s, "variable.set", json!({"name": "nx", "expr": "3"}));
        run(&mut s, "variable.set", json!({"name": "ny", "expr": "2"}));
        run(&mut s, "sketch.create", json!({"name": "Plate"}));
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Plate", "points": [[0, 0], [width, 0], [width, depth], [0, depth]], "closed": true}),
        );
        let (cx, cy) = (20.0 * scale, 15.0 * scale);
        let hole = run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Plate", "center": [cx + ring, cy], "radius": r}),
        );
        let hole = str_of(&hole["entity"]);
        run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Plate", "type": "radius", "entities": [hole], "value": r}),
        );
        run(
            &mut s,
            "sketch.pattern",
            json!({"sketch": "Plate", "entities": [hole], "count": "holes", "angle": "360 deg / holes", "center": [cx, cy]}),
        );
        let grid = run(
            &mut s,
            "sketch.pattern",
            json!({"sketch": "Plate", "entities": [], "patterns": ["p1"], "count": "nx", "step": [pitch_x, 0], "count2": "ny", "step2": [0, pitch_y]}),
        );
        assert_eq!(grid["status"], "ok", "{grid}");
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Plate", "distance": t, "name": "Body"}),
        );
        for (m, nx, ny) in [
            (6_u32, 3_u32, 2_u32),
            (8, 3, 2),
            (8, 2, 2),
            (4, 1, 1),
            (6, 3, 2),
        ] {
            set(&mut s, "holes", m);
            set(&mut s, "nx", nx);
            set(&mut s, "ny", ny);
            let what = format!("m {m} grid {nx} x {ny} at {scale}");
            assert_eq!(status(&mut s, "Body"), "ok", "{what}");
            let holes = f64::from(m * nx * ny);
            close(
                volume(&mut s, "Body"),
                (width * depth - holes * PI * r * r) * t,
                1e-9,
                &what,
            );
            // Hole 1 of the ring in cell (1, 1) where its name says.
            let boxes = face_boxes(&mut s, "Body");
            if nx > 1 && ny > 1 {
                let name = format!("F2:side({hole}[1][1,1])");
                let angle = 2.0 * PI / f64::from(m);
                let x = cx + pitch_x + ring * angle.cos();
                let y = cy + pitch_y + ring * angle.sin();
                let got = boxes
                    .get(&name)
                    .unwrap_or_else(|| panic!("{what}: no {name}"));
                // A circle's edges start and end on its seam: the box has
                // the seam point; it lies on the circle about (x, y).
                let (px, py) = (got[0], got[2]);
                close(((px - x).powi(2) + (py - y).powi(2)).sqrt(), r, 1e-9, &name);
            }
            let cells = boxes
                .keys()
                .filter(|n| n.starts_with(&format!("F2:side({hole}")))
                .count();
            assert_eq!(cells, (m * nx * ny) as usize, "{what}");
        }
    }
}

/// The shoelace area of a polygon.
fn shoelace(points: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum.abs() / 2.0
}

/// A disc with a ring of `n` rows of `q` rectangular slots: each slot's
/// corners turned by the ring's angle, their area by the shoelace formula.
#[test]
fn a_ring_of_rows_of_slots_is_its_shoelace_area() {
    for scale in [1e-3, 1.0, 1e6] {
        let [radius, start, len, wide, pitch, t] =
            [50.0, 12.0, 4.0, 1.5, 7.0, 1.0].map(|v| v * scale);
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(&mut s, "variable.set", json!({"name": "q", "expr": "4"}));
        run(&mut s, "variable.set", json!({"name": "n", "expr": "5"}));
        run(&mut s, "sketch.create", json!({"name": "Disc"}));
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Disc", "center": [0, 0], "radius": radius}),
        );
        let (x0, y0) = (start, -wide / 2.0);
        let slot = run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Disc", "points": [[x0, y0], [x0 + len, y0], [x0 + len, y0 + wide], [x0, y0 + wide]], "closed": true}),
        );
        let lines = strings(&slot["lines"]);
        run(
            &mut s,
            "sketch.pattern",
            json!({"sketch": "Disc", "entities": lines, "count": "q", "step": [pitch, 0]}),
        );
        run(
            &mut s,
            "sketch.pattern",
            json!({"sketch": "Disc", "patterns": ["p1"], "count": "n", "angle": "360 deg / n", "center": [0, 0]}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Disc", "distance": t, "name": "Body"}),
        );
        for (q, n) in [(4_u32, 5_u32), (2, 9), (5, 3), (4, 5)] {
            set(&mut s, "q", q);
            set(&mut s, "n", n);
            let what = format!("{n} rows of {q} at {scale}");
            assert_eq!(status(&mut s, "Body"), "ok", "{what}");
            let mut slots = 0.0;
            for j in 0..n {
                let (sin, cos) = (2.0 * PI * f64::from(j) / f64::from(n)).sin_cos();
                for i in 0..q {
                    let x = x0 + f64::from(i) * pitch;
                    let corners: Vec<[f64; 2]> =
                        [[x, y0], [x + len, y0], [x + len, y0 + wide], [x, y0 + wide]]
                            .iter()
                            .map(|[u, v]| [u * cos - v * sin, u * sin + v * cos])
                            .collect();
                    slots += shoelace(&corners);
                }
            }
            close(
                volume(&mut s, "Body"),
                (PI * radius * radius - slots) * t,
                1e-9,
                &what,
            );
        }
    }
}

/// The same rib edge drawn with one-level patterns, both tabs of a cell
/// in its seed by hand, is the nested rib at two tabs per cell: the same
/// volume, the same number of faces and the same faces.
#[test]
fn a_nested_rib_at_two_tabs_equals_the_rib_drawn_with_two_tabs_per_cell() {
    let [w, c, p, g, h, t] = [1.0, 0.25, 3.0, 2.0, 6.0, 0.5];
    let mut nested = session();
    run(&mut nested, "file.new", json!({"units": "mm"}));
    rib(&mut nested, [w, c, p, g, h, t], 2, 4);
    let mut flat = session();
    run(&mut flat, "file.new", json!({"units": "mm"}));
    run(&mut flat, "variable.set", json!({"name": "n", "expr": "4"}));
    run(&mut flat, "sketch.create", json!({"name": "Edge"}));
    let cell = 2.0 * p + g;
    let made = run(
        &mut flat,
        "sketch.polyline",
        json!({"sketch": "Edge", "points": [[0, 0], [0, c], [w, c], [w, 0], [p, 0], [p, c], [p + w, c], [p + w, 0], [2.0 * p, 0], [cell, 0]]}),
    );
    let points = strings(&made["points"]);
    let lines = strings(&made["lines"]);
    let (a, e) = (points[0].clone(), points[9].clone());
    let constrain = |s: &mut Session, kind: &str, entities: Value, value: Option<f64>| {
        let mut params = json!({"sketch": "Edge", "type": kind, "entities": entities});
        if let Some(value) = value {
            params["value"] = json!(value);
        }
        run(s, "sketch.constrain", params)
    };
    constrain(&mut flat, "fixed", json!([a]), None);
    for (line, vertical, length) in [
        (0, true, Some(c)),
        (1, false, Some(w)),
        (2, true, None),
        (3, false, Some(p - w)),
        (4, true, Some(c)),
        (5, false, Some(w)),
        (6, true, None),
        (7, false, Some(p - w)),
        (8, false, Some(g)),
    ] {
        constrain(
            &mut flat,
            if vertical { "vertical" } else { "horizontal" },
            json!([lines[line]]),
            None,
        );
        if let Some(length) = length {
            constrain(&mut flat, "distance", json!([lines[line]]), Some(length));
        }
    }
    constrain(&mut flat, "horizontal", json!([a, points[4]]), None);
    constrain(&mut flat, "horizontal", json!([a, points[8]]), None);
    run(
        &mut flat,
        "sketch.pattern",
        json!({"sketch": "Edge", "entities": lines, "count": "n", "chain": [a, e]}),
    );
    let made = run(
        &mut flat,
        "sketch.polyline",
        json!({"sketch": "Edge", "points": [format!("{e}[last]"), [4.0 * cell, -h], [0, -h], a]}),
    );
    let closing = strings(&made["lines"]);
    constrain(&mut flat, "vertical", json!([closing[0]]), None);
    constrain(&mut flat, "horizontal", json!([closing[1]]), None);
    constrain(&mut flat, "vertical", json!([closing[2]]), None);
    constrain(&mut flat, "distance", json!([closing[2]]), Some(h));
    run(
        &mut flat,
        "feature.extrude",
        json!({"sketch": "Edge", "distance": t, "name": "Rib"}),
    );
    for n in [4_u32, 7, 1, 4] {
        set(&mut nested, "n", n);
        set(&mut flat, "n", n);
        let (a, b) = (volume(&mut nested, "Rib"), volume(&mut flat, "Rib"));
        close(a, b, 1e-12, &format!("n {n}"));
        let boxes = |s: &mut Session| {
            let mut all: Vec<[i64; 6]> = face_boxes(s, "Rib")
                .values()
                .map(|b| b.map(|v| (v * 1e9).round() as i64))
                .collect();
            all.sort_unstable();
            all
        };
        assert_eq!(
            boxes(&mut nested),
            boxes(&mut flat),
            "n {n}: the same faces"
        );
    }
}

/// A row of holes with an instance skipped: the plate has that hole less,
/// the others keep their names and places; a skip past the count is kept
/// and inactive; `last` is the highest instance not skipped.
#[test]
fn a_skipped_instance_is_left_out_and_the_others_keep_their_names() {
    for scale in [1e-3, 1.0, 1e6] {
        let [width, depth, r, x0, pitch, t] = [60.0, 20.0, 2.0, 5.0, 8.0, 1.0].map(|v| v * scale);
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(&mut s, "variable.set", json!({"name": "n", "expr": "6"}));
        run(&mut s, "sketch.create", json!({"name": "Plate"}));
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Plate", "points": [[0, 0], [width, 0], [width, depth], [0, depth]], "closed": true}),
        );
        let hole = run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Plate", "center": [x0, depth / 2.0], "radius": r}),
        );
        let centre = str_of(&hole["center"]);
        let hole = str_of(&hole["entity"]);
        run(
            &mut s,
            "sketch.pattern",
            json!({"sketch": "Plate", "entities": [hole], "count": "n", "step": [pitch, 0], "skip": [3, "7"]}),
        );
        // An image of the last centre, by a construction line to it.
        let marker = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Plate", "from": format!("{centre}[last]"), "to": [0, 0], "construction": true}),
        );
        let image = str_of(&marker["start"]);
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Plate", "distance": t, "name": "Body"}),
        );
        // Instance 3 skipped, and 7 once there are eight; `last` the
        // highest instance there is (2 at a count of 4).
        for (n, holes, last) in [(6_u32, 5_u32, 5_u32), (8, 6, 6), (4, 3, 2), (5, 4, 4)] {
            set(&mut s, "n", n);
            let what = format!("n {n} at {scale}");
            assert_eq!(status(&mut s, "Body"), "ok", "{what}");
            close(
                volume(&mut s, "Body"),
                (width * depth - f64::from(holes) * PI * r * r) * t,
                1e-9,
                &what,
            );
            let boxes = face_boxes(&mut s, "Body");
            assert!(
                !boxes.contains_key(&format!("F2:side({hole}[3])")),
                "{what}"
            );
            for i in [1_u32, 2, 4] {
                if i < n {
                    let got = &boxes[&format!("F2:side({hole}[{i}])")];
                    let x = x0 + f64::from(i) * pitch;
                    close((got[0] - x).hypot(got[2] - depth / 2.0), r, 1e-9, &what);
                }
            }
            let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Plate"}));
            let pattern = &inspected["patterns"][0];
            assert_eq!(pattern["skip"], json!(["3", "7"]), "{pattern}");
            let inactive = if n > 7 { json!(null) } else { json!(["7"]) };
            assert_eq!(pattern["inactive"], inactive, "{what}: {pattern}");
            assert_eq!(pattern["instances"], holes, "{what}");
            let at = inspected["entities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == image.as_str())
                .unwrap()
                .clone();
            close(
                at["x"].as_f64().unwrap(),
                x0 + f64::from(last) * pitch,
                1e-12,
                &what,
            );
        }
    }
}

/// What nested patterns refuse, with a reason and without a crash.
#[test]
fn nested_patterns_refuse_what_they_cannot_be() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let r = rib(&mut s, [1.0, 0.25, 3.0, 2.0, 6.0, 0.5], 2, 3);
    let pattern = |s: &mut Session, params: Value| {
        let mut params = params;
        params["sketch"] = json!("Edge");
        error(s, "sketch.pattern", params)
    };
    // Itself, unknown, a cycle, in a second seed, four levels.
    for (params, want) in [
        (json!({"replace": "p2", "patterns": ["p1", "p2"]}), "itself"),
        (
            json!({"replace": "p2", "patterns": ["p9"]}),
            "no pattern p9",
        ),
        (json!({"replace": "p1", "patterns": ["p2"]}), "cycle"),
        (
            json!({"entities": [r.closing[1]], "patterns": ["p1"], "count": "2", "step": [0, -20]}),
            "in the seed of p2",
        ),
        (
            json!({"patterns": "p1", "count": "2", "step": [1, 0]}),
            "array of pattern ids",
        ),
        (
            json!({"patterns": [7.5], "count": "2", "step": [1, 0]}),
            "patterns",
        ),
    ] {
        let got = pattern(&mut s, params.clone());
        assert!(got.contains(want), "{params}: {got}");
    }
    run(
        &mut s,
        "sketch.pattern",
        json!({"sketch": "Edge", "patterns": ["p2"], "count": "2", "step": [0, -20]}),
    );
    let deep = pattern(
        &mut s,
        json!({"patterns": ["p3"], "count": "2", "step": [0, -50]}),
    );
    assert!(deep.contains("at most 3 levels"), "{deep}");
    // Malformed image paths in a point slot.
    for path in [
        format!("{}[last][last][last][last]", r.b),
        format!("{}[2][", r.b),
        format!("{}[][1]", r.b),
        format!("{}[1,2,3][0]", r.b),
        format!("{}[last][last][last]", r.closing[0]),
    ] {
        let got = error(
            &mut s,
            "sketch.line",
            json!({"sketch": "Edge", "from": path, "to": [50, 50]}),
        );
        assert!(!got.is_empty(), "{path}");
    }
    // Hostile skips.
    let many: Vec<u32> = (1..=20_000).collect();
    for skip in [
        json!([0]),
        json!([-1]),
        json!(["NaN"]),
        json!([1e12]),
        json!(["2,"]),
        json!(["last"]),
        json!([1.5]),
        json!([[1, 2, 3]]),
        json!(many),
        json!("3"),
    ] {
        let got = pattern(&mut s, json!({"replace": "p3", "skip": skip.clone()}));
        assert!(!got.is_empty(), "{skip}");
    }
    // Skips in a chained run are not supported yet.
    let got = pattern(&mut s, json!({"replace": "p1", "skip": [1]}));
    assert!(got.contains("not supported yet"), "{got}");
    // A product of counts of 10 000 by 10 000.
    run(
        &mut s,
        "variable.set",
        json!({"name": "k", "expr": "10000"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "n", "expr": "10000"}),
    );
    let edge = status(&mut s, "Edge");
    assert!(edge.contains("more than 100000 curves"), "{edge}");
    run(&mut s, "variable.set", json!({"name": "k", "expr": "2"}));
    run(&mut s, "variable.set", json!({"name": "n", "expr": "3"}));
    assert_eq!(status(&mut s, "Edge"), "ok");
}
