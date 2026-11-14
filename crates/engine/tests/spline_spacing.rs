//! `sketch.spline {through, live, spacing}` (J-034): the parameters at
//! which a live spline takes its points, through the commands. The flank
//! of J-034's reproducer (eight points of an involute, evenly spaced in
//! roll angle) with chord spacing is met by a ray where SciPy's
//! `make_interp_spline` on the textbook parameters and knots (*The NURBS
//! Book*, eqs. 9.4, 9.5, 9.8) meets it, at three scales; the default stays
//! the fit 3DCraft 0.1.0 made (its own `craft3d-cli` gave the same ray hit,
//! 15.161711890000714 mm). Spacing is kept by a save; what it cannot do
//! yet (closed, pointOn and tangent, a spline made once) is refused with
//! the reason and changes nothing; hostile points are refused or fitted,
//! never a panic; and a chord spline exports to STEP, read back to the same
//! body, and to DXF and SVG.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{error, run, session, session_in, workdir};
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};

/// The radius at which a ray at the polar angle of roll 0.57 meets the
/// flank: SciPy's values for the spline through the eight points of the
/// reproducer, Greville and chord spacing, and the involute's own (its
/// radius at roll 0.57, RB sqrt(1 + 0.57²)).
const GREVILLE_HIT: f64 = 15.161711890000712;
const CHORD_HIT: f64 = 15.14294101323072;
const INVOLUTE_HIT: f64 = 15.142773011410139;

/// J-034's flank (one involute flank of a 14-tooth module-2 gear, roll 0
/// to 0.745, eight points), at size `k`, closed to a sector and extruded
/// `k` thick, in a document of tolerance `1e-3 k`: what the extrusion
/// says.
fn flank(s: &mut Session, k: f64, spacing: Option<&str>) -> Value {
    run(s, "file.new", json!({"units": "mm", "tolerance": 1e-3 * k}));
    run(
        s,
        "variable.set",
        json!({"name": "rb", "expr": format!("{} mm", 13.155697 * k)}),
    );
    run(s, "variable.set", json!({"name": "ta", "expr": "0.745"}));
    run(s, "sketch.create", json!({"name": "Flank"}));
    let origin = run(
        s,
        "sketch.point",
        json!({"sketch": "Flank", "at": ["0 mm", "0 mm"]}),
    );
    let through: Vec<Value> = (0..8)
        .map(|i| {
            let t = format!("(ta * {i} / 7)");
            json!([
                format!("rb * (cos({t}) + {t} * sin({t}))"),
                format!("rb * (sin({t}) - {t} * cos({t}))")
            ])
        })
        .collect();
    let mut params = json!({"sketch": "Flank", "through": through, "live": true});
    if let Some(spacing) = spacing {
        params["spacing"] = spacing.into();
    }
    let live = run(s, "sketch.spline", params);
    assert_eq!(live["spacing"], spacing.unwrap_or("greville"), "{live}");
    run(
        s,
        "sketch.line",
        json!({"sketch": "Flank", "from": live["end"], "to": origin["entity"]}),
    );
    run(
        s,
        "sketch.line",
        json!({"sketch": "Flank", "from": origin["entity"], "to": live["start"]}),
    );
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Flank", "distance": format!("{k} mm"), "name": "Tooth"}),
    )
}

/// [`flank`], which must build: the radius at which the ray meets it.
fn flank_hit(s: &mut Session, k: f64, spacing: Option<&str>) -> f64 {
    let made = flank(s, k, spacing);
    assert_eq!(made["status"], "ok", "{made}");
    ray(s, k)
}

fn ray(s: &mut Session, k: f64) -> f64 {
    let hit = run(
        s,
        "measure.ray",
        json!({"from": [0, 0, 0.5 * k], "direction": [0.9986518641523303, 0.05190813255238216, 0]}),
    );
    let hits = hit["hits"].as_array().unwrap();
    hits.iter()
        .map(|h| h["distance"].as_f64().unwrap())
        .fold(0.0, f64::max)
}

#[test]
fn the_flank_of_j034_is_met_where_the_involute_is_with_chord_spacing() {
    for k in [1e-3, 1.0, 1e6] {
        for (spacing, expected) in [
            (None, GREVILLE_HIT),
            (Some("greville"), GREVILLE_HIT),
            (Some("chord"), CHORD_HIT),
        ] {
            let hit = flank_hit(&mut session(), k, spacing);
            let got = hit / k;
            assert!(
                (got - expected).abs() <= 1e-9 * expected,
                "{spacing:?} x{k}: {got} against {expected}"
            );
        }
    }
    // Radially: greville 0.0189 beyond the involute, chord 1.7e-4.
    assert!((GREVILLE_HIT - INVOLUTE_HIT - 0.018939).abs() < 1e-6);
    assert!((CHORD_HIT - INVOLUTE_HIT).abs() < 2e-4);
    // The involute's radius at roll 0.57, worked out here.
    let r = 13.155697 * (1.0 + 0.57 * 0.57_f64).sqrt();
    assert!((r - INVOLUTE_HIT).abs() < 1e-12, "{r}");
}

