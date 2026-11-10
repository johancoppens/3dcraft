//! Inset flanges, flanges with an outline of their own, on a flange's side
//! and on a hole's edge, through the commands
//! (docs/design/sheetmetal.md › *Inset flanges, outlines, sides and
//! holes*): a panel with an inset wall, a tab on the wall's side, an arch
//! drawn in a sketch of its own and a tab bent down from a hole's edge,
//! held to its volume and flat area in closed form at three scales, its
//! bend table, saved and reopened to the same body and names, its flat
//! pattern exported; names that stay when another flange is added; bad
//! parameters as errors and bad flanges failing the part with the reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::assert_close;
use common::checked::{body, built, error, failed, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn lines_of(made: &Value) -> Vec<String> {
    made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect()
}

/// The panel's sketches: a 60 × 40 base with a 20 × 15 hole (its lines
/// bottom, right, top, left; the hole's after), and the arch, a 20 × 12
/// rectangle on the base's top edge from x = 20 with a round hole of
/// radius 2.
fn sketches(s: &mut Session, k: f64) -> (Vec<String>, Vec<String>) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let base = lines_of(&run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [60.0 * k, 40.0 * k]}),
    ));
    let hole = lines_of(&run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [20.0 * k, 10.0 * k], "to": [40.0 * k, 25.0 * k]}),
    ));
    run(s, "sketch.create", json!({"plane": "xy", "name": "Arch"}));
    run(
        s,
        "sketch.rect",
        json!({"sketch": "Arch", "from": [20.0 * k, 40.0 * k], "to": [40.0 * k, 52.0 * k]}),
    );
    run(
        s,
        "sketch.circle",
        json!({"sketch": "Arch", "center": [30.0 * k, 46.0 * k], "radius": 2.0 * k}),
    );
    (base, hole)
}

fn mm(v: f64) -> String {
    format!("{v} mm")
}

/// The flanges: 0 the wall on the bottom edge, 5 in at both ends, 10
/// long; 1 a tab 6 long on the wall's end side; 2 the arch; 3 a tab 8
/// long bent down from the hole's bottom edge, 2 in at both ends.
fn flanges(k: f64, base: &[String], hole: &[String]) -> Vec<Value> {
    vec![
        json!({"edge": base[0], "length": mm(10.0 * k), "angle": "90 deg",
               "insetStart": mm(5.0 * k), "insetEnd": mm(5.0 * k)}),
        json!({"side": {"flange": 0, "end": "end"}, "length": mm(6.0 * k), "angle": "90 deg"}),
        json!({"edge": base[2], "outline": "Arch", "angle": "90 deg"}),
        json!({"edge": hole[0], "length": mm(8.0 * k), "angle": "90 deg", "direction": "down",
               "insetStart": mm(2.0 * k), "insetEnd": mm(2.0 * k)}),
    ]
}

fn panel(s: &mut Session, k: f64, flanges: Vec<Value>) -> Value {
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Panel", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5, "flanges": flanges}),
    )
}

