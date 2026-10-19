//! Bodies that went into later features are let go of beyond the cache's
//! budget and derived again when an edit needs them, the same bits
//! (`RegenCache::set_body_budget`, docs/design/regeneration.md).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{
    Block, Boolean, BooleanOp, Document, Expr, FeatureId, FeatureKind, Orientation, Transform,
    Units,
};
use craft3d_regen::{Need, Regen, RegenCache, Status, regenerate, regenerate_cached};

fn exprs(values: [&str; 3]) -> [Expr; 3] {
    values.map(Expr::new)
}

fn block(doc: &mut Document, name: &str, x: &str, size: &str) -> FeatureId {
    doc.add_feature(
        Some(name),
        FeatureKind::Block(Block {
            position: exprs([x, "0", "0"]),
            size: exprs([size, "10", "10"]),
            orientation: Orientation::default(),
        }),
    )
    .unwrap()
}

/// `n` blocks apart along x, united one after the other into a chain
/// (`U1 = B0 ∪ B1`, `U2 = U1 ∪ B2`, …): the blocks, then the unions.
fn chain(n: usize) -> (Document, Vec<FeatureId>, Vec<FeatureId>) {
    let mut doc = Document::new(Units::default());
    let blocks: Vec<FeatureId> = (0..n)
        .map(|i| block(&mut doc, &format!("B{i}"), &format!("{}", 20 * i), "10"))
        .collect();
    let mut unions = Vec::new();
    let mut target = blocks[0];
    for (i, tool) in blocks.iter().enumerate().skip(1) {
        target = doc
            .add_feature(
                Some(&format!("U{i}")),
                FeatureKind::Boolean(Boolean {
                    op: BooleanOp::Union,
                    target,
                    tool: *tool,
                    keep_tool: false,
                }),
            )
            .unwrap();
        unions.push(target);
    }
    (doc, blocks, unions)
}

/// Per feature: its body's faces, edges and vertices, when it is there.
type Bodies = Vec<(FeatureId, Option<(usize, usize, usize)>)>;

/// The bodies of `regen` by feature.
fn bodies(regen: &Regen) -> Bodies {
    regen
        .features
        .iter()
        .map(|f| {
            (
                f.id,
                f.body
                    .as_ref()
                    .map(|b| (b.faces.len(), b.edges.len(), b.vertices.len())),
            )
        })
        .collect()
}

#[test]
fn beyond_the_budget_the_oldest_consumed_bodies_are_let_go_of_and_the_result_is_the_same() {
    let (doc, blocks, unions) = chain(6);
    let full = regenerate(&doc);
    assert!(full.is_ok(), "{full:?}");
    let mut cache = RegenCache::default();
    cache.set_body_budget(0);
    let held = regenerate_cached(&doc, &mut cache);
    // Every body that went into a later feature is gone; the document's
    // own body is there.
    for f in &held.features {
        assert_eq!(f.status, Status::Ok);
        assert_eq!(f.body.is_some(), f.consumed_by.is_none(), "{}", f.id);
    }
    let (bytes, gone) = cache.bodies_held();
    assert_eq!((bytes, gone), (0, blocks.len() + unions.len() - 1));
    // Everything but the bodies let go of is what a full regeneration
    // gives, and so are the bodies when asked for. The names of a body
    // let go of keep their faces (and aliases) and lose their edges and
    // vertices, which are derived from the body.
    let last = *unions.last().unwrap();
    assert_eq!(
        held.feature(last).unwrap().body,
        full.feature(last).unwrap().body
    );
    for (a, b) in held.features.iter().zip(&full.features) {
        assert_eq!(
            (a.id, &a.status, a.consumed_by),
            (b.id, &b.status, b.consumed_by)
        );
        let (a, b) = (a.names.as_deref().unwrap(), b.names.as_deref().unwrap());
        assert_eq!((&a.faces, &a.aliases), (&b.faces, &b.aliases));
        assert_eq!(
            (a.edge_count(), a.vertex_count()),
            (b.edge_count(), b.vertex_count())
        );
        if a.has_entities() {
            assert_eq!(a, b);
        } else {
            assert_eq!(a.edge_name(0), None);
            assert!(
                a.edge_aliases_of(0).is_empty()
                    && a.vertex_named(&b.vertex_name(0).unwrap()).is_none()
            );
        }
    }
    assert!(
        held.feature(last)
            .unwrap()
            .names
            .as_deref()
            .unwrap()
            .has_entities()
    );
    assert!(
        !held
            .feature(unions[0])
            .unwrap()
            .names
            .as_deref()
            .unwrap()
            .has_entities()
    );
    let all = craft3d_regen::regenerate_cached_needing(&doc, &mut cache, Need::All);
    assert_eq!(all, full);
    assert_eq!(cache.last_run_rederived(), gone);
    assert_eq!(cache.bodies_held().1, 0);
    // A budget that holds the newest few keeps exactly those.
    let per_body =
        craft3d_regen::body_bytes(full.feature(blocks[1]).unwrap().body.as_ref().unwrap());
    let last_union =
        craft3d_regen::body_bytes(full.feature(unions[3]).unwrap().body.as_ref().unwrap());
    let mut cache = RegenCache::default();
    cache.set_body_budget(per_body + last_union);
    let held = regenerate_cached(&doc, &mut cache);
    let kept: Vec<FeatureId> = held
        .features
        .iter()
        .filter(|f| f.consumed_by.is_some() && f.body.is_some())
        .map(|f| f.id)
        .collect();
    assert_eq!(kept, vec![blocks[5], unions[3]], "{:?}", bodies(&held));
}

