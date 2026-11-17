//! A fillet that ends by fading, through the commands
//! (docs/design/blends.md › *A fillet that ends in a point*, the command's
//! part): `feature.fillet` with `fade: [{at, over, back}]` runs the radius
//! down to nothing over `over` along the edge and closes the face in a
//! point at the vertex `at` or `back` short of it. Held here to the closed
//! form of a block's edge (the removed volume `(1 − π/4) R² (h − b − L +
//! 9L/14)`, `∫ f² = 9/14` of the fade's cubic), to what the result says of
//! each tip, to its names through an upstream edit, a save and an undo,
//! and every refusal to its reason in words a user can act on.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, deep, error, failed, failure, run, session, session_in};
use common::workdir;
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

/// The block's size, the radius and the edge rounded: the upright edge
/// at x = 0, y = 0 (between faces 2 and 4), from vertex 0 at the bottom
/// to vertex 4 at the top.
const SIZE: [f64; 3] = [40.0, 30.0, 40.0];
const R: f64 = 3.0;
const EDGE: &str = "Box:edge(9)";
const TOP: &str = "Box:vertex(4)";
const BOTTOM: &str = "Box:vertex(0)";

/// What a square corner loses to a round of radius 1, per length.
const SPANDREL: f64 = 1.0 - PI / 4.0;

/// The block's volume with its upright edge rounded `R`, fading to a
/// point over `over` at `back` short of the top (and the same at the
/// bottom when `both`): the strip of one radius and each fade's
/// `∫ f² = 9/14` of its length.
fn closed_form(over: f64, back: f64, both: bool) -> f64 {
    let h = SIZE[2];
    let ends = if both { 2.0 } else { 1.0 };
    let one_radius = h - ends * (over + back);
    SIZE[0] * SIZE[1] * SIZE[2] - SPANDREL * R * R * (one_radius + ends * 9.0 * over / 14.0)
}

fn block(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    built(s, "feature.block", json!({"size": SIZE, "name": "Box"}));
}

fn close(actual: &Value, expected: f64, bound: f64, what: &str) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: {actual}"));
    assert!(
        (actual - expected).abs() <= bound,
        "{what}: {actual}, expected {expected} (off by {:e})",
        actual - expected
    );
}

#[test]
fn a_fillet_that_fades_meets_its_closed_form_and_says_where_it_ends() {
    for (over, back) in [(15.0, 0.0), (15.0, 6.0), (3.0, 0.0), (40.0, 0.0)] {
        let mut s = session();
        block(&mut s);
        let mut fade = json!({"at": TOP, "over": format!("{over} mm")});
        if back > 0.0 {
            fade["back"] = json!(format!("{back} mm"));
        }
        let made = built(
            &mut s,
            "feature.fillet",
            json!({"edges": [EDGE], "radius": "3 mm", "fade": [fade], "name": "Round"}),
        );
        let found = body(&mut s, "Round");
        assert_eq!(found["valid"], true, "{found}");
        // The face is fitted within the tolerance (1e-6 mm) of the arcs:
        // the volume within its area times that, and far less measured.
        close(
            &found["volume"],
            closed_form(over, back, false),
            1e-4,
            &format!("volume, over {over} back {back}"),
        );
        deep(&mut s);
        let fades = made["fades"].as_array().unwrap();
        assert_eq!(fades.len(), 1, "{made}");
        let tip = &fades[0];
        assert_eq!(tip["at"], "F1:vertex(4)", "{tip}");
        assert_eq!(tip["edge"], 0, "{tip}");
        close(&tip["over"], over, 0.0, "over");
        close(&tip["back"], back, 0.0, "back");
        close(&tip["length"], SIZE[2], 1e-12, "length");
        for (axis, expected) in [0.0, 0.0, SIZE[2] - back].iter().enumerate() {
            close(&tip["tip"][axis], *expected, 1e-12, "tip");
        }
        // The tip's vertex, by its name among the body's vertices.
        let vertex = tip["vertex"].as_str().unwrap().to_string();
        assert!(vertex.contains("F2:fillet(0)"), "{vertex}");
        let listing = run(&mut s, "document.edges", json!({"feature": "Round"}));
        let there = listing["vertices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == vertex.as_str())
            .unwrap_or_else(|| panic!("no vertex {vertex} in {listing}"));
        close(
            &there["point"][2],
            SIZE[2] - back,
            1e-9,
            "the vertex's height",
        );
        // document.inspect says the same.
        let inspected = common::checked::feature(&mut s, "Round");
        assert_eq!(inspected["fades"], made["fades"], "{inspected}");
    }
}

#[test]
fn both_ends_fade_and_two_edges_apart_fade_in_one_fillet() {
    let mut s = session();
    block(&mut s);
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE], "radius": 3, "name": "Round",
               "fade": [{"at": TOP, "over": 10, "back": 2}, {"at": BOTTOM, "over": 10, "back": 2}]}),
    );
    let found = body(&mut s, "Round");
    close(
        &found["volume"],
        closed_form(10.0, 2.0, true),
        1e-4,
        "both ends",
    );
    deep(&mut s);
    // Two upright edges that do not meet, each fading at its top; with a
    // third edge of one radius that meets neither.
    let mut s = session();
    block(&mut s);
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE, "Box:edge(11)", "Box:edge(8)"], "radius": 3, "name": "Round",
               "fade": [{"at": TOP, "over": 15}, {"at": "Box:vertex(7)", "over": 15}]}),
    );
    let found = body(&mut s, "Round");
    // Edge 11 (x = 40, y = 30) is a square corner as edge 9 is, fading at
    // its top (vertex 7); edge 8 (x = 40, y = 0) is rounded whole.
    let whole = SIZE[0] * SIZE[1] * SIZE[2];
    let faded = whole - closed_form(15.0, 0.0, false);
    let expected = whole - 2.0 * faded - SPANDREL * R * R * SIZE[2];
    close(&found["volume"], expected, 1e-4, "two fades and a round");
    deep(&mut s);
}

