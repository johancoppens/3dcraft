//! Persistent names: how a feature points at a face, edge or vertex that
//! another feature made.
//!
//! A name says where the entity came from, never where it sits in the
//! body: the feature that made it, the part it plays, and the entities of
//! that feature's input it was made from (for an extrusion, the ids of the
//! sketch entities). It is written `F2:side(e7)`. An upstream edit either
//! leaves the name pointing at the same thing, or leaves it pointing at
//! nothing, which regeneration reports; it never makes it point at
//! something else (docs/architecture.md §4.1).
//!
//! A feature that makes its body out of another body (a transform, a
//! mirror, a pattern) names what it makes after what it was made from:
//! `F5:copy(2)/F2:side(e7)` is the image, in instance 2 of F5, of the face
//! F2 calls `side(e7)`. The part after `/` is a name of its own.
//!
//! A feature whose body the kernel puts together out of other bodies (a
//! boolean, a pattern whose instances are joined) names its edges and
//! vertices by the faces of that body they lie on:
//! `F7:between[F7:copy(0)/F1:face(1),F7:copy(1)/F2:face(0)]` is the edge
//! between those two faces, `F7:corner[a,b,c]` the vertex where those
//! faces meet. When several edges or vertices lie on the same faces, the
//! faces at their ends (for a vertex: at the other ends of its edges)
//! follow after `|`, and when that does not tell them apart either a
//! number follows in brackets: `F7:corner[a,b|c,d](1)`
//! (docs/architecture.md §4.1).

use crate::FeatureId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Most sources a name carries (a cap is named after every curve of its
/// outer loop).
pub const MAX_NAME_SOURCES: usize = 4096;

/// Most face names an edge or a vertex named by its faces lists, those
/// after `|` included.
pub const MAX_NAME_FACES: usize = 256;

/// Most names one name is made of (`a/b/c` is three): how many features in
/// a row can each make their body out of the one before.
pub const MAX_NAME_DEPTH: usize = 32;

/// The part an entity plays in the feature that made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NameRole {
    /// The flat face where an extrusion starts.
    StartCap,
    /// The flat face where it ends.
    EndCap,
    /// The face swept by one curve.
    Side,
    /// The edge of the start cap made from one curve.
    StartEdge,
    EndEdge,
    /// The straight edge swept by a corner, between two curves.
    LateralEdge,
    StartVertex,
    EndVertex,
    /// The image of an entity of another body, in a feature that moves,
    /// mirrors or repeats that body. Its one source is the number of the
    /// instance (0 for the first, or only, one); the name of the entity it
    /// is the image of follows after `/`.
    Copy,
    /// One of several entities that the name after `/` would otherwise
    /// name all at once: a curve that sweeps more than one face (a spline
    /// cut at its corners, a spline that closes on itself, a spline turned
    /// a full turn, which is made in two halves) calls the first
    /// `F2:side(e7)` and the others `F2:piece(1)/F2:side(e7)`, and so on;
    /// their edges and vertices likewise. Its one source is the number,
    /// from 1; the feature is the same before and after `/`.
    Piece,
    /// A face of a primitive, by its number: the faces of a primitive are
    /// made in a fixed order (for a block: 0 bottom, 1 top, 2 front (-y),
    /// 3 back (+y), 4 left (-x), 5 right (+x), in its own axes; for a
    /// cylinder: 0 the wall, 1 the bottom, 2 the top). Its one source is
    /// that number.
    Face,
    /// An edge of a primitive, by its number.
    Edge,
    /// A vertex of a primitive, by its number.
    Vertex,
    /// An edge, by the faces of its body it lies between (twice the same
    /// face for a seam). The faces are in [`PersistentName::faces`].
    Between,
    /// A vertex, by the faces of its body that meet in it.
    Corner,
    /// The face a chamfer puts in the place of an edge. Its sources are
    /// plain numbers: the place of the edge's reference in the chamfer's
    /// list of edges, and, when that reference gave several edges, which
    /// of them, counted in the order of where they lie in space.
    Chamfer,
    /// The flat face in the corner where three chamfered edges meet. Its
    /// sources are plain numbers: the places of the three references in
    /// the chamfer's list, ascending.
    ChamferCorner,
    /// The face a fillet puts in the place of an edge: a piece of a
    /// cylinder or of a torus. Its sources are as a chamfer face's.
    Fillet,
    /// The piece of a ball in the corner where three filleted edges meet.
    /// Its sources are as a chamfer corner's.
    FilletCorner,
}

impl NameRole {
    pub const ALL: [NameRole; 19] = [
        NameRole::StartCap,
        NameRole::EndCap,
        NameRole::Side,
        NameRole::StartEdge,
        NameRole::EndEdge,
        NameRole::LateralEdge,
        NameRole::StartVertex,
        NameRole::EndVertex,
        NameRole::Copy,
        NameRole::Piece,
        NameRole::Face,
        NameRole::Edge,
        NameRole::Vertex,
        NameRole::Between,
        NameRole::Corner,
        NameRole::Chamfer,
        NameRole::ChamferCorner,
        NameRole::Fillet,
        NameRole::FilletCorner,
    ];

