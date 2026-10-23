//! Error types: every failure says where it is and what would fix it.

use crate::parser::Function;
use crate::units::{Dimension, Unit};
use std::fmt;
use std::sync::Arc;

/// A parse or evaluation error, located in the expression's source text.
#[derive(Clone, Debug, PartialEq)]
pub struct ExprError {
    /// Byte offset into the source text of the token the error is about
    /// (for a binary operator: the operator itself).
    pub pos: usize,
    pub kind: ErrorKind,
}

impl ExprError {
    pub(crate) fn new(pos: usize, kind: ErrorKind) -> ExprError {
        ExprError { pos, kind }
    }
}

/// What went wrong. The `Display` text is written for the person or agent
/// who typed the expression: it names what was expected, which name is
/// unknown or which dimensions clashed.
#[derive(Clone, Debug, PartialEq)]
pub enum ErrorKind {
    // --- reading the source text ---
    /// The source text is longer than [`crate::MAX_SOURCE_LEN`] bytes.
    SourceTooLong {
        len: usize,
        max: usize,
    },
    /// A character that is not part of the expression language (this
    /// includes every non-ASCII character).
    UnexpectedChar {
        ch: char,
    },
    /// A number literal too large for an `f64` (with its unit applied).
    NumberOutOfRange,
    /// A lone `=`, `&` or `|`; `meant` is the operator that exists
    /// (`==`, `&&`, `||`).
    LoneOperator {
        found: char,
        meant: &'static str,
    },
    /// The parser needed `expected` and met `found` (a token in backticks,
    /// or "the end of the expression").
    Expected {
        expected: &'static str,
        found: String,
    },
    /// Parentheses and function calls nested deeper than
    /// [`crate::MAX_DEPTH`].
    TooDeep {
        max: usize,
    },
    /// `name(` where `name` is not a function.
    UnknownFunction {
        name: String,
    },
    /// A unit name that does not follow a number, as in `width * mm`.
    UnitWithoutNumber {
        unit: Unit,
    },
    /// A unit after something that is not a number: `(pi / 4) rad`,
    /// `width mm`. A unit belongs to the number just before it.
    UnitAfterExpression {
        unit: Unit,
    },
    /// A word right after a number that is not a unit: `2 inch`, `3 MM`.
    UnknownUnit {
        name: String,
        meant: Unit,
    },
    /// A function called with the wrong number of arguments. `expected` is
    /// prose: "1", "2", "at least 1".
    ArgumentCount {
        function: &'static str,
        expected: &'static str,
        found: usize,
    },
    /// The right-hand side of `^` is not an integer literal without a unit.
    InvalidExponent,
    /// `a ^ 2 ^ 3`: mathematics reads this right to left, most calculators
    /// left to right, so it has to be spelled out with parentheses.
    ChainedPower,
    /// `a < b < c`: a comparison gives 1 or 0, so comparing that again is
    /// almost never what was meant; it has to be spelled out.
    ChainedComparison,

    // --- evaluating ---
    /// A variable the lookup does not know.
    UnknownVariable {
        name: String,
    },
    /// Two operands that must have the same dimension do not.
    /// `operation` is the operator or function name.
    DimensionMismatch {
        operation: &'static str,
        left: Dimension,
        right: Dimension,
    },
    /// A function argument of a dimension the function does not accept.
    /// `expected` is prose: "an angle or a dimensionless number".
    ArgumentDimension {
        function: &'static str,
        expected: &'static str,
        found: Dimension,
    },
    /// `sqrt` of a quantity whose dimension has an odd exponent.
    OddRoot {
        dimension: Dimension,
    },
    /// A dimension exponent left the `i32` range.
    DimensionOverflow,
    DivisionByZero,
    /// An argument outside the function's domain (`sqrt(-1)`, `asin(2)`).
    OutOfDomain {
        function: &'static str,
        value: f64,
    },
    /// The step of a whole-number function (`floor(x, step)`) is zero or
    /// negative: there is no multiple of it to go to.
    InvalidStep {
        function: &'static str,
        value: f64,
    },
    /// The bounds of `clamp(x, lo, hi)` are the wrong way round: `lo` is
    /// larger than `hi` (both in base units).
    InvalidBounds {
        function: &'static str,
        lo: f64,
        hi: f64,
    },
    /// An argument that has to be a whole number is not one (the index of
    /// `choose`), not even within rounding.
    NotWholeNumber {
        function: &'static str,
        value: f64,
    },
    /// The index of `choose(i, a, b, …)` is not one of 1..=`count`.
    ChoiceOutOfRange {
        index: f64,
        count: usize,
    },
    /// The result (or a variable's value) is NaN or infinite.
    /// `operation` is the operator, function or "variable".
    NonFinite {
        operation: &'static str,
    },