fn names(s: &mut Session) -> Vec<String> {
    body(s, "Panel")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_panel_of_every_kind_of_flange_by_its_closed_forms() {
    for k in SCALES {
        let dir = workdir("sheetmetal-flanges", &format!("panel-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (base, hole) = sketches(&mut s, k);
        panel(&mut s, k, flanges(k, &base, &hole));
        let (t, r) = (k, k);
        let ba = FRAC_PI_2 * (r + 0.5 * t);
        // Bent widths: the wall 50, its side 10 (the wall's length), the
        // arch 20, the hole's tab 16.
        let widths = [50.0 * k, 10.0 * k, 20.0 * k, 16.0 * k];
        let arch = (20.0 * 12.0 - PI * 4.0) * k * k;
        let flats = 60.0 * 40.0 * k * k - 20.0 * 15.0 * k * k
            + 50.0 * 10.0 * k * k
            + 10.0 * 6.0 * k * k
            + arch
            + 16.0 * 8.0 * k * k;
        let strips: f64 = widths.iter().map(|w| w * ba).sum();
        let rings: f64 = widths
            .iter()
            .map(|w| FRAC_PI_2 * w * ((r + t).ppowi(2) - r * r) / 2.0)
            .sum();
        // At K = 1/2 the volume is the flat area times T.
        let made = body(&mut s, "Panel");
        assert_close(&made["volume"], t * flats + rings, "volume");
        assert_close(&made["volume"], t * (flats + strips), "volume = A T");
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Panel"}));
        assert_close(&table["flatArea"], flats + strips, "flat area");
        let on: Vec<&str> = table["bends"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["on"].as_str().unwrap())
            .collect();
        assert_eq!(on[1], "end side of flange 0", "{table}");
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "panel.svg", "sheetMetal": "Panel"}),
        );
        assert_close(&out["flatArea"], flats + strips, "exported flat area");
        run(&mut s, "file.save", json!({"path": "panel.3dcraft"}));
        let mut u = session_in(&dir);
        run(&mut u, "file.open", json!({"path": "panel.3dcraft"}));
        let again = body(&mut u, "Panel");
        assert_eq!(made["volume"], again["volume"]);
        assert_eq!(made["faceNames"], again["faceNames"]);
    }
}

/// Rule 4: a flange added later changes no name the part had, but the
/// rim along the edge it is bent from: adding the tab on the wall's side
/// takes the side's rim (that edge is a bend's tangent line now, no end of
/// the part) and nothing else; adding the arch or the hole's tab, inset on
/// its edge, takes that edge's rim (its pieces are named by the bends at
/// their ends) and nothing else.
#[test]
fn names_stay_when_another_flange_is_added() {
    let k = 1.0;
    let dir = workdir("sheetmetal-flanges", "names");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    let (base, hole) = sketches(&mut s, k);
    let all = flanges(k, &base, &hole);
    panel(&mut s, k, vec![all[0].clone()]);
    let mut before = names(&mut s);
    for upto in [2, 3, 4] {
        let list: Vec<Value> = all.iter().take(upto).cloned().collect();
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": list}),
        );
        let after = names(&mut s);
        let lost: Vec<&String> = before.iter().filter(|n| !after.contains(n)).collect();
        assert!(
            lost.len() <= 1 && lost.iter().all(|n| n.contains("rim(")),
            "adding flange {}: lost {lost:?}",
            upto - 1
        );
        before = after;
    }
}

/// Where each rim of the panel lies: its edges' total length and their
/// length-weighted middle, by the rim's name.
fn rims(s: &mut Session) -> std::collections::BTreeMap<String, [f64; 4]> {
    let edges = run(s, "document.edges", json!({"feature": "Panel"}));
    let mut out = std::collections::BTreeMap::new();
    for e in edges["edges"].as_array().unwrap() {
        let len = e["length"].as_f64().unwrap();
        let mid: Vec<f64> = (0..3)
            .map(|i| 0.5 * (e["start"][i].as_f64().unwrap() + e["end"][i].as_f64().unwrap()))
            .collect();
        for f in e["faces"].as_array().unwrap() {
            let f = f.as_str().unwrap();
            if !f.contains("rim(") {
                continue;
            }
            let a: &mut [f64; 4] = out.entry(f.to_string()).or_insert([0.0; 4]);
            a[0] += len;
            for i in 0..3 {
                a[i + 1] += len * mid[i];
            }
        }
    }
    for a in out.values_mut() {
        for i in 1..4 {
            a[i] /= a[0];
        }
    }
    out
}

/// Every rim name in both lists lies in the same place in both.
fn same_places(
    before: &std::collections::BTreeMap<String, [f64; 4]>,
    after: &std::collections::BTreeMap<String, [f64; 4]>,
    k: f64,
    what: &str,
) {
    for (name, a) in before {
        if let Some(b) = after.get(name) {
            let off = a
                .iter()
                .zip(b)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f64::max);
            assert!(
                off <= 1e-6 * k,
                "{what} at scale {k}: {name} moved from {a:?} to {b:?}"
            );
        }
    }
}

