//! Named sets: a query kept in the document under a name (docs/decisions.md,
//! 2026-10-21, B2). A set is not a list of what it held when it was made:
//! it is the query itself, the JSON filters of `select.query`, computed
//! again against every regeneration, so a set of "the 6 mm holes" grows
//! with the design as a formula does.
//!
//! The document keeps the query as text and does not read it: what the
//! filters mean is decided where the bodies are known (`craft3d-engine`,
//! `select.query`), which also checks a query before it is stored. Like
//! the appearance of bodies and faces, sets are beside the history, not in
//! it: regeneration does not read them. Changing them is an undo step all
//! the same, as every change of the document is.

use crate::{DocError, Document, MAX_NAME_LEN};
use serde::{Deserialize, Serialize};

/// Most sets a document holds.
pub const MAX_SETS: usize = 1000;
/// Longest query a set keeps, in bytes.
pub const MAX_SET_QUERY_BYTES: usize = 64 * 1024;

/// A query kept under a name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedSet {
    pub name: String,
    /// The query as JSON text: `{"kind": [...], "where": filter}`, either
    /// part optional, with features referred to by id (`F3`), so that a
    /// renamed feature does not change what the set holds.
    pub query: String,
}

/// Why `name` cannot name a set.
fn check_set_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.trim() != name {
        return Err("a set's name must not be empty or start or end with a space".into());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("a set's name is at most {MAX_NAME_LEN} bytes long"));
    }
    if name.chars().any(char::is_control) {
        return Err("a set's name must not contain control characters".into());
    }
    Ok(())
}

fn check_query(query: &str) -> Result<(), String> {
    if query.len() > MAX_SET_QUERY_BYTES {
        return Err(format!(
            "a set's query is at most {MAX_SET_QUERY_BYTES} bytes long"
        ));
    }
    if query.trim().is_empty() {
        return Err("a set's query must not be empty".into());
    }
    Ok(())
}

impl Document {
    /// The set called `name`.
    pub fn set_named(&self, name: &str) -> Option<&NamedSet> {
        self.sets.iter().find(|set| set.name == name)
    }

    /// Keeps `query` under `name`: a new set at the end, or the query of
    /// the set of that name replaced in its place.
    pub fn define_set(&mut self, name: &str, query: String) -> Result<(), DocError> {
        check_set_name(name).map_err(DocError::Invalid)?;
        check_query(&query).map_err(DocError::Invalid)?;
        if let Some(set) = self.sets.iter_mut().find(|set| set.name == name) {
            set.query = query;
            return Ok(());
        }
        if self.sets.len() >= MAX_SETS {
            return Err(DocError::Invalid(format!(
                "a document holds at most {MAX_SETS} sets"
            )));
        }
        self.sets.push(NamedSet {
            name: name.to_string(),
            query,
        });
        Ok(())
    }

    /// Gives the set `from` the name `to`, in its place.
    pub fn rename_set(&mut self, from: &str, to: &str) -> Result<(), DocError> {
        check_set_name(to).map_err(DocError::Invalid)?;
        if from != to && self.set_named(to).is_some() {
            return Err(DocError::Invalid(format!(
                "a set named `{to}` already exists"
            )));
        }
        let set = self
            .sets
            .iter_mut()
            .find(|set| set.name == from)
            .ok_or_else(|| DocError::Invalid(format!("there is no set named `{from}`")))?;
        set.name = to.to_string();
        Ok(())
    }

    /// Takes the set `name` away, and returns it.
    pub fn remove_set(&mut self, name: &str) -> Result<NamedSet, DocError> {
        let at = self
            .sets
            .iter()
            .position(|set| set.name == name)
            .ok_or_else(|| DocError::Invalid(format!("there is no set named `{name}`")))?;
        Ok(self.sets.remove(at))
    }

    pub(crate) fn validate_sets(&self) -> Result<(), DocError> {
        if self.sets.len() > MAX_SETS {
            return Err(DocError::Invalid(format!(
                "the document has {} sets; at most {MAX_SETS} are read",
                self.sets.len()
            )));
        }
        for (k, set) in self.sets.iter().enumerate() {
            let bad = |reason: String| {
                let shown: String = set.name.chars().take(80).collect();
                DocError::Invalid(format!("the set `{shown}`: {reason}"))
            };
            check_set_name(&set.name).map_err(bad)?;
            check_query(&set.query).map_err(bad)?;
            if self
                .sets
                .iter()
                .take(k)
                .any(|earlier| earlier.name == set.name)
            {
                return Err(bad("two sets have that name".into()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn sets_are_defined_replaced_renamed_and_removed_in_place() {
        let mut doc = Document::default();
        doc.define_set("holes", r#"{"where":{"hole":true}}"#.into())
            .unwrap();
        doc.define_set("tops", r#"{"where":{"normal":{"along":"z"}}}"#.into())
            .unwrap();
        doc.define_set("holes", r#"{"kind":["face"]}"#.into())
            .unwrap();
        assert_eq!(doc.sets.len(), 2);
        assert_eq!(doc.sets[0].name, "holes");
        assert_eq!(doc.sets[0].query, r#"{"kind":["face"]}"#);
        assert!(doc.rename_set("holes", "tops").is_err());
        doc.rename_set("holes", "bores").unwrap();
        assert_eq!(doc.sets[0].name, "bores");
        assert!(doc.validate_sets().is_ok());
        assert_eq!(
            doc.remove_set("bores").unwrap().query,
            r#"{"kind":["face"]}"#
        );
        assert!(doc.remove_set("bores").is_err());
        for bad in ["", " x", "x ", "a\u{7}b"] {
            assert!(doc.define_set(bad, "{}".into()).is_err(), "{bad:?}");
        }
        assert!(doc.define_set("x", String::new()).is_err());
        assert!(
            doc.define_set("x", "x".repeat(MAX_SET_QUERY_BYTES + 1))
                .is_err()
        );
        // A file with two sets of one name is refused.
        doc.sets.push(doc.sets[0].clone());
        assert!(doc.validate_sets().is_err());
    }
}
