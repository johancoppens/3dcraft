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

use crate::FeatureId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Most sources a name carries (a cap is named after every curve of its
/// outer loop).
pub const MAX_NAME_SOURCES: usize = 4096;

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
}

impl NameRole {
    pub const ALL: [NameRole; 8] = [
        NameRole::StartCap,
        NameRole::EndCap,
        NameRole::Side,
        NameRole::StartEdge,
        NameRole::EndEdge,
        NameRole::LateralEdge,
        NameRole::StartVertex,
        NameRole::EndVertex,
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
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }

    /// True for the roles that are faces.
    pub fn is_face(self) -> bool {
        matches!(self, NameRole::StartCap | NameRole::EndCap | NameRole::Side)
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
    /// role defines.
    pub sources: Vec<u64>,
}

impl fmt::Display for PersistentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}(", self.feature, self.role.name())?;
        for (i, source) in self.sources.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            write!(f, "e{source}")?;
        }
        f.write_str(")")
    }
}

impl From<PersistentName> for String {
    fn from(name: PersistentName) -> String {
        name.to_string()
    }
}

impl FromStr for PersistentName {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let shown: String = text.chars().take(60).collect();
        let wrong =
            |what: &str| format!("`{shown}` is not a name: {what}; a name looks like F2:side(e7)");
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
                if sources.len() >= MAX_NAME_SOURCES {
                    return Err(wrong("it has too many sources"));
                }
                let id = part
                    .trim()
                    .strip_prefix('e')
                    .and_then(|digits| digits.parse::<u64>().ok())
                    .ok_or_else(|| wrong("a source must be written e and a number"))?;
                sources.push(id);
            }
        }
        Ok(PersistentName {
            feature: crate::FeatureId(feature),
            role,
            sources,
        })
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
        assert!(serde_json::from_value::<PersistentName>(serde_json::json!(7)).is_err());
    }
}
