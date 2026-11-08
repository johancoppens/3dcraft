//! What the sea-kayak Jig, built natively, found the platform refusing
//! (2026-10-27), each from the steps that made it (`kayak_findings/`):
//!
//! 1. A developable between a curve that inflects and one that does not
//!    (a profile scaled by 1.1 against its plan, so not a generalised
//!    cylinder): the branches of the condition that cross at an inflection
//!    pass each other, and the rulings turned back. Now bridged by a fan.
//! 2. A plate on a developable panel: refused, "the joint reaches past
//!    the faces' surfaces continued", as the panel's surface was continued
//!    across its rulings by as far as the longest one moves. Now each
//!    border is continued far enough past every vertex on it.
//! 3. Two plates joined by `touch` along a chine: their rims share the
//!    inner chine edge, each traced past the chine's ends in its own way,
//!    and every question of their overlap was refused at the marcher's
//!    evaluation cap. Now a stretch of a border on the other surface is a
//!    meeting curve: the plates touch, the intersection is empty.
//! 5. A 5 m hull's panels: the fit near their ends did not settle (the
//!    walk crept into a side, rulings 2e-6 apart beside rulings 3e-3
//!    apart). Now the path is thinned before the fit; the curvature
//!    measure and its assertion settle.
//! 6. The coaming: a boolean of the hull's cylinders refused ("imprint
//!    lost"); built since 2026-10-27, the ring cut from the hull owed.
//! 7. The hull's panels with a deep check: all built and passing (the
//!    check's finding on TipHalfD was its ray casting, fixed 2026-10-27).
//! 8. The hull's panels unfolded: panels 2 and 3 had end fans 1e-5 mm
//!    long (rail ends that line up with a ruling within the tolerance),
//!    which changed their faces' and edges' names; now no fan within the
//!    tolerance of the strip beside it. The unfolds were refused by a
//!    stretch charged over the cube of the whole 5 m panel, then by the
//!    long edges' developments, integrated across the chines' knots; now
//!    the stretch is the strip's and the edges are integrated span by
//!    span. The deck does not build here (owed).
//! 9. The level for the hull's displacement: a step of the search far
//!    below the keel found nothing, and that was refused ("it may reach
//!    878 below it"), as each trimmed face of the hull, cut from faces
//!    3000 mm long, was bound by its whole surface. Now a trimmed freeform
//!    face is bound by what of its surface lies in its trim.
//! 12. The plates on the hull's faces (the steps of repro 12 up to the
//!     plates): the sheer plate `Top4S` was refused, "the rim could not
//!     be traced within a quarter of the tolerance", along its end on
//!     the stern plane: that edge, a spline whose first span is a small
//!     part of it, was continued a whole edge length past its end by
//!     evaluating the spline there, which scattered its points by 1e-5
//!     mm. Now a spline edge is continued as the same function evaluated
//!     inside a longer domain of its own. And `Ov3D` (the steps of repro
//!     12 that it needs): the sheer plate against the deck over it was
//!     refused, "tangent or nearly coincident over too large an area":
//!     the sheer plate's inner skin runs a plate's thickness beside the
//!     deck's rim over five metres, and the marcher ran out of cells.
//!     Now cells are left out by a bound on the field's second
//!     derivative where they ran out, and faces that lie on one another
//!     where one is over the other are coincident: the plates touch.
//! 14. Plates on the developable panels (`onto` the end planes, `touch`
//!     along the chines): refused, "the offset of the freeform face could
//!     not be approximated", as the panel's surface was continued across
//!     its rulings by a box of parameters as wide as the tip's short
//!     ruling needs, and the offset fitted to the tolerance over all of
//!     it. Now the offset is held to the tolerance only within the length
//!     along the surface that the plate uses. The sheer panel itself
//!     (`DevTop4S`) was refused, "the rulings between two found ones
//!     cannot be followed": near each end its condition has a second
//!     branch whose rulings run across the panel's, and the two pass each
//!     other a millimetre apart; the walk stepped from one to the other.
//!     Now the walk follows its branch into the fold there and closes it
//!     with a fan (2026-10-28): the panel is an exact developable of eight
//!     faces named by place, and the plate on it is refused only by the
//!     names of its joints' edges (the Jig names the edges of `face(0)`
//!     alone).
//! 16. The overlap assertions of neighbouring plates (repros 16 to 19: the
//!     garboards either side of the keel, 17 and 18: planks above one
//!     another and the sheer plate, 19: the deck plates either side of
//!     the centreline), each from the steps that make the two plates with
//!     their drilled holes. 16 to 18 pass on main (the touching plates of
//!     repro 3 and 12); 19 was refused, "the box has no size, or the
//!     tolerance is finer than rounding allows at its size": an end face
//!     of a deck plate 17 mm across had an edge on a circle 860 m in
//!     radius (a nearly straight arc of the deck's crown), and its box
//!     held the whole circle. Now a pair whose surfaces cannot be met in
//!     the room of the faces' boxes is met in the room of the boxes that
//!     hold each edge by the part of its curve it is.
//! 20. The inner hull (`InHull`, the half hull's intersection with its
//!     mirror image): refused, "an intersection curve could not be fitted
//!     within the tolerance", as a march stepped over a stretch where the
//!     curve runs past the edge of the other face's surface and back.
//!     Now a branch is divided where it leaves the other surface.
//! 21. The kayak Jig at its own tolerance (0.01 mm, its `new.json`; every
//!     repro above runs at the default 1e-6 mm): the drilled sheer plate
//!     and the deck over it (`Top4Sh`, `Deck4Sh`, at the defaults), the
//!     sheer plate and the plank under it with the Fine and the Full ends
//!     (22, 23: `Plank2Sh`, `Top4Sh`) and the garboards on a single-chine
//!     hull (24: `Plank0Sh`, `Plank0Ph`) were refused, "two points closer
//!     together than the tolerance can tell apart": a hole's wall meets
//!     the plate beside it in a closed curve along the hole's rim, and
//!     where its trace started (its seam) lay one or two tolerances from
//!     the vertex of the rim's own edge. Now the seam of a closed curve is
//!     the place next to it, as an end of an open curve is.
//! 25. The least distance from a plate's stitch holes to its own rim
//!     (`Plank0S` at the defaults and the Jig's tolerance) ran for
//!     thirteen minutes: the settling of a hole's wall against the rim,
//!     faces that run beside each other along the plate's normal and part
//!     only by rounding along it, took all of its steps, creeping along
//!     that line, each projection onto the five-metre rim costing up to a
//!     fifth of a second. Now a settling stops when the steps it has left
//!     could gain no more than a ten-thousandth of the tolerance at the
//!     rate of its last ones, or could not come nearer than another's;
//!     and nearly all of the time was telling the rim's points in or out
//!     (a winding number on a strip 4 mm by 5 m, whose chart was bounded
//!     by its slow direction: six thousand steps a winding): the chart
//!     of a surface draws u at the pace of v where they are far apart.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Runs the steps of `file`: the session, and what failed (a step that is
/// an error, a deep check that is not ok, an assertion that cannot be
/// measured, a feature that failed).
fn steps(file: &str) -> (Session, Vec<String>) {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/kayak_findings")
            .join(file),
    )
    .unwrap();
    let steps: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    s.execute("file.new", &json!({"units": "mm"})).unwrap();
    let mut failed = Vec::new();
    for (id, params) in &steps {
        match s.execute(id, params) {
            Ok(v) => {
                if id == "document.check" && v["ok"] != true {
                    failed.push(format!("{id}: {v}"));
                }
                // On a freeform panel the developability is measured at
                // samples: not proven, which is an answer, not a failure.
                let sampled = v["status"] == "notProven"
                    && v["message"]
                        .as_str()
                        .is_some_and(|m| m.contains("sampled points only"));
                if id == "assert.add" && v["status"] != "passed" && !sampled {
                    failed.push(format!("{id}: {v}"));
                }
            }
            Err(e) => failed.push(format!("{id}: {e}")),
        }
    }
    let document = s.execute("document.inspect", &json!({})).unwrap();
    for f in document["features"].as_array().unwrap() {
        // A feature switched off by its condition (`when`) is not a failure.
        if f["status"] != "ok" && f["status"] != "inactive" {
            failed.push(format!("feature {}: {}", f["name"], f["error"]));
        }
    }
    (s, failed)
}

