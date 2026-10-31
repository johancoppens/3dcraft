//! Units after a parenthesis and angles written with `pi` (R1, 2026-10-28):
//! `(360 / n) deg` and `2 * pi rad / n` are angles, a plain number with a
//! bare `pi` where an angle is expected keeps its meaning (degrees) and
//! `document.check` warns about it, and nothing else changes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::run;
use serde_json::{Value, json};

fn session() -> craft3d_engine::Session {
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    s
}

fn plain_angles(s: &mut craft3d_engine::Session) -> Vec<Value> {
    let check = run(s, "document.check", json!({}));
    check["warnings"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter(|w| w["kind"] == "plainAngle")
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// The turn of a unit cube about z, read off its centroid: the cube's
/// centre (1/2, 1/2) turned by the angle.
fn turn_of(s: &mut craft3d_engine::Session, angle: &str) -> f64 {
    run(
        s,
        "feature.block",
        json!({"size": [1, 1, 1], "angle": angle, "name": "T"}),
    );
    let c = run(s, "document.measure", json!({}))["centroid"].clone();
    run(s, "feature.delete", json!({"feature": "T"}));
    let (x, y) = (c[0].as_f64().unwrap(), c[1].as_f64().unwrap());
    // (x, y) = R(angle) (1/2, 1/2): the angle of (x, y) less 45 degrees.
    y.patan2(x) - std::f64::consts::FRAC_PI_4
}

#[test]
fn a_unit_after_a_parenthesis_or_pi_gives_a_plain_number_its_unit() {
    let mut s = session();
    run(&mut s, "variable.set", json!({"name": "n", "expr": "6"}));
    let want = 60f64.to_radians();
    for angle in ["60 deg", "(360 / n) deg", "2 * pi rad / n", "60"] {
        let got = turn_of(&mut s, angle);
        assert!((got - want).abs() < 1e-9, "{angle}: {got} against {want}");
    }
    // `2 * pi / n` is a plain number, so degrees here: 1.047 degrees, kept.
    let small = (2.0 * std::f64::consts::PI / 6.0).to_radians();
    let got = turn_of(&mut s, "2 * pi / n");
    assert!((got - small).abs() < 1e-9, "{got} against {small}");
    // A length after a parenthesis.
    let set = run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "(n + 4) mm"}),
    );
    assert_eq!(set["dimension"], "length", "{set}");
    assert_eq!(set["value"], json!(10.0), "{set}");
    // On a value that has a unit already, an error that says so.
    run(&mut s, "variable.set", json!({"name": "a", "expr": "3 mm"}));
    let wrong = run(
        &mut s,
        "variable.set",
        json!({"name": "b", "expr": "(a + 1 mm) mm"}),
    );
    let error = wrong["error"].as_str().unwrap_or_else(|| panic!("{wrong}"));
    assert!(error.contains("plain number"), "{error}");
}

#[test]
fn a_plain_angle_with_pi_in_it_is_a_warning_of_document_check() {
    let mut s = session();
    run(&mut s, "variable.set", json!({"name": "n", "expr": "6"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "turn", "expr": "2 * pi / n"}),
    );
    assert!(plain_angles(&mut s).is_empty());
    for (angle, warns) in [
        ("2 * pi / n", true),
        ("turn / 2", true),
        ("2 * pi rad / n", false),
        ("360 deg / n", false),
        ("360 / n", false),
        ("asin(0.5)", false),
        ("sin(pi / 6) * 60", false),
    ] {
        run(
            &mut s,
            "feature.block",
            json!({"size": [1, 1, 1], "angle": angle, "name": "B"}),
        );
        let warnings = plain_angles(&mut s);
        assert_eq!(!warnings.is_empty(), warns, "{angle}: {warnings:?}");
        if warns {
            let w = &warnings[0];
            assert_eq!(w["parameter"], "angle", "{w}");
            assert_eq!(w["expr"], angle, "{w}");
            let text = w["warning"].as_str().unwrap();
            assert!(text.contains("2 * pi rad / n"), "{text}");
            assert_eq!(
                angle == "turn / 2",
                text.contains("through `turn`"),
                "{text}"
            );
        }
        run(&mut s, "feature.delete", json!({"feature": "B"}));
    }
    // An angle constraint of a sketch, too.
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let a = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "S", "from": [0, 0], "to": [10, 0]}),
    );
    let b = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "S", "from": [0, 0], "to": [5, 5]}),
    );
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "S", "type": "angle", "entities": [a["entity"], b["entity"]], "value": "pi / 4"}),
    );
    let warnings = plain_angles(&mut s);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0]["parameter"]
            .as_str()
            .unwrap()
            .ends_with(".value")
    );
}
