//! A screw pattern: a circular pattern whose instances also climb along
//! the axis they turn about (`PatternLayout::Circular::step`). Instance k
//! is the source turned by k times the angle and moved by k times the step
//! along the axis; the two commute. Held to closed forms: where each
//! instance's centroid lands on the helix, the volume, and that a pattern
//! without a step (or with one within the tolerance) is the plain turn it
//! was, map for map.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{
    AxisBy, Block, DatumAxis, Document, Expr, FeatureId, FeatureKind, Pattern, PatternLayout,
    PointRef, Units,
};
use craft3d_math::Vec3;
use craft3d_regen::{Regen, Status, regenerate};
use craft3d_topo::mass_properties;

fn exprs(values: [&str; 3]) -> [Expr; 3] {
    values.map(Expr::new)
}

/// A block 20 x 10 x 4 (volume 800) with its low corner at (30, -5, 0),
/// so its centroid is (40, 0, 2); and a datum axis along +z through
/// (1, 2, 0).
fn block_and_axis() -> (Document, FeatureId, FeatureId) {
    let mut doc = Document::new(Units::default());
    let block = doc
        .add_feature(
            Some("Tread"),
            FeatureKind::Block(Block {
                position: exprs(["30", "-5", "0"]),
                size: exprs(["20", "10", "4"]),
                orientation: Default::default(),
            }),
        )
        .unwrap();
    let axis = doc
        .add_feature(
            Some("Post"),
            FeatureKind::DatumAxis(DatumAxis {
                by: AxisBy::TwoPoints {
                    points: [
                        PointRef::At {
                            position: exprs(["1", "2", "0"]),
                        },
                        PointRef::At {
                            position: exprs(["1", "2", "7"]),
                        },
                    ],
                },
            }),
        )
        .unwrap();
    (doc, block, axis)
}

fn screw(
    source: FeatureId,
    about: Option<FeatureId>,
    count: &str,
    angle: &str,
    step: Option<&str>,
) -> FeatureKind {
    FeatureKind::Pattern(Pattern {
        source,
        count: Expr::new(count),
        layout: PatternLayout::Circular {
            point: exprs(["1", "2", "0"]),
            axis: exprs(["0", "0", "1"]),
            angle: Expr::new(angle),
            about,
            step: step.map(Expr::new),
        },
    })
}

/// The centroid of the block turned by `turns` degrees about the line
/// through (1, 2) along z, `rise` higher.
fn on_helix(turns: f64, rise: f64) -> Vec3 {
    let (s, c) = turns.to_radians().sin_cos();
    let (dx, dy) = (39.0, -2.0);
    Vec3::new(1.0 + c * dx - s * dy, 2.0 + s * dx + c * dy, 2.0 + rise)
}

fn close(a: Vec3, b: Vec3) -> bool {
    (a - b).length() <= 1e-9 * (1.0 + b.length())
}

fn reason(regen: &Regen, id: FeatureId) -> String {
    match &regen.feature(id).unwrap().status {
        Status::Failed(reason) => reason.clone(),
        other => panic!("F{} should fail, it is {other:?}", id.0),
    }
}

#[test]
fn every_instance_of_a_screw_lies_on_the_helix() {
    for about_datum in [false, true] {
        let (mut doc, block, axis) = block_and_axis();
        let about = about_datum.then_some(axis);
        let id = doc
            .add_feature(
                Some("Stair"),
                screw(block, about, "7", "30 deg", Some("25")),
            )
            .unwrap();
        let regen = regenerate(&doc);
        assert!(regen.is_ok(), "{:?}", regen.feature(id).unwrap().status);
        let result = regen.feature(id).unwrap();
        let maps = result.instances.as_ref().unwrap();
        assert_eq!(maps.len(), 7);
        let source = Vec3::new(40.0, 0.0, 2.0);
        let mut expected_centroid = Vec3::ZERO;
        for (k, map) in maps.iter().enumerate() {
            let landed = map.point(source);
            let on = on_helix(30.0 * k as f64, 25.0 * k as f64);
            assert!(
                close(landed, on),
                "instance {k} ({about_datum}): {landed:?}, expected {on:?}"
            );
            expected_centroid += on;
        }
        // The body: seven blocks, apart (25 up and 4 thick), as one body.
        let body = result.body.as_ref().unwrap();
        let mass = mass_properties(body).unwrap();
        assert!(
            (mass.volume - 7.0 * 800.0).abs() <= 1e-9 * 5600.0,
            "{}",
            mass.volume
        );
        let centroid = mass.centroid.unwrap();
        let expected_centroid = expected_centroid * (1.0 / 7.0);
        assert!(
            close(centroid, expected_centroid),
            "{centroid:?} vs {expected_centroid:?}"
        );
        assert_eq!(body.faces.len(), 42);
    }
}

