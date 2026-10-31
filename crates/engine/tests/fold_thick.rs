//! Thick panels through the commands (docs/design/origami.md › *Thick
//! panels*): `feature.fold` with `panels` builds one solid prism per
//! facet, set back from the creases by the clearance; a crease folded
//! towards the panels is refused past 2 atan(clearance / thickness), with
//! the facets named; saved and reopened, edited back to the sheet; bad
//! panels are errors. At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{
    assert_close, body, built, error, failed, new_document, run, session_in, workdir,
};
use serde_json::json;

#[test]
fn thick_panels_fold_until_the_clearance_runs_out() {
    for k in [1e-3, 1.0, 1e6] {
        let dir = workdir("fold_thick", &format!("hinge-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let c = 10.0 * k;
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Paper"}),
        );
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Paper", "points": [[-c, -c], [c, -c], [c, c], [-c, c]], "closed": true}),
        );
        let line = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Paper", "from": [0.0, -c], "to": [0.0, c]}),
        )["entity"]
            .as_str()
            .unwrap()
            .to_string();
        let (t, gap) = (c / 10.0, c / 20.0);
        let limit = 2.0 * (gap / t).patan().to_degrees();
        let panels = json!({"thickness": format!("{t} mm"), "clearance": format!("{gap} mm")});
        let at = |a: f64| json!([{"line": line, "kind": "mountain", "angle": format!("{a} deg")}]);
        built(
            &mut s,
            "feature.fold",
            json!({"sketch": "Paper", "name": "Panels", "fixed": [-c / 2.0, 0.0],
                   "creases": at(limit - 1.0), "panels": panels}),
        );
        let found = body(&mut s, "Panels");
        assert_eq!(found["kind"], "solid", "{found}");
        assert_eq!(found["valid"], true);
        // Two prisms (c − clearance) × 2c × t.
        assert_close(&found["volume"], 2.0 * (c - gap) * 2.0 * c * t, "volume");
        // Past the limit: the two panels meet, named by their facets.
        let why = failed(
            &mut s,
            "fold.edit",
            json!({"fold": "Panels", "creases": at(limit + 1.0)}),
        );
        assert!(
            why.contains("thick panels") && why.contains("2 atan"),
            "{why}"
        );
        // Panels on the front: a mountain folds away from them.
        built(
            &mut s,
            "fold.edit",
            json!({"fold": "Panels", "panels": {"thickness": format!("{t} mm"), "clearance": format!("{gap} mm"), "side": "front"}}),
        );
        built(
            &mut s,
            "fold.edit",
            json!({"fold": "Panels", "creases": at(150.0)}),
        );
        // Saved and reopened, the same.
        run(&mut s, "file.save", json!({"path": "thick.3dcraft"}));
        let before = body(&mut s, "Panels");
        let mut r = session_in(&dir);
        run(&mut r, "file.open", json!({"path": "thick.3dcraft"}));
        let after = body(&mut r, "Panels");
        assert_eq!(before["bbox"], after["bbox"]);
        assert_eq!(before["faceNames"], after["faceNames"]);
        // Back to the sheet.
        built(
            &mut s,
            "fold.edit",
            json!({"fold": "Panels", "panels": null}),
        );
        assert_eq!(body(&mut s, "Panels")["kind"], "sheet");
        // Bad panels.
        for bad in [
            json!(3),
            json!({"thickness": "1 mm"}),
            json!({"thickness": "1 mm", "clearance": "1 mm", "side": "middle"}),
            json!({"thickness": "1 mm", "clearance": "1 mm", "depth": 2}),
        ] {
            error(
                &mut s,
                "fold.edit",
                json!({"fold": "Panels", "panels": bad}),
            );
        }
        let why = failed(
            &mut s,
            "fold.edit",
            json!({"fold": "Panels", "panels": {"thickness": "0 mm", "clearance": "1 mm"}}),
        );
        assert!(why.contains("thickness"), "{why}");
        let why = failed(
            &mut s,
            "fold.edit",
            json!({"fold": "Panels", "panels": {"thickness": format!("{t} mm"), "clearance": format!("{} mm", 2.0 * c)}}),
        );
        assert!(why.contains("does not fit"), "{why}");
    }
}
