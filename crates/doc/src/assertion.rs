//! Assertions: the design intent written down as checks the document
//! carries (docs/design/verification.md). An assertion names a measure of
//! some part of the design (what is measured and of what, as the JSON
//! object `assert.add` was given: the measure, its subjects as
//! `select.query` filters and its options) and what it is expected to be.
//!
//! Like the appearance of bodies, assertions are not part of the history:
//! regeneration does not read them, and a broken one never stops a
//! feature from being built. The engine checks them after regeneration.

use crate::{DocError, Document, Expr};
use serde::{Deserialize, Serialize};

/// Most assertions a document holds.
pub const MAX_ASSERTIONS: usize = 1_000;
/// Longest text of what an assertion measures, in bytes: room for the two
/// things a relation measures, each named in as many bytes as a selection
/// takes (1 MiB, the longest request), with what else it says (the review
/// of 2026-11-17, round three: two faces of a profile of 2000 segments,
/// named in 12 KB each, could not be kept as a check under 16 KiB).
pub const MAX_ASSERTION_TEXT: usize = 4 << 20;
/// Longest text of what a document's assertions measure, all together, in
/// bytes: 16 MiB, a quarter of the 64 MiB a document's manifest may take
/// (`craft3d_format::MAX_MANIFEST_BYTES`). The text is written escaped in
/// the manifest, up to twice as long, and the features, sketches and
/// variables need the rest; without it 1000 assertions of 4 MiB would be 4
/// GiB that could never be saved (the review of 2026-11-17). An ordinary
/// check measures a few hundred bytes: some 50 000 of them would fit.
pub const MAX_ASSERTIONS_TEXT: usize = 16 << 20;

/// One assertion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assertion {
    /// Unique within the document, as a feature's name is.
    pub name: String,
    /// What is measured and of what: a JSON object, kept as the text the
    /// engine wrote (canonical: keys in order, no spaces). The engine
    /// reads it; this crate only keeps it.
    pub measure: String,
    pub expect: Expect,
    /// A condition that switches the assertion on and off, as a feature's
    /// `when` does: an expression of the variables and outputs. While it
    /// is zero the assertion is *inactive*: not measured, and counted as
    /// neither passed nor failed nor unproven. `None`: always on.
    #[serde(default)]
    pub when: Option<Expr>,
}

impl Assertion {
    /// Every expression it holds: what it expects, then its condition.
    pub fn exprs(&self) -> Vec<&Expr> {
        let mut out = self.expect.exprs();
        out.extend(self.when.as_ref());
        out
    }

    pub fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        let mut out = self.expect.exprs_mut();
        out.extend(self.when.as_mut());
        out
    }
}

/// What a measure is expected to be. Values are expressions in the
/// document's variables, so an assertion follows the design like a
/// formula does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Expect {
    /// A yes-or-no measure (through, no interference, symmetric) is this.
    Is(bool),
    /// Equal to `value` within `tol` (the engine's default when absent).
    Equal { value: Expr, tol: Option<Expr> },
    /// At least `min` and at most `max`; at least one is given.
    Within {
        min: Option<Expr>,
        max: Option<Expr>,
    },
}

impl Expect {
    /// The expressions it holds.
    pub fn exprs(&self) -> Vec<&Expr> {
        match self {
            Expect::Is(_) => Vec::new(),
            Expect::Equal { value, tol } => std::iter::once(value).chain(tol.as_ref()).collect(),
            Expect::Within { min, max } => min.iter().chain(max.iter()).collect(),
        }
    }

    pub fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        match self {
            Expect::Is(_) => Vec::new(),
            Expect::Equal { value, tol } => std::iter::once(value).chain(tol.as_mut()).collect(),
            Expect::Within { min, max } => min.iter_mut().chain(max.iter_mut()).collect(),
        }
    }
}

impl Document {
    /// The assertion named `name`.
    pub fn assertion(&self, name: &str) -> Option<&Assertion> {
        self.assertions.iter().find(|a| a.name == name)
    }

    /// Adds an assertion at the end.
    pub fn add_assertion(&mut self, assertion: Assertion) -> Result<(), DocError> {
        if self.assertions.len() >= MAX_ASSERTIONS {
            return Err(DocError::Invalid(format!(
                "a document holds at most {MAX_ASSERTIONS} assertions"
            )));
        }
        check_assertion(&assertion)?;
        if self.assertion(&assertion.name).is_some() {
            return Err(DocError::Invalid(format!(
                "an assertion named `{}` already exists",
                assertion.name
            )));
        }
        self.check_assertions_text(None, &assertion)?;
        self.assertions.push(assertion);
        Ok(())
    }