/// Rule 4 on one edge: a second inset flange added on the edge the arch
/// sits on, ahead of the arch along it, takes only the rim of the piece
/// it splits; every rim name that survives (the piece from the edge's
/// start, the piece past the arch, every other rim) lies where it lay;
/// deleting the flange again gives back every name in its place.
#[test]
fn names_stay_in_place_when_a_flange_is_added_on_the_same_edge() {
    for k in SCALES {
        let dir = workdir("sheetmetal-flanges", &format!("same-edge-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (base, hole) = sketches(&mut s, k);
        let mut all = flanges(k, &base, &hole);
        panel(&mut s, k, all.clone());
        let first = rims(&mut s);
        // The top edge runs from x = 60 to x = 0; the arch is at 40..20,
        // the new tab at 55..45, ahead of it.
        all.push(
            json!({"edge": base[2], "length": mm(10.0 * k), "angle": "90 deg",
                        "insetStart": mm(5.0 * k), "insetEnd": mm(45.0 * k)}),
        );
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": all}),
        );
        let added = rims(&mut s);
        let lost: Vec<&String> = first.keys().filter(|n| !added.contains_key(*n)).collect();
        assert_eq!(lost.len(), 1, "adding the tab at scale {k}: lost {lost:?}");
        same_places(&first, &added, k, "adding the tab");
        all.pop();
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": all}),
        );
        let deleted = rims(&mut s);
        assert_eq!(
            first.keys().collect::<Vec<_>>(),
            deleted.keys().collect::<Vec<_>>(),
            "deleting the tab at scale {k}"
        );
        same_places(&first, &deleted, k, "deleting the tab");
        same_places(&added, &deleted, k, "deleting the tab");
    }
}

/// The rims along the base's bottom edge (y = 0, between its ends, in
/// the sheet): the pieces of that edge between its tabs.
fn bottom_rims(s: &mut Session, k: f64) -> std::collections::BTreeMap<String, [f64; 4]> {
    rims(s)
        .into_iter()
        .filter(|(_, a)| {
            a[2].abs() <= 1e-9 * k && a[1] > 0.0 && a[1] < 60.0 * k && a[3] > 0.0 && a[3] < k
        })
        .collect()
}

