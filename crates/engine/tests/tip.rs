//! `feature.tip` through the commands: the end of a loft closed in one
//! rounded point (docs/design/sweeps.md › *A tip on a body*).
//!
//! The rod is lofted through three equal circles, so its side is a
//! spline surface that is the cylinder exactly; tipped as long as its
//! radius with the default fullness its end is a half ball, and with
//! parabolas (fullness ½) eleven fifteenths of base times length: closed
//! forms (oracle 1 in docs/testing.md) at the scales 1e-3, 1 and 1e6 and
//! in a turned frame, measured by `document.measure` and through the mesh.
//! Every body passes the deep check, exports to STEP and reads back to the
//! same measure, saves and reopens to the same body bit for bit, follows
//! an edit upstream, and keeps every face behind the end as it was.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, deep, failed, failure, new_document, refused, run, workdir};
use craft3d_engine::Session;
use craft3d_geom::Surface;
use craft3d_tess::{TessParams, tessellate_body};
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// A rod of radius `r` and length `h` lofted along z through circles at 0,
/// `h / 2` and `h` (each sketch's circle is e2), named `Rod`.
fn rod(s: &mut Session, r: Value, h: f64) {
    for (index, z) in [0.0, 0.5 * h, h].into_iter().enumerate() {
        let name = format!("S{index}");
        run(
            s,
            "sketch.create",
            json!({"plane": "xy", "offset": z, "name": name}),
        );
        run(
            s,
            "sketch.circle",
            json!({"sketch": name, "center": [0, 0], "radius": r}),
        );
    }
    let made = run(
        s,
        "feature.loft",
        json!({"sections": ["S0", "S1", "S2"], "name": "Rod"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
}

fn volume(s: &mut Session, name: &str) -> f64 {
    body(s, name)["volume"].as_f64().unwrap()
}

#[track_caller]
fn near(got: f64, want: f64, relative: f64, what: &str) {
    assert!(
        (got - want).abs() <= relative * want.abs(),
        "{what}: {got:e} against {want:e} (off by {:e})",
        ((got - want) / want).abs()
    );
}

/// The document of the session regenerated from its bytes.
fn regenerated(s: &Session) -> craft3d_regen::Regen {
    let bytes = s.document_bytes().unwrap();
    let doc = craft3d_format::from_bytes(&bytes).unwrap();
    craft3d_regen::regenerate(&doc)
}

/// The body of the feature named `name`, regenerated from the document's
/// bytes.
fn body_of(s: &Session, name: &str) -> craft3d_topo::Body {
    let bytes = s.document_bytes().unwrap();
    let doc = craft3d_format::from_bytes(&bytes).unwrap();
    let id = doc.features.iter().find(|f| f.name == name).unwrap().id;
    let regen = craft3d_regen::regenerate(&doc);
    let found = regen.features.iter().find(|f| f.id == id).unwrap();
    (**found.body.as_ref().unwrap()).clone()
}

/// The mesh of a body is watertight at two tolerances and holds the
/// volume within what its chords allow.
#[track_caller]
fn meshed(body: &craft3d_topo::Body, volume: f64, area: f64, size: f64, what: &str) {
    for chord in [2e-2, 2e-3] {
        let chord = chord * size;
        let params = TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        };
        let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
        let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert!(welded.is_closed(), "{what}: the mesh is not watertight");
        assert!(
            (welded.volume() - volume).abs() <= area * chord,
            "{what}: mesh volume {} against {volume}",
            welded.volume()
        );
    }
}

#[test]
fn a_rod_tipped_as_long_as_its_radius_ends_in_a_half_ball() {
    for k in SCALES {
        for turned in [false, true] {
            let what = format!("k {k:e}, turned {turned}");
            let (r, h) = (5.0 * k, 20.0 * k);
            let mut s = common::session();
            new_document(&mut s, k);
            rod(&mut s, json!(r), h);
            let mut end = "Rod:endCap(e2)".to_string();
            if turned {
                // Turned and moved away: the tip's face is named as the
                // rod had it and followed to the moved body.
                run(
                    &mut s,
                    "feature.transform",
                    json!({"source": "Rod", "angle": "37 deg", "axis": [1, 2, 2],
                           "translate": [3.0 * k, -2.0 * k, 5.0 * k], "name": "Moved"}),
                );
                end = "Moved:copy(0)/Rod:endCap(e2)".to_string();
            }
            for (length, fullness, share) in [
                (Some(r), None, 2.0 / 3.0),
                // The length chosen: the radius of the circle of the end's
                // area, which never closes in here.
                (None, None, 2.0 / 3.0),
                (Some(r), Some(0.5), 11.0 / 15.0),
            ] {
                let mut params = json!({"face": end, "name": "Nose"});
                if let Some(length) = length {
                    params["length"] = json!(length);
                }
                if let Some(fullness) = fullness {
                    params["fullness"] = json!(fullness);
                }
                let made = run(&mut s, "feature.tip", params);
                assert_eq!(made["status"], "ok", "{what}: {made}");
                let want = PI * r * r * h + share * PI * r * r * r;
                let measured = run(&mut s, "document.measure", json!({"bound": true}));
                let got = measured["volume"].as_f64().unwrap();
                near(got, want, 1e-11, &what);
                // The measure's own bound holds the closed form, a pole
                // and all.
                let bound = measured["volumeBound"].as_f64().unwrap();
                assert!(
                    (got - want).abs() <= bound + 1e-13 * want,
                    "{what}: {got} against {want}, bound {bound}"
                );
                near(volume(&mut s, "Nose"), want, 1e-11, &what);
                deep(&mut s);
                let tipped = body_of(&s, "Nose");
                let area = measured["area"].as_f64().unwrap();
                meshed(&tipped, want, area, r, &what);
                run(&mut s, "edit.undo", json!({}));
            }
        }
    }
}

#[test]
fn the_tip_s_faces_are_named_after_the_sides_they_continue_and_nothing_behind_changes() {
    let mut s = common::session();
    new_document(&mut s, 1.0);
    rod(&mut s, json!(5.0), 20.0);
    let before = body(&mut s, "Rod")["faceNames"].clone();
    let made = run(
        &mut s,
        "feature.tip",
        json!({"face": "Rod:endCap(e2)", "length": 5, "name": "Nose"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let names: Vec<String> = body(&mut s, "Nose")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    let sides: Vec<&str> = before
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .filter(|n| n.contains(":side("))
        .collect();
    assert!(!sides.is_empty(), "{before}");
    for side in &sides {
        assert!(
            names.iter().any(|n| n == &format!("F5:copy(0)/{side}")),
            "{side} kept: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == &format!("F5:tip(0)/{side}")),
            "{side} continued: {names:?}"
        );
    }
    assert!(!names.iter().any(|n| n.contains("endCap")), "{names:?}");
    // Every face of the rod but its end, bit for bit, in its order (the
    // pieces of the circle share their name, as the loft's sides do; so
    // do the faces of the tip that continue them).
    let regen = regenerated(&s);
    let rod = regen.features.iter().find(|f| f.id.0 == 4).unwrap();
    let nose = regen.features.iter().find(|f| f.id.0 == 5).unwrap();
    let (rod_body, nose_body) = (rod.body.as_ref().unwrap(), nose.body.as_ref().unwrap());
    let (rod_names, nose_names) = (rod.names.as_ref().unwrap(), nose.names.as_ref().unwrap());
    let kept: Vec<usize> = (0..rod_body.faces.len())
        .filter(|&i| !rod_names.faces[i].to_string().contains("endCap"))
        .collect();
    assert_eq!(kept.len(), rod_body.faces.len() - 1);
    for (now, &was) in kept.iter().enumerate() {
        let name = &rod_names.faces[was];
        assert_eq!(
            nose_names.faces[now].to_string(),
            format!("F5:copy(0)/{name}")
        );
        assert_eq!(
            nose_body.faces[now].surface, rod_body.faces[was].surface,
            "{name}"
        );
        assert_eq!(
            nose_body.faces[now].reversed, rod_body.faces[was].reversed,
            "{name}"
        );
    }
    assert_eq!(rod_body.edges[..], nose_body.edges[..rod_body.edges.len()]);
    assert_eq!(
        rod_body.vertices[..],
        nose_body.vertices[..rod_body.vertices.len()]
    );
    // After them the faces of the tip, one per edge of the end, each on a
    // spline surface.
    let tips: Vec<usize> = (kept.len()..nose_body.faces.len()).collect();
    assert_eq!(tips.len(), sides.len());
    for &face in &tips {
        assert!(
            nose_names.faces[face]
                .to_string()
                .starts_with("F5:tip(0)/F4:side(")
        );
        assert!(matches!(nose_body.faces[face].surface, Surface::Nurbs(_)));
    }
    // One new vertex (the tip) and one new edge per corner of the end.
    assert_eq!(nose_body.vertices.len(), rod_body.vertices.len() + 1);
    assert_eq!(nose_body.edges.len(), rod_body.edges.len() + tips.len());
}

#[test]
fn a_tipped_rod_exports_to_step_saves_and_reopens_to_the_same_body() {
    let dir = workdir("tip", "files");
    let mut s = common::session_in(&dir);
    new_document(&mut s, 1.0);
    rod(&mut s, json!(5.0), 20.0);
    run(
        &mut s,
        "feature.tip",
        json!({"face": "Rod:endCap(e2)", "name": "Nose"}),
    );
    let want = PI * 25.0 * 20.0 + 2.0 / 3.0 * PI * 125.0;
    near(volume(&mut s, "Nose"), want, 1e-11, "nose");
    // STEP out and in.
    let exported = run(&mut s, "file.export", json!({"path": "nose.step"}));
    assert_eq!(exported["bodies"], 1, "{exported}");
    let stl = run(&mut s, "file.export", json!({"path": "nose.stl"}));
    assert_eq!(stl["closed"], 1, "{stl}");
    let before = body_of(&s, "Nose");
    let inspected = run(&mut s, "document.inspect", json!({}));
    run(&mut s, "file.save", json!({"path": "nose.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "nose.3dcraft"}));
    let after = body_of(&s, "Nose");
    assert_eq!(format!("{before:?}"), format!("{after:?}"));
    assert_eq!(
        run(&mut s, "document.inspect", json!({}))["bodies"],
        inspected["bodies"]
    );
    let mut t = common::session_in(&dir);
    new_document(&mut t, 1.0);
    let imported = run(&mut t, "file.import", json!({"path": "nose.step"}));
    assert_eq!(imported["bodies"][0]["status"], "ok", "{imported}");
    let measured = run(&mut t, "document.measure", json!({}));
    near(measured["volume"].as_f64().unwrap(), want, 1e-9, "imported");
    let check = run(&mut t, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

#[test]
fn the_tip_follows_an_edit_upstream_and_a_stale_name_fails() {
    let mut s = common::session();
    new_document(&mut s, 1.0);
    run(&mut s, "variable.set", json!({"name": "r", "expr": "5 mm"}));
    rod(&mut s, json!("r"), 20.0);
    run(
        &mut s,
        "feature.tip",
        json!({"face": "Rod:endCap(e2)", "length": "r", "name": "Nose"}),
    );
    for r in [5.0, 7.0, 3.0] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": format!("{r} mm")}),
        );
        let want = PI * r * r * 20.0 + 2.0 / 3.0 * PI * r * r * r;
        near(volume(&mut s, "Nose"), want, 1e-11, &format!("r {r}"));
        deep(&mut s);
    }
    // The end is gone into the tip: a second tip of it is not given
    // another face.
    let reason = failed(
        &mut s,
        "feature.tip",
        json!({"face": "Rod:endCap(e2)", "length": 1, "name": "Again"}),
    );
    assert!(reason.contains("endCap"), "{reason}");
    // Back to the length chosen, through feature.edit.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Nose", "length": null, "fullness": 0.5}),
    );
    let want = PI * 9.0 * 20.0 + 11.0 / 15.0 * PI * 27.0;
    near(volume(&mut s, "Nose"), want, 1e-11, "edited");
    // A null alone is a change: automatic again (the place, the fullness).
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Nose", "at": ["0 mm", "0 mm"]}),
    );
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Nose", "at": null}),
    );
    assert_eq!(edited["status"], "ok", "{edited}");
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Nose", "fullness": null}),
    );
    assert_eq!(edited["status"], "ok", "{edited}");
    let want = PI * 9.0 * 20.0 + 2.0 / 3.0 * PI * 27.0;
    near(volume(&mut s, "Nose"), want, 1e-11, "fullness back");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Nose", "length": 2}),
    );
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Nose", "length": null}),
    );
    assert_eq!(edited["status"], "ok", "{edited}");
    near(volume(&mut s, "Nose"), want, 1e-11, "length back");
    // The face by its feature's name, as feature.tip takes it.
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Nose", "face": "Rod:endCap(e2)"}),
    );
    assert_eq!(edited["status"], "ok", "{edited}");
}

#[test]
fn what_cannot_be_tipped_is_refused_with_the_reason() {
    let mut s = common::session();
    // No document yet.
    let text = refused(&mut s, "feature.tip", json!({"face": "F1:face(1)"}));
    assert!(text.contains("document"), "{text}");
    new_document(&mut s, 1.0);
    rod(&mut s, json!(5.0), 20.0);
    for (params, says) in [
        (json!({}), "face"),
        (json!({"face": 5}), "face"),
        (json!({"face": "nonsense"}), "face"),
        (json!({"face": "Rod:between[a,b]"}), "face"),
        (json!({"face": "Rod:endCap(e2)", "at": "x"}), "at"),
        (json!({"face": "Rod:endCap(e2)", "at": [1]}), "at"),
        (json!({"face": "Rod:endCap(e2)", "at": [1, {}]}), "at"),
        (json!({"face": "Rod:endCap(e2)", "length": []}), "length"),
        (
            json!({"face": "Rod:endCap(e2)", "shape": "pointed"}),
            "not built yet",
        ),
        (
            json!({"face": "Rod:endCap(e2)", "shape": "square"}),
            "shape",
        ),
        (json!({"face": "Rod:endCap(e2)", "shape": 3}), "shape"),
        (
            json!({"face": "Rod:endCap(e2)", "extra": 1}),
            "unknown parameter",
        ),
    ] {
        let text = refused(&mut s, "feature.tip", params.clone());
        assert!(text.contains(says), "{params}: {text}");
    }
    // Accepted, and failed by regeneration with the reason.
    for (params, says) in [
        (json!({"face": "Rod:side(e2,e2,e2)"}), "names 4 faces"),
        (json!({"face": "Rod:side(e2)"}), "no longer has a face"),
        (json!({"face": "Rod:endCap(e2)", "length": 0}), "length"),
        (
            json!({"face": "Rod:endCap(e2)", "length": "-1 mm"}),
            "length",
        ),
        (
            json!({"face": "Rod:endCap(e2)", "length": "0 / 0"}),
            "length",
        ),
        (
            json!({"face": "Rod:endCap(e2)", "length": "5 deg"}),
            "length",
        ),
        (json!({"face": "Rod:endCap(e2)", "fullness": 1}), "fullness"),
        (json!({"face": "Rod:endCap(e2)", "fullness": 0}), "fullness"),
        (
            json!({"face": "Rod:endCap(e2)", "fullness": "1 mm"}),
            "fullness",
        ),
        (json!({"face": "Rod:endCap(e2)", "at": [50, 0]}), "tip"),
        (json!({"face": "F9:endCap(e2)"}), "F9"),
    ] {
        let reason = failed(&mut s, "feature.tip", params.clone());
        assert!(reason.contains(says), "{params}: {reason}");
        assert!(!reason.contains("internal error"), "{params}: {reason}");
        run(&mut s, "edit.undo", json!({}));
    }
    // A length far beyond the part: built or refused, never wrong.
    let made = run(
        &mut s,
        "feature.tip",
        json!({"face": "Rod:endCap(e2)", "length": "1e9 mm", "name": "Far"}),
    );
    if made["status"] == "ok" {
        deep(&mut s);
    } else {
        let reason = failure(&mut s, "Far");
        assert!(!reason.contains("internal error"), "{reason}");
    }
    run(&mut s, "edit.undo", json!({}));
    // A block's top: the faces round it are planes, not taken yet.
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "position": [40, 0, 0], "name": "Box"}),
    );
    let reason = failed(&mut s, "feature.tip", json!({"face": "Box:face(1)"}));
    assert!(reason.contains("plane"), "{reason}");
    run(&mut s, "edit.undo", json!({}));
    // A ring: a closed spline round a circle, extruded; its end has a hole.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "offset": 100, "name": "Ring"}),
    );
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Ring", "closed": true,
               "control": [[10, 0], [7, 7], [0, 10], [-7, 7], [-10, 0], [-7, -7], [0, -10], [7, -7]]}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Ring", "center": [0, 0], "radius": 3}),
    );
    let made = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Ring", "distance": 10, "name": "Collar"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let end = body(&mut s, "Collar")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .find(|n| n.contains("endCap"))
        .unwrap();
    let reason = failed(&mut s, "feature.tip", json!({"face": end}));
    assert!(reason.contains("hole"), "{reason}");
}

