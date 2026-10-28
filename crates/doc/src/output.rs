//! Outputs: read-only variables computed after regeneration, from a
//! measure of the bodies or from an expression of the variables and of
//! other outputs (docs/design/verification.md › *Outputs*; decided by the
//! developer, 2026-10-27).
//!
//! Regeneration stays one way: geometry first, then the measures, then
//! the outputs and the assertions. An output may be used by other outputs
//! (their expressions and the expressions inside their measures) and by
//! assertions, never by a variable, a feature, a sketch, a datum, a
//! public parameter's range, a named set or a body's colour or material:
//! so a measure can never depend on itself through the geometry. The
//! engine refuses such a use by name when an edit makes it; this crate
//! keeps the names apart (an output, a variable and a table never share
//! one).
//!
//! Like assertions, outputs are not part of the history: regeneration
//! does not read them.

use crate::{DocError, Document, Expr};
use serde::{Deserialize, Serialize};

/// Most outputs a document holds.
pub const MAX_OUTPUTS: usize = 1_000;

/// One output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Output {
    /// A name as a variable's, unique among the variables, the tables and
    /// the outputs.
    pub name: String,
    pub def: OutputDef,
}

/// How an output is computed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OutputDef {
    /// A measure, kept as an assertion keeps what it measures: a JSON
    /// object `{"measure": name, ...its parameters}` with the subjects'
    /// features by id, as the text the engine wrote. The engine reads it;
    /// this crate only keeps it.
    Measure(String),
    /// An expression of the variables and of other outputs.
    Expr(Expr),
}

impl Output {
    /// The expression of an output computed from one.
    pub fn expr(&self) -> Option<&Expr> {
        match &self.def {
            OutputDef::Expr(e) => Some(e),
            OutputDef::Measure(_) => None,
        }
    }
}

impl Document {
    /// The output named `name`.
    pub fn output(&self, name: &str) -> Option<&Output> {
        self.outputs.iter().find(|o| o.name == name)
    }

    /// Defines an output or replaces how it is computed (it keeps its
    /// place). Refused for a name a variable or a table has.
    pub fn set_output(&mut self, output: Output) -> Result<(), DocError> {
        check_output(&output)?;
        if self.variable(&output.name).is_some() {
            return Err(DocError::Invalid(format!(
                "`{}` is a variable (an input); an output needs a name of its own",
                output.name
            )));
        }
        if self.table(&output.name).is_some() {
            return Err(DocError::Invalid(format!(
                "`{}` is a table; an output needs a name of its own",
                output.name
            )));
        }
        if let Some(slot) = self.outputs.iter_mut().find(|o| o.name == output.name) {
            *slot = output;
            return Ok(());
        }
        if self.outputs.len() >= MAX_OUTPUTS {
            return Err(DocError::Invalid(format!(
                "a document holds at most {MAX_OUTPUTS} outputs"
            )));
        }
        self.outputs.push(output);
        Ok(())
    }

    /// Removes the output named `name` and returns it.
    pub fn remove_output(&mut self, name: &str) -> Result<Output, DocError> {
        let at = self
            .outputs
            .iter()
            .position(|o| o.name == name)
            .ok_or_else(|| DocError::Invalid(format!("there is no output named `{name}`")))?;
        Ok(self.outputs.remove(at))
    }

    /// Renames output `old` to `new` in its definition, in the expressions
    /// of the other outputs and in the assertions' bounds. The
    /// expressions inside what measures measure (JSON text this crate
    /// does not read) are the engine's to rename. All or nothing.
    pub fn rename_output(&mut self, old: &str, new: &str) -> Result<(), DocError> {
        craft3d_expr::check_variable_name(new)?;
        if self.output(old).is_none() {
            return Err(DocError::Invalid(format!(
                "there is no output named `{old}`"
            )));
        }
        if old == new {
            return Ok(());
        }
        if self.output(new).is_some() || self.variable(new).is_some() || self.table(new).is_some() {
            return Err(DocError::Invalid(format!(
                "`{new}` is taken (a variable, a table or an output)"
            )));
        }
        let mut outputs = self.outputs.clone();
        for output in &mut outputs {
            if let OutputDef::Expr(e) = &mut output.def {
                *e = Expr::new(craft3d_expr::rename_variable(e.source(), old, new)?);
            }
            if output.name == old {
                output.name = new.to_string();
            }
        }
        let mut assertions = self.assertions.clone();
        for assertion in &mut assertions {
            for e in assertion.exprs_mut() {
                *e = Expr::new(craft3d_expr::rename_variable(e.source(), old, new)?);
            }
        }
        self.outputs = outputs;
        self.assertions = assertions;
        Ok(())
    }