#[test]
fn an_edit_at_the_end_reuses_the_newest_bodies_and_one_further_up_derives_the_rest_again() {
    let (mut doc, blocks, unions) = chain(6);
    let mut cache = RegenCache::default();
    let full = regenerate(&doc);
    let bytes =
        |id: FeatureId| craft3d_regen::body_bytes(full.feature(id).unwrap().body.as_ref().unwrap());
    // Room for the last tool and the union before the last.
    cache.set_body_budget(bytes(blocks[5]) + bytes(unions[3]));
    regenerate_cached(&doc, &mut cache);
    // The last tool changes: the last union is rebuilt from bodies that
    // are there.
    if let FeatureKind::Block(b) = &mut doc.feature_mut(blocks[5]).unwrap().kind {
        b.size = exprs(["12", "10", "10"]);
    }
    let held = regenerate_cached(&doc, &mut cache);
    assert_eq!((cache.last_run(), cache.last_run_rederived()), ((2, 9), 0));
    let full = regenerate(&doc);
    assert_eq!(bodies(&held).iter().filter(|(_, b)| b.is_some()).count(), 3);
    assert_eq!(
        held.feature(*unions.last().unwrap()).unwrap().body,
        full.feature(*unions.last().unwrap()).unwrap().body
    );
    // The second tool changes: the chain from the second union on is
    // rebuilt, and the first union, let go of, is derived again first.
    if let FeatureKind::Block(b) = &mut doc.feature_mut(blocks[1]).unwrap().kind {
        b.size = exprs(["12", "10", "10"]);
    }
    let held = regenerate_cached(&doc, &mut cache);
    let full = regenerate(&doc);
    assert_eq!(
        held.feature(*unions.last().unwrap()).unwrap().body,
        full.feature(*unions.last().unwrap()).unwrap().body
    );
    // B1 and U1..U5 rebuilt; each union needs its tool and U1 its
    // target, blocks whose bodies were let go of: B0, B2, B3 and B4 are
    // derived again, B5 was kept.
    assert_eq!((cache.last_run(), cache.last_run_rederived()), ((6, 5), 4));
    assert_eq!(
        craft3d_regen::regenerate_cached_needing(&doc, &mut cache, Need::All),
        full
    );
    // Nothing changed: nothing is rebuilt or derived again.
    let held = regenerate_cached(&doc, &mut cache);
    assert_eq!((cache.last_run(), cache.last_run_rederived()), ((0, 11), 0));
    assert_eq!(bodies(&held), bodies(&regenerate_cached(&doc, &mut cache)));
}

#[test]
fn a_body_let_go_of_is_the_documents_own_again_when_its_taker_goes() {
    let (mut doc, blocks, unions) = chain(4);
    let mut cache = RegenCache::default();
    cache.set_body_budget(0);
    regenerate_cached(&doc, &mut cache);
    // The last union is deleted: the union before it and the last block
    // are bodies of the document again, derived again.
    doc.remove_feature(*unions.last().unwrap()).unwrap();
    let held = regenerate_cached(&doc, &mut cache);
    let full = regenerate(&doc);
    let own = |regen: &Regen| -> Bodies {
        bodies(regen)
            .into_iter()
            .filter(|(id, _)| regen.feature(*id).unwrap().consumed_by.is_none())
            .collect()
    };
    assert_eq!(own(&held), own(&full));
    // U2 and B3 are derived again, and before U2 everything it is made
    // from that was let go of: B0, B1, U1 and B2.
    assert_eq!(cache.last_run_rederived(), 6);
    let ids: Vec<FeatureId> = held.bodies().map(|(id, _)| id).collect();
    assert_eq!(ids, vec![blocks[3], unions[1]]);
    assert_eq!(
        held.feature(unions[1]).unwrap().body,
        full.feature(unions[1]).unwrap().body
    );
    // Suppressing the taker does the same; undoing it lets go again.
    doc.feature_mut(unions[1]).unwrap().suppressed = true;
    let held = regenerate_cached(&doc, &mut cache);
    assert_eq!(own(&held), own(&regenerate(&doc)));
    assert_eq!(cache.last_run_rederived(), 4);
    doc.feature_mut(unions[1]).unwrap().suppressed = false;
    let held = regenerate_cached(&doc, &mut cache);
    assert_eq!(
        held.feature(unions[1]).unwrap().body,
        full.feature(unions[1]).unwrap().body
    );
    assert!(held.feature(unions[0]).unwrap().body.is_none());
}

