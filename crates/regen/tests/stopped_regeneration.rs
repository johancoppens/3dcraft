//! A regeneration a host with one thread stops part way
//! (`craft3d_regen::progress::with_stop`) goes on where it stopped the
//! next time, and ends with exactly what one regeneration builds: the
//! same results, bit for bit, every feature built once.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{
    Block, Boolean, BooleanOp, Document, Expr, FeatureId, FeatureKind, Orientation, Units,
};
use craft3d_regen::progress::with_stop;
use craft3d_regen::{RegenCache, Status, regenerate, regenerate_cached};

fn block(doc: &mut Document, name: &str, x: f64, size: f64) -> FeatureId {
    let e = |v: f64| Expr::new(format!("{v}"));
    doc.add_feature(
        Some(name),
        FeatureKind::Block(Block {
            position: [e(x), e(0.0), e(0.0)],
            size: [e(size), e(size), e(size)],
            orientation: Orientation::default(),
        }),
    )
    .unwrap()
}

/// Blocks along x at scale `s`, each united into the one before it.
fn chain(n: usize, s: f64) -> Document {
    let mut doc = Document::new(Units::default());
    let mut target = block(&mut doc, "B0", 0.0, 10.0 * s);
    for i in 1..n {
        let tool = block(&mut doc, &format!("B{i}"), 8.0 * s * i as f64, 10.0 * s);
        target = doc
            .add_feature(
                Some(&format!("U{i}")),
                FeatureKind::Boolean(Boolean {
                    op: BooleanOp::Union,
                    target,
                    tool,
                    keep_tool: false,
                }),
            )
            .unwrap();
    }
    doc
}

#[test]
fn a_regeneration_stopped_after_every_feature_ends_as_one_that_never_stopped() {
    for s in [1e-3, 1.0, 1e4] {
        let doc = chain(6, s);
        let whole = regenerate(&doc);
        assert!(whole.is_ok(), "{whole:?}");
        let mut cache = RegenCache::default();
        let mut passes = 0;
        let mut built = 0;
        let last = loop {
            passes += 1;
            assert!(passes <= doc.features.len() + 1, "no progress");
            // Stop as soon as it may: after one feature.
            let (regen, stopped) = with_stop(|| true, || regenerate_cached(&doc, &mut cache));
            let (rebuilt, _) = cache.last_run();
            built += rebuilt;
            if !stopped {
                break regen;
            }
            assert_eq!(rebuilt, 1, "one feature per pass at scale {s}");
            // What was not built is rolled back in the answer.
            let done = regen
                .features
                .iter()
                .take_while(|f| f.status != Status::RolledBack)
                .count();
            assert_eq!(done, passes, "scale {s}");
            assert!(
                regen.features[done..]
                    .iter()
                    .all(|f| f.status == Status::RolledBack && f.body.is_none())
            );
        };
        assert_eq!(passes, doc.features.len(), "scale {s}");
        assert_eq!(
            built,
            doc.features.len(),
            "each feature built once, scale {s}"
        );
        assert_eq!(last.features, whole.features, "scale {s}");
        // And the next regeneration reuses all of it.
        let again = regenerate_cached(&doc, &mut cache);
        assert_eq!(cache.last_run().0, 0, "scale {s}");
        assert_eq!(again.features, whole.features, "scale {s}");
    }
}

/// A stop that never says so changes nothing, and without one nothing
/// stops; an edit after a stopped pass is regenerated from what was
/// built, with the old entries of the rest reused where they still fit.
#[test]
fn a_stop_that_is_not_used_changes_nothing_and_a_stopped_pass_keeps_later_entries() {
    let doc = chain(5, 1.0);
    let whole = regenerate(&doc);
    let mut cache = RegenCache::default();
    let (regen, stopped) = with_stop(|| false, || regenerate_cached(&doc, &mut cache));
    assert!(!stopped);
    assert_eq!(regen.features, whole.features);
    // Move the last block: one pass stops after the first rebuilt
    // feature, the next reuses it; the features before it are reused
    // throughout.
    let mut moved = doc.clone();
    let last = moved.features.len() - 2;
    if let FeatureKind::Block(b) = &mut moved.features[last].kind {
        b.position[1] = Expr::new("1");
    } else {
        panic!("not the last block");
    }
    let fresh = regenerate(&moved);
    let (_, stopped) = with_stop(|| true, || regenerate_cached(&moved, &mut cache));
    assert!(stopped);
    assert_eq!(cache.last_run().0, 1);
    let (regen, stopped) = with_stop(|| true, || regenerate_cached(&moved, &mut cache));
    assert!(!stopped, "the last union is the only one left");
    assert_eq!(cache.last_run().0, 1);
    assert_eq!(regen.features, fresh.features);
}