#[test]
fn the_names_of_a_faded_fillet_hold_through_an_edit_a_save_and_an_undo() {
    let dir = workdir("fillet-fade", "names");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "h", "expr": "40 mm"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [40, 30, "h"], "name": "Box"}),
    );
    let made = built(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE], "radius": 3, "name": "Round",
               "fade": [{"at": TOP, "over": 15, "back": 6}]}),
    );
    let tip = made["fades"][0]["vertex"].as_str().unwrap().to_string();
    // The tip's vertex and the edge beyond it, named by their faces: a
    // datum point at the tip, the edge beyond found by its own name.
    let edge_named = |s: &mut Session, name: &str| -> Value {
        let listing = run(s, "document.edges", json!({"feature": "Round"}));
        listing["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == name)
            .unwrap_or_else(|| panic!("no edge {name} in {listing}"))
            .clone()
    };
    let beyond = run(
        &mut s,
        "document.edges",
        json!({"feature": "Round", "between": ["F2:copy(0)/F1:face(2)", "F2:copy(0)/F1:face(4)"]}),
    );
    assert_eq!(beyond["count"], 1, "{beyond}");
    let edge = beyond["edges"][0]["name"].as_str().unwrap().to_string();
    // Named by the end it belongs to (the top face near it), and not by
    // the edge's own name, which would stay on the piece beyond a fade
    // moved to the other end.
    assert!(edge.ends_with("|F2:copy(0)/F1:face(1)]"), "{edge}");
    assert!(beyond["edges"][0].get("aliases").is_none(), "{beyond}");
    close(&beyond["edges"][0]["length"], 6.0, 1e-9, "the edge beyond");
    built(&mut s, "datum.point", json!({"vertex": tip, "name": "Tip"}));
    // Taller: the tip moves up with the top, every name holds.
    run(
        &mut s,
        "variable.set",
        json!({"name": "h", "expr": "50 mm"}),
    );
    let point = common::checked::feature(&mut s, "Tip");
    assert_eq!(point["status"], "ok", "{point}");
    close(
        &point["datum"]["at"][2],
        44.0,
        1e-9,
        "the tip after the edit",
    );
    let after = edge_named(&mut s, &edge);
    close(
        &after["start"][2],
        44.0,
        1e-9,
        "the edge beyond after the edit",
    );
    close(
        &after["end"][2],
        50.0,
        1e-9,
        "the edge beyond after the edit",
    );
    let round = common::checked::feature(&mut s, "Round");
    assert_eq!(round["fades"][0]["vertex"], tip.as_str(), "{round}");
    let volume = body(&mut s, "Round")["volume"].clone();
    // Saved and opened again: the same fades, the same body.
    run(&mut s, "file.save", json!({"path": "faded.3dcraft"}));
    let mut again = session_in(&dir);
    run(&mut again, "file.open", json!({"path": "faded.3dcraft"}));
    let reopened = common::checked::feature(&mut again, "Round");
    assert_eq!(reopened["params"], round["params"], "{reopened}");
    assert_eq!(reopened["fades"], round["fades"], "{reopened}");
    assert_eq!(body(&mut again, "Round")["volume"], volume);
    // Undone back to the fillet of one radius and redone.
    let plain = {
        let mut p = session();
        run(&mut p, "file.new", json!({"units": "mm"}));
        built(
            &mut p,
            "feature.block",
            json!({"size": [40, 30, 50], "name": "Box"}),
        );
        built(
            &mut p,
            "feature.fillet",
            json!({"edges": [EDGE], "radius": 3, "name": "Round"}),
        );
        body(&mut p, "Round")["volume"].clone()
    };
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "fade": null}),
    );
    assert_eq!(body(&mut s, "Round")["volume"], plain);
    run(&mut s, "edit.undo", json!({}));
    let back = common::checked::feature(&mut s, "Round");
    assert_eq!(back["fades"], round["fades"], "{back}");
}

