//! Reference geometry in regeneration, for documents no command builds:
//! a file edited by hand can point a datum at itself, at a datum of
//! another kind, at a body, or two datums at each other. Each is a failed
//! feature with the reason, never a panic or a loop, and regeneration
//! with a cache gives what a full one gives.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{
    AxisBy, AxisRef, Block, DatumAxis, DatumPlane, DatumPoint, Document, Expr, FeatureId,
    FeatureKind, Mirror, Orientation, Pattern, PatternLayout, PlaneBy, PlaneRef, PointRef, Revolve,
    RevolveAxis, Sketch, SketchPlane, Units, WorldPlane,
};
use craft3d_regen::{Datum, RegenCache, Status, regenerate, regenerate_cached};

fn exprs(values: [&str; 3]) -> [Expr; 3] {
    values.map(Expr::new)
}

fn offset(from: PlaneRef) -> FeatureKind {
    FeatureKind::DatumPlane(DatumPlane {
        by: PlaneBy::Offset {
            from,
            distance: Expr::new("1"),
        },
    })
}

fn datum(id: u64) -> PlaneRef {
    PlaneRef::Datum {
        feature: FeatureId(id),
    }
}

fn reason(doc: &Document, id: u64) -> String {
    let regen = regenerate(doc);
    match &regen.feature(FeatureId(id)).unwrap().status {
        Status::Failed(reason) => reason.clone(),
        other => panic!("F{id} should fail, it is {other:?}"),
    }
}

#[test]
fn datums_that_refer_in_circles_fail_and_end() {
    // F1 by F2 and F2 by F1.
    let mut doc = Document::new(Units::default());
    doc.add_feature(None, offset(datum(2))).unwrap();
    doc.add_feature(None, offset(datum(1))).unwrap();
    // Until 2026-10-14 the reason sent the user to feature.move here.
    // In a circle that advice cannot work, whatever the code prints:
    // with F2 moved before F1 it is F2 that needs a later feature. So the
    // reason says that the two are made from each other. (The wording has
    // no oracle but this test: a regression test, not a proof.)
    assert_eq!(
        reason(&doc, 1),
        "from: `Plane2` comes after this feature in the history and is itself made from this feature, directly or through others: the two cannot both be built; point one of them at something else"
    );
    assert_eq!(reason(&doc, 2), "from: `Plane1` failed; fix it first");
    // By itself (no feature comes after itself: the reason says what it
    // is), and by what is not there.
    doc.add_feature(None, offset(datum(3))).unwrap();
    doc.add_feature(None, offset(datum(77))).unwrap();
    assert_eq!(
        reason(&doc, 3),
        "from: `Plane3` is this feature itself; point the reference at something else"
    );
    // Not in a circle, moving it before is the remedy and the reason
    // says so.
    let mut apart = Document::new(Units::default());
    apart.add_feature(None, offset(datum(2))).unwrap();
    apart
        .add_feature(
            None,
            offset(PlaneRef::World {
                plane: WorldPlane::Xy,
            }),
        )
        .unwrap();
    assert_eq!(
        reason(&apart, 1),
        "from: `Plane2` comes after this feature in the history; move it before (feature.move)"
    );
    assert_eq!(
        reason(&doc, 4),
        "from: there is no feature F77 (was the datum plane deleted?)"
    );
    // A long chain, each plane by the one before: every one is placed.
    let mut chain = Document::new(Units::default());
    chain
        .add_feature(
            None,
            offset(PlaneRef::World {
                plane: WorldPlane::Xy,
            }),
        )
        .unwrap();
    for id in 1..200 {
        chain.add_feature(None, offset(datum(id))).unwrap();
    }
    let regen = regenerate(&chain);
    assert!(regen.is_ok());
    let Some(Datum::Plane(last)) = regen.features.last().unwrap().datum else {
        panic!("the last plane has no frame")
    };
    assert_eq!(last.origin().z, 200.0);
    // The same chain the other way round: nothing is placed, nothing
    // hangs.
    chain.features.reverse();
    let regen = regenerate(&chain);
    assert_eq!(
        regen
            .features
            .iter()
            .filter(|f| matches!(f.status, Status::Failed(_)))
            .count(),
        199
    );
}

