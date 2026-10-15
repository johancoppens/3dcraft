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