    pub fn name(self) -> &'static str {
        match self {
            NameRole::StartCap => "startCap",
            NameRole::EndCap => "endCap",
            NameRole::Side => "side",
            NameRole::StartEdge => "startEdge",
            NameRole::EndEdge => "endEdge",
            NameRole::LateralEdge => "lateralEdge",
            NameRole::StartVertex => "startVertex",
            NameRole::EndVertex => "endVertex",
            NameRole::Copy => "copy",
            NameRole::Piece => "piece",
            NameRole::Face => "face",
            NameRole::Edge => "edge",
            NameRole::Vertex => "vertex",
            NameRole::Between => "between",
            NameRole::Corner => "corner",
            NameRole::Chamfer => "chamfer",
            NameRole::ChamferCorner => "chamferCorner",
            NameRole::Fillet => "fillet",
            NameRole::FilletCorner => "filletCorner",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }

    /// True for the roles that are faces. A copy is whatever it is a copy
    /// of: ask [`PersistentName::is_face`].
    pub fn is_face(self) -> bool {
        matches!(
            self,
            NameRole::StartCap
                | NameRole::EndCap
                | NameRole::Side
                | NameRole::Face
                | NameRole::Chamfer
                | NameRole::ChamferCorner
                | NameRole::Fillet
                | NameRole::FilletCorner
        )
    }

    /// True for the roles that are vertices. A copy is whatever it is a
    /// copy of: ask [`PersistentName::is_vertex`].
    pub fn is_vertex(self) -> bool {
        matches!(
            self,
            NameRole::StartVertex | NameRole::EndVertex | NameRole::Vertex | NameRole::Corner
        )
    }

    /// True for the roles that are edges. A copy is whatever it is a copy
    /// of: ask [`PersistentName::is_edge`].
    pub fn is_edge(self) -> bool {
        matches!(
            self,
            NameRole::StartEdge
                | NameRole::EndEdge
                | NameRole::LateralEdge
                | NameRole::Edge
                | NameRole::Between
        )
    }

    /// True for a role that names an entity by the faces it lies on, which
    /// are written between `[` and `]`.
    pub fn is_by_faces(self) -> bool {
        matches!(self, NameRole::Between | NameRole::Corner)
    }

    /// True for a role whose name continues after `/` with the name of the
    /// entity it was made from.
    pub fn is_derived(self) -> bool {
        matches!(self, NameRole::Copy | NameRole::Piece)
    }

    /// True for a role whose sources are plain numbers, one or more,
    /// written without the `e` of a sketch entity: `chamfer(0,1)`.
    pub fn is_counted(self) -> bool {
        matches!(
            self,
            NameRole::Chamfer | NameRole::ChamferCorner | NameRole::Fillet | NameRole::FilletCorner
        )
    }

    /// True for a role with exactly one source that is a plain number (an
    /// instance, or the number of an entity of a primitive) and is written
    /// without the `e` of a sketch entity.
    pub fn is_numbered(self) -> bool {
        matches!(
            self,
            NameRole::Copy | NameRole::Piece | NameRole::Face | NameRole::Edge | NameRole::Vertex
        )
    }
}

/// The name of one face, edge or vertex. As text: `F2:side(e7)`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PersistentName {
    /// The feature that made the entity.
    pub feature: FeatureId,
    pub role: NameRole,
    /// The ids of the input entities it was made from, in the order the
    /// role defines. For a copy: the number of the instance. For an edge
    /// or a vertex named by its faces: empty, or the one number that tells
    /// it from the others on the same faces.
    pub sources: Vec<u64>,
    /// For a derived role (a copy): the name, in the body this feature
    /// was made from, of the entity this one is the image of. `None` for
    /// every other role.
    pub of: Option<Box<PersistentName>>,
    /// For an edge or a vertex named by its faces (`between`, `corner`)
    /// that lists any: the faces. `None` for every other role. Behind a
    /// pointer, because most names are of faces and there are many.
    pub by: Option<Box<FaceLists>>,
}

/// The faces an edge or a vertex named by its faces lists.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct FaceLists {
    /// The names of the faces of the feature's body the entity lies on,
    /// in ascending order (see [`PersistentName::with_faces`]). A seam
    /// has its one face twice.
    pub on: Vec<PersistentName>,
    /// When several entities lie on the same faces: the names of the
    /// other faces near it (at the ends of the edge, at the other ends
    /// of the vertex's edges, or further away), in ascending order.
    pub near: Vec<PersistentName>,
}

impl PersistentName {
    /// A name that is not derived from another: `F2:side(e7)`.
    pub fn new(feature: FeatureId, role: NameRole, sources: Vec<u64>) -> Self {
        Self {
            feature,
            role,
            sources,
            of: None,
            by: None,
        }
    }

    /// The image of `of` in instance `instance` of `feature`:
    /// `F5:copy(2)/F2:side(e7)`.
    pub fn copy(feature: FeatureId, instance: u64, of: PersistentName) -> Self {
        Self {
            of: Some(Box::new(of)),
            ..Self::new(feature, NameRole::Copy, vec![instance])
        }
    }

    /// Entity number `piece` of those `of` would name in its own feature
    /// (see [`NameRole::Piece`]); `of` itself for number 0.
    pub fn piece(piece: u32, of: PersistentName) -> Self {
        if piece == 0 {
            return of;
        }
        Self {
            of: Some(Box::new(of.clone())),
            ..Self::new(of.feature, NameRole::Piece, vec![u64::from(piece)])
        }
    }