#[test]
fn a_screw_climbs_the_other_way_with_a_negative_step() {
    let (mut doc, block, axis) = block_and_axis();
    let id = doc
        .add_feature(None, screw(block, Some(axis), "4", "90 deg", Some("-10")))
        .unwrap();
    let regen = regenerate(&doc);
    let maps = regen.feature(id).unwrap().instances.clone().unwrap();
    for (k, map) in maps.iter().enumerate() {
        let landed = map.point(Vec3::new(40.0, 0.0, 2.0));
        let on = on_helix(90.0 * k as f64, -10.0 * k as f64);
        assert!(
            close(landed, on),
            "instance {k}: {landed:?}, expected {on:?}"
        );
    }
}

#[test]
fn without_a_step_or_with_a_step_of_zero_it_is_the_plain_turn() {
    let (mut doc, block, _) = block_and_axis();
    let id = doc
        .add_feature(None, screw(block, None, "6", "60 deg", None))
        .unwrap();
    let plain = regenerate(&doc);
    // Debug prints every bit of an f64 (-0.0 and 0.0 apart), so equal
    // text is equal bits.
    let plain_maps = format!("{:?}", plain.feature(id).unwrap().instances);
    let plain_body = format!("{:?}", plain.feature(id).unwrap().body);
    for step in ["0", "0 mm", "0 * 3 mm"] {
        let (mut doc, block, _) = block_and_axis();
        let id = doc
            .add_feature(None, screw(block, None, "6", "60 deg", Some(step)))
            .unwrap();
        let regen = regenerate(&doc);
        assert!(
            regen.is_ok(),
            "{step}: {:?}",
            regen.feature(id).unwrap().status
        );
        let result = regen.feature(id).unwrap();
        assert_eq!(format!("{:?}", result.instances), plain_maps, "{step}");
        assert_eq!(format!("{:?}", result.body), plain_body, "{step}");
    }
}

#[test]
fn a_step_within_the_tolerance_is_refused_not_dropped() {
    // Neither a climb nor nothing: dropping it would show a step in the
    // document that the body does not have. Refused, as the linear
    // pattern refuses a step within the tolerance.
    for step in ["1e-9", "-1e-7 mm", "0.5e-6"] {
        let (mut doc, block, _) = block_and_axis();
        let id = doc
            .add_feature(None, screw(block, None, "6", "60 deg", Some(step)))
            .unwrap();
        let reason = reason(&regenerate(&doc), id);
        for word in ["step", "within the tolerance", "step: null"] {
            assert!(
                reason.contains(word),
                "{step}: `{word}` is not in: {reason}"
            );
        }
    }
}

#[test]
fn a_screw_may_go_past_a_full_turn_but_a_plain_turn_may_not() {
    // Fifteen instances a quarter turn apart: instance k + 4 is right
    // above instance k, 100 higher (four steps of 25), so nothing meets.
    let (mut doc, block, axis) = block_and_axis();
    let id = doc
        .add_feature(None, screw(block, Some(axis), "15", "90 deg", Some("25")))
        .unwrap();
    let regen = regenerate(&doc);
    assert!(regen.is_ok(), "{:?}", regen.feature(id).unwrap().status);
    let body = regen.feature(id).unwrap().body.clone().unwrap();
    let mass = mass_properties(&body).unwrap();
    assert!(
        (mass.volume - 15.0 * 800.0).abs() <= 1e-9 * 12000.0,
        "{}",
        mass.volume
    );
    assert_eq!(body.faces.len(), 90);
    for step in [None, Some("0")] {
        let (mut doc, block, axis) = block_and_axis();
        let id = doc
            .add_feature(None, screw(block, Some(axis), "15", "90 deg", step))
            .unwrap();
        let regen = regenerate(&doc);
        let reason = reason(&regen, id);
        assert!(
            reason.contains("more than a full turn"),
            "{step:?}: {reason}"
        );
    }
}

