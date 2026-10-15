//! Units, expressions and variables for 3DCraft.
//!
//! Every numeric parameter of a document is an expression stored as source
//! text (`"25 mm"`, `"width / 2 + 3 mm"`, `"height - 2 * wall"`). This crate
//! turns that text into a [`Quantity`]: a number in base units (millimetres,
//! radians) with a [`Dimension`], so that `10 mm + 1 in` is 35.4 mm and
//! `10 mm + 30 deg` is an error (docs/design/plan.md, docs/design/document.md).
//!
//! Three steps, kept apart:
//!
//! 1. [`Expr::parse`]: source text → expression tree. Knows nothing about
//!    variable values.
//! 2. [`Expr::eval`]: tree + variable lookup → [`Quantity`].
//! 3. [`VariableTable`]: named expressions that refer to each other; gives
//!    the evaluation order, finds cycles, and evaluates what can be
//!    evaluated when some variables are in error.
//!
//! Source text and names come from files and from AI agents, so they are
//! treated as hostile: lengths, nesting depth and table size are capped
//! (the `MAX_*` constants), nothing here panics, and a result is never NaN
//! or infinite. Errors carry a byte position and a message that says what
//! to change.
//!
//! ```
//! use craft3d_expr::{Dimension, Expr, Unit, VariableTable};
//!
//! let mut table = VariableTable::new();
//! table.set("width", "40 mm")?;
//! table.set("wall", "0.1 in")?;
//! table.set("inner", "width - 2 * wall")?;
//! let values = table.evaluate();
//!
//! let expr = Expr::parse("inner / 2 + 3 mm")?;
//! assert_eq!(expr.variables(), ["inner"]);
//! let half = expr.eval(|name| values.value(name))?;
//! assert_eq!(half.dimension, Dimension::LENGTH);
//! assert!((half.to_unit(Unit::Centimetre)? - 2.046).abs() < 1e-12);
//!
//! let error = Expr::parse("10 mm + 30 deg")?.eval(|_| None).unwrap_err();
//! assert_eq!(error.pos, 6);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! No dependencies, by design: the crate sits at the bottom of the document
//! model and is compiled into every target, wasm included.

mod error;
mod eval;
mod lexer;
mod parser;
#[cfg(test)]
mod robustness;
mod units;
mod variables;

pub use error::{ErrorKind, ExprError, UnitMismatch, VariableError};
pub use eval::evaluate;
pub use parser::{BinaryOp, Expr, Function, Node, NodeId, NodeKind};
pub use units::{Dimension, Quantity, Unit};
pub use variables::{
    EvaluatedVariable, Evaluation, Variable, VariableTable, check_variable_name, is_reserved_name,
    rename_variable,
};

/// Longest expression source accepted, in bytes. Real expressions are a few
/// dozen bytes; the cap bounds the token list and the tree.
pub const MAX_SOURCE_LEN: usize = 4096;

/// Deepest nesting of parentheses and function calls. This is what bounds
/// the parser's recursion.
pub const MAX_DEPTH: usize = 64;

/// Longest variable name, in bytes.
pub const MAX_NAME_LEN: usize = 128;

/// Most variables a [`VariableTable`] holds.
pub const MAX_VARIABLES: usize = 4096;