/// The reason a fillet with these fades fails, on the block.
fn fails_with(fade: Value, edges: Value, chain: bool) -> String {
    let mut s = session();
    block(&mut s);
    let reason = failed(
        &mut s,
        "feature.fillet",
        json!({"edges": edges, "radius": 3, "chain": chain, "fade": fade, "name": "Round"}),
    );
    assert!(!reason.contains("internal error"), "{reason}");
    reason
}

/// The error a `feature.fillet` with these fades is refused with at once.
fn refused_with(fade: Value) -> String {
    let mut s = session();
    block(&mut s);
    error(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE], "radius": 3, "fade": fade}),
    )
}

#[test]
fn hostile_fades_are_refused_with_what_to_change() {
    // Not an end of the edge: another vertex of the body.
    let reason = fails_with(
        json!([{"at": "Box:vertex(7)", "over": 15}]),
        json!([EDGE]),
        false,
    );
    assert!(reason.contains("fade.0.at"), "{reason}");
    assert!(
        reason.contains("is not an end of an edge this fillet rounds"),
        "{reason}"
    );
    assert!(
        reason.contains("F1:vertex(4)") && reason.contains("F1:vertex(0)"),
        "{reason}"
    );
    // A vertex of another body.
    let mut s = session();
    block(&mut s);
    built(
        &mut s,
        "feature.block",
        json!({"size": [5, 5, 5], "position": [100, 0, 0], "name": "Other"}),
    );
    let reason = failed(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE], "radius": 3, "fade": [{"at": "Other:vertex(4)", "over": 15}]}),
    );
    assert!(
        reason.contains("fade.0.at") && reason.contains("is a vertex of `Other`"),
        "{reason}"
    );
    // Not a vertex at all.
    let message = refused_with(json!([{"at": "Box:face(1)", "over": 15}]));
    assert!(message.contains("is not a vertex"), "{message}");
    // Lengths that are not lengths.
    for (over, words) in [
        (json!(0), "has to be more than zero"),
        (json!(-1), "has to be more than zero"),
        (json!("-1 mm"), "has to be more than zero"),
        (json!("0 / 0"), "fade.0.over"),
        (json!("nope + 1"), "fade.0.over"),
        (json!("30 deg"), "fade.0.over"),
        (json!("1e999"), "fade.0.over"),
    ] {
        let reason = fails_with(json!([{"at": TOP, "over": over}]), json!([EDGE]), false);
        assert!(reason.contains(words), "{over}: {reason}");
    }
    for over in [json!(null), json!([]), json!({}), json!(true), json!("")] {
        let message = refused_with(json!([{"at": TOP, "over": over}]));
        assert!(message.contains("over"), "{over}: {message}");
    }
    let reason = fails_with(
        json!([{"at": TOP, "over": 15, "back": -1}]),
        json!([EDGE]),
        false,
    );
    assert!(
        reason.contains("fade.0.back") && reason.contains("zero or more"),
        "{reason}"
    );
    // Longer than the edge, with its length.
    let reason = fails_with(
        json!([{"at": TOP, "over": 15, "back": 30}]),
        json!([EDGE]),
        false,
    );
    assert!(
        reason.contains("over and back take 45 mm") && reason.contains("which is 40 mm long"),
        "{reason}"
    );
    let reason = fails_with(
        json!([{"at": TOP, "over": 25}, {"at": BOTTOM, "over": 25}]),
        json!([EDGE]),
        false,
    );
    assert!(
        reason.contains("fade.0 and fade.1") && reason.contains("50 mm"),
        "{reason}"
    );
    // Too short for the radius: the least that builds.
    let reason = fails_with(json!([{"at": TOP, "over": 1}]), json!([EDGE]), false);
    assert!(
        reason.contains("fade.0 (at F1:vertex(4))")
            && reason.contains("give an over of at least 3 mm"),
        "{reason}"
    );
    // A point the tolerance cannot tell from the vertex.
    let reason = fails_with(
        json!([{"at": TOP, "over": 15, "back": 1e-6}]),
        json!([EDGE]),
        false,
    );
    assert!(reason.contains("give a back of more than"), "{reason}");
    // Two fades at one end.
    let message = refused_with(json!([{"at": TOP, "over": 15}, {"at": TOP, "over": 10}]));
    assert!(message.contains("already fades at"), "{message}");
    // The shapes of a fade.
    for (fade, words) in [
        (json!({"at": TOP, "over": 15}), "must be a list"),
        (json!([TOP]), "must be an object"),
        (json!([{"over": 15}]), "needs `at`"),
        (json!([{"at": TOP}]), "needs `over`"),
        (
            json!([{"at": TOP, "over": 15, "to": 1}]),
            "has no parameter `to`",
        ),
        (json!([{"at": 7, "over": 15}]), "fade.0.at"),
    ] {
        let message = refused_with(fade.clone());
        assert!(message.contains(words), "{fade}: {message}");
    }
    let many: Vec<Value> = (0..2049).map(|_| json!({"at": TOP, "over": 15})).collect();
    let message = refused_with(Value::Array(many));
    assert!(message.contains("at most"), "{message}");
    // Fades on edges that meet: at their shared vertex, and at the far
    // end of one of them.
    let reason = fails_with(
        json!([{"at": TOP, "over": 15}]),
        json!([EDGE, "Box:edge(4)"]),
        false,
    );
    assert!(
        reason.contains("is an end of edges.0") && reason.contains("and of edges.1"),
        "{reason}"
    );
    let reason = fails_with(
        json!([{"at": BOTTOM, "over": 15}]),
        json!([EDGE, "Box:edge(4)"]),
        false,
    );
    assert!(
        reason.contains("ends in its vertex") && reason.contains("on its own"),
        "{reason}"
    );
}

