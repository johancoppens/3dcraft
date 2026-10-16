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
        let doc = self.document().ok().map(|open| open.history.current());
        let evaluation = doc
            .and_then(|doc| doc.variable_table().ok())
            .map(|table| table.evaluate());
        let quantity = craft3d_expr::evaluate(source, |name| {
            evaluation.as_ref().and_then(|e| e.value(name))
        })
        .map_err(|e| CommandError::new(format!("`{source}`: {e}")))?;
        let value = match kind {
            TypedKind::Length if quantity.dimension == Dimension::LENGTH => {
                let unit = doc.map_or(craft3d_expr::Unit::Millimetre, |doc| {
                    doc.units.length.as_expr_unit()
                });
                quantity
                    .to_unit(unit)
                    .map_err(|e| CommandError::new(format!("`{source}`: {e}")))?
            }
            _ if quantity.dimension.is_dimensionless() => quantity.value,
            TypedKind::Length => {
                return Err(CommandError::new(format!(
                    "`{source}` is a {}, but a length is needed",
                    quantity.dimension
                )));
            }
            TypedKind::Number => {
                return Err(CommandError::new(format!(
                    "`{source}` is a {}, but a plain number is needed",
                    quantity.dimension
                )));
            }
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err(CommandError::new(format!(
                "`{source}` is not a finite number"
            )))
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