#[test]
fn a_tip_past_where_the_sides_meet_or_into_another_part_of_the_body_is_refused() {
    let mut s = common::session();
    new_document(&mut s, 1.0);
    // A cone of circles 10, 7.5 and 5 at 0, 10, 20: its side would meet
    // its axis 20 beyond the end.
    for (index, (z, r)) in [(0.0, 10.0), (10.0, 7.5), (20.0, 5.0)]
        .into_iter()
        .enumerate()
    {
        let name = format!("C{index}");
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "offset": z, "name": name}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": name, "center": [0, 0], "radius": r}),
        );
    }
    run(
        &mut s,
        "feature.loft",
        json!({"sections": ["C0", "C1", "C2"], "name": "Cone"}),
    );
    let reason = failed(
        &mut s,
        "feature.tip",
        json!({"face": "Cone:endCap(e2)", "length": 30}),
    );
    assert!(reason.contains("shorter"), "{reason}");
    // With the longest length that fits over the same place, in the
    // document's unit: just short of where the side meets the axis.
    assert!(
        reason.contains("the longest length that fits over this place is 19.99")
            && reason.contains(" mm"),
        "{reason}"
    );
    run(&mut s, "edit.undo", json!({}));
    // The length chosen stays below half of where they meet.
    let made = run(
        &mut s,
        "feature.tip",
        json!({"face": "Cone:endCap(e2)", "name": "Tip"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    deep(&mut s);
    run(&mut s, "edit.undo", json!({}));
    // A block hovering over the end, united with the cone into one body:
    // a tip long enough runs into it.
    run(
        &mut s,
        "feature.block",
        json!({"size": [2, 2, 2], "position": [-1, -1, 22], "name": "Lid"}),
    );
    let united = run(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Cone", "tool": "Lid", "name": "Both"}),
    );
    assert_eq!(united["status"], "ok", "{united}");
    let reason = failed(
        &mut s,
        "feature.tip",
        json!({"face": "Both:copy(0)/Cone:endCap(e2)", "length": 5}),
    );
    assert!(
        reason.contains("passes through itself") || reason.contains("cross"),
        "{reason}"
    );
    run(&mut s, "edit.undo", json!({}));
    // Short of it, the tip is built under the lid.
    let made = run(
        &mut s,
        "feature.tip",
        json!({"face": "Both:copy(0)/Cone:endCap(e2)", "length": 1}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    deep(&mut s);
}

/// The result and `document.inspect` say where the tip stands: the point,
/// the foot on the end and the same in the end face's own axes, and those
/// axes in the model. A place given as a point in the model is taken to
/// the end's plane along its normal, kept in that form through an edit, a
/// save and a reopening; behind the end it fails, saying how far, and an
/// `at` of four values is refused.
#[test]
fn a_tip_says_where_it_stands_and_takes_a_point_in_the_model() {
    let mut s = common::session();
    new_document(&mut s, 1.0);
    rod(&mut s, json!(5.0), 20.0);
    let made = run(
        &mut s,
        "feature.tip",
        json!({"face": "Rod:endCap(e2)", "length": 5, "name": "Nose"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let said = &made["tip"];
    let v = |value: &Value| -> [f64; 3] {
        let a = value.as_array().unwrap();
        [
            a[0].as_f64().unwrap(),
            a[1].as_f64().unwrap(),
            a[2].as_f64().unwrap(),
        ]
    };
    let close = |a: [f64; 3], b: [f64; 3], what: &str| {
        let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
        let off = (dx * dx + dy * dy + dz * dz).sqrt();
        assert!(off <= 1e-12, "{what}: {a:?} against {b:?}");
    };
    assert_eq!(said["placed"], "shoulder", "{said}");
    assert_eq!(said["lengthGiven"], true, "{said}");
    close(v(&said["point"]), [0.0, 0.0, 25.0], "point");
    close(v(&said["foot"]), [0.0, 0.0, 20.0], "foot");
    close(v(&said["frame"]["normal"]), [0.0, 0.0, 1.0], "normal");
    // The foot is `at` along the frame's own axes from its origin.
    let (o, x, y) = (
        v(&said["frame"]["origin"]),
        v(&said["frame"]["x"]),
        v(&said["frame"]["y"]),
    );
    let at = said["at"].as_array().unwrap();
    let (ax, ay) = (at[0].as_f64().unwrap(), at[1].as_f64().unwrap());
    close(
        [
            o[0] + ax * x[0] + ay * y[0],
            o[1] + ax * x[1] + ay * y[1],
            o[2] + ax * x[2] + ay * y[2],
        ],
        v(&said["foot"]),
        "at in the frame",
    );
    let want = PI * 25.0 * 20.0 + 2.0 / 3.0 * PI * 125.0;
    // The point in the model where the tip is to be: the same half ball.
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Nose", "at": ["0 mm", "0 mm", "25 mm"]}),
    );
    assert_eq!(edited["status"], "ok", "{edited}");
    assert_eq!(edited["tip"]["placed"], "givenPoint", "{edited}");
    close(v(&edited["tip"]["point"]), [0.0, 0.0, 25.0], "point given");
    near(volume(&mut s, "Nose"), want, 1e-11, "point given");
    let listed = common::checked::feature(&mut s, "Nose");
    assert_eq!(
        listed["params"]["at"],
        json!(["0 mm", "0 mm", "25 mm"]),
        "{listed}"
    );
    assert_eq!(listed["tip"]["placed"], "givenPoint", "{listed}");
    // Saved and opened again: the same body, bit for bit.
    let before = body_of(&s, "Nose");
    let path = workdir("tip", "point").join("nose.3dcraft");
    run(&mut s, "file.save", json!({"path": path}));
    let mut again = common::session();
    run(&mut again, "file.open", json!({"path": path}));
    assert_eq!(
        format!("{:?}", body_of(&again, "Nose")),
        format!("{before:?}")
    );
    let reopened = common::checked::feature(&mut again, "Nose");
    assert_eq!(reopened["params"]["at"], listed["params"]["at"]);
    // Behind the end: failed, blaming at and saying how far.
    let reason = failed(
        &mut s,
        "feature.edit",
        json!({"feature": "Nose", "at": ["0 mm", "0 mm", "12 mm"]}),
    );
    assert!(
        reason.starts_with("at:") && reason.contains("behind") && reason.contains("8e0"),
        "{reason}"
    );
    run(&mut s, "edit.undo", json!({}));
    // Four values are no place.
    let text = refused(
        &mut s,
        "feature.edit",
        json!({"feature": "Nose", "at": [0, 0, 25, 1]}),
    );
    assert!(text.contains("at"), "{text}");
}

/// The drone's blade (five sections drawn about its stacking line, the
/// line through the sketches' origins, narrowed and turned along it): left
/// to choose, its tip stands on that line, so the point lies on the
/// propeller's radius in the hub's plane; and in a document large enough
/// that `document.inspect` meshes at a tolerance as coarse as the blade's
/// thickness, every body has its box, the tip at its default place and at
/// the one chosen before 2026-11-17 alike (whose mesh at that tolerance
/// was refused, so the blade had none).
#[test]
fn a_blade_s_tip_stands_on_its_stacking_line_and_every_body_has_a_box() {
    let script: Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tip_blade/blade.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for step in script.as_array().unwrap() {
        let id = step[0].as_str().unwrap();
        let out = run(&mut s, id, step[1].clone());
        assert_ne!(out["status"], "failed", "{id}: {out}");
    }
    // A large part beside it: the box is meshed at a thousandth of the
    // side of a square with the bodies' area, here about 1 mm.
    run(
        &mut s,
        "feature.block",
        json!({"size": ["400 mm", "400 mm", "400 mm"], "position": ["-500 mm", 0, 0], "name": "Crate"}),
    );
    let length = "(1 - blade_sections[5].radius_fraction) * prop_diameter / 2 - blade_sections[5].radius_offset";
    let made = run(
        &mut s,
        "feature.tip",
        json!({"face": "Blade loft:endCap(e9)", "length": length, "name": "Blade"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    assert_eq!(made["tip"]["placed"], "taper", "{made}");
    let point: Vec<f64> = made["tip"]["point"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    // The radius 5.1 in / 2 from the hub's axis at (79.77, 79.77), on the
    // stacking line at the hub's mid-height.
    let on_line = [79.77 + 64.77, 79.77, 29.4];
    for (got, want) in point.iter().zip(on_line) {
        assert!((got - want).abs() <= 1e-9, "{point:?} against {on_line:?}");
    }
    run(
        &mut s,
        "assert.add",
        json!({"name": "reach", "measure": "sectionExtent", "of": "Blade", "plane": "zx",
               "offset": "motor_offset", "axis": "x", "eq": "prop_diameter / 2 - 5 mm", "tol": "0.001 mm"}),
    );
    let listed = run(&mut s, "assert.list", json!({"wait": true}));
    assert_eq!(listed["passed"], 1, "{listed}");
    for at in [None, Some(json!(["0.8009 mm", "0.1018 mm"]))] {
        if let Some(at) = &at {
            let edited = run(
                &mut s,
                "feature.edit",
                json!({"feature": "Blade", "at": at}),
            );
            assert_eq!(edited["status"], "ok", "{edited}");
        }
        let document = run(&mut s, "document.inspect", json!({}));
        for found in document["bodies"].as_array().unwrap() {
            assert!(
                found.get("bbox").is_some(),
                "{at:?}: {} has no box",
                found["name"]
            );
            assert!(
                found["bboxTolerance"].as_f64().unwrap() > 0.9,
                "{at:?}: {found}"
            );
        }
    }
    deep(&mut s);
}

#[test]
fn a_tipped_body_is_not_hollowed_or_offset_yet_and_says_why() {
    let mut s = common::session();
    new_document(&mut s, 1.0);
    rod(&mut s, json!(5.0), 20.0);
    run(
        &mut s,
        "feature.tip",
        json!({"face": "Rod:endCap(e2)", "length": 5, "name": "Nose"}),
    );
    let tip_face = "Nose:tip(0)/Rod:side(e2,e2,e2)";
    for (id, params) in [
        ("feature.shell", json!({"source": "Nose", "thickness": 0.5})),
        (
            "feature.shell",
            json!({"source": "Nose", "thickness": 0.5, "open": ["Rod:startCap(e2)"]}),
        ),
        (
            "feature.offsetFace",
            json!({"faces": [tip_face], "distance": 0.5}),
        ),
        (
            "feature.thicken",
            json!({"faces": [tip_face], "thickness": 0.5}),
        ),
        (
            "feature.plate",
            json!({"faces": [tip_face], "thickness": 0.5}),
        ),
    ] {
        let reason = failed(&mut s, id, params.clone());
        assert!(
            reason.contains("tip") && reason.contains("point"),
            "{id} {params}: {reason}"
        );
        run(&mut s, "edit.undo", json!({}));
    }
}
