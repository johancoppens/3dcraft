//! Which parts of a name are proven to say how their entity was made.
//!
//! A feature that keeps a reference to one entity (a chamfer or a fillet
//! given an edge by its own name, and the face given with it) has to be
//! sure that no edit can move the name to another entity without a
//! failure. Some numbers in a name say where something lies or where it
//! stands in a list that can change (`between[…](1)`, a blend face's
//! `fillet(k)` after references were left out, a developable's faces);
//! those can move. So the rule is the other way round from a list of what
//! is forbidden: a name is accepted only when every part of it, at every
//! depth (the name, every link it is a copy of, every face it lists on
//! either side of `|`), is of a kind on the list of kinds proven to be
//! identity; everything else is refused until someone proves it or fixes
//! it (docs/design/names.md › *Which parts of a name are proven to be
//! identity*). A split that chose its lump by place, on the way of the
//! bodies the name's features grew from, is refused too.
//! The matches below name every role and every feature kind, with no arm
//! for the rest, so a role or a kind added later is refused until it is
//! decided here.

use crate::name::is_text_word;
use crate::{Document, FeatureId, FeatureKind, MAX_NAME_DEPTH, NameRole, PersistentName, Rail};
use std::collections::BTreeSet;

/// What makes a name not proven to identify its entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unproven<'a> {
    /// A part of the name of a kind not proven to be identity: a link of
    /// the name's own chain, or the whole of a face it lists.
    Part(&'a PersistentName),
    /// A split the name's body grew from, which chose its lump of its
    /// source by where the lumps lie in space (its `number`), or by a
    /// face whose name is not proven. A copy of instance 0 of a copy of
    /// instance 0 is one link, so the split need not be written in the
    /// name; the bodies the name's features grew from are looked at.
    Split(FeatureId),
}

impl Unproven<'_> {
    /// Why `name`, which this was found in, may move to another entity.
    pub fn reason(&self, doc: &Document, name: &PersistentName) -> String {
        match self {
            Unproven::Part(part) if *part == name && name.role.is_by_faces() => format!(
                "{name} tells its entity from the others on the same faces only by where it lies in space (the number at its end), which an edit can move to another one without any failure"
            ),
            Unproven::Part(part) if *part == name => format!(
                "{name} is {}: not proven to stay with its entity, so an edit may move the name to another one without any failure",
                unproven_kind(doc, part)
            ),
            Unproven::Part(part) => format!(
                "{name} holds {part}, {}: not proven to stay with its entity, so an edit may move the name to another one without any failure",
                unproven_kind(doc, part)
            ),
            Unproven::Split(split) => {
                let called = doc
                    .feature(*split)
                    .map_or_else(|| split.to_string(), |f| f.name.clone());
                format!(
                    "{name} is of a body grown from the split `{called}`, which chose its lump by where the lumps lie in space (or by a face whose name is not proven), so an edit may give it another lump and the name another entity without any failure"
                )
            }
        }
    }
}

/// What in `name` is not proven to identify its entity by how it was
/// made: a part of it, at any depth, of a kind not on the list; or a
/// split the bodies of its features grew from that chose its lump by
/// place. `None` when everything is proven. A part of a feature the
/// document does not have is left to resolution, which fails it with
/// that reason.
pub fn unproven<'a>(doc: &Document, name: &'a PersistentName) -> Option<Unproven<'a>> {
    if let Some(part) = unproven_within(doc, name, 0) {
        return Some(Unproven::Part(part));
    }
    split_by_place(doc, name).map(Unproven::Split)
}

/// The first split the bodies of the features in `name` grew from that
/// chose its lump by place or by a face whose name is not proven
/// ([`Unproven::Split`]), whatever the parts of `name` are: for a name
/// an edge's own name was followed to, whose way to the newest body may
/// have gone through one.
pub fn split_by_place(doc: &Document, name: &PersistentName) -> Option<FeatureId> {
    let mut from = Vec::new();
    features_in(name, 0, &mut from);
    split_by_place_upstream(doc, from)
}