#[test]
fn a_closed_edge_and_an_unproven_vertex_are_refused() {
    // A rod's rim closes on itself in its one vertex.
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 10, "height": 20, "name": "Rod"}),
    );
    let listing = run(&mut s, "document.edges", json!({"feature": "Rod"}));
    let ring = listing["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["closed"] == true)
        .unwrap()
        .clone();
    let rim = ring["name"].as_str().unwrap().to_string();
    // Its vertex: the one where the ring starts and ends.
    let vertex = listing["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["point"] == ring["start"])
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_string();
    let reason = failed(
        &mut s,
        "feature.fillet",
        json!({"edges": [rim], "radius": 1, "fade": [{"at": vertex, "over": 5}]}),
    );
    assert!(reason.contains("closes on itself"), "{reason}");
    // A vertex named after a fillet's face is not proven to stay with its
    // vertex: refused at once, as an edge so named is.
    let mut s = session();
    block(&mut s);
    let made = built(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE], "radius": 3, "fade": [{"at": TOP, "over": 15, "back": 6}], "name": "Round"}),
    );
    let tip = made["fades"][0]["vertex"].clone();
    let message = error(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F2:copy(0)/F1:face(1)", "F2:copy(0)/F1:face(3)"]], "radius": 1,
               "fade": [{"at": tip, "over": 5}]}),
    );
    assert!(
        message.contains("fade.0.at") && message.contains("not proven"),
        "{message}"
    );
}