#[test]
fn a_datum_of_another_kind_or_a_body_is_refused_with_what_it_is() {
    let mut doc = Document::new(Units::default());
    let block = doc
        .add_feature(
            Some("Base"),
            FeatureKind::Block(Block {
                position: exprs(["0", "0", "0"]),
                size: exprs(["4", "4", "4"]),
                orientation: Orientation::default(),
            }),
        )
        .unwrap();
    let plane = doc
        .add_feature(
            Some("Shelf"),
            offset(PlaneRef::World {
                plane: WorldPlane::Xy,
            }),
        )
        .unwrap();
    let point = doc
        .add_feature(
            Some("Dot"),
            FeatureKind::DatumPoint(DatumPoint {
                at: PointRef::At {
                    position: exprs(["1", "2", "3"]),
                },
            }),
        )
        .unwrap();
    let axis = doc
        .add_feature(
            Some("Line"),
            FeatureKind::DatumAxis(DatumAxis {
                by: AxisBy::TwoPoints {
                    points: [
                        PointRef::Datum { feature: point },
                        PointRef::At {
                            position: exprs(["1", "2", "9"]),
                        },
                    ],
                },
            }),
        )
        .unwrap();
    let sketch = doc
        .add_feature(Some("Flat"), FeatureKind::Sketch(Sketch::default()))
        .unwrap();
    assert!(regenerate(&doc).is_ok());
    let cases: Vec<(FeatureKind, &str)> = vec![
        (
            offset(PlaneRef::Datum { feature: axis }),
            "from: `Line` is a datum axis, not a datum plane",
        ),
        (
            offset(PlaneRef::Datum { feature: block }),
            "from: `Base` is a block, not a datum plane",
        ),
        (
            FeatureKind::DatumPlane(DatumPlane {
                by: PlaneBy::Angle {
                    from: PlaneRef::World {
                        plane: WorldPlane::Zx,
                    },
                    about: AxisRef::Datum { feature: plane },
                    angle: Expr::new("10 deg"),
                },
            }),
            "about: `Shelf` is a datum plane, not a datum axis",
        ),
        (
            FeatureKind::DatumPoint(DatumPoint {
                at: PointRef::Datum { feature: axis },
            }),
            "at: `Line` is a datum axis, not a datum point",
        ),
        (
            FeatureKind::DatumAxis(DatumAxis {
                by: AxisBy::TwoPoints {
                    points: [
                        PointRef::Datum { feature: sketch },
                        PointRef::Datum { feature: point },
                    ],
                },
            }),
            "through.0: `Flat` is a sketch, not a datum point",
        ),
        (
            FeatureKind::Sketch(Sketch::new(SketchPlane::Datum {
                plane: point,
                offset: Expr::new("0"),
            })),
            "plane: `Dot` is a datum point, not a datum plane",
        ),
        (
            FeatureKind::Mirror(Mirror {
                source: block,
                point: exprs(["0", "0", "0"]),
                normal: exprs(["0", "0", "1"]),
                plane: Some(axis),
                keep: true,
            }),
            "plane: `Line` is a datum axis, not a datum plane",
        ),
        (
            FeatureKind::Pattern(Pattern {
                source: block,
                count: Expr::new("2"),
                layout: PatternLayout::Circular {
                    point: exprs(["0", "0", "0"]),
                    axis: exprs(["0", "0", "1"]),
                    angle: Expr::new("90 deg"),
                    about: Some(plane),
                },
            }),
            "axis: `Shelf` is a datum plane, not a datum axis",
        ),
        (
            FeatureKind::Revolve(Revolve {
                sketch,
                axis: RevolveAxis::Datum { axis: point },
                angle: Expr::new("360 deg"),
                symmetric: false,
            }),
            "sketch `Flat` has nothing to revolve",
        ),
    ];
    let mut cache = RegenCache::default();
    for (kind, expected) in cases {
        let mut tried = doc.clone();
        let id = tried.add_feature(Some("Tried"), kind).unwrap();
        let regen = regenerate_cached(&tried, &mut cache);
        assert_eq!(regen, regenerate(&tried), "{expected}");
        let Status::Failed(reason) = &regen.feature(id).unwrap().status else {
            panic!("{expected}: it was built")
        };
        assert!(reason.contains(expected), "{reason}");
        // Everything before it is as it was.
        assert!(
            regen
                .features
                .iter()
                .filter(|f| f.id != id)
                .all(|f| f.status == Status::Ok)
        );
    }
}

