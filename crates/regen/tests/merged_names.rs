//! Names of faces that a boolean, or the union that joins the touching
//! instances of a pattern, merged from faces of several operands: the face
//! is named after the first contributor in the kernel's order and answers
//! to the names of the others as aliases (docs/architecture.md §4.1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{
    Block, Boolean, BooleanOp, Document, Expr, FeatureId, FeatureKind, Orientation, Pattern,
    PatternLayout, PersistentName, Units,
};
use craft3d_regen::{Names, Regen, Status, faces_named, regenerate};
use craft3d_topo::FaceId;

fn exprs(values: [&str; 3]) -> [Expr; 3] {
    values.map(Expr::new)
}

fn block(doc: &mut Document, name: &str, position: [&str; 3], size: [&str; 3]) -> FeatureId {
    doc.add_feature(
        Some(name),
        FeatureKind::Block(Block {
            position: exprs(position),
            size: exprs(size),
            orientation: Orientation::default(),
        }),
    )
    .unwrap()
}

fn name(text: &str) -> PersistentName {
    text.parse().unwrap()
}

fn names_of(regen: &Regen, id: FeatureId) -> Names {
    let outcome = regen.feature(id).unwrap();
    assert_eq!(outcome.status, Status::Ok, "{:?}", outcome.status);
    (**outcome.names.as_ref().unwrap()).clone()
}

fn texts(names: &[PersistentName]) -> Vec<String> {
    names.iter().map(ToString::to_string).collect()
}

/// A plate of 40 x 20 x 5 at the origin and a wing of 30 x 20 x 5 whose
/// low corner is at (`x`, 0, 0), united.
fn united(x: &str) -> (Document, FeatureId) {
    let mut doc = Document::new(Units::default());
    let plate = block(&mut doc, "Plate", ["0", "0", "0"], ["40", "20", "5"]);
    let wing = block(&mut doc, "Wing", [x, "0", "0"], ["30", "20", "5"]);
    let joint = doc
        .add_feature(
            Some("Joint"),
            FeatureKind::Boolean(Boolean {
                op: BooleanOp::Union,
                target: plate,
                tool: wing,
                keep_tool: false,
            }),
        )
        .unwrap();
    (doc, joint)
}

#[test]
fn a_face_merged_from_both_operands_answers_to_both_names() {
    let (doc, joint) = united("40");
    let regen = regenerate(&doc);
    assert!(regen.is_ok(), "{regen:?}");
    let names = names_of(&regen, joint);
    // The wing's left face and the plate's right face lay on each other
    // and are gone; the bottoms, tops, fronts and backs continue each
    // other and are one face each: a box of six faces.
    assert_eq!(names.faces.len(), 6, "{:?}", texts(&names.faces));
    for (plate_face, wing_face) in [(0, 0), (1, 1), (2, 2), (3, 3)] {
        let own = name(&format!("F3:copy(0)/F1:face({plate_face})"));
        let alias = name(&format!("F3:copy(1)/F2:face({wing_face})"));
        let by_own = faces_named(&names, &own);
        let by_alias = faces_named(&names, &alias);
        assert_eq!(by_own.len(), 1, "{own}: {by_own:?}");
        assert_eq!(by_own, by_alias, "{own} and {alias} are one face");
        let face = by_own[0].0 as usize;
        assert_eq!(names.faces[face], own, "named after the target's face");
        assert_eq!(names.aliases_of(face), &[alias.clone()][..]);
        assert_eq!(
            names.names_of(face).cloned().collect::<Vec<_>>(),
            [own.clone(), alias.clone()]
        );
        assert!(names.face_answers_to(face, &own) && names.face_answers_to(face, &alias));
        // An alias is not listed as a face's own name.
        assert!(!names.faces.contains(&alias), "{:?}", texts(&names.faces));
    }
    // The plate's left and the wing's right are each their own, with no
    // alias; the faces that lay on each other are gone.
    for (text, there) in [
        ("F3:copy(0)/F1:face(4)", true),
        ("F3:copy(1)/F2:face(5)", true),
        ("F3:copy(0)/F1:face(5)", false),
        ("F3:copy(1)/F2:face(4)", false),
    ] {
        let found = faces_named(&names, &name(text));
        assert_eq!(found.len(), usize::from(there), "{text}: {found:?}");
        if let Some(FaceId(face)) = found.first() {
            assert!(names.aliases_of(*face as usize).is_empty(), "{text}");
        }
    }
    assert_eq!(names.aliases.len(), 4);
    assert_eq!(
        names.all_face_names().count(),
        10,
        "six faces, four of them with one alias"
    );
    // Deterministic, down to the order of the aliases.
    assert_eq!(names_of(&regenerate(&doc), joint), names);
}