/// Documents with fillets of one radius (every edge of a block with its
/// corners, a rod's rims, a cone's foot, the ridge of a ruled loft: a
/// fitted face), each built and saved by `main` at 28eb96df (format 73),
/// and the FNV-1a of the DATA section of the STEP export that build made
/// of it (`plan/fillet-fade/s3/scripts/main_fixtures.py`). A STEP file
/// writes every real as the shortest text that reads back as it, so the
/// same digest is the same faces, edges, vertices, curves and surfaces to
/// the last bit.
const MAIN_BUILT: &[(&str, u64, usize)] = &[
    ("block-all", 0xad19_ca0e_f166_d475, 23587),
    ("rod-rims", 0x1c23_476d_9e9c_6af8, 4378),
    ("cone-foot", 0x8c9e_2f91_98db_e4cd, 3240),
    ("ridge", 0x43c6_4557_6b95_93d0, 37589),
];

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[test]
fn fillets_without_a_fade_are_built_as_main_built_them() {
    let fixtures = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fillet_fade");
    let dir = workdir("fillet-fade", "main-built");
    for (name, digest, bytes) in MAIN_BUILT {
        let file = format!("{name}.3dcraft");
        std::fs::copy(fixtures.join(&file), dir.join(&file)).unwrap();
        let mut s = session_in(&dir);
        run(&mut s, "file.open", json!({"path": file}));
        let step = format!("{name}.step");
        run(&mut s, "file.export", json!({"path": step}));
        let written = std::fs::read(dir.join(&step)).unwrap();
        let start = written
            .windows(5)
            .position(|w| w == b"DATA;")
            .expect("a DATA section");
        let data = &written[start..];
        assert_eq!(
            (fnv(data), data.len()),
            (*digest, *bytes),
            "{name}: the STEP export is not the one main made"
        );
    }
}

/// The stem of the half-section hull (`plan/loft-tip/BOW.md`: two lofted
/// halves sewn, a block cut by them, the deck cut off; tolerance 0.02 mm,
/// saved by `main` at 9ad00032), from the deck to its foot, where it runs
/// on into the keel between the same two faces.
const STEM: &str = "F23:between[F23:copy(0)/F13:copy(1)/F9:side(e9,e9,e9,e9,e9,e9,e9,e7),F23:copy(0)/F13:copy(1,1)/F9:side(e9,e9,e9,e9,e9,e9,e9,e7)|F23:copy(1)/F22:side(e4,e4,e4,e4,e4,e4,e4,e4)]";
const FOOT: &str = "F23:corner[F23:copy(0)/F13:copy(1)/F9:side(e9,e9,e9,e9,e9,e9,e9,e7),F23:copy(0)/F13:copy(1,1)/F9:side(e9,e9,e9,e9,e9,e9,e9,e7)]";

/// The hull with its stem rounded `radius`, fading over `over` to a point
/// `back` short of the foot: the fillet's result.
fn hull_stem(s: &mut Session, radius: f64, over: f64, back: f64) -> Value {
    run(
        s,
        "feature.fillet",
        json!({"edges": [STEM], "radius": radius, "name": "Stem fillet",
               "fade": [{"at": FOOT, "over": over, "back": back}]}),
    )
}