    // --- variable tables ---
    /// The variable is part of a dependency cycle. `names` lists every
    /// variable on it, in table order. Shared, because every member of the
    /// cycle carries the same list and a cycle can span the whole table.
    Cycle {
        names: Arc<[String]>,
    },
    /// The variable refers to `name`, which itself has an error.
    DependencyFailed {
        name: String,
    },

    /// An expression tree that the parser cannot produce. Reported instead
    /// of panicking; seeing it is a bug in this crate.
    Internal,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.pos, self.kind)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::SourceTooLong { len, max } => write!(
                f,
                "the expression is {len} bytes long, the maximum is {max}"
            ),
            ErrorKind::UnexpectedChar { ch } => write!(
                f,
                "unexpected character {ch:?}; an expression consists of numbers, units, \
                 names (ASCII letters, digits, `_`), `+ - * / ^`, the comparisons \
                 `< <= > >= == !=`, `&& || !`, parentheses and commas"
            ),
            ErrorKind::NumberOutOfRange => write!(f, "the number is too large"),
            ErrorKind::LoneOperator { found, meant } => {
                write!(f, "`{found}` alone is not an operator; write `{meant}`")
            }
            ErrorKind::Expected { expected, found } => {
                write!(f, "expected {expected}, found {found}")
            }
            ErrorKind::TooDeep { max } => write!(
                f,
                "parentheses and function calls are nested more than {max} levels deep"
            ),
            ErrorKind::UnknownFunction { name } => {
                write!(f, "unknown function `{name}`; the functions are ")?;
                let all = Function::ALL;
                for (i, function) in all.iter().enumerate() {
                    let separator = match all.len().saturating_sub(i) {
                        _ if i == 0 => "",
                        1 => " and ",
                        _ => ", ",
                    };
                    write!(f, "{separator}{}", function.name())?;
                }
                Ok(())
            }
            ErrorKind::UnitWithoutNumber { unit } => write!(
                f,
                "the unit `{unit}` must follow a number, as in `1 {unit}`; it cannot be \
                 used as a name"
            ),
            ErrorKind::UnitAfterExpression { unit } => write!(
                f,
                "the unit `{unit}` must follow a number directly, as in `45 {unit}`; to give \
                 a whole expression a unit, multiply by one: `(pi / 4) * 1 {unit}`"
            ),
            ErrorKind::UnknownUnit { name, meant } => write!(
                f,
                "`{name}` is not a unit: write `{meant}`; the units are um, mm, cm, m, in, \
                 ft, deg and rad, in lower case after a number"
            ),
            ErrorKind::ArgumentCount {
                function,
                expected,
                found,
            } => write!(
                f,
                "`{function}` takes {expected} argument(s), but {found} were given"
            ),
            ErrorKind::InvalidExponent => write!(
                f,
                "the exponent of `^` must be an integer literal without a unit, as in `x ^ 2` \
                 or `x ^ -1`"
            ),
            ErrorKind::ChainedPower => write!(
                f,
                "`a ^ b ^ c` is ambiguous; write `(a ^ b) ^ c` or multiply the exponents"
            ),
            ErrorKind::ChainedComparison => write!(
                f,
                "comparisons cannot be chained: write `a < b && b < c` instead of `a < b < c`"
            ),
            ErrorKind::UnknownVariable { name } => write!(f, "unknown variable `{name}`"),
            ErrorKind::DimensionMismatch {
                operation,
                left,
                right,
            } => write!(
                f,
                "`{operation}` needs operands of the same dimension, but the left one is \
                 {left} and the right one is {right}"
            ),
            ErrorKind::ArgumentDimension {
                function,
                expected,
                found,
            } => write!(
                f,
                "`{function}` needs {expected}, but its argument is {found}"
            ),
            ErrorKind::OddRoot { dimension } => write!(
                f,
                "`sqrt` needs even dimension exponents, but its argument is {dimension}"
            ),
            ErrorKind::DimensionOverflow => {
                write!(f, "the exponent of the result's dimension is too large")
            }
            ErrorKind::DivisionByZero => write!(f, "division by zero"),
            ErrorKind::OutOfDomain { function, value } => {
                write!(f, "`{function}` is not defined for {value}")
            }
            ErrorKind::InvalidStep { function, value } => write!(
                f,
                "the step of `{function}` (its second argument) has to be positive, got {value}"
            ),
            ErrorKind::InvalidBounds { function, .. } => write!(
                f,
                "the lower bound of `{function}` (its second argument) is larger than the \
                 upper bound (its third)"
            ),
            ErrorKind::NotWholeNumber { function, value } => write!(
                f,
                "the first argument of `{function}` has to be a whole number, got {value}"
            ),
            ErrorKind::ChoiceOutOfRange { index, count } => write!(
                f,
                "`choose` has {count} choice(s), numbered from 1, so there is no choice {index}"
            ),
            ErrorKind::NonFinite { operation } => write!(
                f,
                "the result of `{operation}` is not a finite number (overflow)"
            ),
            ErrorKind::Cycle { names } => {
                write!(f, "circular definition: ")?;
                for (i, name) in names.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "`{name}`")?;
                }
                if names.len() == 1 {
                    write!(f, " refers to itself")
                } else {
                    write!(f, " depend on each other")
                }
            }
            ErrorKind::DependencyFailed { name } => {
                write!(f, "depends on `{name}`, which has an error")
            }
            ErrorKind::Internal => write!(
                f,
                "internal error: malformed expression tree (please report this)"
            ),
        }
    }
}

