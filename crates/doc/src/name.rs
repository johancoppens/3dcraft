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

use crate::FeatureId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Most sources a name carries (a cap is named after every curve of its
/// outer loop).
pub const MAX_NAME_SOURCES: usize = 4096;

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
}

impl NameRole {
    pub const ALL: [NameRole; 12] = [
        NameRole::StartCap,
        NameRole::EndCap,
        NameRole::Side,
        NameRole::StartEdge,
        NameRole::EndEdge,
        NameRole::LateralEdge,
        NameRole::StartVertex,
        NameRole::EndVertex,
        NameRole::Copy,
        NameRole::Face,
        NameRole::Edge,
        NameRole::Vertex,
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
            NameRole::Face => "face",
            NameRole::Edge => "edge",
            NameRole::Vertex => "vertex",
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
            NameRole::StartCap | NameRole::EndCap | NameRole::Side | NameRole::Face
        )
    }

    /// True for a role whose name continues after `/` with the name of the
    /// entity it was made from.
    pub fn is_derived(self) -> bool {
        matches!(self, NameRole::Copy)
    }

    /// True for a role with exactly one source that is a plain number (an
    /// instance, or the number of an entity of a primitive) and is written
    /// without the `e` of a sketch entity.
    pub fn is_numbered(self) -> bool {
        matches!(
            self,
            NameRole::Copy | NameRole::Face | NameRole::Edge | NameRole::Vertex
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
    /// role defines. For a copy: the number of the instance.
    pub sources: Vec<u64>,
    /// For a derived role (a copy): the name, in the body this feature
    /// was made from, of the entity this one is the image of. `None` for
    /// every other role.
    pub of: Option<Box<PersistentName>>,
}

impl PersistentName {
    /// A name that is not derived from another: `F2:side(e7)`.
    pub fn new(feature: FeatureId, role: NameRole, sources: Vec<u64>) -> Self {
        Self {
            feature,
            role,
            sources,
            of: None,
        }
    }

    /// The image of `of` in instance `instance` of `feature`:
    /// `F5:copy(2)/F2:side(e7)`.
    pub fn copy(feature: FeatureId, instance: u64, of: PersistentName) -> Self {
        Self {
            feature,
            role: NameRole::Copy,
            sources: vec![instance],
            of: Some(Box::new(of)),
        }
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

    /// How many names this one is made of.
    pub fn depth(&self) -> usize {
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

    /// True when the entity is a face.
    pub fn is_face(&self) -> bool {
        self.root().role.is_face()
    }

    /// Whether the name is one that can be written and read back: a copy
    /// has one instance number and the name it is a copy of, nothing else
    /// is followed by another name, and the caps on depth and sources
    /// hold. A name read from text always is; one built from parts (a
    /// file) may not be.
    pub fn check(&self) -> Result<(), &'static str> {
        let mut sources = 0usize;
        let mut name = self;
        for _ in 0..MAX_NAME_DEPTH {
            sources = sources.saturating_add(name.sources.len());
            if sources > MAX_NAME_SOURCES {
                return Err("it has too many sources");
            }
            if name.role.is_numbered() && name.sources.len() != 1 {
                return Err("a copy, face, edge or vertex has one number");
            }
            match (name.role.is_derived(), &name.of) {
                (true, Some(next)) => name = next,
                (true, None) => return Err("a copy without the name it is a copy of"),
                (false, Some(_)) => return Err("only a copy is followed by another name"),
                (false, None) => return Ok(()),
            }
        }
        Err("it is made of too many names")
    }
}

impl fmt::Display for PersistentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut name = self;
        for depth in 0..=MAX_NAME_DEPTH {
            if depth > 0 {
                f.write_str("/")?;
            }
            write!(f, "{}:{}(", name.feature, name.role.name())?;
            let prefix = if name.role.is_numbered() { "" } else { "e" };
            for (i, source) in name.sources.iter().enumerate() {
                if i > 0 {
                    f.write_str(",")?;
                }
                write!(f, "{prefix}{source}")?;
            }
            f.write_str(")")?;
            match &name.of {
                Some(next) => name = next,
                None => break,
            }
        }
        Ok(())
    }
}

impl From<PersistentName> for String {
    fn from(name: PersistentName) -> String {
        name.to_string()
    }
}

/// One name of a chain: `F2:side(e7)`.
fn parse_link(
    text: &str,
    wrong: &dyn Fn(&str) -> String,
    budget: &mut usize,
) -> Result<PersistentName, String> {
    let (feature, rest) = text
        .split_once(':')
        .ok_or_else(|| wrong("there is no `:` after the feature"))?;
    let feature = feature
        .strip_prefix('F')
        .and_then(|digits| digits.parse::<u64>().ok())
        .ok_or_else(|| wrong("the feature must be written F and a number"))?;
    let (role, rest) = rest
        .split_once('(')
        .ok_or_else(|| wrong("there is no `(` after the role"))?;
    let role = NameRole::from_name(role).ok_or_else(|| {
        let roles: Vec<&str> = NameRole::ALL.iter().map(|r| r.name()).collect();
        wrong(&format!("the role must be one of {}", roles.join(", ")))
    })?;
    let inner = rest
        .strip_suffix(')')
        .ok_or_else(|| wrong("it does not end with `)`"))?;
    let mut sources = Vec::new();
    if !inner.is_empty() {
        for part in inner.split(',') {
            if *budget == 0 {
                return Err(wrong("it has too many sources"));
            }
            *budget -= 1;
            let part = part.trim();
            let id = if role.is_numbered() {
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
    }
    if role.is_numbered() && sources.len() != 1 {
        return Err(wrong(
            "this role takes one number, as in copy(0) or face(2)",
        ));
    }
    Ok(PersistentName::new(FeatureId(feature), role, sources))
}

impl FromStr for PersistentName {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let shown: String = text.chars().take(60).collect();
        let wrong =
            |what: &str| format!("`{shown}` is not a name: {what}; a name looks like F2:side(e7)");
        let mut links = Vec::new();
        let mut budget = MAX_NAME_SOURCES;
        for part in text.split('/') {
            if links.len() >= MAX_NAME_DEPTH {
                return Err(wrong("it is made of too many names"));
            }
            links.push(parse_link(part, &wrong, &mut budget)?);
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
}