#[test]
fn the_hulls_stem_fades_to_a_point_above_the_forefoot() {
    let fixtures = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fillet_fade");
    let dir = workdir("fillet-fade", "hull");
    std::fs::copy(fixtures.join("hull.3dcraft"), dir.join("hull.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "hull.3dcraft"}));
    // Rounded 3 mm from the deck down, fading over 15 mm to a point half
    // a millimetre above the foot: built, valid, clear in the deep check,
    // a little less than the hull (10,781,617.55 mm³) by what the round
    // takes.
    // The point of the proof first, 0.33 above the foot: the volume the
    // kernel's probe gave, to the last digit it printed.
    let made = hull_stem(&mut s, 3.0, 15.0, 0.33);
    assert_eq!(made["status"], "ok", "{made}");
    close(
        &body(&mut s, "Stem fillet")["volume"],
        10_781_506.254006842,
        1e-3,
        "the hull's volume, back 0.33",
    );
    run(&mut s, "edit.undo", json!({}));
    let made = hull_stem(&mut s, 3.0, 15.0, 0.5);
    assert_eq!(made["status"], "ok", "{made}");
    let tip = &made["fades"][0];
    close(&tip["back"], 0.5, 0.0, "back");
    close(&tip["length"], 87.18465383984673, 1e-6, "the stem's length");
    let found = body(&mut s, "Stem fillet");
    assert_eq!(found["valid"], true, "{found}");
    close(
        &found["volume"],
        10_781_506.716083191,
        1e-3,
        "the hull's volume",
    );
    deep(&mut s);
    // In the foot itself the two halves fold: no ball, with how far
    // short of the foot the last one was found.
    run(&mut s, "edit.undo", json!({}));
    let made = hull_stem(&mut s, 3.0, 15.0, 0.0);
    assert_eq!(made["status"], "failed", "{made}");
    let reason = made["error"].as_str().unwrap();
    assert!(
        reason.contains("no ball touching both faces was found past")
            && reason.contains("a back larger by"),
        "{reason}"
    );
}

/// The review of the kernel's slices (`review_fade_hull.rs` ›
/// `a_short_fade_on_the_stem_is_not_called_a_bug`, on
/// `check/review-fade-law`), at the command: fades short for their radius
/// where the hull's stem bends towards its foot were refused by the
/// kernel's post-check of tangency in the words of a bug, and two others
/// came back built with a folded face. The kernel refuses them now before
/// building, as a face that would turn back on itself
/// (`FadeFault::TurnsBack`); here, at the command, at the fade with the
/// place and the ball's radius there, never in the words of a bug. The
/// reasons are printed (`HULL …`, with `--nocapture`).
#[test]
fn a_short_fade_on_the_hulls_stem_is_not_called_a_bug() {
    let fixtures = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fillet_fade");
    let dir = workdir("fillet-fade", "hull-short");
    std::fs::copy(fixtures.join("hull.3dcraft"), dir.join("hull.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "hull.3dcraft"}));
    for (radius, over, back) in [
        (3.0, 5.0, 0.5),
        (5.0, 5.0, 1.0),
        (5.0, 10.0, 0.5),
        (3.0, 5.0, 1.0),
        (5.0, 10.0, 1.0),
    ] {
        let made = hull_stem(&mut s, radius, over, back);
        let reason = made["error"].as_str().unwrap_or("");
        println!(
            "HULL radius {radius} over {over} back {back}: {} {reason}",
            made["status"]
        );
        if made["status"] != "ok" {
            assert!(
                !reason.contains("internal error") && !reason.contains("a bug"),
                "radius {radius} over {over} back {back}: {reason}"
            );
            assert!(
                reason.starts_with("fade.0 (at F23:corner["),
                "radius {radius} over {over} back {back}: {reason}"
            );
        } else {
            // Built: sound, then (the two that were built folded before
            // the kernel's mend are refused now).
            deep(&mut s);
        }
        run(&mut s, "edit.undo", json!({}));
    }
}

#[test]
fn feature_edit_changes_and_clears_the_fades() {
    let mut s = session();
    block(&mut s);
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE], "radius": 3, "fade": [{"at": TOP, "over": 15}], "name": "Round"}),
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "fade": [{"at": TOP, "over": 15, "back": 6}]}),
    );
    close(
        &body(&mut s, "Round")["volume"],
        closed_form(15.0, 6.0, false),
        1e-4,
        "edited",
    );
    // The radius is edited as ever; a fade that no longer fits fails the
    // feature with the reason, and is mended by another edit.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "radius": 20}),
    );
    let reason = failure(&mut s, "Round");
    assert!(
        reason.contains("give an over of at least 20 mm"),
        "{reason}"
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "radius": 3}),
    );
    let message = error(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "fade": [{"at": TOP}]}),
    );
    assert!(message.contains("needs `over`"), "{message}");
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "fade": []}),
    );
    close(
        &body(&mut s, "Round")["volume"],
        SIZE[0] * SIZE[1] * SIZE[2] - SPANDREL * R * R * SIZE[2],
        1e-9,
        "one radius again",
    );
    assert!(
        common::checked::feature(&mut s, "Round")
            .get("fades")
            .is_none()
    );
}