impl std::error::Error for ExprError {}

/// A quantity was asked for in a unit of another dimension
/// (`Quantity::to_unit`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitMismatch {
    pub unit: Unit,
    pub dimension: Dimension,
}

impl fmt::Display for UnitMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot express a quantity of dimension {} in `{}`, a unit of {}",
            self.dimension,
            self.unit,
            self.unit.dimension()
        )
    }
}

impl std::error::Error for UnitMismatch {}

/// A rejected edit of a [`crate::VariableTable`], or a rejected name.
#[derive(Clone, Debug, PartialEq)]
pub enum VariableError {
    EmptyName,
    /// The name is longer than [`crate::MAX_NAME_LEN`] bytes.
    NameTooLong {
        len: usize,
        max: usize,
    },
    NameStartsWithDigit,
    /// The name contains something other than ASCII letters, digits and
    /// `_`. `pos` is the byte offset of `ch` in the name.
    InvalidNameChar {
        pos: usize,
        ch: char,
    },
    /// The name is a unit, a function or `pi`.
    ReservedName {
        name: String,
    },
    /// Rename target that already exists.
    DuplicateName {
        name: String,
    },
    UnknownVariable {
        name: String,
    },
    /// The table already holds [`crate::MAX_VARIABLES`] variables.
    TooManyVariables {
        max: usize,
    },
    /// An expression source longer than [`crate::MAX_SOURCE_LEN`] bytes;
    /// for a rename, the expression of `variable` would grow past it.
    SourceTooLong {
        variable: String,
        len: usize,
        max: usize,
    },
}

impl fmt::Display for VariableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariableError::EmptyName => write!(f, "a variable name cannot be empty"),
            VariableError::NameTooLong { len, max } => write!(
                f,
                "the variable name is {len} bytes long, the maximum is {max}"
            ),
            VariableError::NameStartsWithDigit => {
                write!(f, "a variable name cannot start with a digit")
            }
            VariableError::InvalidNameChar { pos, ch } => write!(
                f,
                "invalid character {ch:?} at byte {pos} of the variable name; use ASCII \
                 letters, digits and `_`"
            ),
            VariableError::ReservedName { name } => write!(
                f,
                "`{name}` is reserved (a unit, a function or the constant pi) and cannot \
                 be a variable name"
            ),
            VariableError::DuplicateName { name } => {
                write!(f, "a variable named `{name}` already exists")
            }
            VariableError::UnknownVariable { name } => {
                write!(f, "there is no variable named `{name}`")
            }
            VariableError::TooManyVariables { max } => {
                write!(f, "a table holds at most {max} variables")
            }
            VariableError::SourceTooLong { variable, len, max } => write!(
                f,
                "the expression of `{variable}` would be {len} bytes long, the maximum is {max}"
            ),
        }
    }
}

impl std::error::Error for VariableError {}