    pub(crate) fn validate_outputs(&self) -> Result<(), DocError> {
        if self.outputs.len() > MAX_OUTPUTS {
            return Err(DocError::Invalid(format!(
                "the document has {} outputs; at most {MAX_OUTPUTS} are read",
                self.outputs.len()
            )));
        }
        let mut names = std::collections::BTreeSet::new();
        for output in &self.outputs {
            check_output(output)?;
            if !names.insert(output.name.as_str()) {
                return Err(DocError::Invalid(format!(
                    "two outputs are named `{}`",
                    output.name
                )));
            }
            if self.variable(&output.name).is_some() || self.table(&output.name).is_some() {
                return Err(DocError::Invalid(format!(
                    "`{}` names both an output and a variable or a table",
                    output.name
                )));
            }
        }
        Ok(())
    }
}

fn check_output(output: &Output) -> Result<(), DocError> {
    craft3d_expr::check_variable_name(&output.name)?;
    let len = match &output.def {
        OutputDef::Measure(text) => text.len(),
        OutputDef::Expr(e) => e.source().len(),
    };
    if len > crate::MAX_ASSERTION_TEXT {
        return Err(DocError::Invalid(format!(
            "how output `{}` is computed is longer than {} bytes",
            output.name,
            crate::MAX_ASSERTION_TEXT
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::Units;

    fn doc() -> Document {
        let mut doc = Document::new(Units::default());
        doc.set_variable("KG", "180 mm").unwrap();
        doc
    }

    fn expr(name: &str, source: &str) -> Output {
        Output {
            name: name.to_string(),
            def: OutputDef::Expr(Expr::new(source)),
        }
    }

    #[test]
    fn an_output_takes_no_name_a_variable_or_a_table_has() {
        let mut doc = doc();
        assert!(doc.set_output(expr("KG", "1 mm")).is_err());
        doc.set_output(expr("GM", "KB - KG")).unwrap();
        // Nor may a variable take an output's name.
        assert!(doc.set_variable("GM", "1 mm").is_err());
        doc.set_output(expr("GM", "KB + BM - KG")).unwrap();
        assert_eq!(doc.outputs.len(), 1);
        assert!(doc.validate().is_ok());
        assert!(doc.remove_output("GM").is_ok());
        assert!(doc.remove_output("GM").is_err());
    }

    #[test]
    fn renaming_an_output_renames_its_users() {
        let mut doc = doc();
        doc.set_output(expr("BM", "1 mm")).unwrap();
        doc.set_output(expr("GM", "BM - KG")).unwrap();
        doc.add_assertion(crate::Assertion {
            name: "stable".into(),
            measure: "{\"measure\":\"value\",\"expr\":\"GM\"}".into(),
            expect: crate::Expect::Within {
                min: Some(Expr::new("BM / 10")),
                max: None,
            },
            when: Some(Expr::new("BM > 0 mm")),
        })
        .unwrap();
        doc.rename_output("BM", "metacentre").unwrap();
        assert_eq!(
            doc.output("GM").and_then(Output::expr).map(Expr::source),
            Some("metacentre - KG")
        );
        assert!(doc.output("metacentre").is_some());
        let bound = doc.assertions.first().unwrap().expect.exprs()[0].source();
        assert_eq!(bound, "metacentre / 10");
        let when = doc.assertions.first().unwrap().when.as_ref().unwrap();
        assert_eq!(when.source(), "metacentre > 0 mm");
        assert!(doc.rename_output("GM", "KG").is_err());
    }
}
