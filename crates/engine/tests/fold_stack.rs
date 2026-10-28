//! A stack folded on through the commands (docs/design/origami.md › *A
//! stack folded on*): a square folded in half by a valley at 180 degrees,
//! the doubled layers folded on by a crossing crease (a valley on the
//! half that stays face up, a mountain solved on the other); fold.inspect
//! says it is stacked, not flat, with the layers of each stack; the body
//! keeps the area and reaches up to c sin b; saved and reopened, the same.
//! At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{
    assert_close, body, built, deep, failed, new_document, run, session_in, workdir,
};
use serde_json::json;

#[test]
fn a_square_folded_in_half_is_folded_again_across() {
    for k in [1e-3, 1.0, 1e6] {
        let dir = workdir("fold_stack", &format!("half-{k}"));
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
        let line = |s: &mut craft3d_engine::Session, a: [f64; 2], b: [f64; 2]| {
            run(
                s,
                "sketch.line",
                json!({"sketch": "Paper", "from": a, "to": b}),
            )["entity"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let half = line(&mut s, [0.0, -c], [0.0, c]);
        let left = line(&mut s, [-c, 0.0], [0.0, 0.0]);
        let right = line(&mut s, [0.0, 0.0], [c, 0.0]);
        run(
            &mut s,
            "variable.set",
            json!({"name": "b", "expr": "90 deg"}),
        );
        built(
            &mut s,
            "feature.fold",
            json!({"sketch": "Paper", "name": "Stack", "fixed": [-c / 2.0, -c / 2.0], "creases": [
                {"line": half, "kind": "valley", "angle": "180 deg"},
                {"line": left, "kind": "valley", "angle": "b"},
                {"line": right, "kind": "mountain"}]}),
        );
        for degrees in [90.0f64, 40.0] {
            run(
                &mut s,
                "variable.set",
                json!({"name": "b", "expr": format!("{degrees} deg")}),
            );
            let inspect = run(&mut s, "fold.inspect", json!({"fold": "Stack"}));
            assert_eq!(inspect["ok"], true, "{inspect}");
            assert_eq!(inspect["stacked"], true, "{inspect}");
            assert_eq!(inspect["flat"], false, "{inspect}");
            assert_eq!(inspect["layers"]["count"], 2, "{inspect}");
            let found = body(&mut s, "Stack");
            assert_eq!(found["valid"], true, "{found}");
            assert_close(&found["area"], 4.0 * c * c, "area");
            assert_close(
                &found["bbox"][1][2],
                c * degrees.to_radians().sin(),
                "height",
            );
            deep(&mut s);
        }
        run(&mut s, "file.save", json!({"path": "stack.3dcraft"}));
        let before = body(&mut s, "Stack");
        let mut r = session_in(&dir);
        run(&mut r, "file.open", json!({"path": "stack.3dcraft"}));
        assert_eq!(before["bbox"], body(&mut r, "Stack")["bbox"]);
        // The crossing crease a valley on both halves: the half that lies
        // face over would have to turn against its kind.
        let why = failed(
            &mut s,
            "fold.edit",
            json!({"fold": "Stack", "set": [{"line": right, "kind": "valley"}]}),
        );
        assert!(
            why.contains("the valley e") && why.contains("other way"),
            "{why}"
        );
    }
}
