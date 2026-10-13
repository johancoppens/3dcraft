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
    PatternLayout, PersistentName, Transform, Units,
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

/// What a face, an edge or a vertex answers to besides its own name, it
/// still answers to in a body made from that body: as a copy.
#[test]
fn aliases_follow_the_body_through_a_copy_and_through_another_boolean() {
    // Moved: the merged top of the joint answers to the wing's name for
    // it, as the copy of that name.
    let (mut doc, joint) = united("40");
    let moved = doc
        .add_feature(
            Some("Moved"),
            FeatureKind::Transform(Transform {
                source: joint,
                translate: exprs(["0", "0", "50"]),
                pivot: exprs(["0", "0", "0"]),
                axis: exprs(["0", "0", "1"]),
                angle: Expr::new("0"),
                keep: false,
            }),
        )
        .unwrap();
    let regen = regenerate(&doc);
    assert!(regen.is_ok(), "{regen:?}");
    let names = names_of(&regen, moved);
    let own = name("F4:copy(0)/F3:copy(0)/F1:face(1)");
    let alias = name("F4:copy(0)/F3:copy(1)/F2:face(1)");
    let top = faces_named(&names, &alias);
    assert_eq!(top.len(), 1, "{:?}", names.aliases);
    assert_eq!(top, faces_named(&names, &own));
    assert_eq!(names.aliases_of(top[0].0 as usize), &[alias][..]);
    assert_eq!(names.aliases.len(), 4);
    // The corners and edges of the joint keep, moved, the names the
    // plate and the wing gave them.
    assert_eq!(
        (names.vertices.len(), names.vertex_aliases.len()),
        (8, 8),
        "{:?}",
        names.vertex_aliases
    );
    for (text, at) in [
        ("F4:copy(0)/F3:copy(0)/F1:vertex(0)", [0.0, 0.0, 50.0]),
        ("F4:copy(0)/F3:copy(1)/F2:vertex(7)", [70.0, 20.0, 55.0]),
    ] {
        let index = names.vertex_named(&name(text)).expect(text);
        let body = regen.feature(moved).unwrap().body.as_ref().unwrap();
        let p = body.vertices[index].point;
        assert_eq!([p.x, p.y, p.z], at, "{text}");
        assert!(
            names.vertices[index]
                .to_string()
                .starts_with("F4:copy(0)/F3:corner[")
        );
    }
    assert_eq!(names.edges.len(), 12);
    let edge = names
        .edge_named(&name("F4:copy(0)/F3:copy(1)/F2:edge(5)"))
        .expect("the wing's top right edge");
    assert!(
        names.edges[edge]
            .to_string()
            .starts_with("F4:copy(0)/F3:between[")
    );

    // United with a third block that stands on it: the top is cut round
    // the block's foot and still answers to both names.
    let (mut doc, joint) = united("40");
    let post = block(&mut doc, "Post", ["30", "5", "5"], ["20", "10", "8"]);
    let stand = doc
        .add_feature(
            Some("Stand"),
            FeatureKind::Boolean(Boolean {
                op: BooleanOp::Union,
                target: joint,
                tool: post,
                keep_tool: false,
            }),
        )
        .unwrap();
    let regen = regenerate(&doc);
    assert!(regen.is_ok(), "{regen:?}");
    let names = names_of(&regen, stand);
    let own = name("F5:copy(0)/F3:copy(0)/F1:face(1)");
    let alias = name("F5:copy(0)/F3:copy(1)/F2:face(1)");
    let top = faces_named(&names, &alias);
    assert_eq!(top.len(), 1, "{:?}", names.aliases);
    assert_eq!(top, faces_named(&names, &own));
    assert_eq!(names.faces[top[0].0 as usize], own);
    // A corner of the wing, two booleans on, by the name the wing gave.
    let index = names
        .vertex_named(&name("F5:copy(0)/F3:copy(1)/F2:vertex(7)"))
        .expect("the wing's far corner");
    let body = regen.feature(stand).unwrap().body.as_ref().unwrap();
    let p = body.vertices[index].point;
    assert_eq!([p.x, p.y, p.z], [70.0, 20.0, 5.0]);
    // Its own name is by the faces of this body, and it carries no name
    // by the faces of the body in between.
    assert!(names.vertices[index].to_string().starts_with("F5:corner["));
    assert_eq!(
        texts(names.vertex_aliases_of(index)),
        ["F5:copy(0)/F3:copy(1)/F2:vertex(7)"]
    );
    // Every edge and vertex has a name, and no two share one.
    for list in [&names.edges, &names.vertices] {
        let mut seen = std::collections::BTreeSet::new();
        assert!(list.iter().all(|n| seen.insert(n)), "{:?}", texts(list));
    }
    assert_eq!(names.edges.len(), body.edges.len());
    assert_eq!(names.vertices.len(), body.vertices.len());
}
