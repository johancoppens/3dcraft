//! Acceptance checks written from the specification (docs/design/plan.md
//! "Units", docs/design/document.md), independently of the crate's own unit tests.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_expr::{Dimension, ErrorKind, Quantity, Unit, VariableTable, evaluate};

fn no_variables(_: &str) -> Option<Quantity> {
    None
}

fn mm(source: &str) -> f64 {
    evaluate(source, no_variables)
        .unwrap()
        .to_unit(Unit::Millimetre)
        .unwrap()
}

#[test]
fn the_examples_of_the_architecture_document() {
    // "10 mm + 1 in is valid, 10 mm + 30 deg is an error".
    assert!((mm("10 mm + 1 in") - 35.4).abs() < 1e-12);
    let error = evaluate("10 mm + 30 deg", no_variables).unwrap_err();
    assert!(
        matches!(error.kind, ErrorKind::DimensionMismatch { .. }),
        "{error}"
    );
    assert!(
        error.to_string().contains("length") && error.to_string().contains("angle"),
        "{error}"
    );

    assert!((mm("25 mm") - 25.0).abs() < 1e-12);
    assert!((mm("1.5e-3 m") - 1.5).abs() < 1e-12);
    assert!((mm("2 * 3 mm") - 6.0).abs() < 1e-12);
    let area = evaluate("10 mm * 2 cm", no_variables).unwrap();
    assert_eq!(area.dimension, Dimension::new(2, 0));
    assert!((area.value - 200.0).abs() < 1e-12);
    assert!((mm("sqrt(9 mm * 4 mm)") - 6.0).abs() < 1e-12);
    assert!(evaluate("sqrt(2 mm)", no_variables).is_err());
    let right_angle = evaluate("atan2(1 mm, 1 mm) * 2", no_variables).unwrap();
    assert!((right_angle.to_unit(Unit::Degree).unwrap() - 90.0).abs() < 1e-12);
    assert!((evaluate("sin(30 deg)", no_variables).unwrap().value - 0.5).abs() < 1e-15);
    assert!(evaluate("1 / 0", no_variables).is_err());
    assert!(evaluate("1 mm / (2 - 2)", no_variables).is_err());
}

#[test]
fn the_bracket_of_the_phase_one_scenario() {
    // docs/roadmap.md: variables drive a part; changing one updates the rest.
    let mut table = VariableTable::new();
    table.set("width", "40 mm").unwrap();
    table.set("height", "2 * wall + 21 mm").unwrap();
    table.set("wall", "thickness").unwrap();
    table.set("thickness", "3 mm").unwrap();
    table.set("hole_d", "width / 8").unwrap();
    table.set("volume", "width * height * thickness").unwrap();

    // `wall` and `height` depend on a variable defined after them.
    let order: Vec<String> = table
        .evaluation_order()
        .iter()
        .map(|n| n.to_string())
        .collect();
    let at = |name: &str| order.iter().position(|n| *n == name).unwrap();
    assert!(
        at("thickness") < at("wall") && at("wall") < at("height") && at("height") < at("volume")
    );

    let values = table.evaluate();
    assert!(values.is_ok());
    let get = |name: &str| values.value(name).unwrap();
    assert!((get("height").to_unit(Unit::Millimetre).unwrap() - 27.0).abs() < 1e-12);
    assert!((get("hole_d").to_unit(Unit::Millimetre).unwrap() - 5.0).abs() < 1e-12);
    assert_eq!(get("volume").dimension, Dimension::new(3, 0));
    assert!((get("volume").value - 40.0 * 27.0 * 3.0).abs() < 1e-9);

    table.set("width", "2 in").unwrap();
    let values = table.evaluate();
    assert!(
        (values
            .value("hole_d")
            .unwrap()
            .to_unit(Unit::Millimetre)
            .unwrap()
            - 6.35)
            .abs()
            < 1e-12
    );
    // The order does not change when a value does.
    assert_eq!(table.evaluation_order(), order);
}

#[test]
fn one_bad_variable_does_not_take_the_others_down() {
    let mut table = VariableTable::new();
    table.set("a", "b + 1 mm").unwrap();
    table.set("b", "a * 2").unwrap();
    table.set("c", "a + 1 mm").unwrap();
    table.set("good", "5 mm").unwrap();
    table.set("typo", "goood * 2").unwrap();
    table.set("mixed", "good + 3 deg").unwrap();
    table.set("fine", "good * 2").unwrap();
    let values = table.evaluate();
    assert!(!values.is_ok());
    for (name, expect_cycle) in [("a", true), ("b", true)] {
        let error = values.get(name).unwrap().as_ref().unwrap_err();
        assert_eq!(
            matches!(error.kind, ErrorKind::Cycle { .. }),
            expect_cycle,
            "{name}: {error}"
        );
        assert!(error.to_string().contains('a') && error.to_string().contains('b'));
    }
    assert!(matches!(
        values.get("c").unwrap().as_ref().unwrap_err().kind,
        ErrorKind::DependencyFailed { .. }
    ));
    assert!(matches!(
        values.get("typo").unwrap().as_ref().unwrap_err().kind,
        ErrorKind::UnknownVariable { .. }
    ));
    assert!(matches!(
        values.get("mixed").unwrap().as_ref().unwrap_err().kind,
        ErrorKind::DimensionMismatch { .. }
    ));
    assert_eq!(values.value("good").unwrap().value, 5.0);
    assert_eq!(values.value("fine").unwrap().value, 10.0);
}

#[test]
fn renaming_rewrites_references_and_nothing_else() {
    let mut table = VariableTable::new();
    table.set("w", "40 mm").unwrap();
    table.set("ww", "w*2").unwrap();
    table.set("x", "  w /2+ww -  max( w ,ww)").unwrap();
    table.rename("w", "width").unwrap();
    assert_eq!(table.get("ww").unwrap().source(), "width*2");
    assert_eq!(
        table.get("x").unwrap().source(),
        "  width /2+ww -  max( width ,ww)"
    );
    assert!(table.get("w").is_none());
    assert!(table.evaluate().is_ok());
    // Reserved and colliding names are refused, and nothing changes.
    for bad in ["mm", "sin", "pi", "ww", "2x", "a b", ""] {
        assert!(table.rename("width", bad).is_err(), "`{bad}`");
    }
    assert_eq!(table.get("ww").unwrap().source(), "width*2");
}

#[test]
fn hostile_input_is_an_error() {
    let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
    for source in [
        "",
        " ",
        "1 +",
        "mm",
        "é",
        "1 💥 2",
        "\u{0}",
        "1e999",
        &"9".repeat(5000),
        deep.as_str(),
    ] {
        assert!(
            evaluate(source, no_variables).is_err(),
            "`{}`",
            source.chars().take(20).collect::<String>()
        );
    }
    // A lookup that returns garbage cannot produce a garbage result.
    assert!(evaluate("x + 1", |_| Some(Quantity::dimensionless(f64::NAN))).is_err());
}