    /// The edge of `feature`'s body between `faces` (two names; a seam has
    /// its face twice): `F7:between[a,b]`. `near` and `index` are what
    /// tells it from other edges between the same faces, when there are.
    pub fn between(
        feature: FeatureId,
        faces: Vec<PersistentName>,
        near: Vec<PersistentName>,
        index: Option<u64>,
    ) -> Self {
        Self::new(feature, NameRole::Between, index.into_iter().collect()).with_faces(faces, near)
    }

    /// The vertex of `feature`'s body where `faces` meet:
    /// `F7:corner[a,b,c]`. `near` and `index` are what tells it from
    /// other vertices on the same faces, when there are.
    pub fn corner(
        feature: FeatureId,
        faces: Vec<PersistentName>,
        near: Vec<PersistentName>,
        index: Option<u64>,
    ) -> Self {
        Self::new(feature, NameRole::Corner, index.into_iter().collect()).with_faces(faces, near)
    }

    /// This link listing `on` and `near`, each put in the order names
    /// are compared in (by feature, then role, then sources, then what a
    /// copy is a copy of), which is the order regeneration lists them in:
    /// a name typed or stored with its faces in another order is the
    /// same name. Listing nothing is listing no lists.
    pub fn with_faces(
        mut self,
        mut on: Vec<PersistentName>,
        mut near: Vec<PersistentName>,
    ) -> Self {
        on.sort();
        near.sort();
        self.by = if on.is_empty() && near.is_empty() {
            None
        } else {
            Some(Box::new(FaceLists { on, near }))
        };
        self
    }

    /// The faces this link lies on, when it is an edge or a vertex named
    /// by its faces.
    pub fn faces(&self) -> &[PersistentName] {
        self.by.as_ref().map_or(&[], |by| by.on.as_slice())
    }

    /// The faces near this link that tell it from others on the same
    /// faces.
    pub fn near(&self) -> &[PersistentName] {
        self.by.as_ref().map_or(&[], |by| by.near.as_slice())
    }

    /// The name at the end of the chain: what the entity was when it was
    /// first made.
    pub fn root(&self) -> &PersistentName {
        let mut name = self;
        // The chain is finite; the cap is for a name built by hand.
        for _ in 0..MAX_NAME_DEPTH {
            match &name.of {
                Some(next) => name = next,
                None => break,
            }
        }
        name
    }

    /// How many names follow each other in this one, without the faces a
    /// link lists.
    fn chain(&self) -> usize {
        let mut depth = 1;
        let mut name = self;
        while let Some(next) = &name.of {
            depth += 1;
            name = next;
            if depth > MAX_NAME_DEPTH {
                break;
            }
        }
        depth
    }

    /// How many names this one is made of: how many features in a row
    /// each made their body from the one before. An edge or a vertex
    /// named by its faces is as deep as the deepest of them: it is of the
    /// same body.
    pub fn depth(&self) -> usize {
        let listed = self
            .root()
            .listed()
            .map(PersistentName::chain)
            .max()
            .unwrap_or(1);
        self.chain().saturating_sub(1).saturating_add(listed.max(1))
    }

    /// The faces this link lists: those it lies on, then those near it.
    fn listed(&self) -> impl Iterator<Item = &PersistentName> {
        self.faces().iter().chain(self.near())
    }

    /// True when the entity is a face.
    pub fn is_face(&self) -> bool {
        self.root().role.is_face()
    }

    /// True when the entity is an edge.
    pub fn is_edge(&self) -> bool {
        self.root().role.is_edge()
    }

    /// True when the entity is a vertex.
    pub fn is_vertex(&self) -> bool {
        self.root().role.is_vertex()
    }