/// Rule 4 (the coordinator's decision on the review of this slice): the
/// tip short of the vertex and the edge beyond it are named by the end
/// they belong to, so a fade moved to the other end of the edge takes
/// their names with it. A datum point at the tip then fails naming the
/// vertex, and a note on the edge beyond comes loose of it, where named by
/// their faces alone both would have moved to the new tip and piece
/// without a word; moved back, both are where they were.
#[test]
fn a_fade_moved_to_the_other_end_takes_its_names_with_it() {
    let mut s = session();
    block(&mut s);
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE], "radius": 3, "name": "Round",
               "fade": [{"at": TOP, "over": 15, "back": 6}]}),
    );
    let tip = common::checked::feature(&mut s, "Round")["fades"][0]["vertex"]
        .as_str()
        .unwrap()
        .to_string();
    let beyond = run(
        &mut s,
        "document.edges",
        json!({"feature": "Round", "between": ["F2:copy(0)/F1:face(2)", "F2:copy(0)/F1:face(4)"]}),
    );
    let edge = beyond["edges"][0]["name"].as_str().unwrap().to_string();
    built(&mut s, "datum.point", json!({"vertex": tip, "name": "Tip"}));
    run(
        &mut s,
        "note.add",
        json!({"text": "sharp from here up", "on": edge}),
    );
    let attachment = |s: &mut Session| {
        let notes = run(s, "note.list", json!({}));
        notes["notes"][0]["attachment"].clone()
    };
    assert_eq!(attachment(&mut s), "attached");
    // To the foot of the edge.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "fade": [{"at": BOTTOM, "over": 15, "back": 6}]}),
    );
    let moved = common::checked::feature(&mut s, "Round");
    let new_tip = moved["fades"][0]["vertex"].as_str().unwrap();
    assert_ne!(new_tip, tip, "{moved}");
    let point = common::checked::feature(&mut s, "Tip");
    assert_eq!(point["status"], "failed", "{point}");
    assert!(point["error"].as_str().unwrap().contains(&tip), "{point}");
    assert_eq!(attachment(&mut s), "detached");
    // Back to the top: the tip and the edge beyond are what they were.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Round", "fade": [{"at": TOP, "over": 15, "back": 6}]}),
    );
    let point = common::checked::feature(&mut s, "Tip");
    assert_eq!(point["status"], "ok", "{point}");
    close(
        &point["datum"]["at"][2],
        SIZE[2] - 6.0,
        1e-9,
        "the tip again",
    );
    assert_eq!(attachment(&mut s), "attached");
}

/// The review's dead end: after an earlier fillet rounded an edge that
/// ends at the vertex, the vertex is gone from the body (the end of the
/// edge is a vertex named after that fillet's face), and the refusal
/// offered that name, which a fade then refused as not proven. Now it says
/// what cannot be faded and why, and offers only names a fade takes: the
/// other end of the edge, which is then taken as an end.
#[test]
fn a_fade_at_an_end_an_earlier_fillet_rounded_is_refused_without_a_dead_end() {
    let mut s = session();
    block(&mut s);
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": ["Box:edge(4)"], "radius": 2, "name": "First"}),
    );
    let reason = failed(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE], "radius": 3, "name": "Round",
               "fade": [{"at": TOP, "over": 15}]}),
    );
    assert!(
        reason.starts_with("fade.0.at: F1:vertex(4) is a vertex of `Box`")
            && reason.contains("went into `First`")
            && reason.contains("is not built yet"),
        "{reason}"
    );
    assert!(!reason.contains("give the vertex by its name"), "{reason}");
    // What it offers is taken as an end of the edge.
    let offered = reason
        .split("that a fade takes: ")
        .nth(1)
        .and_then(|rest| rest.split("; not ").next())
        .unwrap_or_else(|| panic!("nothing offered: {reason}"));
    assert!(!offered.contains("fillet("), "{reason}");
    for name in offered.split(", ") {
        run(&mut s, "edit.undo", json!({}));
        let made = run(
            &mut s,
            "feature.fillet",
            json!({"edges": [EDGE], "radius": 3, "name": "Round",
                   "fade": [{"at": name, "over": 15}]}),
        );
        if made["status"] != "ok" {
            let why = made["error"].as_str().unwrap();
            assert!(
                !why.starts_with("fade.0.at") && !why.contains("internal error"),
                "{name}: {why}"
            );
        }
    }
}

/// The review's minor findings: a huge length is said as `1e300`, not in
/// three hundred digits; a feature's name that names none is refused at
/// the fade's own place.
#[test]
fn huge_lengths_and_unknown_features_are_said_where_they_are() {
    let mut s = session();
    block(&mut s);
    let reason = failed(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE], "radius": 3, "fade": [{"at": TOP, "over": "1e300 mm"}]}),
    );
    assert!(
        reason.contains("over and back take 1e300 mm") && !reason.contains("0000000000"),
        "{reason}"
    );
    let message = error(
        &mut s,
        "feature.fillet",
        json!({"edges": [EDGE], "radius": 3, "fade": [{"at": "Nope:vertex(4)", "over": 15}]}),
    );
    assert!(message.contains("`fade.0.at`"), "{message}");
}
