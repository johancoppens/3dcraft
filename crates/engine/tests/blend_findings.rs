//! Freeform blends that two showcases found refused or wrong, run from
//! their reproducers (a compressor wheel's blades, a lamp shade's cells;
//! docs/design/blends.md › *Blend findings of the impeller and the
//! lamp*): each builds now and passes the deep check, or is refused with
//! the reason that holds.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A session with the steps of a reproducer run (its `document.check`
/// steps left out), and the results of its steps by feature name.
fn replay(steps: &str) -> (Session, Vec<Value>) {
    let steps: Vec<(String, Value)> = serde_json::from_str(steps).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    });
    let mut results = Vec::new();
    // `craft3d-cli run` starts from a new document (at the default
    // tolerance) when the script does not open one.
    if steps.first().is_none_or(|(id, _)| id != "file.new") {
        run(&mut s, "file.new", json!({"units": "mm"}));
    }
    for (id, params) in &steps {
        if id == "document.check" {
            continue;
        }
        results.push(run(&mut s, id, params.clone()));
    }
    (s, results)
}

fn feature(session: &mut Session, name: &str) -> Value {
    let document = run(session, "document.inspect", json!({}));
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}: {document}"))
        .clone()
}

fn deep_ok(session: &mut Session) {
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

/// F3: a spline wall's top rim and the edge where it meets a flat end,
/// rounded together (the side meets the spline at 85.5°: the two fillets
/// reach the end's top edge two hundredths of a millimetre apart).
#[test]
fn the_cap_and_the_end_of_a_spline_wall() {
    let (mut s, _) = replay(include_str!("blend_findings/meet-cap-and-end.json"));
    let wall = feature(&mut s, "Wall");
    assert_eq!(wall["status"], "ok", "{wall}");
    deep_ok(&mut s);
}

/// F3 and F4 on the blades themselves: the leading edge and the tip of
/// the full blade and of the splitter, rounded together. The splitter's
/// four fillets build (they had made an invalid body: the shared curve's
/// end was found a third of a tenth of a millimetre off the other
/// fillet's border); on the full blade, whose end face is a strip 0.6 mm
/// wide meeting the blade's sides at a sharp angle, the two meets at the
/// tip take more than the whole tip edge at 0.12 mm and are refused as
/// too large, and build at 0.08 mm.
#[test]
fn the_leading_edges_and_tips_of_the_blades() {
    let (mut s, _) = replay(include_str!("blend_findings/meet-le-tip-blade.json"));
    let splitter = feature(&mut s, "Splitter");
    assert_eq!(splitter["status"], "ok", "{splitter}");
    let blade = feature(&mut s, "Blade");
    assert_eq!(blade["status"], "failed", "{blade}");
    let error = blade["error"].as_str().unwrap_or_default().to_owned();
    assert!(error.contains("too large"), "{error}");
    run(
        &mut s,
        "variable.set",
        json!({"name": "edgeR", "expr": "0.08 mm"}),
    );
    let blade = feature(&mut s, "Blade");
    assert_eq!(blade["status"], "ok", "{blade}");
    deep_ok(&mut s);
}

/// F2: the top rim of a spline blade whose end is buried in a wall: the
/// fillet runs into the wall's face (met at concave edges) and is cut
/// there.
#[test]
fn a_blade_rim_that_runs_into_a_wall() {
    let (mut s, _) = replay(include_str!("blend_findings/edge-into-wall.json"));
    let top = feature(&mut s, "Top");
    assert_eq!(top["status"], "ok", "{top}");
    deep_ok(&mut s);
}

/// F6: a lofted blade's tip fillet at the default tolerance (1e-6 mm)
/// needs more samples than a dense interpolation took.
#[test]
fn a_blade_tip_at_the_default_tolerance() {
    let (mut s, _) = replay(include_str!(
        "blend_findings/approx-tip-fillet-default-tol.json"
    ));
    let blade = feature(&mut s, "Blade");
    assert_eq!(blade["status"], "ok", "{blade}");
    deep_ok(&mut s);
}

/// F1: the root of a spline blade on a plate, extruded and lofted: the
/// root fillet runs out at the blade's end faces, past the corners of
/// the plate's edges along them (lengthened), and the lofted blade's end
/// face is grown to take it.
#[test]
fn the_root_of_a_blade_on_a_plate() {
    for steps in [
        include_str!("blend_findings/root-plate-extruded-spline.json"),
        include_str!("blend_findings/root-plate-loft.json"),
    ] {
        let (mut s, _) = replay(steps);
        let root = feature(&mut s, "Root");
        assert_eq!(root["status"], "ok", "{root}");
        deep_ok(&mut s);
    }
}

/// The lamp shade's B2: the rim of a closed-spline cell cut through the
/// revolved shade, rounded all round. Where the rim crosses from one piece
/// of the shade to the next, the two are tangent only to about 2e-6 rad
/// (cells 70 and 92, refused or not depending on where the cell's seam
/// lay, since that was judged halfway along the edge between the pieces);
/// cell 22 needed more samples than a dense interpolation took.
#[test]
fn the_rims_of_lamp_shade_cells() {
    for steps in [
        include_str!("blend_findings/lamp-cell22-rim-fillet.json"),
        include_str!("blend_findings/lamp-cell70-rim-fillet.json"),
        include_str!("blend_findings/lamp-cell92-rim-fillet.json"),
    ] {
        let (mut s, _) = replay(steps);
        let round = feature(&mut s, "Round");
        assert_eq!(round["status"], "ok", "{round}");
        deep_ok(&mut s);
    }
}

/// The bike's B7: a tube welded into a tube (one closed edge where two
/// cylinders meet, a fitted spline with continuous tangents only), square
/// and at 35°, a stay into the seat tube, the seat tube into the bottom
/// bracket shell. The planes across such an edge turn unevenly at every
/// knot of its curve; the ring's balls are taken in the planes square to
/// the closed path of their centres.
#[test]
fn welds_of_a_tube_into_a_tube() {
    for steps in [
        include_str!("blend_findings/bike-weld-tee-90.json"),
        include_str!("blend_findings/bike-weld-tee-35.json"),
        include_str!("blend_findings/bike-weld-stay-seat-tube.json"),
        include_str!("blend_findings/bike-weld-seat-tube-bb.json"),
    ] {
        let (mut s, _) = replay(steps);
        let weld = feature(&mut s, "Weld");
        assert_eq!(weld["status"], "ok", "{weld}");
        deep_ok(&mut s);
    }
}

/// The bike's B5: a stay welded to a dropout plate near the weld's size
/// limit. The refinement of the weld's face crept (each pass pushed the
/// steps next to the ones it halved over the bound) and gave up as "could
/// not be approximated"; it builds now, and the same weld on a plain
/// cylinder stay is refused with what stops it, its border running off
/// the stay's wall.
#[test]
fn welds_of_a_stay_to_a_dropout() {
    let (mut s, _) = replay(include_str!("blend_findings/bike-weld-stay-dropout.json"));
    let rear = feature(&mut s, "RearL");
    assert_eq!(rear["status"], "ok", "{rear}");
    deep_ok(&mut s);
    let (mut s, _) = replay(include_str!("blend_findings/bike-weld-cyl-dropout.json"));
    let weld = feature(&mut s, "Weld");
    assert_eq!(weld["status"], "failed", "{weld}");
    let error = weld["error"].as_str().unwrap_or_default().to_owned();
    assert!(error.contains("runs off a face"), "{error}");
}

/// The bike's B2: an oval tube welded to a conical head tube so near the
/// head tube's end that the weld's ball runs past it. Refused as running
/// off the face (the border crosses the end's rim on the cone, which
/// chords of the two curves did not see), no longer an invalid body that
/// could not be meshed.
#[test]
fn a_weld_that_runs_past_the_end_of_the_head_tube() {
    let (mut s, _) = replay(include_str!("blend_findings/bike-weld-oval-head-tube.json"));
    let weld = feature(&mut s, "Weld");
    assert_eq!(weld["status"], "failed", "{weld}");
    let error = weld["error"].as_str().unwrap_or_default().to_owned();
    assert!(error.contains("runs off a face"), "{error}");
}

/// The turbofan's F8: the rim of a spinner whose revolved spline side is
/// two faces (each half of the rim an edge). One half alone goes on
/// smoothly into the other at both ends, so it cannot stop there: refused
/// with that reason (it was "the rolling ball could not be followed"), and
/// with chain the whole rim is built, from 0.5 mm to 2 mm.
#[test]
fn the_rim_of_the_spinner() {
    let steps = include_str!("blend_findings/turbofan-spinner-rim.json");
    let (mut s, _) = replay(steps);
    let spinner = feature(&mut s, "Spinner");
    assert_eq!(spinner["status"], "failed", "{spinner}");
    let error = spinner["error"].as_str().unwrap_or_default().to_owned();
    assert!(error.contains("goes on smoothly"), "{error}");
    for radius in ["0.5 mm", "1 mm", "2 mm"] {
        let mut chained: Vec<(String, Value)> = serde_json::from_str(steps).unwrap();
        let last = chained.last_mut().unwrap();
        last.1["chain"] = json!(true);
        last.1["radius"] = json!(radius);
        let (mut s, _) = replay(&serde_json::to_string(&chained).unwrap());
        let spinner = feature(&mut s, "Spinner");
        assert_eq!(spinner["status"], "ok", "{radius}: {spinner}");
        deep_ok(&mut s);
    }
}