/// The same flank twice gives the same bits; saved and opened again it
/// keeps its spacing and its shape; `sketch.inspect` says which.
#[test]
fn spacing_is_kept_by_a_save_and_inspect_says_which() {
    let dir = workdir("spline-spacing", "save");
    for spacing in ["greville", "chord"] {
        let mut s = session_in(&dir);
        let hit = flank_hit(&mut s, 1.0, Some(spacing));
        let again = flank_hit(&mut session(), 1.0, Some(spacing));
        assert_eq!(hit.to_bits(), again.to_bits(), "{spacing}");
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Flank"}));
        let live = inspected["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "interpolated")
            .unwrap()
            .clone();
        assert_eq!(live["spacing"], spacing, "{live}");
        // Saved, and opened again in another session.
        let path = format!("flank-{spacing}.3dcraft");
        run(
            &mut s,
            "file.save",
            json!({"path": path, "overwrite": true}),
        );
        let mut other = session_in(&dir);
        run(&mut other, "file.open", json!({"path": path}));
        let reopened = ray(&mut other, 1.0);
        assert_eq!(hit.to_bits(), reopened.to_bits(), "{spacing}");
    }
}

/// What spacing cannot do (yet) is refused with the reason, and a refused
/// command changes nothing.
#[test]
fn what_spacing_cannot_do_is_refused_with_the_reason() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let four = json!([[0, 0], [1, 2], [3, 2], [4, 0]]);
    let before = run(&mut s, "sketch.inspect", json!({"sketch": "S"}));
    for (params, says) in [
        (
            json!({"control": four, "spacing": "chord"}),
            "give `through`",
        ),
        (json!({"through": four, "spacing": "chord"}), "live: true"),
        (
            json!({"through": four, "live": true, "closed": true, "spacing": "chord"}),
            "not supported yet",
        ),
        // Even spacing is not offered (docs/design/document.md › Spacing).
        (
            json!({"through": four, "live": true, "spacing": "even"}),
            "`spacing` is greville or chord, got \"even\"",
        ),
        (
            json!({"through": four, "live": true, "spacing": "centripetal"}),
            "`spacing` is greville or chord",
        ),
        (
            json!({"through": four, "live": true, "spacing": 3}),
            "spacing",
        ),
        (
            json!({"through": [[0, 0], [1, 1], [1, 1], [2, 0]], "live": true, "spacing": "chord"}),
            "points 2 and 3",
        ),
    ] {
        let mut params = params;
        params["sketch"] = "S".into();
        let why = error(&mut s, "sketch.spline", params.clone());
        assert!(why.contains(says), "{params}: {why}");
    }
    assert_eq!(
        run(&mut s, "sketch.inspect", json!({"sketch": "S"})),
        before
    );
}

/// A pointOn or a tangent on a live spline with chord spacing is refused
/// by sketch.constrain, saying what can be done instead, and the document
/// is left as it was: the sketch still solves and its extrusion still
/// builds.
#[test]
fn a_point_on_or_a_tangent_to_a_chord_spline_is_refused_and_changes_nothing() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let chord = run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "S", "through": [[10, 0], [11, 2], [13, 2.5], [14, 0]], "live": true, "spacing": "chord"}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "S", "from": chord["end"], "to": chord["start"]}),
    );
    let made = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "S", "distance": "2 mm", "name": "Hump"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let p = run(
        &mut s,
        "sketch.point",
        json!({"sketch": "S", "at": [12, 2]}),
    );
    let lead = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "S", "from": chord["start"], "to": [9, 3]}),
    );
    let inspect = |s: &mut Session| {
        (
            run(s, "sketch.inspect", json!({"sketch": "S"})),
            run(s, "document.inspect", json!({})),
        )
    };
    let before = inspect(&mut s);
    for (kind, entities) in [
        ("pointOn", json!([p["entity"], chord["entity"]])),
        ("tangent", json!([lead["entity"], chord["entity"]])),
        ("tangent", json!([chord["entity"], lead["entity"]])),
    ] {
        let why = error(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "S", "type": kind, "entities": entities}),
        );
        assert!(
            why.contains("chord spacing")
                && why.contains("make the spline with the default spacing (greville)")
                && !why.contains("even"),
            "{kind}: {why}"
        );
        assert_eq!(inspect(&mut s), before, "{kind}");
    }
}