/// Rule 4 across the list (review 2026-10-28: the pieces of a split edge
/// were named by the flanges' places in the list, so deleting an earlier
/// flange gave the name of the gap between the first two tabs to the gap
/// between the next two): three tabs on the bottom edge behind a wall on
/// the left edge. Deleting the wall, putting a flange in front of the
/// list, or changing a tab with its id keeps every piece's name where it
/// was; changing a tab without its id makes it a new flange, whose
/// pieces' names are new (the old ones name nothing), never another
/// piece's.
#[test]
fn split_edge_names_follow_their_flanges_not_their_places_in_the_list() {
    for k in SCALES {
        let dir = workdir("sheetmetal-flanges", &format!("ids-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        let base = lines_of(&run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [0.0, 0.0], "to": [60.0 * k, 40.0 * k]}),
        ));
        let tab = |from: f64, to: f64| {
            json!({"edge": base[0], "length": mm(8.0 * k), "angle": "90 deg",
                   "insetStart": mm(from * k), "insetEnd": mm((60.0 - to) * k)})
        };
        let wall = json!({"edge": base[3], "length": mm(10.0 * k), "angle": "90 deg"});
        let tabs = [tab(5.0, 15.0), tab(25.0, 35.0), tab(45.0, 55.0)];
        let mut all = vec![wall];
        all.extend(tabs.iter().cloned());
        panel(&mut s, k, all);
        let ids: Vec<Value> = run(&mut s, "sheetMetal.bends", json!({"feature": "Panel"}))["bends"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["id"].clone())
            .collect();
        assert_eq!(ids, [json!(1), json!(2), json!(3), json!(4)], "scale {k}");
        let first = bottom_rims(&mut s, k);
        // Four pieces: before, between and after the three tabs.
        assert_eq!(first.len(), 4, "scale {k}: {first:?}");

        // The wall deleted: the tabs sent back as they were keep their ids.
        let edit = |s: &mut Session, flanges: Vec<Value>| {
            run(
                s,
                "sheetMetal.edit",
                json!({"feature": "Panel", "flanges": flanges}),
            );
            bottom_rims(s, k)
        };
        let deleted = edit(&mut s, tabs.to_vec());
        assert_eq!(
            first.keys().collect::<Vec<_>>(),
            deleted.keys().collect::<Vec<_>>(),
            "deleting the wall at scale {k}"
        );
        same_places(&first, &deleted, k, "deleting the wall");
        let ids: Vec<Value> = run(&mut s, "sheetMetal.bends", json!({"feature": "Panel"}))["bends"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["id"].clone())
            .collect();
        assert_eq!(ids, [json!(2), json!(3), json!(4)], "scale {k}");

        // A flange put in front of the list: a new id (5), never the
        // deleted wall's 1.
        let mut front = vec![json!({"edge": base[1], "length": mm(6.0 * k), "angle": "90 deg"})];
        front.extend(tabs.iter().cloned());
        let added = edit(&mut s, front);
        assert_eq!(
            first.keys().collect::<Vec<_>>(),
            added.keys().collect::<Vec<_>>(),
            "a flange in front at scale {k}"
        );
        same_places(&first, &added, k, "a flange in front");
        let bends = run(&mut s, "sheetMetal.bends", json!({"feature": "Panel"}));
        assert_eq!(bends["bends"][0]["id"], 5, "{bends}");

        // The middle tab moved along the edge, given with its id: its
        // pieces keep their names (and move with it).
        let mut moved = vec![tabs[0].clone(), tab(27.0, 37.0), tabs[2].clone()];
        moved[1]["id"] = json!(3);
        let shifted = edit(&mut s, moved.clone());
        assert_eq!(
            first.keys().collect::<Vec<_>>(),
            shifted.keys().collect::<Vec<_>>(),
            "the middle tab moved with its id at scale {k}"
        );
        // Without its id it is a new flange: the two pieces beside it get
        // new names, the old ones name nothing; the end pieces stay.
        moved[1] = tab(29.0, 39.0);
        let renamed = edit(&mut s, moved);
        let kept: Vec<&String> = first.keys().filter(|n| renamed.contains_key(*n)).collect();
        assert_eq!(kept.len(), 2, "scale {k}: kept {kept:?}");
        same_places(&first, &renamed, k, "the middle tab changed without its id");

        // An id no flange of the part has, or one given twice, is refused.
        let mut bad = tabs.to_vec();
        bad[0]["id"] = json!(1);
        let got = error(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": bad}),
        );
        assert!(got.contains("no flange of this part"), "{got}");
        let mut twice = tabs.to_vec();
        twice[0]["id"] = json!(2);
        twice[1]["id"] = json!(2);
        let got = error(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": twice}),
        );
        assert!(got.contains("another flange"), "{got}");
        for id in [json!(0), json!(-1), json!("2"), json!(1u64 << 33)] {
            let mut odd = tabs.to_vec();
            odd[0]["id"] = id.clone();
            let got = error(
                &mut s,
                "sheetMetal.edit",
                json!({"feature": "Panel", "flanges": odd}),
            );
            assert!(got.contains("flanges.0.id"), "{id}: {got}");
        }
    }
}

/// Per face of the panel, by its name, its edges' end points.
fn face_edges(s: &mut Session) -> std::collections::BTreeMap<String, Vec<[f64; 6]>> {
    let edges = run(s, "document.edges", json!({"feature": "Panel"}));
    let mut out: std::collections::BTreeMap<String, Vec<[f64; 6]>> = Default::default();
    for e in edges["edges"].as_array().unwrap() {
        let p = |end: &str, i: usize| e[end][i].as_f64().unwrap();
        let seg = [
            p("start", 0),
            p("start", 1),
            p("start", 2),
            p("end", 0),
            p("end", 1),
            p("end", 2),
        ];
        for f in e["faces"].as_array().unwrap() {
            out.entry(f.as_str().unwrap().to_string())
                .or_default()
                .push(seg);
        }
    }
    out
}