    /// Whether the name is one that can be written and read back: a copy
    /// has one instance number and the name it is a copy of, nothing else
    /// is followed by another name, only an edge or a vertex named by its
    /// faces lists faces (plain face names, which list none themselves),
    /// and the caps on depth, sources and faces hold. A name read from
    /// text always is; one built from parts (a file) may not be.
    pub fn check(&self) -> Result<(), &'static str> {
        let mut sources = 0usize;
        let root = self.check_chain(&mut sources, true)?;
        if root.listed().count() > MAX_NAME_FACES {
            return Err("it lists too many faces");
        }
        for face in root.listed() {
            let end = face.check_chain(&mut sources, false)?;
            if !end.role.is_face() {
                return Err("an edge or a vertex lists faces, and this is not one");
            }
        }
        if self.depth() > MAX_NAME_DEPTH {
            return Err("it is made of too many names");
        }
        Ok(())
    }

    /// Checks the links of one chain and returns its last. `sources` is
    /// what the whole name has used of [`MAX_NAME_SOURCES`]; `lists` says
    /// whether the last link may list faces.
    fn check_chain(
        &self,
        sources: &mut usize,
        lists: bool,
    ) -> Result<&PersistentName, &'static str> {
        let mut name = self;
        for _ in 0..MAX_NAME_DEPTH {
            *sources = sources.saturating_add(name.sources.len());
            if *sources > MAX_NAME_SOURCES {
                return Err("it has too many sources");
            }
            if name.role.is_by_faces() {
                if !lists {
                    return Err("a face listed by an edge or a vertex lists no faces itself");
                }
                if name.sources.len() > 1 {
                    return Err("an edge or a vertex named by its faces has at most one number");
                }
            } else {
                if name.by.is_some() {
                    return Err("only a between and a corner list faces");
                }
                if name.role.is_numbered() && name.sources.len() != 1 {
                    return Err("a copy, face, edge or vertex has one number");
                }
                if name.role.is_counted() && name.sources.is_empty() {
                    return Err("a chamfer or fillet face has at least one number");
                }
            }
            match (name.role.is_derived(), &name.of) {
                (true, Some(next)) => name = next,
                (true, None) => return Err("a copy without the name it is a copy of"),
                (false, Some(_)) => return Err("only a copy is followed by another name"),
                (false, None) => return Ok(name),
            }
        }
        Err("it is made of too many names")
    }

    /// Writes the chain; a link that lists faces writes them when `lists`
    /// (the faces themselves are written without, so this never goes
    /// deeper than one list).
    fn write(&self, f: &mut fmt::Formatter<'_>, lists: bool) -> fmt::Result {
        let mut name = self;
        for depth in 0..=MAX_NAME_DEPTH {
            if depth > 0 {
                f.write_str("/")?;
            }
            write!(f, "{}:{}", name.feature, name.role.name())?;
            if name.role.is_by_faces() {
                f.write_str("[")?;
                if lists {
                    for (i, face) in name.faces().iter().enumerate() {
                        if i > 0 {
                            f.write_str(",")?;
                        }
                        face.write(f, false)?;
                    }
                    for (i, face) in name.near().iter().enumerate() {
                        f.write_str(if i > 0 { "," } else { "|" })?;
                        face.write(f, false)?;
                    }
                }
                f.write_str("]")?;
            }
            if !(name.role.is_by_faces() && name.sources.is_empty()) {
                f.write_str("(")?;
                let prefix =
                    if name.role.is_numbered() || name.role.is_by_faces() || name.role.is_counted()
                    {
                        ""
                    } else {
                        "e"
                    };
                for (i, source) in name.sources.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{prefix}{source}")?;
                }
                f.write_str(")")?;
            }
            match &name.of {
                Some(next) => name = next,
                None => break,
            }
        }
        Ok(())
    }
}

impl fmt::Display for PersistentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, true)
    }
}

impl From<PersistentName> for String {
    fn from(name: PersistentName) -> String {
        name.to_string()
    }
}

/// Cuts `text` at every `separator` that is outside brackets of both
/// kinds. Brackets that do not match leave the rest in one piece, which
/// then fails to read as a name.
fn split_outside(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut from) = (0usize, 0usize);
    for (at, c) in text.char_indices() {
        match c {
            '(' | '[' => depth = depth.saturating_add(1),
            ')' | ']' => depth = depth.saturating_sub(1),
            c if c == separator && depth == 0 => {
                parts.push(text.get(from..at).unwrap_or_default());
                from = at.saturating_add(c.len_utf8());
            }
            _ => {}
        }
    }
    parts.push(text.get(from..).unwrap_or_default());
    parts
}

/// What a name is read with: how to word an error, and how many sources
/// and listed faces the whole name may still have.
struct Reading<'a> {
    wrong: &'a dyn Fn(&str) -> String,
    sources: usize,
    faces: usize,
}

/// The numbers between the brackets of `F2:side(e7,e8)`.
fn parse_sources(
    inner: &str,
    numbered: bool,
    reading: &mut Reading<'_>,
) -> Result<Vec<u64>, String> {
    let wrong = reading.wrong;
    let mut sources = Vec::new();
    if inner.is_empty() {
        return Ok(sources);
    }
    for part in inner.split(',') {
        if reading.sources == 0 {
            return Err(wrong("it has too many sources"));
        }
        reading.sources -= 1;
        let part = part.trim();
        let id = if numbered {
            part.parse::<u64>()
                .ok()
                .filter(|_| part.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| wrong("this role takes a plain number, as in copy(0)"))?
        } else {
            part.strip_prefix('e')
                .and_then(|digits| digits.parse::<u64>().ok())
                .ok_or_else(|| wrong("a source must be written e and a number"))?
        };
        sources.push(id);
    }
    Ok(sources)
}

/// The face names of one list of `F7:corner[a,b|c]`.
fn parse_faces(inner: &str, reading: &mut Reading<'_>) -> Result<Vec<PersistentName>, String> {
    let wrong = reading.wrong;
    let mut faces = Vec::new();
    if inner.trim().is_empty() {
        return Ok(faces);
    }
    for part in split_outside(inner, ',') {
        if reading.faces == 0 {
            return Err(wrong("it lists too many faces"));
        }
        reading.faces -= 1;
        let face = parse_chain(part.trim(), reading, false)?;
        if !face.is_face() {
            return Err(wrong(
                "an edge or a vertex lists the faces it lies on, and this is not a face",
            ));
        }
        faces.push(face);
    }
    Ok(faces)
}