fn unproven_within<'a>(
    doc: &Document,
    name: &'a PersistentName,
    depth: usize,
) -> Option<&'a PersistentName> {
    // A listed face lists none, so the depth is one; the cap is for a
    // name built by hand, and what lies past it is not proven.
    if depth > 2 {
        return Some(name);
    }
    let mut link = name;
    for _ in 0..=MAX_NAME_DEPTH {
        if !link_proven(doc, link) {
            return Some(link);
        }
        let listed = link.faces().iter().chain(link.near());
        for face in listed {
            if unproven_within(doc, face, depth.saturating_add(1)).is_some() {
                return Some(face);
            }
        }
        // The end of the chain: every link and every face proven.
        link = link.of.as_deref()?;
    }
    Some(name)
}

/// The features of every link of `name` and of the faces it lists, into
/// `out` (as deep as [`unproven_within`] looks).
fn features_in(name: &PersistentName, depth: usize, out: &mut Vec<FeatureId>) {
    if depth > 2 {
        return;
    }
    let mut link = name;
    for _ in 0..=MAX_NAME_DEPTH {
        out.push(link.feature);
        for face in link.faces().iter().chain(link.near()) {
            features_in(face, depth.saturating_add(1), out);
        }
        match link.of.as_deref() {
            Some(next) => link = next,
            None => return,
        }
    }
}

/// The first split, among `from` and the features whose bodies they were
/// made from (each looked at once, so a cycle in a bad file ends), that
/// chose its lump by place or by a face whose name is not proven.
fn split_by_place_upstream(doc: &Document, mut todo: Vec<FeatureId>) -> Option<FeatureId> {
    todo.reverse();
    let mut seen = BTreeSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(feature) = doc.feature(id) else {
            continue;
        };
        if lump_chosen_unproven(doc, &feature.kind) {
            return Some(id);
        }
        let sources = feature.kind.sources();
        todo.extend(sources.into_iter().rev().map(|(source, _)| source));
    }
    None
}