#[test]
fn a_developable_between_a_curve_that_inflects_and_one_that_does_not_is_built() {
    let (_, failed) = steps("sea_kayak_1.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn a_plate_on_a_developable_panel_is_built_as_on_the_face_it_lies_on() {
    let (_, failed) = steps("sea_kayak_2.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn the_panels_of_a_five_metre_hull_are_built_and_measured() {
    let (mut s, failed) = steps("sea_kayak_5.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    let measured = s
        .execute("measure.developability", &json!({"of": ["Panel3S"]}))
        .unwrap();
    assert_eq!(measured["developable"], true, "{measured}");
}

#[test]
fn the_coaming_boolean_is_refused_as_owed_not_wrong() {
    // The hull's first intersect lost the ruling in which an end plane
    // cuts a neighbouring cylinder ("imprint lost"), and the tips' first
    // was refused at the edge of tolerance: both were the distance from a
    // segment to a flat box read as its rounding (docs/design/booleans.md,
    // *The sea kayak's tips*, 2026-10-27), and both are built now. Owed:
    // the coaming's ring cut from the hull, refused as a curve that
    // cannot be fitted; nothing else fails, and what is built passes the
    // deep check.
    let (mut s, failed) = steps("sea_kayak_6.steps.json");
    assert_eq!(failed.len(), 2, "{failed:#?}");
    assert!(
        failed[0].starts_with("feature \"RingIn\"") && failed[0].contains("does not support"),
        "{failed:#?}"
    );
    assert!(failed[1].starts_with("feature \"Coaming\""), "{failed:#?}");
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    let problems = check["problems"].as_array().unwrap();
    assert!(
        problems
            .iter()
            .all(|p| p["name"] == "RingIn" || p["name"] == "Coaming"),
        "{check}"
    );
}

#[test]
fn the_hull_panels_pass_the_deep_check() {
    // Every feature is built and the deep check finds nothing: its one
    // finding, the intersect of the tips, was the check's own ray casting
    // missing a crossing (docs/design/booleans.md, *The sea kayak's tips*,
    // 2026-10-27; boolean_findings.rs › the_sea_kayak_tips_against_the_deck).
    let (_, failed) = steps("sea_kayak_7.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn the_level_for_the_hull_displacement_is_found() {
    // A regression of 2026-10-27 (r1-consolidate): the waterline was
    // found at 111.7 mm in 10 steps before nothing found below was
    // checked against each freeform face's whole surface.
    let (mut s, failed) = steps("sea_kayak_9.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    let level = s
        .execute(
            "measure.levelFor",
            &json!({"of": "Hull", "volume": "98865474"}),
        )
        .unwrap();
    let offset = level["offset"].as_f64().unwrap();
    assert!((offset - 111.7).abs() < 0.1, "{level}");
    let volume = level["volume"].as_f64().unwrap();
    assert!(
        (volume - 98_865_474.0).abs() <= 1e-9 * 98_865_474.0,
        "{level}"
    );
}

/// `measure.floating` on the hull (R1, 2026-10-28): with the centre of
/// gravity on the vertical through the centre of buoyancy `levelFor`
/// gives upright, started heeled by 4° and trimmed by 1°, it comes back
/// to upright at the very level `levelFor` found.
#[test]
fn the_hull_floats_upright_at_the_level_for_its_displacement() {
    let (mut s, failed) = steps("sea_kayak_9.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    let level = s
        .execute(
            "measure.levelFor",
            &json!({"of": "Hull", "volume": "98865474"}),
        )
        .unwrap();
    let b = level["centroid"].as_array().unwrap();
    let (bx, by, bz) = (
        b[0].as_f64().unwrap(),
        b[1].as_f64().unwrap(),
        b[2].as_f64().unwrap(),
    );
    let found = s
        .execute(
            "measure.floating",
            &json!({"of": "Hull", "volume": "98865474", "centerOfGravity": [bx, by, bz], "heel": 4, "trim": 1}),
        )
        .unwrap();
    let e = &found["equilibria"][0];
    let number = |v: &Value| v.as_f64().unwrap();
    assert!(number(&e["tilt"]).abs() < 1e-5, "{found}");
    let offset = number(&level["offset"]);
    assert!(
        (number(&e["offset"]) - offset).abs() < 1e-5,
        "{offset} {found}"
    );
    assert_eq!(found["floats"], 0, "{found}");
    assert!(e["settled"].as_bool().unwrap(), "{found}");
}

#[test]
fn the_hull_is_cut_round_30_mm() {
    // The boolean refused the hull's cut at 30.28 mm as an invalid body
    // (docs/design/sections-and-levels.md, Open, until 2026-10-27); on
    // main 4d82484c it is measured. Held to the section: the volume below
    // grows by the section's area, (V(30.3) − V(30.2)) / 0.1 within 1e-4
    // of the mean of the sections there.
    let (mut s, failed) = steps("sea_kayak_9.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    let below = |s: &mut Session, offset: &str| {
        let m = s
            .execute("measure.below", &json!({"of": "Hull", "offset": offset}))
            .unwrap_or_else(|e| panic!("{offset}: {e}"));
        (
            m["volume"].as_f64().unwrap(),
            m["section"]["area"].as_f64().unwrap(),
        )
    };
    let (low, low_area) = below(&mut s, "30.2 mm");
    let (at, _) = below(&mut s, "30.28 mm");
    let (high, high_area) = below(&mut s, "30.3 mm");
    assert!(low < at && at < high, "{low} {at} {high}");
    let rate = (high - low) / 0.1;
    let mean = 0.5 * (low_area + high_area);
    assert!((rate - mean).abs() <= 1e-4 * mean, "{rate} for {mean}");
}

#[test]
fn the_hull_panels_have_no_fans_where_their_ends_line_up_with_a_ruling() {
    let (mut s, failed) = steps("sea_kayak_8.steps.json");
    // The four hull panels are built and unfold (U0 to U3); only the deck
    // (Panel4S, sheer to ridge) is refused, its condition turning back
    // before the end of a rail (owed: docs/design/developable.md), and
    // its unfold U4 with it.
    for f in &failed {
        assert!(f.contains("Panel4S") || f.contains("U4"), "{failed:#?}");
    }
    assert!(!failed.is_empty(), "the deck was built: update this test");
    for panel in ["Panel0S", "Panel1S", "Panel2S", "Panel3S"] {
        let edges = s
            .execute("document.edges", &json!({"feature": panel}))
            .unwrap();
        let edges = edges["edges"].as_array().unwrap();
        // One face: two rails and the two rulings at their ends.
        assert_eq!(edges.len(), 4, "{panel}: {edges:#?}");
        for e in edges {
            assert!(
                e["length"].as_f64().unwrap() > 10.0,
                "{panel}: a short edge {e}"
            );
            assert_eq!(
                e["faces"],
                json!([format!(
                    "{}:face(0)",
                    e["name"].as_str().unwrap().split(':').next().unwrap()
                )]),
                "{panel}: {e}"
            );
        }
    }
}

#[test]
fn plates_on_the_long_developable_panels_are_built() {
    let started = std::time::Instant::now();
    let (mut s, failed) = steps("sea_kayak_14.steps.json");
    eprintln!("repro 14 (starboard plates) in {:?}", started.elapsed());
    // The sheer panel (DevTop4S, chine to sheer) was refused by the
    // developable until 2026-10-28 ("the rulings between two found ones
    // cannot be followed": near each end its condition has a second
    // branch that passes the panel's own, and the walk stepped from one to
    // the other). The walk now follows its branch into the fold and closes
    // it with a fan (docs/design/developable.md, Method 4), so the panel is
    // an exact developable of eight faces named by place: the fans at its
    // ends `face(1)` and `face(2)` (its rails' ends do not line up with a
    // ruling), the first run `face(0)`, the fan at the bow's fold (the
    // first from the start) `face(10000000000)`, the run after it
    // `face(110000000000)`, at the stern the fan across the fold
    // `face(20000000001)`, which lands on the other branch short of that
    // branch's own fold, and the fan that closes that one
    // `face(20000000000)` (the first from the end), and the run to the end
    // `face(300000000000)`.
    // Its plate `Top4S` is refused by name only: the Jig plates `face(0)`
    // and names its joints' edges as the edges of that one face
    // (`between[DevTop4S:face(0)](2)` and `(3)`), which it no longer has
    // (expectation changed 2026-10-28, develop-r2; the panel's geometry is
    // held by the kernel's
    // `rails_whose_condition_has_branches_that_pass_each_other_are_closed_by_fans`).
    assert_eq!(failed.len(), 1, "{failed:#?}");
    assert!(
        failed[0].contains("Top4S") && failed[0].contains("has no edge"),
        "{failed:#?}"
    );
    let edges = s
        .execute("document.edges", &json!({"feature": "DevTop4S"}))
        .unwrap();
    for e in edges["edges"].as_array().unwrap() {
        eprintln!(
            "DevTop4S edge {} on {} length {}",
            e["name"], e["faces"], e["length"]
        );
    }
    // The same panels plated square (no joints): the plate's volume is
    // the panel's area times the thickness, less the mean curvature's
    // term (`t² ∫H dA`, under one per cent for a hull's radii of a few
    // hundred millimetres). With its joints the plate is less by what the
    // touch joints along the chines and the ends on the end planes take,
    // a few per cent of a panel about 135 mm wide.
    for panel in ["Dev1S", "Dev2S"] {
        s.execute(
            "feature.plate",
            &json!({"faces": [format!("{panel}:face(0)")], "thickness": "tHull",
                    "side": "in", "name": format!("Square{panel}")}),
        )
        .unwrap_or_else(|e| panic!("{panel}: {e}"));
    }
    let document = s.execute("document.inspect", &json!({})).unwrap();
    let bodies = document["bodies"].as_array().unwrap();
    let body = |name: &str| {
        bodies
            .iter()
            .find(|b| b["name"] == name)
            .unwrap_or_else(|| panic!("no body {name}"))
    };
    let sheer = body("DevTop4S");
    assert_eq!(sheer["valid"], true, "{sheer}");
    assert_eq!(sheer["faces"], 8, "{sheer}");
    assert_eq!(
        sheer["faceNames"],
        json!([
            "F33:face(1)",
            "F33:face(0)",
            "F33:face(10000000000)",
            "F33:face(110000000000)",
            "F33:face(20000000001)",
            "F33:face(20000000000)",
            "F33:face(300000000000)",
            "F33:face(2)"
        ]),
        "{sheer}"
    );
    for (plate, panel) in [("Plank1S", "Dev1S"), ("Plank2S", "Dev2S")] {
        let square = body(&format!("Square{panel}"));
        let (plate, panel) = (body(plate), body(panel));
        assert_eq!(plate["valid"], true, "{plate}");
        assert_eq!(square["valid"], true, "{square}");
        let volume = plate["volume"].as_f64().unwrap();
        let plain = square["volume"].as_f64().unwrap();
        let expected = panel["area"].as_f64().unwrap() * 4.0;
        eprintln!(
            "{}: volume {volume}, square {plain}, area x t {expected}",
            plate["name"]
        );
        assert!(
            (plain - expected).abs() <= 0.01 * expected,
            "{square} against {panel}"
        );
        assert!(
            volume < plain && volume > 0.95 * plain,
            "{plate} against {square}"
        );
    }
}

#[test]
fn plates_that_touch_along_their_chine_share_no_volume() {
    // Two plates on the hull's bottom and side panels, joined by `touch`
    // along their chine: their rims meet along the inner chine edge,
    // which each traces past the chine's ends in its own way. Refused
    // after 119 s ("more evaluations than the cap allows"); now the
    // overlap assertion passes, the interference is a touch, and the
    // intersection is empty.
    let (mut s, failed) = steps("sea_kayak_3.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    let found = s
        .execute(
            "document.interference",
            &json!({"pairs": [["Bottom", "Side"]], "details": false}),
        )
        .unwrap();
    let summary = &found["summary"];
    assert_eq!(summary["touch"], 1, "{found}");
    assert_eq!(summary["overlap"], 0, "{found}");
    assert_eq!(summary["unknown"], 0, "{found}");
    let common = s
        .execute(
            "feature.boolean",
            &json!({"op": "intersect", "target": "Bottom", "tool": "Side", "keepTool": true, "name": "Common"}),
        )
        .unwrap();
    assert_eq!(common["status"], "failed", "{common}");
    assert!(
        common["error"]
            .as_str()
            .unwrap()
            .contains("the result is empty"),
        "{common}"
    );
}

#[test]
fn the_sheer_plate_on_the_hull_face_is_built() {
    let (mut s, failed) = steps("sea_kayak_12.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    // The same face plated square (no joints): with its joints the plate
    // is less by what the touch and under joints and the ends take.
    s.execute(
        "feature.plate",
        &json!({"faces": ["Hull:copy(0)/Half3:copy(1)/Cyl3:side(e3)"], "thickness": "tHull",
                "side": "in", "name": "SquareTop4S"}),
    )
    .unwrap();
    let document = s.execute("document.inspect", &json!({})).unwrap();
    let bodies = document["bodies"].as_array().unwrap();
    let body = |name: &str| {
        bodies
            .iter()
            .find(|b| b["name"] == name)
            .unwrap_or_else(|| panic!("no body {name}"))
    };
    let (plate, square) = (body("Top4S"), body("SquareTop4S"));
    assert_eq!(plate["valid"], true, "{plate}");
    assert_eq!(square["valid"], true, "{square}");
    let (volume, plain) = (
        plate["volume"].as_f64().unwrap(),
        square["volume"].as_f64().unwrap(),
    );
    eprintln!("Top4S: volume {volume}, square {plain}");
    assert!(
        volume < plain && volume > 0.9 * plain,
        "{plate} against {square}"
    );
}

#[test]
fn the_sheer_plate_and_the_deck_over_it_share_no_volume() {
    // `Ov3D` of repro 12: the sheer plate (holes cut) against the deck
    // over it. Refused, "tangent or nearly coincident over too large an
    // area": the sheer plate's inner skin runs a plate's thickness from
    // the deck's rim (which lies on the sheer plate's outer skin) over
    // five metres. Now the intersection is empty, the interference a
    // touch, and the plates' union and difference hold what they should.
    let (mut s, failed) = steps("sea_kayak_12_ov3d.steps.json");
    let failed: Vec<&String> = failed.iter().filter(|f| !f.contains("\"Ov3D\"")).collect();
    assert!(failed.is_empty(), "{failed:#?}");
    let document = s.execute("document.inspect", &json!({})).unwrap();
    let ov = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Ov3D")
        .unwrap();
    assert!(
        ov["error"]
            .as_str()
            .unwrap()
            .contains("the result is empty"),
        "{ov}"
    );
    let found = s
        .execute(
            "document.interference",
            &json!({"pairs": [["Top4Sh", "Deck4Sh"]], "details": false}),
        )
        .unwrap();
    let summary = &found["summary"];
    assert_eq!(summary["overlap"], 0, "{found}");
    assert_eq!(summary["unknown"], 0, "{found}");
    assert_eq!(summary["touch"], 1, "{found}");
    let volume = |s: &mut Session, name: &str| {
        let document = s.execute("document.inspect", &json!({})).unwrap();
        document["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == name)
            .unwrap_or_else(|| panic!("no body {name}"))["volume"]
            .as_f64()
            .unwrap()
    };
    let (top, deck) = (volume(&mut s, "Top4Sh"), volume(&mut s, "Deck4Sh"));
    for (op, name) in [("union", "Both"), ("subtract", "Less")] {
        let copy = format!("{name}Top");
        s.execute(
            "feature.transform",
            &json!({"source": "Top4Sh", "keep": true, "name": copy}),
        )
        .unwrap();
        let made = s
            .execute(
                "feature.boolean",
                &json!({"op": op, "target": copy, "tool": "Deck4Sh", "keepTool": true, "name": name}),
            )
            .unwrap();
        assert_ne!(made["status"], "failed", "{made}");
    }
    let close = |found: f64, expected: f64, what: &str| {
        assert!(
            (found - expected).abs() <= 1e-6 * expected,
            "{what}: {found} against {expected}"
        );
    };
    close(volume(&mut s, "Both"), top + deck, "the union");
    close(volume(&mut s, "Less"), top, "the sheer plate less the deck");
}

#[test]
fn neighbouring_plates_share_no_volume() {
    for file in [
        "sea_kayak_16.steps.json",
        "sea_kayak_17.steps.json",
        "sea_kayak_18.steps.json",
        "sea_kayak_19.steps.json",
    ] {
        let (_, failed) = steps(file);
        assert!(failed.is_empty(), "{file}: {failed:#?}");
    }
}

#[test]
fn the_inner_hull_is_the_half_hull_cut_by_its_mirror_image() {
    let (mut s, failed) = steps("sea_kayak_20.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    assert_eq!(check["ok"], true, "{check}");
    // The same again, keeping the half hull (a copy, the mirror image of
    // its mirror image) and its mirror image: what is left of the half
    // hull outside its mirror image and the inner hull together make the
    // half hull.
    let volume = |s: &mut Session, name: &str| {
        let document = s.execute("document.inspect", &json!({})).unwrap();
        document["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == name)
            .unwrap_or_else(|| panic!("no body {name}: {document}"))["volume"]
            .as_f64()
            .unwrap()
    };
    let inner = volume(&mut s, "InHull");
    let mirrored = |s: &mut Session, source: &str, name: &str| {
        s.execute(
            "feature.mirror",
            &json!({"source": source, "normal": [0, 1, 0], "keep": true, "name": name}),
        )
        .unwrap();
    };
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/kayak_findings/sea_kayak_20.steps.json"),
    )
    .unwrap();
    let all: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    s.execute("file.new", &json!({"units": "mm"})).unwrap();
    for (id, params) in all.iter().take(all.len() - 1) {
        s.execute(id, params).unwrap();
    }
    mirrored(&mut s, "InPort", "Again");
    let half = volume(&mut s, "Again");
    s.execute(
        "feature.boolean",
        &json!({"op": "subtract", "target": "Again", "tool": "InPort", "keepTool": true, "name": "Outside"}),
    )
    .unwrap();
    let outside = volume(&mut s, "Outside");
    eprintln!("half hull {half}, inner hull {inner}, outside {outside}");
    assert!(inner > 0.0 && outside > 0.0, "{inner} {outside}");
    assert!(
        (inner + outside - half).abs() <= 1e-6 * half,
        "{inner} + {outside} against {half}"
    );
}

/// Whether `a` and `b` share no volume, as the Jig asks it (the `overlap`
/// assertion) and as `document.interference` tells it: a touch.
fn touch(s: &mut Session, a: &str, b: &str) {
    let added = s
        .execute(
            "assert.add",
            &json!({"name": format!("{a} and {b} share no volume"), "measure": "overlap", "of": [a, b], "is": false}),
        )
        .unwrap();
    assert_eq!(added["status"], "passed", "{added}");
    let found = s
        .execute(
            "document.interference",
            &json!({"pairs": [[a, b]], "details": false}),
        )
        .unwrap();
    let summary = &found["summary"];
    assert_eq!(summary["unknown"], 0, "{found}");
    assert_eq!(summary["overlap"], 0, "{found}");
    assert_eq!(summary["touch"], 1, "{found}");
}

#[test]
fn the_drilled_sheer_plate_and_deck_touch_at_the_jigs_tolerance() {
    let (mut s, failed) = steps("sea_kayak_21.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    touch(&mut s, "Top4Sh", "Deck4Sh");
}

#[test]
fn the_sheer_plate_and_the_plank_under_it_touch_with_fine_and_full_ends() {
    for file in ["sea_kayak_22.steps.json", "sea_kayak_23.steps.json"] {
        let (mut s, failed) = steps(file);
        assert!(failed.is_empty(), "{file}: {failed:#?}");
        touch(&mut s, "Plank2Sh", "Top4Sh");
    }
}

#[test]
fn the_garboards_of_a_single_chine_hull_touch_at_the_jigs_tolerance() {
    let (mut s, failed) = steps("sea_kayak_24.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    touch(&mut s, "Plank0Sh", "Plank0Ph");
}

#[test]
fn the_holes_of_a_plate_are_measured_to_its_own_rim_in_seconds() {
    let (mut s, failed) = steps("sea_kayak_25.steps.json");
    assert!(failed.is_empty(), "{failed:#?}");
    let started = std::time::Instant::now();
    let found = s
        .execute(
            "measure.distance",
            &json!({"from": {"kind": "face", "where": {"all": [{"in": "Plank0Sh"}, {"hole": true}]}},
                    "to": {"kind": "face", "where": {"all": [{"madeBy": "Plank0S"}, {"name": "*rim(*"}]}}}),
        )
        .unwrap();
    let took = started.elapsed();
    eprintln!("repro 25: {took:?}: {found}");
    // Thirteen minutes before; two seconds in release now. A minute
    // leaves room for a slow, busy machine and a test build.
    assert!(took.as_secs() < 60, "{took:?}");
    // Measured before (release, thirteen minutes): 6.9830674 mm, proven
    // at least 6.9806185 mm; the hole's edge distance is 8 mm less the
    // radius, 1 mm, less what the plate's turn takes.
    let distance = found["distance"].as_f64().unwrap();
    let at_least = found["atLeast"].as_f64().unwrap();
    assert!((distance - 6.983_067).abs() < 1e-4, "{found}");
    assert!(
        at_least <= distance && at_least > distance - 0.01,
        "{found}"
    );
}