/// One name of a chain: `F2:side(e7)`, or with `lists`
/// `F7:corner[a,b|c](1)`.
fn parse_link(
    text: &str,
    reading: &mut Reading<'_>,
    lists: bool,
) -> Result<PersistentName, String> {
    let wrong = reading.wrong;
    let (feature, rest) = text
        .split_once(':')
        .ok_or_else(|| wrong("there is no `:` after the feature"))?;
    let feature = feature
        .strip_prefix('F')
        .and_then(|digits| digits.parse::<u64>().ok())
        .ok_or_else(|| wrong("the feature must be written F and a number"))?;
    let at = rest
        .find(['(', '['])
        .ok_or_else(|| wrong("there is no `(` after the role"))?;
    let (role, rest) = rest.split_at(at);
    let role = NameRole::from_name(role).ok_or_else(|| {
        let roles: Vec<&str> = NameRole::ALL.iter().map(|r| r.name()).collect();
        wrong(&format!("the role must be one of {}", roles.join(", ")))
    })?;
    if role.is_by_faces() {
        if !lists {
            return Err(wrong(
                "an edge or a vertex lists faces, which list no faces themselves",
            ));
        }
        let inner = rest
            .strip_prefix('[')
            .ok_or_else(|| wrong("a between or a corner is followed by `[` and its faces"))?;
        // The faces have no `]` in them.
        let (inner, rest) = inner
            .split_once(']')
            .ok_or_else(|| wrong("there is no `]` after the faces"))?;
        let (on, near) = match split_outside(inner, '|').as_slice() {
            [on] => (*on, ""),
            [on, near] => (*on, *near),
            _ => return Err(wrong("there is one `|` at most between the faces")),
        };
        let faces = parse_faces(on, reading)?;
        let near = parse_faces(near, reading)?;
        let sources = match rest {
            "" => Vec::new(),
            _ => {
                let inner = rest
                    .strip_prefix('(')
                    .and_then(|rest| rest.strip_suffix(')'))
                    .ok_or_else(|| wrong("after `]` comes nothing, or one number as in (1)"))?;
                let sources = parse_sources(inner, true, reading)?;
                if sources.len() != 1 {
                    return Err(wrong("after `]` comes nothing, or one number as in (1)"));
                }
                sources
            }
        };
        return Ok(PersistentName::new(FeatureId(feature), role, sources).with_faces(faces, near));
    }
    let rest = rest
        .strip_prefix('(')
        .ok_or_else(|| wrong("there is no `(` after the role"))?;
    let inner = rest
        .strip_suffix(')')
        .ok_or_else(|| wrong("it does not end with `)`"))?;
    let sources = parse_sources(inner, role.is_numbered() || role.is_counted(), reading)?;
    if role.is_counted() && sources.is_empty() {
        return Err(wrong(
            "this role takes at least one number, as in chamfer(0)",
        ));
    }
    if role.is_numbered() && sources.len() != 1 {
        return Err(wrong(
            "this role takes one number, as in copy(0) or face(2)",
        ));
    }
    Ok(PersistentName::new(FeatureId(feature), role, sources))
}

/// A chain of names: `F5:copy(2)/F2:side(e7)`. With `lists` its last may
/// be an edge or a vertex that lists faces.
fn parse_chain(
    text: &str,
    reading: &mut Reading<'_>,
    lists: bool,
) -> Result<PersistentName, String> {
    let wrong = reading.wrong;
    let mut links = Vec::new();
    for part in split_outside(text, '/') {
        if links.len() >= MAX_NAME_DEPTH {
            return Err(wrong("it is made of too many names"));
        }
        links.push(parse_link(part, reading, lists)?);
    }
    // Built from the end: every name but the last is derived from the
    // one after it, and the last is derived from nothing.
    let mut name: Option<PersistentName> = None;
    while let Some(mut link) = links.pop() {
        match (link.role.is_derived(), name.take()) {
            (true, Some(of)) => link.of = Some(Box::new(of)),
            (true, None) => {
                return Err(wrong(
                    "a copy is followed by `/` and the name it is a copy of",
                ));
            }
            (false, Some(_)) => {
                return Err(wrong("only a copy is followed by `/` and another name"));
            }
            (false, None) => {}
        }
        name = Some(link);
    }
    name.ok_or_else(|| wrong("it is empty"))
}

impl FromStr for PersistentName {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let shown: String = text.chars().take(60).collect();
        let wrong =
            |what: &str| format!("`{shown}` is not a name: {what}; a name looks like F2:side(e7)");
        let mut reading = Reading {
            wrong: &wrong,
            sources: MAX_NAME_SOURCES,
            faces: MAX_NAME_FACES,
        };
        let name = parse_chain(text, &mut reading, true)?;
        name.check().map_err(wrong)?;
        Ok(name)
    }
}

impl TryFrom<String> for PersistentName {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_piece_is_named_after_the_first_with_its_number() {
        let side = PersistentName::new(FeatureId(2), NameRole::Side, vec![7]);
        assert_eq!(PersistentName::piece(0, side.clone()), side);
        let second = PersistentName::piece(1, side.clone());
        assert_eq!(second.to_string(), "F2:piece(1)/F2:side(e7)");
        assert!(second.is_face() && !second.is_edge());
        assert_eq!(second.check(), Ok(()));
        assert_eq!(second.root(), &side);
        assert_ne!(second, PersistentName::piece(2, side));
    }