/// Whether a feature of this kind chose its body among the lumps of
/// another by place: a split with a number (which counts the lumps that
/// hold its face by where they lie), or one whose face's name is not
/// proven. Every other kind makes or changes its body without choosing.
fn lump_chosen_unproven(doc: &Document, kind: &FeatureKind) -> bool {
    match kind {
        FeatureKind::Split(split) => {
            split.number.is_some() || unproven_within(doc, &split.face, 1).is_some()
        }
        FeatureKind::Block(_)
        | FeatureKind::Cylinder(_)
        | FeatureKind::Sphere(_)
        | FeatureKind::Cone(_)
        | FeatureKind::Torus(_)
        | FeatureKind::Import(_)
        | FeatureKind::SheetMetal(_)
        | FeatureKind::Developable(_)
        | FeatureKind::SurfaceTrim(_)
        | FeatureKind::SurfaceNsided(_)
        | FeatureKind::SurfaceNetwork(_)
        | FeatureKind::Unfold(_)
        | FeatureKind::Fold(_)
        | FeatureKind::Sketch(_)
        | FeatureKind::Extrude(_)
        | FeatureKind::Revolve(_)
        | FeatureKind::Loft(_)
        | FeatureKind::Sweep(_)
        | FeatureKind::Transform(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Pattern(_)
        | FeatureKind::Boolean(_)
        | FeatureKind::DatumPlane(_)
        | FeatureKind::DatumAxis(_)
        | FeatureKind::DatumPoint(_)
        | FeatureKind::DatumHelix(_)
        | FeatureKind::Chamfer(_)
        | FeatureKind::Fillet(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::MoveFace(_)
        | FeatureKind::Thicken(_)
        | FeatureKind::Plate(_)
        | FeatureKind::Draft(_)
        | FeatureKind::Shell(_)
        | FeatureKind::DeleteFace(_)
        | FeatureKind::Thread(_)
        | FeatureKind::OffsetProfile(_)
        | FeatureKind::Fill(_) => false,
    }
}

/// Whether one link's own role and numbers are proven identity (what it
/// is a copy of and the faces it lists are looked at by the caller).
fn link_proven(doc: &Document, link: &PersistentName) -> bool {
    let kind = doc.feature(link.feature).map(|feature| &feature.kind);
    match link.role {
        // The instance: a boolean's operand, a pattern's or a mirror's
        // instance, a plate's or a shell's side, 0 for a feature with one
        // source; the entity after `/` is looked at in turn, and a split
        // on the way by `split_by_place_upstream`.
        NameRole::Copy => true,
        // The root and the two flanks of a thread's groove.
        NameRole::Thread => true,
        // By the faces it lies on and near, each looked at in turn; a
        // number after them is a place in space.
        NameRole::Between | NameRole::Corner => link.sources.is_empty(),
        NameRole::Face | NameRole::Edge | NameRole::Vertex => {
            kind.is_none_or(|kind| entity_numbered_by_origin(kind, link))
        }
        NameRole::StartCap
        | NameRole::EndCap
        | NameRole::Side
        | NameRole::StartEdge
        | NameRole::EndEdge
        | NameRole::LateralEdge
        | NameRole::StartVertex
        | NameRole::EndVertex => kind.is_none_or(|kind| swept_by_curves(kind, link)),
        // Not proven: a piece's place among the pieces of one curve (a
        // spline piece shorter than the tolerance merged renumbers the
        // rest); a blend's face and corner, numbered by the reference's
        // place among those left after the absent ones are left out, and by
        // place in space; a rim, by a hash of an edge name that may be
        // numbered by place and cannot be read back.
        NameRole::Piece
        | NameRole::Chamfer
        | NameRole::ChamferCorner
        | NameRole::Fillet
        | NameRole::FilletCorner
        | NameRole::Rim => false,
    }
}

/// Whether a `face(n)`, `edge(n)` or `vertex(n)` of a feature of this kind
/// is numbered by how the entity was made.
fn entity_numbered_by_origin(kind: &FeatureKind, link: &PersistentName) -> bool {
    match kind {
        // A primitive's entities are made in a fixed order, the kernel's
        // contract (`kernel::primitives`, pinned by `engine/tests/naming.rs`).
        FeatureKind::Block(_)
        | FeatureKind::Cylinder(_)
        | FeatureKind::Sphere(_)
        | FeatureKind::Cone(_)
        | FeatureKind::Torus(_) => true,
        // An imported solid's entities are numbered in the order of the
        // stored file, which an edit does not change.
        FeatureKind::Import(_) => true,
        // A sheet-metal part's faces by a flange's id, a bend's `2n - 1`
        // and a flat's `2n` (one number below 2^32; a piece above it is
        // numbered by its place among a cell's pieces), and a formed
        // louvre's by its id and the role of the face (`[15, id, what,
        // role]`); its edges are named by places along edges and among
        // holes, not proven.
        FeatureKind::SheetMetal(_) => {
            link.role == NameRole::Face
                && match link.sources.as_slice() {
                    [number] => *number < (1 << 32),
                    [15, _, _, _] => true,
                    _ => false,
                }
        }
        // A developable's faces are numbered by place along it, counted
        // from the nearer end of a run of folds; a trimmed or filled
        // face's entities by places in its rail lists; every other kind
        // names no such entity, or not by origin.
        FeatureKind::Developable(_)
        | FeatureKind::SurfaceTrim(_)
        | FeatureKind::SurfaceNsided(_)
        | FeatureKind::SurfaceNetwork(_)
        | FeatureKind::Unfold(_)
        | FeatureKind::Fold(_)
        | FeatureKind::Sketch(_)
        | FeatureKind::Extrude(_)
        | FeatureKind::Revolve(_)
        | FeatureKind::Loft(_)
        | FeatureKind::Sweep(_)
        | FeatureKind::Transform(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Pattern(_)
        | FeatureKind::Boolean(_)
        | FeatureKind::DatumPlane(_)
        | FeatureKind::DatumAxis(_)
        | FeatureKind::DatumPoint(_)
        | FeatureKind::DatumHelix(_)
        | FeatureKind::Chamfer(_)
        | FeatureKind::Fillet(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::MoveFace(_)
        | FeatureKind::Thicken(_)
        | FeatureKind::Plate(_)
        | FeatureKind::Draft(_)
        | FeatureKind::Shell(_)
        | FeatureKind::DeleteFace(_)
        | FeatureKind::Thread(_)
        | FeatureKind::OffsetProfile(_)
        | FeatureKind::Fill(_)
        | FeatureKind::Split(_) => false,
    }
}

/// Whether a cap, side, cap edge, lateral edge or cap vertex of a feature
/// of this kind is named by the curves that swept it.
fn swept_by_curves(kind: &FeatureKind, link: &PersistentName) -> bool {
    match kind {
        // By sketch entity ids (never reused), the indices of a sketch
        // pattern's instances from the seed, and an offset's heads; not by
        // a glyph of a text, numbered by its place in the text's string.
        FeatureKind::Extrude(_)
        | FeatureKind::Revolve(_)
        | FeatureKind::Loft(_)
        | FeatureKind::Sweep(_)
        | FeatureKind::Fill(_) => !link.sources.iter().any(|word| is_text_word(*word)),
        // A network's face by the ids of the sketches of its first and
        // last curves each way; a curve given by edges is known by a hash
        // of an edge's name, which may be numbered by place. Its edges and
        // vertices are not proven.
        FeatureKind::SurfaceNetwork(network) => {
            link.role == NameRole::Side
                && [
                    network.u.first(),
                    network.u.last(),
                    network.v.first(),
                    network.v.last(),
                ]
                .into_iter()
                .all(|rail| matches!(rail, Some(Rail::Sketch(_))))
        }
        // A trimmed face's and a fill's entities by places in rail lists,
        // a fold's by its panels: not proven.
        FeatureKind::SurfaceTrim(_)
        | FeatureKind::SurfaceNsided(_)
        | FeatureKind::Developable(_)
        | FeatureKind::Unfold(_)
        | FeatureKind::Fold(_)
        | FeatureKind::SheetMetal(_)
        | FeatureKind::Block(_)
        | FeatureKind::Cylinder(_)
        | FeatureKind::Sphere(_)
        | FeatureKind::Cone(_)
        | FeatureKind::Torus(_)
        | FeatureKind::Import(_)
        | FeatureKind::Sketch(_)
        | FeatureKind::Transform(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Pattern(_)
        | FeatureKind::Boolean(_)
        | FeatureKind::DatumPlane(_)
        | FeatureKind::DatumAxis(_)
        | FeatureKind::DatumPoint(_)
        | FeatureKind::DatumHelix(_)
        | FeatureKind::Chamfer(_)
        | FeatureKind::Fillet(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::MoveFace(_)
        | FeatureKind::Thicken(_)
        | FeatureKind::Plate(_)
        | FeatureKind::Draft(_)
        | FeatureKind::Shell(_)
        | FeatureKind::DeleteFace(_)
        | FeatureKind::Thread(_)
        | FeatureKind::OffsetProfile(_)
        | FeatureKind::Split(_) => false,
    }
}

/// In words, what kind of number `part` (an unproven part of a name) has,
/// for a reason that says why it is refused.
fn unproven_kind(doc: &Document, part: &PersistentName) -> &'static str {
    let mut link = part;
    // The unproven link of a listed face's chain says what it is.
    for _ in 0..=MAX_NAME_DEPTH {
        if !link_proven(doc, link) {
            break;
        }
        match link.of.as_deref() {
            Some(next) => link = next,
            None => break,
        }
    }
    match link.role {
        NameRole::Between | NameRole::Corner => {
            "an edge or a vertex told from the others on the same faces only by a number at its end, which says where it lies in space"
        }
        NameRole::Chamfer | NameRole::ChamferCorner | NameRole::Fillet | NameRole::FilletCorner => {
            "a blend's face or corner, numbered by the place of its reference among those the blend is built from (which change when one is left out) and by where its edges lie"
        }
        NameRole::Rim => {
            "a plate's rim, named by a hash of the name of the sheet's edge, which may be numbered by place and cannot be read back"
        }
        NameRole::Piece => "a piece numbered by its place among the pieces of one curve",
        NameRole::Face | NameRole::Edge | NameRole::Vertex => {
            "an entity the feature that made it numbers by place (a developable's faces, the pieces of a sheet-metal part, a trimmed face's edges)"
        }
        NameRole::StartCap
        | NameRole::EndCap
        | NameRole::Side
        | NameRole::StartEdge
        | NameRole::EndEdge
        | NameRole::LateralEdge
        | NameRole::StartVertex
        | NameRole::EndVertex => {
            "a face, an edge or a vertex swept by a curve named by its place in a list or a text (a glyph's place, a rail's place, a hash of an edge's name), not by a sketch entity"
        }
        NameRole::Copy | NameRole::Thread => "a part past the depth a name may have",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Block, Boolean, BooleanOp, Developable, Expr, Extrude, ExtrudeEnd, FeatureId, Orientation,
        Split, Units,
    };

    fn block() -> FeatureKind {
        FeatureKind::Block(Block {
            size: [Expr::new("1 mm"), Expr::new("1 mm"), Expr::new("1 mm")],
            position: [Expr::new("0 mm"), Expr::new("0 mm"), Expr::new("0 mm")],
            orientation: Orientation::default(),
        })
    }

    fn split(face: &str, number: Option<u64>) -> FeatureKind {
        FeatureKind::Split(Split {
            source: FeatureId(1),
            face: face.parse().unwrap(),
            number,
        })
    }

    /// F1 a block, F2 a developable between two sketches, F3 a split of
    /// the block's lump by its number in space, F4 one by a face only,
    /// F5 a union with F3 as its target, F6 a split by a developable's
    /// face, F7 an extrusion (none of which the test builds), F8 to F10
    /// named in the names below only.
    fn doc() -> Document {
        let mut doc = Document::new(Units::default());
        doc.add_feature(Some("Box"), block()).unwrap();
        doc.add_feature(
            Some("Panel"),
            FeatureKind::Developable(Developable {
                a: Rail::Sketch(FeatureId(5)),
                b: Rail::Sketch(FeatureId(6)),
            }),
        )
        .unwrap();
        doc.add_feature(Some("Left"), split("F1:face(0)", Some(1)))
            .unwrap();
        doc.add_feature(Some("Right"), split("F1:face(1)", None))
            .unwrap();
        doc.add_feature(
            Some("Joined"),
            FeatureKind::Boolean(Boolean {
                op: BooleanOp::Union,
                target: FeatureId(3),
                tool: FeatureId(2),
                keep_tool: false,
            }),
        )
        .unwrap();
        doc.add_feature(Some("ByPanel"), split("F2:face(0)", None))
            .unwrap();
        doc.add_feature(
            Some("Raised"),
            FeatureKind::Extrude(Extrude {
                sketch: FeatureId(1),
                distance: Expr::new("1 mm"),
                symmetric: false,
                end: ExtrudeEnd::default(),
                reversed: false,
            }),
        )
        .unwrap();
        doc
    }

    fn unproven(doc: &Document, text: &str) -> Option<String> {
        let name: PersistentName = text.parse().unwrap();
        super::unproven(doc, &name).map(|found| match found {
            Unproven::Part(part) => part.to_string(),
            Unproven::Split(split) => format!("split {split}"),
        })
    }

    #[test]
    fn only_kinds_proven_to_be_identity_are_taken_at_any_depth() {
        let doc = doc();
        // Proven: a primitive's entities, copies, threads, names by faces
        // without a number whose faces are proven; parts of features the
        // document does not have are left to resolution.
        for text in [
            "F1:face(3)",
            "F1:edge(7)",
            "F7:copy(1,3)/F1:face(2)",
            "F7:between[F7:copy(0)/F1:face(1),F7:copy(1)/F1:face(0)|F7:copy(0)/F1:face(4)]",
            "F9:thread(1)",
            "F7:lateralEdge(e5,e6)",
            "F7:side(e5)",
            "F8:lateralEdge(e5,e6)",
            "F4:copy(0)/F1:edge(2)",
        ] {
            assert_eq!(unproven(&doc, text), None, "{text}");
        }
        // Refused, each kind: the part named.
        for (text, part) in [
            (
                "F7:between[F7:copy(0)/F1:face(1),F7:copy(1)/F1:face(0)](1)",
                "F7:between[F7:copy(0)/F1:face(1),F7:copy(1)/F1:face(0)](1)",
            ),
            (
                "F7:corner[F7:copy(0)/F1:face(1),F7:copy(0)/F1:face(2),F7:copy(1)/F1:face(0)](0)",
                "F7:corner[F7:copy(0)/F1:face(1),F7:copy(0)/F1:face(2),F7:copy(1)/F1:face(0)](0)",
            ),
            (
                "F9:copy(1)/F7:between[F7:copy(0)/F1:face(1),F7:copy(1)/F1:face(0)](0)",
                "F7:between[F7:copy(0)/F1:face(1),F7:copy(1)/F1:face(0)](0)",
            ),
            ("F9:chamfer(0)", "F9:chamfer(0)"),
            ("F9:fillet(0,1)", "F9:fillet(0,1)"),
            ("F9:filletCorner(0,1,2)", "F9:filletCorner(0,1,2)"),
            ("F9:chamferCorner(0,1,2)", "F9:chamferCorner(0,1,2)"),
            ("F10:copy(0)/F9:fillet(4)", "F9:fillet(4)"),
            (
                "F8:between[F8:copy(0)/F1:face(1),F8:copy(0)/F10:chamfer(0)]",
                "F8:copy(0)/F10:chamfer(0)",
            ),
            (
                "F8:between[F8:copy(0)/F1:face(1),F8:copy(1)/F1:face(0)|F8:copy(0)/F10:fillet(0,0)]",
                "F8:copy(0)/F10:fillet(0,0)",
            ),
            ("F9:rim(77)/F10:face(0)", "F9:rim(77)/F10:face(0)"),
            ("F9:piece(3)/F9:thread(1)", "F9:piece(3)/F9:thread(1)"),
            ("F2:face(10000000001)", "F2:face(10000000001)"),
            (
                "F8:between[F8:copy(0)/F2:face(0),F8:copy(1)/F1:face(0)]",
                "F8:copy(0)/F2:face(0)",
            ),
            // A split by its number in space, written in the name or
            // hidden in a copy of instance 0 that merged with it, and a
            // split by a face whose name is not proven.
            ("F3:copy(0)/F1:edge(2)", "split F3"),
            ("F5:copy(0)/F1:edge(2)", "split F3"),
            ("F6:copy(0)/F1:edge(2)", "split F6"),
            // A glyph of a text, numbered by its place in the string.
            (
                "F7:side(e7.glyph(0,U+004F).contour(0))",
                "F7:side(e7.glyph(0,U+004F).contour(0))",
            ),
        ] {
            assert_eq!(unproven(&doc, text).as_deref(), Some(part), "{text}");
        }
    }
}