#[test]
fn a_long_screw_keeps_its_precision() {
    // Two thousand instances a quarter turn each, 500 turns in all: every
    // fourth instance lies right above instance 0, and the angle brought
    // back into one turn keeps it there to the bit of a short screw.
    let (mut doc, block, axis) = block_and_axis();
    let id = doc
        .add_feature(None, screw(block, Some(axis), "2000", "90 deg", Some("25")))
        .unwrap();
    let regen = regenerate(&doc);
    assert!(regen.is_ok(), "{:?}", regen.feature(id).unwrap().status);
    let maps = regen.feature(id).unwrap().instances.clone().unwrap();
    let source = Vec3::new(40.0, 0.0, 2.0);
    for k in (0..2000).step_by(4) {
        let landed = maps[k].point(source);
        let on = Vec3::new(40.0, 0.0, 2.0 + 25.0 * k as f64);
        assert!(
            (landed - on).length() <= 1e-9 * (1.0 + on.length()),
            "instance {k}: {landed:?}, expected {on:?}"
        );
    }
}

#[test]
fn a_step_that_is_not_a_length_fails_the_feature() {
    // (The evaluator never gives an infinite value: `1 / 0` is a division
    // by zero, said at its byte.)
    for (step, words) in [
        ("5 deg", ["step", "length is needed"]),
        ("1 / 0", ["step", "division by zero"]),
    ] {
        let (mut doc, block, _) = block_and_axis();
        let id = doc
            .add_feature(None, screw(block, None, "3", "30 deg", Some(step)))
            .unwrap();
        let reason = reason(&regenerate(&doc), id);
        for word in words {
            assert!(reason.contains(word), "`{word}` is not in: {reason}");
        }
    }
}

/// The volume of three instances of the block, `angle` apart and `step`
/// up, and how many shells the body has.
fn three(angle: &str, step: &str) -> (f64, usize) {
    let (mut doc, block, axis) = block_and_axis();
    let id = doc
        .add_feature(None, screw(block, Some(axis), "3", angle, Some(step)))
        .unwrap();
    let regen = regenerate(&doc);
    assert!(
        regen.is_ok(),
        "{angle} {step}: {:?}",
        regen.feature(id).unwrap().status
    );
    let body = regen.feature(id).unwrap().body.clone().unwrap();
    (mass_properties(&body).unwrap().volume, body.shells.len())
}

#[test]
fn instances_of_a_tight_screw_that_meet_are_joined_as_those_of_any_pattern() {
    // Ten degrees apart the next tread lies partly over the one before.
    // Apart (five up, four thick): three shells side by side. On it
    // (four up): one piece, nothing lost. Into it by one or by two: one
    // piece, and what is lost is the overlap in plan times the depth, so
    // twice as much at twice the depth (neighbours only: instances 0 and
    // 2 are at most touching).
    let (apart, shells) = three("10 deg", "5");
    assert!((apart - 2400.0).abs() <= 1e-9 * 2400.0, "{apart}");
    assert_eq!(shells, 3);
    let (touching, shells) = three("10 deg", "4");
    assert!((touching - 2400.0).abs() <= 1e-9 * 2400.0, "{touching}");
    assert_eq!(shells, 1);
    let (by_one, shells) = three("10 deg", "3");
    assert_eq!(shells, 1);
    let (by_two, shells) = three("10 deg", "2");
    assert_eq!(shells, 1);
    let lost_one = 2400.0 - by_one;
    let lost_two = 2400.0 - by_two;
    assert!(lost_one > 1.0, "{by_one}");
    assert!(
        (lost_two - 2.0 * lost_one).abs() <= 1e-9 * 2400.0,
        "{lost_one} {lost_two}"
    );
}