#[test]
fn faces_that_do_not_merge_have_no_aliases() {
    // Apart: two shells, every face its own.
    let (doc, joint) = united("50");
    let regen = regenerate(&doc);
    let names = names_of(&regen, joint);
    assert_eq!(names.faces.len(), 12);
    assert!(names.aliases.is_empty(), "{:?}", names.aliases);
    // Overlapping: the tops still continue each other and merge, but the
    // wing's bottom lies in the plate's and is covered where they
    // overlap; what is left of it is a piece of the wing's bottom only.
    let (doc, joint) = united("30");
    let regen = regenerate(&doc);
    let names = names_of(&regen, joint);
    let top = faces_named(&names, &name("F3:copy(1)/F2:face(1)"));
    assert_eq!(top.len(), 1, "{top:?}");
    assert_eq!(
        names.faces[top[0].0 as usize],
        name("F3:copy(0)/F1:face(1)")
    );
    assert_eq!(
        texts(names.aliases_of(top[0].0 as usize)),
        ["F3:copy(1)/F2:face(1)"]
    );
}

#[test]
fn a_joined_pattern_names_a_merged_cap_after_every_instance() {
    let mut doc = Document::new(Units::default());
    let tile = block(&mut doc, "Tile", ["0", "0", "0"], ["10", "20", "5"]);
    let row = doc
        .add_feature(
            Some("Row"),
            FeatureKind::Pattern(Pattern {
                source: tile,
                count: Expr::new("3"),
                layout: PatternLayout::Linear {
                    step: exprs(["10", "0", "0"]),
                },
            }),
        )
        .unwrap();
    let regen = regenerate(&doc);
    assert!(regen.is_ok(), "{regen:?}");
    let names = names_of(&regen, row);
    assert_eq!(names.faces.len(), 6, "{:?}", texts(&names.faces));
    let top = faces_named(&names, &name("F2:copy(2)/F1:face(1)"));
    assert_eq!(top, faces_named(&names, &name("F2:copy(0)/F1:face(1)")));
    assert_eq!(top.len(), 1);
    let face = top[0].0 as usize;
    assert_eq!(names.faces[face], name("F2:copy(0)/F1:face(1)"));
    // In the order of the instances, each once.
    assert_eq!(
        texts(names.aliases_of(face)),
        ["F2:copy(1)/F1:face(1)", "F2:copy(2)/F1:face(1)"]
    );
    // The left end is the first instance's alone, the right end the last's.
    assert!(
        names
            .aliases_of(faces_named(&names, &name("F2:copy(0)/F1:face(4)"))[0].0 as usize)
            .is_empty()
    );
    assert_eq!(faces_named(&names, &name("F2:copy(2)/F1:face(5)")).len(), 1);
    assert!(faces_named(&names, &name("F2:copy(1)/F1:face(5)")).is_empty());
    // Apart: no union, no aliases, and the edges keep their names.
    if let FeatureKind::Pattern(pattern) = &mut doc.feature_mut(row).unwrap().kind {
        pattern.layout = PatternLayout::Linear {
            step: exprs(["14", "0", "0"]),
        };
    }
    let names = names_of(&regenerate(&doc), row);
    assert_eq!(names.faces.len(), 18);
    assert!(names.aliases.is_empty());
    assert_eq!(names.edges.len(), 36);
}