/// Every face name in both lies where it lay: it keeps at least one of
/// its edges exactly (a name given to another face, elsewhere on the
/// part, keeps none). How many names are lost.
fn kept_in_place(
    before: &std::collections::BTreeMap<String, Vec<[f64; 6]>>,
    after: &std::collections::BTreeMap<String, Vec<[f64; 6]>>,
    k: f64,
    what: &str,
) -> usize {
    let same = |a: &[f64; 6], b: &[f64; 6]| {
        let off = |x: &[f64], y: &[f64]| {
            x.iter()
                .zip(y)
                .map(|(p, q)| (p - q).abs())
                .fold(0.0, f64::max)
        };
        off(a, b) <= 1e-6 * k || off(&a[..3], &b[3..]).max(off(&a[3..], &b[..3])) <= 1e-6 * k
    };
    let mut lost = 0;
    for (name, edges) in before {
        let Some(now) = after.get(name) else {
            lost += 1;
            continue;
        };
        assert!(
            edges.iter().any(|a| now.iter().any(|b| same(a, b))),
            "{what} at scale {k}: {name} now names another face ({edges:?} before, {now:?} after)"
        );
    }
    lost
}

/// Rule 4 for every kind of name of a part (review 2026-10-29: a
/// flange's faces `face(2i + 1)`, `face(2i + 2)`, its tangent lines and
/// arcs, its rectangle's sides and tip, its outline's curves and holes,
/// and a relief's edges were numbered by the flanges' places in the list,
/// so deleting or inserting an earlier flange gave their names to other
/// flanges): a panel with a tab on the top edge first in the list, then a
/// wall on the bottom edge and one on the right (a relieved corner), a
/// flange on the right wall's tip, one on the bottom wall's start side,
/// the arch (an outline of its own with a hole) and a louver on the
/// hole's edge. Deleting the tab, or putting a new tab in front of the
/// list (each flange sent with its id, the tip and side by their new
/// places), leaves every name that is still there on the face it named;
/// the names lost are only those of the tab and of the top edge's pieces
/// it splits.
#[test]
fn every_name_stays_in_place_when_an_earlier_flange_is_deleted_or_inserted() {
    for k in SCALES {
        let dir = workdir("sheetmetal-flanges", &format!("rule4-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (base, hole) = sketches(&mut s, k);
        let tab = |from: f64, to: f64| {
            json!({"edge": base[2], "length": mm(8.0 * k), "angle": "90 deg",
                   "insetStart": mm((60.0 - from) * k), "insetEnd": mm(to * k)})
        };
        let rest = |tip: usize, side: usize| {
            vec![
                json!({"edge": base[0], "length": mm(10.0 * k), "angle": "90 deg"}),
                json!({"edge": base[1], "length": mm(10.0 * k), "angle": "90 deg"}),
                json!({"tip": tip, "length": mm(4.0 * k), "angle": "90 deg", "direction": "down"}),
                json!({"side": {"flange": side, "end": "start"}, "length": mm(6.0 * k),
                       "angle": "90 deg"}),
                json!({"edge": base[2], "outline": "Arch", "angle": "90 deg"}),
                json!({"edge": hole[0], "length": mm(8.0 * k), "angle": "90 deg",
                       "direction": "down", "insetStart": mm(2.0 * k), "insetEnd": mm(2.0 * k)}),
            ]
        };
        let with_ids = |list: Vec<Value>, ids: &[u64]| -> Vec<Value> {
            list.into_iter()
                .zip(ids)
                .map(|(mut f, id)| {
                    f["id"] = json!(id);
                    f
                })
                .collect()
        };
        let mut all = vec![tab(55.0, 45.0)];
        all.extend(rest(2, 1));
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Panel", "thickness": mm(k), "radius": mm(k),
                   "kFactor": 0.5, "relief": {"shape": "rect", "size": mm(2.0 * k)},
                   "flanges": all.clone()}),
        );
        let first = face_edges(&mut s);
        assert!(first.len() > 40, "scale {k}: {} faces", first.len());

        // The tab deleted.
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": with_ids(rest(1, 0), &[2, 3, 4, 5, 6, 7])}),
        );
        let deleted = face_edges(&mut s);
        let lost = kept_in_place(&first, &deleted, k, "the tab deleted");
        assert!(lost > 0 && lost <= 16, "scale {k}: {lost} names lost");

        // Back as it was (the tab a new flange, id 8), then a tab put in
        // front of the list.
        let mut again = vec![tab(55.0, 45.0)];
        again.extend(with_ids(rest(2, 1), &[2, 3, 4, 5, 6, 7]));
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": again.clone()}),
        );
        let back = face_edges(&mut s);
        kept_in_place(&deleted, &back, k, "the tab added again");
        let mut front = vec![tab(12.0, 8.0)];
        front.extend(with_ids(again[..1].to_vec(), &[8]));
        front.extend(with_ids(rest(3, 2), &[2, 3, 4, 5, 6, 7]));
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "Panel", "flanges": front}),
        );
        let inserted = face_edges(&mut s);
        let lost = kept_in_place(&back, &inserted, k, "a tab put in front");
        assert!(lost <= 6, "scale {k}: {lost} names lost");
        let ids: Vec<Value> = run(&mut s, "sheetMetal.bends", json!({"feature": "Panel"}))["bends"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["id"].clone())
            .collect();
        assert_eq!(ids, [9, 8, 2, 3, 4, 5, 6, 7].map(|n| json!(n)), "scale {k}");
    }
}

