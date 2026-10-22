//! Numbers a person types into a field of the window: evaluated as the
//! active document's expressions are (its variables, units such as `mm`),
//! without changing the document. The window's number fields use it; the
//! value they set is view state, not a command.

use crate::{CommandError, Session};
use craft3d_expr::Dimension;

/// Which kind of number a field wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedKind {
    /// A length, in the document's length unit; a bare number is one.
    Length,
    /// A plain number.
    Number,
}

/// The longest message an error of a typed number has, in characters.
const MESSAGE: usize = 160;

/// An error whose message is cut at [`MESSAGE`] characters: the parser's
/// own message may repeat a long name.
fn failed(message: String) -> CommandError {
    if message.chars().count() <= MESSAGE {
        CommandError::new(message)
    } else {
        let mut cut: String = message.chars().take(MESSAGE - 1).collect();
        cut.push('…');
        CommandError::new(cut)
    }
}

impl Session {
    /// The active document's length unit (`mm`, `in`, ...), or nothing
    /// without a document.
    pub fn length_unit(&self) -> Option<&'static str> {
        self.document()
            .ok()
            .map(|open| open.history.current().units.length.symbol())
    }

    /// `source` evaluated as an expression of the active document (or,
    /// without one, of no variables in millimetres): a length in the
    /// document's unit or a plain number, as `kind` says. An expression
    /// that does not parse, names a variable that has no value, or is of
    /// another dimension is an error that says so.
    pub fn evaluate_typed(&self, source: &str, kind: TypedKind) -> Result<f64, CommandError> {
        self.typed().evaluate(source, kind)
    }

    /// The active document's variables evaluated once, for a command that
    /// evaluates many expressions (the corners of a polyline).
    pub(crate) fn typed(&self) -> Typed<'_> {
        let doc = self.document().ok().map(|open| open.history.current());
        let evaluation = doc
            .and_then(|doc| doc.variable_table().ok())
            .map(|table| table.evaluate());
        Typed {
            doc: doc.map(|d| &**d),
            evaluation,
        }
    }
}

/// Expressions evaluated against one state of the active document's
/// variables ([`Session::typed`]).
pub(crate) struct Typed<'a> {
    doc: Option<&'a craft3d_doc::Document>,
    evaluation: Option<craft3d_expr::Evaluation>,
}

impl Typed<'_> {
    /// As [`Session::evaluate_typed`].
    pub(crate) fn evaluate(&self, source: &str, kind: TypedKind) -> Result<f64, CommandError> {
        let doc = self.doc;
        // The text as the message repeats it: a pasted page is cut short.
        let shown = if source.chars().count() > 40 {
            let mut cut: String = source.chars().take(39).collect();
            cut.push('…');
            cut
        } else {
            source.to_string()
        };
        let quantity = craft3d_expr::evaluate(source, |name| {
            self.evaluation.as_ref().and_then(|e| e.value(name))
        })
        .map_err(|e| failed(format!("`{shown}`: {e}")))?;
        let value = match kind {
            TypedKind::Length if quantity.dimension == Dimension::LENGTH => {
                let unit = doc.map_or(craft3d_expr::Unit::Millimetre, |doc| {
                    doc.units.length.as_expr_unit()
                });
                quantity
                    .to_unit(unit)
                    .map_err(|e| failed(format!("`{shown}`: {e}")))?
            }
            _ if quantity.dimension.is_dimensionless() => quantity.value,
            TypedKind::Length => {
                return Err(failed(format!(
                    "`{shown}` is {}, but a length is needed{}",
                    quantity.dimension.described(),
                    quantity.dimension.unit_hint(source)
                )));
            }
            TypedKind::Number => {
                return Err(failed(format!(
                    "`{shown}` is {}, but a plain number is needed{}",
                    quantity.dimension.described(),
                    quantity.dimension.unit_hint(source)
                )));
            }
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err(failed(format!("`{shown}` is not a finite number")))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::Workspace;
    use serde_json::json;

    #[test]
    fn typed_numbers_are_the_documents_expressions() {
        let mut session = Session::new(Workspace::none());
        // Without a document: plain numbers and millimetres.
        assert_eq!(
            session.evaluate_typed("2 * 3", TypedKind::Number).unwrap(),
            6.0
        );
        assert_eq!(
            session.evaluate_typed("1 cm", TypedKind::Length).unwrap(),
            10.0
        );
        assert_eq!(session.length_unit(), None);
        session
            .execute("file.new", &json!({"units": "cm"}))
            .unwrap();
        session
            .execute("variable.set", &json!({"name": "width", "expr": "40 mm"}))
            .unwrap();
        assert_eq!(session.length_unit(), Some("cm"));
        // A long text is evaluated whole; its error repeats only its start.
        let long = format!("{}1", "1 + ".repeat(30));
        assert_eq!(
            session.evaluate_typed(&long, TypedKind::Number).unwrap(),
            31.0
        );
        let error = session
            .evaluate_typed(&"x".repeat(5000), TypedKind::Number)
            .unwrap_err();
        assert!(error.message().chars().count() < 200, "{}", error.message());
        // In the document's unit: 40 mm / 2 - 3 mm = 17 mm = 1.7 cm.
        let v = session
            .evaluate_typed("width / 2 - 3 mm", TypedKind::Length)
            .unwrap();
        assert!((v - 1.7).abs() < 1e-12, "{v}");
        assert_eq!(session.evaluate_typed("4", TypedKind::Length).unwrap(), 4.0);
        for (bad, kind) in [
            ("width +", TypedKind::Length),
            ("nothing", TypedKind::Length),
            ("width * width", TypedKind::Length),
            ("width", TypedKind::Number),
            ("1 / 0", TypedKind::Number),
            ("", TypedKind::Number),
        ] {
            assert!(session.evaluate_typed(bad, kind).is_err(), "{bad}");
        }
    }
}