    /// Refuses `assertion` (in place of the one named `instead`) when what
    /// the document's assertions measure would then pass
    /// [`MAX_ASSERTIONS_TEXT`] together, with what to do about it.
    fn check_assertions_text(
        &self,
        instead: Option<&str>,
        assertion: &Assertion,
    ) -> Result<(), DocError> {
        let others: usize = self
            .assertions
            .iter()
            .filter(|a| Some(a.name.as_str()) != instead)
            .map(|a| a.measure.len())
            .fold(0, usize::saturating_add);
        let mib = |bytes: usize| bytes as f64 / f64::from(1_u32 << 20);
        if others.saturating_add(assertion.measure.len()) > MAX_ASSERTIONS_TEXT {
            return Err(DocError::Invalid(format!(
                "what the document's assertions measure is {:.1} MiB together; `{}` would add {:.1} MiB to it, past the {} MiB a document holds (so it can still be saved): delete checks that are no longer needed (assert.delete), or name fewer things or things with shorter names",
                mib(others),
                assertion.name,
                mib(assertion.measure.len()),
                MAX_ASSERTIONS_TEXT >> 20
            )));
        }
        Ok(())
    }

    /// Replaces the assertion named `name` (which may be renamed) in place.
    pub fn replace_assertion(&mut self, name: &str, assertion: Assertion) -> Result<(), DocError> {
        check_assertion(&assertion)?;
        if assertion.name != name && self.assertion(&assertion.name).is_some() {
            return Err(DocError::Invalid(format!(
                "an assertion named `{}` already exists",
                assertion.name
            )));
        }
        self.check_assertions_text(Some(name), &assertion)?;
        let slot = self
            .assertions
            .iter_mut()
            .find(|a| a.name == name)
            .ok_or_else(|| DocError::Invalid(format!("there is no assertion named `{name}`")))?;
        *slot = assertion;
        Ok(())
    }

    /// Removes the assertion named `name` and returns it.
    pub fn remove_assertion(&mut self, name: &str) -> Result<Assertion, DocError> {
        let at = self
            .assertions
            .iter()
            .position(|a| a.name == name)
            .ok_or_else(|| DocError::Invalid(format!("there is no assertion named `{name}`")))?;
        Ok(self.assertions.remove(at))
    }

    pub(crate) fn validate_assertions(&self) -> Result<(), DocError> {
        if self.assertions.len() > MAX_ASSERTIONS {
            return Err(DocError::Invalid(format!(
                "the document has {} assertions; at most {MAX_ASSERTIONS} are read",
                self.assertions.len()
            )));
        }
        let total = self
            .assertions
            .iter()
            .map(|a| a.measure.len())
            .fold(0, usize::saturating_add);
        if total > MAX_ASSERTIONS_TEXT {
            return Err(DocError::Invalid(format!(
                "what the document's assertions measure is {total} bytes together; at most {MAX_ASSERTIONS_TEXT} are read"
            )));
        }
        let mut names = std::collections::BTreeSet::new();
        for assertion in &self.assertions {
            check_assertion(assertion)?;
            if !names.insert(assertion.name.as_str()) {
                return Err(DocError::Invalid(format!(
                    "two assertions are named `{}`",
                    assertion.name
                )));
            }
        }
        Ok(())
    }
}

fn check_assertion(assertion: &Assertion) -> Result<(), DocError> {
    let name = &assertion.name;
    if name.is_empty() || name.trim() != name {
        return Err(DocError::Invalid(
            "an assertion's name must not be empty or start or end with a space".to_string(),
        ));
    }
    if name.len() > crate::MAX_NAME_LEN || name.chars().any(char::is_control) {
        return Err(DocError::Invalid(format!(
            "an assertion's name must be at most {} bytes without control characters",
            crate::MAX_NAME_LEN
        )));
    }
    if assertion.measure.len() > MAX_ASSERTION_TEXT {
        return Err(DocError::Invalid(format!(
            "what assertion `{name}` measures is longer than {MAX_ASSERTION_TEXT} bytes"
        )));
    }
    if let Expect::Within {
        min: None,
        max: None,
    } = assertion.expect
    {
        return Err(DocError::Invalid(format!(
            "assertion `{name}` expects a range without a bound"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Units;

    fn measuring(name: &str, bytes: usize) -> Assertion {
        Assertion {
            name: name.to_string(),
            measure: "x".repeat(bytes),
            expect: Expect::Is(true),
            when: None,
        }
    }

    /// The review of 2026-11-17: what a document's assertions measure is
    /// held to MAX_ASSERTIONS_TEXT together, each to MAX_ASSERTION_TEXT;
    /// the refusal says how much there is and what to do; a replacement
    /// counts in place of what it replaces; a file past it is not read.
    #[test]
    fn what_assertions_measure_is_held_together_to_a_total() {
        let mut doc = Document::new(Units::default());
        for k in 0..4 {
            doc.add_assertion(measuring(&format!("check {k}"), MAX_ASSERTION_TEXT))
                .unwrap();
        }
        let refused = doc
            .add_assertion(measuring("one more", 1))
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("16.0 MiB together") && refused.contains("assert.delete"),
            "{refused}"
        );
        doc.replace_assertion("check 0", measuring("check 0", MAX_ASSERTION_TEXT))
            .unwrap();
        doc.remove_assertion("check 3").unwrap();
        doc.add_assertion(measuring("one more", 1)).unwrap();
        doc.validate().unwrap();
        doc.assertions
            .push(measuring("past it", MAX_ASSERTION_TEXT));
        assert!(doc.validate().is_err());
    }
}