/// The highest ids (review 2026-10-29: an id of 2³² − 1 was taken, the
/// part's next id could not lie above it, and from then on every save
/// and autosave failed): ids from 2³¹ on (a flange's faces are numbered
/// `2 · id − 1` and `2 · id`, below 2³²) up to 2³² − 1 are refused when
/// the part is made and on an edit, 2³¹ − 1 is taken and the document
/// saves and opens, and a new flange past it is refused (the ids are used
/// up) rather than given an id saving would refuse.
#[test]
fn the_highest_flange_ids_never_make_a_document_that_cannot_be_saved() {
    let k = 1.0;
    let dir = workdir("sheetmetal-flanges", "highest-ids");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Base"}),
    );
    let base = lines_of(&run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [60.0, 40.0]}),
    ));
    let wall = |edge: usize, id: Option<u64>| {
        let mut f = json!({"edge": base[edge], "length": "10 mm", "angle": "90 deg"});
        if let Some(id) = id {
            f["id"] = json!(id);
        }
        f
    };
    let max = (1u64 << 31) - 1;
    for too_high in [max + 1, u64::from(u32::MAX) - 1, u64::from(u32::MAX)] {
        let got = error(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Panel", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.5,
                   "flanges": [wall(0, Some(too_high))]}),
        );
        assert!(got.contains("flanges.0.id"), "{too_high}: {got}");
    }
    panel(&mut s, k, vec![wall(0, Some(max))]);
    let bends = run(&mut s, "sheetMetal.bends", json!({"feature": "Panel"}));
    assert_eq!(bends["bends"][0]["id"], max, "{bends}");
    // Its faces are numbered by it, below 2³².
    let faces = names(&mut s);
    assert!(
        faces
            .iter()
            .any(|n| n.contains(&format!("face({})", 2 * max))),
        "{faces:?}"
    );
    run(&mut s, "file.save", json!({"path": "highest.3dcraft"}));
    // A new flange would need the id 2³¹.
    let got = error(
        &mut s,
        "sheetMetal.edit",
        json!({"feature": "Panel", "flanges": [wall(0, Some(max)), wall(1, None)]}),
    );
    assert!(got.contains("used up"), "{got}");
    let got = error(
        &mut s,
        "sheetMetal.edit",
        json!({"feature": "Panel", "flanges": [wall(0, Some(u64::from(u32::MAX)))]}),
    );
    assert!(got.contains("flanges.0.id"), "{got}");
    run(&mut s, "file.save", json!({"path": "highest.3dcraft"}));
    let mut u = session_in(&dir);
    run(&mut u, "file.open", json!({"path": "highest.3dcraft"}));
    let bends = run(&mut u, "sheetMetal.bends", json!({"feature": "Panel"}));
    assert_eq!(bends["bends"][0]["id"], max, "{bends}");
}

