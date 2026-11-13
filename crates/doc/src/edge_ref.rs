//! How a chamfer or a fillet refers to an edge: by the two faces it lies
//! between, which means every edge between them, or by the edge's own
//! name, which means that one edge (docs/design/names.md).
//!
//! Two faces can meet in several edges (a pipe sunk into a plate leaves
//! it along two lines; two discs merged meet in two corners), and the pair
//! names all of them; the edge's own name, as `document.edges` and
//! `view.pick` give it (`F7:between[a,b|c]`, `F2:lateralEdge(e5,e6)`),
//! picks one.
//!
//! As data and as JSON a reference is one of:
//! - `["face", "face"]`: every edge between the two faces; the first is
//!   the face a chamfer's first distance is measured along;
//! - `"edge"`: that one edge;
//! - `["edge", "face"]`: that one edge, with the face of it a chamfer's
//!   first distance is measured along.
//!
//! The first name of a list of two says which form it is: an edge's name
//! starts the third, a face's the first.

use crate::PersistentName;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One edge reference of a chamfer or a fillet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeRef {
    /// Every edge between two faces, by their names. Their order says
    /// which face a chamfer's first distance is measured along.
    Faces([PersistentName; 2]),
    /// One edge, by its own name or an alias of it, and, when given, the
    /// face of that edge a chamfer's first distance is measured along.
    Edge {
        edge: PersistentName,
        first: Option<PersistentName>,
    },
}

impl EdgeRef {
    /// The name the body this reference is of is found by: the first face
    /// of a pair, or the edge.
    pub fn lead(&self) -> &PersistentName {
        match self {
            EdgeRef::Faces([first, _]) => first,
            EdgeRef::Edge { edge, .. } => edge,
        }
    }

    /// Every name the reference holds: the two faces, or the edge and the
    /// face given with it.
    pub fn names(&self) -> Vec<&PersistentName> {
        match self {
            EdgeRef::Faces([a, b]) => vec![a, b],
            EdgeRef::Edge { edge, first } => std::iter::once(edge).chain(first).collect(),
        }
    }

    /// [`EdgeRef::names`], to change.
    pub fn names_mut(&mut self) -> Vec<&mut PersistentName> {
        match self {
            EdgeRef::Faces([a, b]) => vec![a, b],
            EdgeRef::Edge { edge, first } => std::iter::once(edge).chain(first).collect(),
        }
    }

    /// The reference in words, for what is said of it: "the edge between
    /// a and b", "the edge e".
    pub fn describe(&self) -> String {
        match self {
            EdgeRef::Faces([a, b]) => format!("the edge between {a} and {b}"),
            EdgeRef::Edge { edge, .. } => format!("the edge {edge}"),
        }
    }

    /// The reference as it is written in a command's parameters: `a / b`
    /// for a pair, the edge's name otherwise (with ` / face` when a face
    /// is given with it).
    pub fn written(&self) -> String {
        self.names()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// Whether `other` refers to the same edges by the same names: a pair
    /// in either order, an edge by the same name.
    pub fn same_as(&self, other: &EdgeRef) -> bool {
        match (self, other) {
            (EdgeRef::Faces([a, b]), EdgeRef::Faces([c, d])) => {
                (a == c && b == d) || (a == d && b == c)
            }
            (EdgeRef::Edge { edge: a, .. }, EdgeRef::Edge { edge: b, .. }) => a == b,
            _ => false,
        }
    }
}

impl Serialize for EdgeRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            EdgeRef::Faces(pair) => pair.serialize(serializer),
            EdgeRef::Edge { edge, first: None } => edge.serialize(serializer),
            EdgeRef::Edge {
                edge,
                first: Some(face),
            } => [edge, face].serialize(serializer),
        }
    }
}

/// What is read before it is told which form it is.
#[derive(Deserialize)]
#[serde(untagged)]
enum Written {
    One(PersistentName),
    Two([PersistentName; 2]),
}

impl<'de> Deserialize<'de> for EdgeRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Written::deserialize(deserializer)? {
            Written::One(edge) if edge.is_edge() => Ok(EdgeRef::Edge { edge, first: None }),
            Written::One(other) => Err(D::Error::custom(format!(
                "{other} is not an edge; an edge is its own name or the two faces it lies between"
            ))),
            Written::Two([edge, face]) if edge.is_edge() => Ok(EdgeRef::Edge {
                edge,
                first: Some(face),
            }),
            Written::Two(pair) => Ok(EdgeRef::Faces(pair)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> PersistentName {
        text.parse().unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    #[test]
    fn every_form_is_written_and_read_back() {
        let top = name("F3:copy(0)/F1:face(1)");
        let wall = name("F3:copy(1)/F2:face(0)");
        let edge = name("F3:between[F3:copy(0)/F1:face(1),F3:copy(1)/F2:face(0)](1)");
        for (reference, json) in [
            (
                EdgeRef::Faces([top.clone(), wall.clone()]),
                serde_json::json!([top.to_string(), wall.to_string()]),
            ),
            (
                EdgeRef::Edge {
                    edge: edge.clone(),
                    first: None,
                },
                serde_json::json!(edge.to_string()),
            ),
            (
                EdgeRef::Edge {
                    edge: edge.clone(),
                    first: Some(wall.clone()),
                },
                serde_json::json!([edge.to_string(), wall.to_string()]),
            ),
        ] {
            assert_eq!(serde_json::to_value(&reference).unwrap(), json);
            let read: EdgeRef = serde_json::from_value(json).unwrap();
            assert_eq!(read, reference);
        }
        // A face alone is no edge.
        assert!(serde_json::from_value::<EdgeRef>(serde_json::json!(top.to_string())).is_err());
        assert!(serde_json::from_value::<EdgeRef>(serde_json::json!([1, 2])).is_err());
        assert!(serde_json::from_value::<EdgeRef>(serde_json::json!(["F1:face(1)"])).is_err());
    }

    #[test]
    fn the_lead_names_and_sameness() {
        let a = name("F1:face(1)");
        let b = name("F1:face(2)");
        let e = name("F1:edge(4)");
        let pair = EdgeRef::Faces([a.clone(), b.clone()]);
        assert_eq!(pair.lead(), &a);
        assert!(pair.same_as(&EdgeRef::Faces([b.clone(), a.clone()])));
        assert_eq!(pair.written(), "F1:face(1) / F1:face(2)");
        let one = EdgeRef::Edge {
            edge: e.clone(),
            first: Some(b.clone()),
        };
        assert_eq!(one.lead(), &e);
        assert_eq!(one.names(), vec![&e, &b]);
        assert!(one.same_as(&EdgeRef::Edge {
            edge: e.clone(),
            first: None
        }));
        assert!(!one.same_as(&pair));
        assert_eq!(one.describe(), "the edge F1:edge(4)");
    }
}