/// Hostile points for a live spline with chord spacing, as numbers or as
/// formulas that move them later: two in a row at one place, 1e-310 and
/// 1e300 apart, numbers past what a float holds, two points, 400 and 401
/// points. Each is refused with a reason or makes a sketch whose spline
/// is finite; none panics, and a formula that moves two points onto one
/// place fails the sketch naming them.
#[test]
fn hostile_points_for_a_chord_spline_are_refused_or_fitted_never_a_panic() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let many = |n: usize| -> Value {
        (0..n)
            .map(|i| {
                let t = i as f64 * 0.01;
                json!([t, (t * 3.0).psin()])
            })
            .collect()
    };
    let cases = [
        json!([[0, 0], [1, 1], [1, 1], [2, 0]]),
        json!([[0, 0], [1e-310, 0], [2e-310, 1e-310], [3e-310, 0]]),
        json!([[0, 0], [1e300, 0], [2e300, 1e300], [3e300, 0]]),
        json!([[0, 0], [1e308, 0], [-1e308, 1e308], [1e308, 1e308]]),
        json!([["1e308 mm * 10", "0 mm"], [1, 1], [2, 0], [3, 1]]),
        json!([["0 mm / 0", "0 mm"], [1, 1], [2, 0], [3, 1]]),
        json!([[0, 0], [1, 1]]),
        many(400),
        many(401),
    ];
    for (i, through) in cases.into_iter().enumerate() {
        for degree in [1, 3] {
            let made = s.execute(
                "sketch.spline",
                &json!({"sketch": "S", "through": through, "live": true, "spacing": "chord", "degree": degree}),
            );
            if let Ok(made) = made {
                let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "S"}));
                for entity in inspected["entities"].as_array().unwrap() {
                    for key in ["x", "y"] {
                        if let Some(v) = entity.get(key).and_then(Value::as_f64) {
                            assert!(v.is_finite(), "case {i}: {entity}");
                        }
                    }
                }
                run(
                    &mut s,
                    "sketch.delete",
                    json!({"sketch": "S", "entity": made["entity"]}),
                );
            }
        }
    }
    // Two points held by formulas, moved onto one place later.
    run(
        &mut s,
        "variable.set",
        json!({"name": "gap", "expr": "1 mm"}),
    );
    let live = run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "S", "through": [[0, 20], ["5 mm", "20 mm"], ["5 mm + gap", "20 mm + gap"], [8, 20]], "live": true, "spacing": "chord"}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "S", "from": live["end"], "to": live["start"]}),
    );
    let set = s.execute("variable.set", &json!({"name": "gap", "expr": "0 mm"}));
    let said = match set {
        Ok(result) => result.to_string(),
        Err(e) => e.to_string(),
    };
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "S"}));
    assert!(
        inspected.to_string().contains("points 2 and 3") || said.contains("points 2 and 3"),
        "{said} / {inspected}"
    );
}

/// A body whose wall is a chord spline (its knots not uniform) exports to
/// STEP and reads back through 3DCraft's own reader to the same volume and
/// area, within 1e-9; its outline exports to DXF and SVG, each with one
/// spline.
#[test]
fn a_chord_spline_exports_to_step_dxf_and_svg() {
    let dir = workdir("spline-spacing", "export");
    let mut s = session_in(&dir);
    let hit = flank_hit(&mut s, 1.0, Some("chord"));
    assert!((hit - CHORD_HIT).abs() <= 1e-9 * CHORD_HIT);
    let body = |s: &mut Session| {
        let inspected = run(s, "document.inspect", json!({}));
        let bodies = inspected["bodies"].as_array().unwrap().clone();
        assert_eq!(bodies.len(), 1, "{inspected}");
        bodies[0].clone()
    };
    let ours = body(&mut s);
    run(
        &mut s,
        "file.export",
        json!({"path": "flank.step", "format": "step"}),
    );
    for (path, format) in [("flank.dxf", "dxf"), ("flank.svg", "svg")] {
        let written = run(
            &mut s,
            "file.export",
            json!({"path": path, "format": format, "sketch": "Flank"}),
        );
        assert_eq!(written["splines"], 1, "{written}");
        assert_eq!(written["lines"], 2, "{written}");
        assert_eq!(written["closed"], 1, "{written}");
        let text = std::fs::read_to_string(dir.join(path)).unwrap();
        assert!(!text.is_empty());
    }
    let mut t = session_in(&dir);
    run(&mut t, "file.new", json!({"units": "mm"}));
    let imported = run(&mut t, "file.import", json!({"path": "flank.step"}));
    assert_eq!(imported["bodies"][0]["status"], "ok", "{imported}");
    let theirs = body(&mut t);
    for key in ["faces", "edges", "vertices"] {
        assert_eq!(ours[key], theirs[key], "{key}");
    }
    for key in ["volume", "area"] {
        let (a, b) = (ours[key].as_f64().unwrap(), theirs[key].as_f64().unwrap());
        assert!((a - b).abs() <= 1e-9 * a, "{key}: {a} against {b}");
    }
}