    #[test]
    fn a_name_is_written_and_read_back() {
        let cases = [
            ("F2:side(e7)", NameRole::Side, vec![7]),
            ("F2:endCap(e7,e8,e9)", NameRole::EndCap, vec![7, 8, 9]),
            ("F2:lateralEdge(e9,e7)", NameRole::LateralEdge, vec![9, 7]),
            (
                "F18446744073709551615:startVertex()",
                NameRole::StartVertex,
                vec![],
            ),
            ("F5:copy(2)/F2:side(e7)", NameRole::Copy, vec![2]),
            ("F3:face(5)", NameRole::Face, vec![5]),
            ("F3:edge(11)", NameRole::Edge, vec![11]),
            ("F3:vertex(0)", NameRole::Vertex, vec![0]),
            ("F4:copy(1)/F3:face(5)", NameRole::Copy, vec![1]),
            ("F2:piece(1)/F2:side(e7)", NameRole::Piece, vec![1]),
            (
                "F5:copy(0)/F2:piece(3)/F2:lateralEdge(e7,e7)",
                NameRole::Copy,
                vec![0],
            ),
            (
                "F9:copy(0)/F5:copy(11)/F2:endCap(e7,e8)",
                NameRole::Copy,
                vec![0],
            ),
        ];
        for (text, role, sources) in cases {
            let name: PersistentName = text.parse().unwrap();
            assert_eq!((name.role, &name.sources), (role, &sources), "{text}");
            assert_eq!(name.to_string(), text);
            // In JSON a name is its text.
            let json = serde_json::to_value(&name).unwrap();
            assert_eq!(json, serde_json::json!(text));
            assert_eq!(
                serde_json::from_value::<PersistentName>(json).unwrap(),
                name
            );
        }
        assert_eq!(
            "F3:side( e4, e5 )"
                .parse::<PersistentName>()
                .unwrap()
                .sources,
            [4, 5],
            "spaces after commas are accepted"
        );
        for role in NameRole::ALL {
            assert_eq!(NameRole::from_name(role.name()), Some(role));
            assert_eq!(
                serde_json::to_value(role).unwrap(),
                serde_json::json!(role.name())
            );
        }
        assert!(NameRole::Side.is_face() && !NameRole::LateralEdge.is_face());
        assert!(NameRole::Face.is_face() && !NameRole::Edge.is_face());
        assert!(!NameRole::Copy.is_face(), "a copy is what it is a copy of");
    }

    #[test]
    fn a_copy_is_what_it_is_a_copy_of() {
        let side: PersistentName = "F2:side(e7)".parse().unwrap();
        let copy = PersistentName::copy(FeatureId(5), 2, side.clone());
        assert_eq!(copy.to_string(), "F5:copy(2)/F2:side(e7)");
        assert_eq!(copy.of.as_deref(), Some(&side));
        assert_eq!((copy.depth(), copy.root()), (2, &side));
        assert!(copy.is_face() && !copy.role.is_face());
        let twice = PersistentName::copy(FeatureId(9), 0, copy.clone());
        assert_eq!(twice.to_string().parse::<PersistentName>().unwrap(), twice);
        assert_eq!((twice.depth(), twice.root()), (3, &side));
        let edge: PersistentName = "F7:copy(0)/F2:lateralEdge(e7,e8)".parse().unwrap();
        assert!(!edge.is_face());
        // The deepest name that is accepted, and one deeper.
        let deep = "F1:copy(0)/".repeat(MAX_NAME_DEPTH - 1) + "F2:side(e7)";
        let name: PersistentName = deep.parse().unwrap();
        assert_eq!(name.depth(), MAX_NAME_DEPTH);
        assert_eq!(name.to_string(), deep);
        assert!(
            ("F1:copy(0)/".to_string() + &deep)
                .parse::<PersistentName>()
                .is_err()
        );
        // Built from parts, a name can be one that cannot be written.
        assert_eq!(name.check(), Ok(()));
        assert!(PersistentName::copy(FeatureId(1), 0, name).check().is_err());
        let mut bare = copy.clone();
        bare.of = None;
        assert!(bare.check().is_err());
        let mut two = copy.clone();
        two.sources.push(1);
        assert!(two.check().is_err());
        let mut tail = side.clone();
        tail.of = Some(Box::new(side.clone()));
        assert!(tail.check().is_err());
        let mut wide = side.clone();
        wide.sources = vec![1; MAX_NAME_SOURCES + 1];
        assert!(wide.check().is_err());
    }