#[test]
fn bad_flanges_are_errors_or_fail_the_part() {
    let k = 1.0;
    let dir = workdir("sheetmetal-flanges", "bad");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    let (base, hole) = sketches(&mut s, k);
    let e = &base[0];
    let make = |flanges: Value| {
        json!({"sketch": "Base", "name": "Bad", "thickness": "1 mm", "radius": "1 mm",
               "kFactor": 0.5, "flanges": flanges})
    };
    for (flanges, why) in [
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg", "tip": 0}]),
            "give one of them",
        ),
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg"},
                   {"side": {"flange": 0, "end": "middle"}, "length": "5 mm", "angle": "90 deg"}]),
            "start or end",
        ),
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg"},
                   {"side": {"flange": 3, "end": "end"}, "length": "5 mm", "angle": "90 deg"}]),
            "earlier flange",
        ),
        (
            json!([{"edge": e, "outline": "Nowhere", "angle": "90 deg"}]),
            "names no sketch",
        ),
        (
            json!([{"edge": e, "outline": "Arch", "length": "5 mm", "angle": "90 deg"}]),
            "give no `length`",
        ),
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg", "insetStart": [1]}]),
            "insetStart",
        ),
    ] {
        let got = error(&mut s, "feature.sheetMetal", make(flanges));
        assert!(got.contains(why), "{why}: {got}");
    }
    for (flanges, why) in [
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg", "insetStart": "40 mm",
                    "insetEnd": "30 mm"}]),
            "leave nothing",
        ),
        (
            json!([{"edge": e, "length": "5 mm", "angle": "90 deg", "insetStart": "-1 mm"}]),
            "inset",
        ),
        (
            json!([{"edge": e, "outline": "Arch", "angle": "90 deg"}]),
            "no line on its edge",
        ),
        (
            json!([{"edge": hole[0], "length": "5 mm", "angle": "90 deg"}]),
            "overlaps itself",
        ),
    ] {
        let got = failed(&mut s, "feature.sheetMetal", make(flanges));
        assert!(got.contains(why), "{why}: {got}");
        run(&mut s, "edit.undo", json!({}));
    }
}

#[test]
fn the_guide_cover_runs_as_it_says() {
    let guide = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/guide/sheet-metal.md");
    let text = std::fs::read_to_string(guide).unwrap();
    let fence = "`".repeat(3);
    let section = text
        .split("## Inset flanges, tabs and outlines")
        .nth(1)
        .unwrap();
    let block = section
        .split(&format!("{fence}json"))
        .nth(1)
        .unwrap()
        .split(&fence)
        .next()
        .unwrap();
    let steps: Value = serde_json::from_str(block).unwrap();
    let dir = workdir("sheetmetal-flanges", "guide");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    for step in steps.as_array().unwrap() {
        let id = step[0].as_str().unwrap();
        let result = run(&mut s, id, step[1].clone());
        assert_ne!(result["status"], "failed", "{id}: {result}");
    }
    let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Cover"}));
    let ba = FRAC_PI_2 * (1.0 + 0.42);
    // The wall 80 wide, its side 15, the ear 20, the tab 14; the hole 20 × 25.
    let widths = [80.0, 15.0, 20.0, 14.0];
    let flats = 6000.0 - 500.0 + 80.0 * 15.0 + 15.0 * 10.0 + (300.0 - 9.0 * PI) + 14.0 * 12.0;
    let area = flats + widths.iter().sum::<f64>() * ba;
    assert_close(&table["flatArea"], area, "the cover's flat area");
    assert!(dir.join("cover.svg").exists());
}
