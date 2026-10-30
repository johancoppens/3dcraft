//! A loft with guides speaks a loft's words: a wrong number of sections or
//! guides is told as sections and guides (not as a network's u and v
//! curves), and a section's `start`, which such a loft cannot honour (its
//! open sections run the way the guides cross them), is refused rather
//! than ignored.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{
    Block, Document, EntityId, Expr, FeatureId, FeatureKind, Loft, LoftSection, Orientation, Rail,
    Units,
};
use craft3d_regen::{Status, regenerate};

fn failure(sections: Vec<LoftSection>, guides: Vec<Rail>) -> String {
    let mut doc = Document::new(Units::default());
    let e = |v: f64| Expr::new(format!("{v}"));
    for name in ["A", "B"] {
        doc.add_feature(
            Some(name),
            FeatureKind::Block(Block {
                position: [e(0.0), e(0.0), e(0.0)],
                size: [e(1.0), e(1.0), e(1.0)],
                orientation: Orientation::default(),
            }),
        )
        .unwrap();
    }
    let id = doc
        .add_feature(
            Some("Guided"),
            FeatureKind::Loft(Loft {
                sections,
                ruled: false,
                closed: false,
                guides,
            }),
        )
        .unwrap();
    let regen = regenerate(&doc);
    let result = regen.features.iter().find(|f| f.id == id).unwrap();
    match &result.status {
        Status::Failed(why) => why.clone(),
        other => panic!("expected a failure, got {other:?}"),
    }
}

fn section(sketch: u64, start: Option<EntityId>) -> LoftSection {
    LoftSection {
        sketch: FeatureId(sketch),
        start,
    }
}

#[test]
fn a_guided_loft_with_one_guide_says_so_in_a_lofts_words() {
    let why = failure(
        vec![section(1, None), section(2, None)],
        vec![Rail::Sketch(FeatureId(1))],
    );
    assert!(
        why.contains("guides") && why.contains("1 guides") && !why.contains("u, v"),
        "{why}"
    );
}

#[test]
fn a_guided_loft_refuses_a_sections_start_instead_of_ignoring_it() {
    let why = failure(
        vec![section(1, None), section(2, Some(EntityId(3)))],
        vec![Rail::Sketch(FeatureId(1)), Rail::Sketch(FeatureId(2))],
    );
    assert!(why.contains("section 2") && why.contains("start"), "{why}");
}