    #[test]
    fn text_that_is_not_a_name_is_refused_with_a_reason() {
        for (text, reason) in [
            ("", "no `:`"),
            ("side(e7)", "no `:`"),
            ("2:side(e7)", "F and a number"),
            ("F:side(e7)", "F and a number"),
            ("F-1:side(e7)", "F and a number"),
            ("F99999999999999999999:side(e7)", "F and a number"),
            ("F2:side", "no `(`"),
            ("F2:roof(e7)", "the role must be one of"),
            ("F2:side(e7", "does not end with `)`"),
            ("F2:side(7)", "e and a number"),
            ("F2:side(e7,)", "e and a number"),
            ("F2:side(e7)x", "does not end with `)`"),
            ("F2:side(e7;e8)", "e and a number"),
            ("F5:copy(2)", "followed by `/`"),
            ("F5:copy(2)/", "no `:`"),
            ("F5:copy()/F2:side(e7)", "one number"),
            ("F5:copy(1,2)/F2:side(e7)", "one number"),
            ("F3:face()", "one number"),
            ("F3:face(e1)", "plain number"),
            ("F3:edge(1,2)", "one number"),
            ("F3:face(1)/F2:side(e7)", "only a copy"),
            ("F5:copy(e2)/F2:side(e7)", "plain number"),
            ("F5:copy(+2)/F2:side(e7)", "plain number"),
            ("F2:side(e7)/F1:side(e1)", "only a copy"),
            ("/F2:side(e7)", "no `:`"),
        ] {
            let error = text.parse::<PersistentName>().unwrap_err();
            assert!(error.contains(reason), "`{text}`: {error}");
        }
        // Hostile lengths: an error or a name, never a panic or a huge
        // allocation.
        assert!(
            ("F2:side(".to_string() + &"e1,".repeat(100_000) + "e1)")
                .parse::<PersistentName>()
                .is_err()
        );
        assert!("💥".repeat(10_000).parse::<PersistentName>().is_err());
        assert!(
            "F1:copy(0)/"
                .repeat(100_000)
                .parse::<PersistentName>()
                .is_err()
        );
        assert!("/".repeat(100_000).parse::<PersistentName>().is_err());
        // The sources of all links together are capped.
        let many = "e1,".repeat(MAX_NAME_SOURCES - 1) + "e1";
        assert!(format!("F2:side({many})").parse::<PersistentName>().is_ok());
        assert!(
            format!("F5:copy(0)/F2:side({many})")
                .parse::<PersistentName>()
                .is_err()
        );
        assert!(serde_json::from_value::<PersistentName>(serde_json::json!(7)).is_err());
    }

    #[test]
    fn an_edge_and_a_vertex_are_named_by_their_faces() {
        let a = "F7:copy(0)/F1:face(1)";
        let b = "F7:copy(1)/F2:endCap(e7,e8,e9)";
        let c = "F7:copy(1)/F2:side(e7)";
        for (text, role, faces, near, index) in [
            (
                format!("F7:between[{a},{b}]"),
                NameRole::Between,
                2,
                0,
                None,
            ),
            (
                format!("F7:between[{a},{a}]"),
                NameRole::Between,
                2,
                0,
                None,
            ),
            (
                format!("F7:between[{a},{b}|{c}]"),
                NameRole::Between,
                2,
                1,
                None,
            ),
            (
                format!("F7:between[{a},{b}|{c}](1)"),
                NameRole::Between,
                2,
                1,
                Some(1),
            ),
            (
                format!("F7:between[{a},{b}](0)"),
                NameRole::Between,
                2,
                0,
                Some(0),
            ),
            (
                format!("F7:corner[{a},{b},{c}]"),
                NameRole::Corner,
                3,
                0,
                None,
            ),
            (
                format!("F7:corner[{a}|{b},{c}](2)"),
                NameRole::Corner,
                1,
                2,
                Some(2),
            ),
            ("F7:corner[]".to_string(), NameRole::Corner, 0, 0, None),
            (
                "F7:corner[](3)".to_string(),
                NameRole::Corner,
                0,
                0,
                Some(3),
            ),
            (
                format!("F9:copy(1)/F8:copy(0)/F7:corner[{a},{b},{c}]"),
                NameRole::Copy,
                3,
                0,
                None,
            ),
        ] {
            let name: PersistentName = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
            let root = name.root();
            assert_eq!(name.role, role, "{text}");
            assert_eq!(
                (
                    root.faces().len(),
                    root.near().len(),
                    root.sources.first().copied()
                ),
                (faces, near, index),
                "{text}"
            );
            assert_eq!(name.to_string(), text);
            assert_eq!(name.check(), Ok(()), "{text}");
            assert!(!name.is_face(), "{text}");
            assert_eq!(name.is_vertex(), text.contains("corner"), "{text}");
            assert_eq!(name.is_edge(), text.contains("between"), "{text}");
            let json = serde_json::to_value(&name).unwrap();
            assert_eq!(json, serde_json::json!(text));
            assert_eq!(
                serde_json::from_value::<PersistentName>(json).unwrap(),
                name
            );
        }
        // Built from parts, it is the same name.
        let face = |text: &str| text.parse::<PersistentName>().unwrap();
        let built =
            PersistentName::corner(FeatureId(7), vec![face(a), face(b)], vec![face(c)], Some(1));
        assert_eq!(built.to_string(), format!("F7:corner[{a},{b}|{c}](1)"));
        assert_eq!(built.to_string().parse::<PersistentName>().unwrap(), built);
        let edge = PersistentName::between(FeatureId(7), vec![face(a), face(b)], Vec::new(), None);
        assert_eq!(edge.to_string(), format!("F7:between[{a},{b}]"));
        // The faces in any order are the same name, written in the one
        // order.
        let typed: PersistentName = format!("F7:corner[{c},{b},{a}|{c},{a}](1)")
            .parse()
            .unwrap();
        assert_eq!(
            typed.to_string(),
            format!("F7:corner[{a},{b},{c}|{a},{c}](1)")
        );
        assert_eq!(
            format!("F9:copy(0)/F7:between[{b},{a}]")
                .parse::<PersistentName>()
                .unwrap(),
            format!("F9:copy(0)/F7:between[{a},{b}]")
                .parse::<PersistentName>()
                .unwrap()
        );
        // It is as deep as its faces: it is of the same body as they are.
        assert_eq!((face(a).depth(), built.depth()), (2, 2));
        assert_eq!(
            PersistentName::copy(FeatureId(8), 0, built.clone()).depth(),
            3
        );
        assert!(face("F3:vertex(1)").is_vertex() && face("F3:startVertex(e1)").is_vertex());
        assert!(face("F3:edge(1)").is_edge() && face("F3:lateralEdge(e1,e2)").is_edge());
        assert!(face("F4:copy(0)/F3:endVertex(e1)").is_vertex());
    }