#[test]
fn placing_datums_is_deterministic_and_cached() {
    let mut doc = Document::new(Units::default());
    doc.set_variable("lift", "3 mm").unwrap();
    doc.add_feature(
        None,
        FeatureKind::Block(Block {
            position: exprs(["0", "0", "0"]),
            size: exprs(["4", "4", "lift"]),
            orientation: Orientation::default(),
        }),
    )
    .unwrap();
    let top = "F1:face(1)".parse().unwrap();
    let shelf = doc
        .add_feature(None, offset(PlaneRef::Face { face: top }))
        .unwrap();
    doc.add_feature(None, offset(datum(shelf.0))).unwrap();
    doc.add_feature(
        None,
        offset(PlaneRef::World {
            plane: WorldPlane::Yz,
        }),
    )
    .unwrap();
    let mut cache = RegenCache::default();
    let first = regenerate_cached(&doc, &mut cache);
    assert_eq!(first, regenerate(&doc));
    assert_eq!(cache.last_run(), (4, 0));
    // Nothing changed: nothing is rebuilt.
    assert_eq!(regenerate_cached(&doc, &mut cache), first);
    assert_eq!(cache.last_run(), (0, 4));
    // The block grows: the block and the two planes that follow its top
    // are rebuilt, the plane beside yz is not.
    doc.set_variable("lift", "5 mm").unwrap();
    let second = regenerate_cached(&doc, &mut cache);
    assert_eq!(second, regenerate(&doc));
    assert_eq!(cache.last_run(), (3, 1));
    let Some(Datum::Plane(frame)) = second.feature(shelf).unwrap().datum else {
        panic!("the shelf has no frame")
    };
    assert_eq!(frame.origin().z, 6.0);
    // A variable nothing uses: nothing is rebuilt.
    doc.set_variable("unused", "1").unwrap();
    regenerate_cached(&doc, &mut cache);
    assert_eq!(cache.last_run(), (0, 4));
}

#[test]
fn the_root_of_a_chain_of_failures_is_the_feature_that_failed_on_its_own() {
    // F1 by a plane that is not there; F2 by F1; F3 by F2; F4 on its own.
    let mut doc = Document::new(Units::default());
    doc.add_feature(None, offset(datum(77))).unwrap();
    doc.add_feature(None, offset(datum(1))).unwrap();
    doc.add_feature(None, offset(datum(2))).unwrap();
    doc.add_feature(
        None,
        offset(PlaneRef::World {
            plane: WorldPlane::Xy,
        }),
    )
    .unwrap();
    let regen = regenerate(&doc);
    let root = |id: u64| craft3d_regen::root_cause(&doc, &regen.features, FeatureId(id));
    assert_eq!(reason(&doc, 3), "from: `Plane2` failed; fix it first");
    assert_eq!(root(1), None, "it failed for a reason of its own");
    assert_eq!(root(2), Some(FeatureId(1)));
    assert_eq!(root(3), Some(FeatureId(1)));
    assert_eq!(root(4), None, "it did not fail");
    assert_eq!(root(99), None, "there is no such feature");
    // Results that are not this document's: no answer, and it ends.
    assert_eq!(
        craft3d_regen::root_cause(
            &Document::new(Units::default()),
            &regen.features,
            FeatureId(3)
        ),
        None
    );
    // In a circle no feature waits for a failed one before it: each link
    // either needs a later feature or the one that does.
    let mut circle = Document::new(Units::default());
    circle.add_feature(None, offset(datum(2))).unwrap();
    circle.add_feature(None, offset(datum(1))).unwrap();
    let regen = regenerate(&circle);
    let root = |id: u64| craft3d_regen::root_cause(&circle, &regen.features, FeatureId(id));
    assert_eq!((root(1), root(2)), (None, Some(FeatureId(1))));
}
