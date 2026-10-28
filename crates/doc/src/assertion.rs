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
/// Longest text of what an assertion measures, in bytes.
pub const MAX_ASSERTION_TEXT: usize = 16_384;

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
        self.assertions.push(assertion);
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