/// A source a feature keeps (a union's tool with `keepTool`, a transform
/// with `keep`) that a later feature takes: derived again, the feature
/// is built as the first time, when nothing after it had taken anything.
#[test]
fn a_kept_source_that_a_later_feature_took_is_derived_again_as_the_first_time() {
    let mut doc = Document::new(Units::default());
    let b0 = block(&mut doc, "B0", "0", "10");
    let b1 = block(&mut doc, "B1", "20", "10");
    let b2 = block(&mut doc, "B2", "40", "10");
    // U1 keeps its tool B1; T keeps its source B1; then P takes B1.
    let u1 = doc
        .add_feature(
            Some("U1"),
            FeatureKind::Boolean(Boolean {
                op: BooleanOp::Union,
                target: b0,
                tool: b1,
                keep_tool: true,
            }),
        )
        .unwrap();
    let transform = |source: FeatureId, keep: bool| {
        FeatureKind::Transform(Transform {
            source,
            translate: exprs(["0", "30", "0"]),
            pivot: exprs(["0", "0", "0"]),
            axis: exprs(["0", "0", "1"]),
            angle: Expr::new("0"),
            keep,
        })
    };
    let t = doc.add_feature(Some("T"), transform(b1, true)).unwrap();
    let p = doc.add_feature(Some("P"), transform(b1, false)).unwrap();
    // U2 takes U1; T2 takes T.
    let u2 = doc
        .add_feature(
            Some("U2"),
            FeatureKind::Boolean(Boolean {
                op: BooleanOp::Union,
                target: u1,
                tool: b2,
                keep_tool: false,
            }),
        )
        .unwrap();
    let t2 = doc.add_feature(Some("T2"), transform(t, false)).unwrap();
    let full = regenerate(&doc);
    assert!(full.is_ok(), "{full:?}");
    let mut cache = RegenCache::default();
    cache.set_body_budget(0);
    let held = regenerate_cached(&doc, &mut cache);
    assert_eq!(cache.bodies_held(), (0, 5), "{:?}", bodies(&held));
    // Every body asked for again is the one a full regeneration gives:
    // U1 is built with B1 not yet taken by P, T likewise.
    let all = craft3d_regen::regenerate_cached_needing(&doc, &mut cache, Need::All);
    assert_eq!(all, full);
    for id in [b0, b1, b2, u1, t, p, u2, t2] {
        assert_eq!(all.feature(id).unwrap().status, Status::Ok, "{id}");
    }
    // An edit of U2's tool rebuilds U2, which needs U1, derived again
    // from B0 and B1 (B1 taken by P, which comes after U1).
    let held = regenerate_cached(&doc, &mut cache);
    assert_eq!(cache.bodies_held(), (0, 5), "{:?}", bodies(&held));
    if let FeatureKind::Block(b) = &mut doc.feature_mut(b2).unwrap().kind {
        b.size = exprs(["12", "10", "10"]);
    }
    let held = regenerate_cached(&doc, &mut cache);
    let full = regenerate(&doc);
    assert_eq!(held.feature(u2).unwrap().status, Status::Ok);
    assert_eq!(
        held.feature(u2).unwrap().body,
        full.feature(u2).unwrap().body
    );
    assert_eq!(cache.last_run_rederived(), 3);
    // T2's source T changes: T is rebuilt from B1 (taken by P, later).
    if let FeatureKind::Transform(moved) = &mut doc.feature_mut(t).unwrap().kind {
        moved.translate = exprs(["0", "31", "0"]);
    }
    let held = regenerate_cached(&doc, &mut cache);
    let full = regenerate(&doc);
    assert_eq!(
        held.feature(t2).unwrap().body,
        full.feature(t2).unwrap().body
    );
    assert_eq!(
        craft3d_regen::regenerate_cached_needing(&doc, &mut cache, Need::All),
        full
    );
}