    #[test]
    fn text_that_is_not_a_name_by_faces_is_refused_with_a_reason() {
        let a = "F7:copy(0)/F1:face(1)";
        for (text, reason) in [
            ("F7:corner".to_string(), "no `(`"),
            ("F7:corner(1)".to_string(), "followed by `[`"),
            (format!("F7:corner[{a}"), "no `]`"),
            (format!("F7:corner[{a}]x"), "after `]`"),
            (format!("F7:corner[{a}]()"), "after `]`"),
            (format!("F7:corner[{a}](1,2)"), "after `]`"),
            (format!("F7:corner[{a}](e1)"), "plain number"),
            (format!("F7:corner[{a}|{a}|{a}]"), "one `|`"),
            (format!("F7:corner[{a},]"), "no `:`"),
            ("F7:corner[F1:vertex(1)]".to_string(), "not a face"),
            ("F7:corner[F7:copy(0)/F1:edge(1)]".to_string(), "not a face"),
            (
                format!("F7:corner[F7:corner[{a}]]"),
                "list no faces themselves",
            ),
            (format!("F7:corner[{a}]/F2:side(e7)"), "only a copy"),
            (format!("F7:side[{a}]"), "no `(`"),
            (format!("F7:face[{a}](1)"), "no `(`"),
            (format!("F7:corner[{a}"), "no `]`"),
            ("F7:corner[[[[".to_string(), "no `]`"),
            ("F7:corner]".to_string(), "no `(`"),
        ] {
            let error = text.parse::<PersistentName>().unwrap_err();
            assert!(error.contains(reason), "`{text}`: {error}");
        }
        // The caps: faces listed, sources of all of them, depth.
        let many = |n: usize| format!("F7:corner[{}]", vec![a; n].join(","));
        assert!(many(MAX_NAME_FACES).parse::<PersistentName>().is_ok());
        assert!(
            many(MAX_NAME_FACES + 1)
                .parse::<PersistentName>()
                .unwrap_err()
                .contains("too many faces")
        );
        let wide = format!(
            "F2:side({})",
            vec!["e1"; MAX_NAME_SOURCES / 2 + 1].join(",")
        );
        assert!(
            format!("F2:corner[{wide}]")
                .parse::<PersistentName>()
                .is_ok()
        );
        assert!(
            format!("F2:corner[{wide},{wide}]")
                .parse::<PersistentName>()
                .unwrap_err()
                .contains("too many sources")
        );
        let deep = "F1:copy(0)/".repeat(MAX_NAME_DEPTH - 1) + "F2:side(e7)";
        let corner = format!("F1:corner[{deep}]");
        assert_eq!(
            corner.parse::<PersistentName>().unwrap().depth(),
            MAX_NAME_DEPTH
        );
        assert!(
            format!("F0:copy(0)/{corner}")
                .parse::<PersistentName>()
                .is_err()
        );
        // Hostile lengths and nesting: an error, never a panic.
        assert!(
            ("F7:corner[".repeat(10_000) + &"]".repeat(10_000))
                .parse::<PersistentName>()
                .is_err()
        );
        assert!(
            format!("F7:corner[{}", "(".repeat(100_000))
                .parse::<PersistentName>()
                .is_err()
        );
        assert!("|".repeat(100_000).parse::<PersistentName>().is_err());
        // Built from parts, a name can be one that cannot be written.
        let face: PersistentName = a.parse().unwrap();
        let good = PersistentName::corner(FeatureId(7), vec![face.clone()], Vec::new(), None);
        assert_eq!(good.check(), Ok(()));
        let nested = PersistentName::corner(FeatureId(7), vec![good.clone()], Vec::new(), None);
        assert!(nested.check().is_err());
        let listing = face.clone().with_faces(Vec::new(), vec![face.clone()]);
        assert!(listing.check().is_err(), "a face lists no faces");
        let mut two = good.clone();
        two.sources = vec![1, 2];
        assert!(two.check().is_err());
        let vertex = PersistentName::new(FeatureId(1), NameRole::Vertex, vec![1]);
        assert!(
            PersistentName::corner(FeatureId(7), vec![vertex], Vec::new(), None)
                .check()
                .is_err()
        );
        // Written, a name never goes deeper than one list, whatever it is.
        assert_eq!(nested.to_string(), "F7:corner[F7:corner[]]");
    }
}
