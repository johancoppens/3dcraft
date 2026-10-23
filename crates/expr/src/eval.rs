//! Expression tree + variable values → quantity.
//!
//! Evaluation is one pass over the flat node list (operands always precede
//! their node), so it needs no recursion. Every intermediate result is
//! checked: a quantity that leaves this module is finite.
//!
//! **Conditions.** `if`, `choose`, `&&` and `||` take only the result of
//! the operand they choose: an error in an operand they do not choose (a
//! division by zero in the branch of `if` that is not taken) does not
//! reach the result. The single pass still visits every node, so each
//! node holds either a value or the error it would give, and an error
//! travels upwards only through the operands that are used. An operand
//! that is not chosen but has a value is still checked: the branches of
//! `if` and the choices of `choose` must have the same dimension, the
//! operands of `&&` and `||` must be dimensionless.

use crate::error::{ErrorKind, ExprError};
use crate::parser::{BinaryOp, Expr, Function, LookupRow, Node, NodeId, NodeKind};
use crate::tables::{RowRef, Table};
use crate::units::{Dimension, Quantity};

/// What a node evaluated to: a value, or the index of its error in the
/// list of errors met so far (an error is stored once and shared by every
/// node it travels up through).
#[derive(Clone, Copy)]
enum Slot {
    Value(Quantity),
    Failed(usize),
}

/// Why a node has no value: an operand it needs has none (the index of
/// that operand's error), or the node itself failed.
enum Fail {
    Operand(usize),
    Here(ErrorKind),
}

impl From<ErrorKind> for Fail {
    fn from(kind: ErrorKind) -> Fail {
        Fail::Here(kind)
    }
}

impl Expr {
    /// Evaluates the expression. `lookup` returns the value of a variable,
    /// or `None` when there is no such variable. `lookup` is asked about
    /// every variable reference, also those in a branch of `if` that is not
    /// taken.
    pub fn eval(
        &self,
        lookup: impl FnMut(&str) -> Option<Quantity>,
    ) -> Result<Quantity, ExprError> {
        self.eval_with(lookup, &[])
    }

    /// [`Expr::eval`] with tables to look values up in
    /// (`belt[2].pitch`); a lookup in a table not among them is an
    /// [`ErrorKind::UnknownTable`].
    pub fn eval_with(
        &self,
        mut lookup: impl FnMut(&str) -> Option<Quantity>,
        tables: &[Table],
    ) -> Result<Quantity, ExprError> {
        let nodes = self.nodes();
        let mut slots: Vec<Slot> = Vec::with_capacity(nodes.len());
        let mut errors: Vec<ExprError> = Vec::new();
        for node in nodes {
            let slot = match eval_node(node, &slots, &mut lookup, tables) {
                Ok(value) => Slot::Value(value),
                Err(Fail::Operand(error)) => Slot::Failed(error),
                Err(Fail::Here(kind)) => {
                    errors.push(ExprError::new(node.pos, kind));
                    Slot::Failed(errors.len() - 1)
                }
            };
            slots.push(slot);
        }
        match slots.last() {
            Some(Slot::Value(value)) => Ok(*value),
            Some(Slot::Failed(error)) => Err(errors
                .get(*error)
                .cloned()
                .unwrap_or(ExprError::new(0, ErrorKind::Internal))),
            None => Err(ExprError::new(0, ErrorKind::Internal)),
        }
    }
}

/// Parses and evaluates `source` in one go.
pub fn evaluate(
    source: &str,
    lookup: impl FnMut(&str) -> Option<Quantity>,
) -> Result<Quantity, ExprError> {
    Expr::parse(source)?.eval(lookup)
}

/// What an operation is called in an error message.
fn operation_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Number { .. } => "number",
        NodeKind::Pi => "pi",
        NodeKind::Variable(_) => "variable",
        NodeKind::Neg(_) => "-",
        NodeKind::Not(_) => "!",
        NodeKind::Binary { op, .. } => op.symbol(),
        NodeKind::Pow { .. } => "^",
        NodeKind::Call { function, .. } => function.name(),
        NodeKind::Lookup { .. } => "[ ]",
    }
}

/// The value of node `id`, or why it has none. The parser only refers to
/// earlier nodes; `Internal` is the answer to a tree that breaks that
/// rule, instead of an out-of-bounds panic.
fn operand(slots: &[Slot], id: NodeId) -> Result<Quantity, Fail> {
    match slots.get(id) {
        Some(Slot::Value(value)) => Ok(*value),
        Some(Slot::Failed(error)) => Err(Fail::Operand(*error)),
        None => Err(Fail::Here(ErrorKind::Internal)),
    }
}

fn eval_node(
    node: &Node,
    slots: &[Slot],
    lookup: &mut impl FnMut(&str) -> Option<Quantity>,
    tables: &[Table],
) -> Result<Quantity, Fail> {
    let operand = |id: NodeId| operand(slots, id);
    let result = match &node.kind {
        NodeKind::Lookup { table, row, column } => {
            let found = tables.iter().find(|t| t.name == *table).ok_or_else(|| {
                ErrorKind::UnknownTable {
                    name: table.clone(),
                }
            })?;
            let row = match row {
                LookupRow::Number(id) => RowRef::Number(operand(*id)?),
                LookupRow::Key(key) => RowRef::Key(key),
            };
            found.lookup(&row, column)?
        }
        NodeKind::Number { value, unit: None } => Quantity::dimensionless(*value),
        NodeKind::Number {
            value,
            unit: Some(unit),
        } => Quantity::from_unit(*value, *unit),
        NodeKind::Pi => Quantity::dimensionless(std::f64::consts::PI),
        NodeKind::Variable(name) => {
            lookup(name).ok_or_else(|| ErrorKind::UnknownVariable { name: name.clone() })?
        }
        NodeKind::Neg(inner) => {
            let inner = operand(*inner)?;
            Quantity::new(-inner.value, inner.dimension)
        }
        NodeKind::Not(inner) => truth(!condition("!", operand(*inner)?)?),
        NodeKind::Binary {
            op: op @ (BinaryOp::And | BinaryOp::Or),
            left,
            right,
        } => logic(*op, operand(*left)?, operand(*right))?,
        NodeKind::Binary { op, left, right } => binary(*op, operand(*left)?, operand(*right)?)?,
        NodeKind::Pow { base, exponent } => power(operand(*base)?, *exponent)?,
        NodeKind::Call {
            function: Function::If,
            args,
        } => if_then_else(slots, args)?,
        NodeKind::Call {
            function: Function::Choose,
            args,
        } => choose(slots, args)?,
        NodeKind::Call { function, args } => {
            let args = args
                .iter()
                .map(|&id| operand(id))
                .collect::<Result<Vec<Quantity>, Fail>>()?;
            call(*function, &args)?
        }
    };
    if !result.value.is_finite() {
        return Err(Fail::Here(ErrorKind::NonFinite {
            operation: operation_name(&node.kind),
        }));
    }
    Ok(result)
}

/// How much two values may differ and still be equal for `==`, `!=`,
/// `<=` and `>=`, relative to the larger magnitude: 1e-12, about 4500
/// units in the last place. Lengths that are meant to be equal often come
/// from different arithmetic (`3 * 0.1 in` and `0.3 in`, `r * cos(60 deg)`
/// and `r / 2`) and differ by a few units in the last place; no design
/// tells two sizes apart by a part in 10^12. The bound is relative, so it
/// holds at every scale and in every unit, and a value is equal to zero
/// only when it is exactly zero: compare `a == b`, not `a - b == 0 mm`.
pub(crate) const COMPARE_RELATIVE: f64 = 1e-12;

/// `a` and `b` (finite) equal within [`COMPARE_RELATIVE`].
fn nearly_equal(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= COMPARE_RELATIVE * a.abs().max(b.abs())
}

/// 1 for true, 0 for false.
fn truth(value: bool) -> Quantity {
    Quantity::dimensionless(if value { 1.0 } else { 0.0 })
}

/// A condition's truth: dimensionless, and true unless exactly zero.
fn condition(operation: &'static str, value: Quantity) -> Result<bool, ErrorKind> {
    if !value.dimension.is_dimensionless() {
        return Err(ErrorKind::ArgumentDimension {
            function: operation,
            expected: "a condition: a comparison such as `a > b`, or a dimensionless number \
                       (0 is false, anything else true)",
            found: value.dimension,
        });
    }
    Ok(value.value != 0.0)
}

/// `&&` and `||`: the left operand decides when it can (false for `&&`,
/// true for `||`), and then an error of the right one does not count.
fn logic(op: BinaryOp, left: Quantity, right: Result<Quantity, Fail>) -> Result<Quantity, Fail> {
    let name = op.symbol();
    let left = condition(name, left)?;
    let decided = match op {
        BinaryOp::And => !left,
        _ => left,
    };
    if decided {
        // The right operand is not used, but one that has a value must
        // still be a condition.
        if let Ok(right) = right {
            condition(name, right)?;
        }
        return Ok(truth(left));
    }
    Ok(truth(condition(name, right?)?))
}

/// `if(c, a, b)`: `a` when `c` is true, else `b`. Only the chosen branch
/// has to have a value; the other one, when it has one, must have the
/// same dimension.
fn if_then_else(slots: &[Slot], args: &[NodeId]) -> Result<Quantity, Fail> {
    let &[test, then, otherwise] = args else {
        return Err(Fail::Here(ErrorKind::Internal));
    };
    let taken = condition("if", operand(slots, test)?)?;
    let (chosen, other) = if taken {
        (then, otherwise)
    } else {
        (otherwise, then)
    };
    let value = operand(slots, chosen)?;
    if let Ok(other) = operand(slots, other) {
        let (a, b) = if taken {
            (value, other)
        } else {
            (other, value)
        };
        same_dimension("if", a, b)?;
    }
    Ok(value)
}

/// `choose(i, a, b, …)`: the `i`-th choice, counted from 1. `i` is a
/// dimensionless whole number (within rounding, as for `floor`). Only the
/// chosen one has to have a value; the others, when they have one, must
/// have its dimension.
fn choose(slots: &[Slot], args: &[NodeId]) -> Result<Quantity, Fail> {
    let Some((&index, choices)) = args.split_first() else {
        return Err(Fail::Here(ErrorKind::Internal));
    };
    let index = operand(slots, index)?;
    if !index.dimension.is_dimensionless() {
        return Err(Fail::Here(ErrorKind::ArgumentDimension {
            function: "choose",
            expected: "a dimensionless whole number as its first argument",
            found: index.dimension,
        }));
    }
    let whole = index.value.round();
    if snapped(index.value, whole, 1.0) != whole {
        return Err(Fail::Here(ErrorKind::NotWholeNumber {
            function: "choose",
            value: index.value,
        }));
    }
    let count = choices.len();
    // `whole` is finite; the range check comes before the cast.
    if whole < 1.0 || whole > count as f64 {
        return Err(Fail::Here(ErrorKind::ChoiceOutOfRange {
            index: whole,
            count,
        }));
    }
    let chosen = (whole as usize).saturating_sub(1);
    let value = operand(slots, *choices.get(chosen).ok_or(ErrorKind::Internal)?)?;
    for (k, &other) in choices.iter().enumerate() {
        if k == chosen {
            continue;
        }
        if let Ok(other) = operand(slots, other) {
            let (a, b) = if k < chosen {
                (other, value)
            } else {
                (value, other)
            };
            same_dimension("choose", a, b)?;
        }
    }
    Ok(value)
}

fn same_dimension(
    operation: &'static str,
    left: Quantity,
    right: Quantity,
) -> Result<Dimension, ErrorKind> {
    if left.dimension != right.dimension {
        return Err(ErrorKind::DimensionMismatch {
            operation,
            left: left.dimension,
            right: right.dimension,
        });
    }
    Ok(left.dimension)
}

fn binary(op: BinaryOp, left: Quantity, right: Quantity) -> Result<Quantity, ErrorKind> {
    let overflow = ErrorKind::DimensionOverflow;
    Ok(match op {
        BinaryOp::Add => Quantity::new(
            left.value + right.value,
            same_dimension(op.symbol(), left, right)?,
        ),
        BinaryOp::Sub => Quantity::new(
            left.value - right.value,
            same_dimension(op.symbol(), left, right)?,
        ),
        BinaryOp::Mul => Quantity::new(
            left.value * right.value,
            left.dimension
                .checked_mul(right.dimension)
                .ok_or(overflow)?,
        ),
        BinaryOp::Div => {
            if right.value == 0.0 {
                return Err(ErrorKind::DivisionByZero);
            }
            Quantity::new(
                left.value / right.value,
                left.dimension
                    .checked_div(right.dimension)
                    .ok_or(overflow)?,
            )
        }
        BinaryOp::Less
        | BinaryOp::LessEq
        | BinaryOp::Greater
        | BinaryOp::GreaterEq
        | BinaryOp::Equal
        | BinaryOp::NotEqual => {
            same_dimension(op.symbol(), left, right)?;
            let (a, b) = (left.value, right.value);
            let equal = nearly_equal(a, b);
            truth(match op {
                BinaryOp::Less => a < b && !equal,
                BinaryOp::LessEq => a < b || equal,
                BinaryOp::Greater => a > b && !equal,
                BinaryOp::GreaterEq => a > b || equal,
                BinaryOp::Equal => equal,
                _ => !equal,
            })
        }
        // `&&` and `||` are evaluated by `logic`, which may leave the
        // right operand out.
        BinaryOp::And | BinaryOp::Or => return Err(ErrorKind::Internal),
    })
}

fn power(base: Quantity, exponent: i32) -> Result<Quantity, ErrorKind> {
    if base.value == 0.0 && exponent < 0 {
        return Err(ErrorKind::DivisionByZero);
    }
    let dimension = base
        .dimension
        .checked_pow(exponent)
        .ok_or(ErrorKind::DimensionOverflow)?;
    Ok(Quantity::new(base.value.powi(exponent), dimension))
}

/// The single argument of a one-argument function. The parser has checked
/// the count, so a miss is a malformed tree.
fn single(args: &[Quantity]) -> Result<Quantity, ErrorKind> {
    match args {
        [only] => Ok(*only),
        _ => Err(ErrorKind::Internal),
    }
}

fn call(function: Function, args: &[Quantity]) -> Result<Quantity, ErrorKind> {
    let name = function.name();
    match function {
        Function::Sin | Function::Cos | Function::Tan => {
            let x = single(args)?;
            // A plain number is taken as radians, like an angle's base unit.
            if x.dimension != Dimension::ANGLE && !x.dimension.is_dimensionless() {
                return Err(ErrorKind::ArgumentDimension {
                    function: name,
                    expected: "an angle or a dimensionless number (taken as radians)",
                    found: x.dimension,
                });
            }
            let value = match function {
                Function::Sin => x.value.sin(),
                Function::Cos => x.value.cos(),
                _ => x.value.tan(),
            };
            Ok(Quantity::dimensionless(value))
        }
        Function::Asin | Function::Acos | Function::Atan => {
            let x = single(args)?;
            if !x.dimension.is_dimensionless() {
                return Err(ErrorKind::ArgumentDimension {
                    function: name,
                    expected: "a dimensionless number (divide two lengths, for example)",
                    found: x.dimension,
                });
            }
            let value = match function {
                Function::Atan => x.value.atan(),
                _ if !(-1.0..=1.0).contains(&x.value) => {
                    return Err(ErrorKind::OutOfDomain {
                        function: name,
                        value: x.value,
                    });
                }
                Function::Asin => x.value.asin(),
                _ => x.value.acos(),
            };
            Ok(Quantity::new(value, Dimension::ANGLE))
        }
        Function::Atan2 => {
            let [y, x] = args else {
                return Err(ErrorKind::Internal);
            };
            // Any dimension will do as long as it cancels: atan2(dy, dx).
            same_dimension(name, *y, *x)?;
            Ok(Quantity::new(y.value.atan2(x.value), Dimension::ANGLE))
        }
        Function::Sqrt => {
            let x = single(args)?;
            let dimension = x.dimension.checked_sqrt().ok_or(ErrorKind::OddRoot {
                dimension: x.dimension,
            })?;
            if x.value < 0.0 {
                return Err(ErrorKind::OutOfDomain {
                    function: name,
                    value: x.value,
                });
            }
            Ok(Quantity::new(x.value.sqrt(), dimension))
        }
        Function::Abs => {
            let x = single(args)?;
            Ok(Quantity::new(x.value.abs(), x.dimension))
        }
        Function::Min | Function::Max => {
            let (first, rest) = args.split_first().ok_or(ErrorKind::Internal)?;
            let mut best = *first;
            for &next in rest {
                same_dimension(name, best, next)?;
                let better = match function {
                    Function::Min => next.value < best.value,
                    _ => next.value > best.value,
                };
                if better {
                    best = next;
                }
            }
            Ok(best)
        }
        Function::Floor | Function::Ceil | Function::Round | Function::Trunc => {
            whole(function, args)
        }
        Function::Mod => {
            let [a, b] = args else {
                return Err(ErrorKind::Internal);
            };
            modulo(*a, *b)
        }
        Function::Exp
        | Function::Ln
        | Function::Log10
        | Function::Sinh
        | Function::Cosh
        | Function::Tanh => {
            let x = dimensionless(name, single(args)?)?;
            let value = match function {
                Function::Exp => x.exp(),
                Function::Sinh => x.sinh(),
                Function::Cosh => x.cosh(),
                Function::Tanh => x.tanh(),
                _ if x <= 0.0 => {
                    return Err(ErrorKind::OutOfDomain {
                        function: name,
                        value: x,
                    });
                }
                Function::Ln => x.ln(),
                _ => x.log10(),
            };
            Ok(Quantity::dimensionless(value))
        }
        Function::Pow => {
            let [base, exponent] = args else {
                return Err(ErrorKind::Internal);
            };
            let (base, exponent) = (dimensionless(name, *base)?, dimensionless(name, *exponent)?);
            if base == 0.0 && exponent < 0.0 {
                return Err(ErrorKind::DivisionByZero);
            }
            // A negative base has a real power only for a whole exponent.
            if base < 0.0 && exponent.fract() != 0.0 {
                return Err(ErrorKind::OutOfDomain {
                    function: name,
                    value: base,
                });
            }
            Ok(Quantity::dimensionless(base.powf(exponent)))
        }
        Function::Clamp => {
            let [x, lo, hi] = args else {
                return Err(ErrorKind::Internal);
            };
            same_dimension(name, *x, *lo)?;
            let dimension = same_dimension(name, *lo, *hi)?;
            if lo.value > hi.value {
                return Err(ErrorKind::InvalidBounds {
                    function: name,
                    lo: lo.value,
                    hi: hi.value,
                });
            }
            Ok(Quantity::new(x.value.clamp(lo.value, hi.value), dimension))
        }
        // Evaluated by `if_then_else` and `choose`, which may leave
        // arguments out.
        Function::If | Function::Choose => Err(ErrorKind::Internal),
    }
}

/// The value of a dimensionless argument of `function`.
fn dimensionless(function: &'static str, x: Quantity) -> Result<f64, ErrorKind> {
    if !x.dimension.is_dimensionless() {
        let expected = match function {
            "pow" => {
                "dimensionless arguments (for a power of a length, use `^` with a whole \
                 exponent, as in `r ^ 2`)"
            }
            _ => "a dimensionless number (divide by a unit, as in `exp(x / 1 mm)`)",
        };
        return Err(ErrorKind::ArgumentDimension {
            function,
            expected,
            found: x.dimension,
        });
    }
    Ok(x.value)
}

/// How far an argument may lie from a point where a whole-number function
/// jumps and still be taken to lie on it, in units in the last place of
/// that point: four. A ratio of two quantities typed as decimals
/// (`0.3 / 0.1`, `1 in / 0.1 in`) carries the rounding of each literal,
/// of a unit conversion and of the division, half a unit in the last
/// place each; so a whole number meant by the person typing it comes out
/// as 2.9999999999999996, and `floor` of it would be 2. Every argument is
/// treated so, a typed literal as well as a computed one:
/// `floor(2.9999999999999996)` is 3. In units in the last place the bound
/// is relative, except at the jump at zero, where it is four times the
/// machine epsilon, absolute (a value of -1e-16 there is taken as 0).
const JUMP_SNAP_ULPS: f64 = 4.0;

/// The gap from `at` (finite) to the next `f64` larger in magnitude; the
/// machine epsilon at zero.
fn ulp(at: f64) -> f64 {
    let magnitude = at.abs();
    if magnitude == 0.0 {
        return f64::EPSILON;
    }
    f64::from_bits(magnitude.to_bits().saturating_add(1)) - magnitude
}

/// `x`, or the jump point `at` when `x` lies within [`JUMP_SNAP_ULPS`]
/// units in the last place of it. `spacing` is the distance between
/// neighbouring jump points (1 for the whole numbers, 1/2 for the halves
/// `round` jumps at): where four units in the last place reach half of it
/// (from 2^49 for the whole numbers, 2^48 for the halves) a snap could
/// carry `x` past a jump that was meant, so there is none.
fn snapped(x: f64, at: f64, spacing: f64) -> f64 {
    let bound = JUMP_SNAP_ULPS * ulp(at);
    if bound < spacing / 2.0 && (x - at).abs() <= bound {
        at
    } else {
        x
    }
}

/// The whole number a whole-number function gives for the plain number
/// `x`, after `x` has been taken onto a point where the function jumps
/// when it is within rounding of one (the whole numbers, and for `round`
/// the halves between them). Ties of `round` go to the even whole number
/// (2.5 to 2, 3.5 to 4, -2.5 to -2): the developer's decision of
/// 2026-10-16 (docs/decisions.md), so that a row of ties does not drift
/// one way.
fn whole_of(function: Function, x: f64) -> f64 {
    // From 2^52 on every `f64` is a whole number, and doubling it could
    // overflow.
    if x.abs() >= 4_503_599_627_370_496.0 {
        return x;
    }
    match function {
        Function::Round => {
            // `round` jumps at the halves: 2.4999999999999996 is 2.5.
            let half = snapped(x, (x * 2.0).round() / 2.0, 0.5);
            // `+ 0.0` turns a negative zero (round(-0.4)) into zero.
            half.round_ties_even() + 0.0
        }
        _ => {
            let x = snapped(x, x.round(), 1.0);
            let whole = match function {
                Function::Floor => x.floor(),
                Function::Ceil => x.ceil(),
                _ => x.trunc(),
            };
            whole + 0.0
        }
    }
}

/// `floor`, `ceil`, `round` and `trunc`: of a plain number, or to a
/// multiple of a step of the same dimension as the argument. A length
/// alone is refused, since its whole number would depend on the unit the
/// value is held in (millimetres) and not on the one the document shows.
fn whole(function: Function, args: &[Quantity]) -> Result<Quantity, ErrorKind> {
    let name = function.name();
    match args {
        [x] => {
            if !x.dimension.is_dimensionless() {
                return Err(ErrorKind::ArgumentDimension {
                    function: name,
                    expected: "a dimensionless number (divide by a unit, as in \
                               `floor(length / 1 mm)`, or give a step, as in \
                               `floor(length, 0.5 mm)`)",
                    found: x.dimension,
                });
            }
            Ok(Quantity::dimensionless(whole_of(function, x.value)))
        }
        [x, step] => {
            let dimension = same_dimension(name, *x, *step)?;
            if step.value <= 0.0 {
                return Err(ErrorKind::InvalidStep {
                    function: name,
                    value: step.value,
                });
            }
            let ratio = x.value / step.value;
            if !ratio.is_finite() {
                return Err(ErrorKind::NonFinite { operation: name });
            }
            Ok(Quantity::new(
                whole_of(function, ratio) * step.value + 0.0,
                dimension,
            ))
        }
        _ => Err(ErrorKind::Internal),
    }
}

/// `mod(a, b)`: what is left of `a` after taking out a whole number of
/// `b`, `a - b * floor(a / b)` with the `floor` of [`whole_of`]. Any
/// dimension, the same for both; the result has the sign of `b` (lies in
/// `[0, b)` for a positive `b`, in `(b, 0]` for a negative one). A zero
/// `b` is a division by zero. From |a / b| = 2^52 on the quotient is a
/// whole number in `f64`, so the result is 0: `a` is a whole number of
/// `b` as far as the arithmetic can tell.
fn modulo(a: Quantity, b: Quantity) -> Result<Quantity, ErrorKind> {
    let dimension = same_dimension("mod", a, b)?;
    if b.value == 0.0 {
        return Err(ErrorKind::DivisionByZero);
    }
    let ratio = a.value / b.value;
    if !ratio.is_finite() {
        return Err(ErrorKind::NonFinite { operation: "mod" });
    }
    if snapped(ratio, ratio.round(), 1.0) != ratio || ratio == ratio.round() {
        // A whole number of `b`, or within rounding of one.
        return Ok(Quantity::new(0.0, dimension));
    }
    // `%` is exact for floating-point numbers; it has the sign of `a`.
    let mut rest = a.value % b.value;
    if rest != 0.0 && (rest < 0.0) != (b.value < 0.0) {
        rest += b.value;
    }
    // The shift can round onto `b` itself when the exact rest lies within
    // half a unit in the last place of it: that is a whole number of `b`.
    if rest == b.value {
        rest = 0.0;
    }
    Ok(Quantity::new(rest + 0.0, dimension))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Unit;
    use std::f64::consts::PI;

    const LENGTH: Dimension = Dimension::LENGTH;
    const ANGLE: Dimension = Dimension::ANGLE;
    const NONE: Dimension = Dimension::NONE;
    const AREA: Dimension = Dimension::new(2, 0);

    /// Bound for the handful of floating-point operations an expression in
    /// these tests performs, relative to the larger of the two values.
    const EPS: f64 = 1e-14;

    fn vars(name: &str) -> Option<Quantity> {
        match name {
            "width" => Some(Quantity::new(40.0, LENGTH)),
            "height" => Some(Quantity::new(30.0, LENGTH)),
            "wall" => Some(Quantity::new(2.0, LENGTH)),
            "r" => Some(Quantity::new(5.0, LENGTH)),
            "a" => Some(Quantity::new(3.0, LENGTH)),
            "b" => Some(Quantity::new(4.0, LENGTH)),
            "n" => Some(Quantity::dimensionless(6.0)),
            "tilt" => Some(Quantity::new(PI / 6.0, ANGLE)),
            "zero" => Some(Quantity::dimensionless(0.0)),
            "nan" => Some(Quantity::dimensionless(f64::NAN)),
            "inf" => Some(Quantity::new(f64::INFINITY, LENGTH)),
            _ => None,
        }
    }

    fn eval(source: &str) -> Quantity {
        match evaluate(source, vars) {
            Ok(q) => q,
            Err(e) => panic!("`{source}`: {e}"),
        }
    }

    fn err(source: &str) -> ExprError {
        match evaluate(source, vars) {
            Ok(q) => panic!("`{source}` evaluated to {q:?}"),
            Err(e) => e,
        }
    }

    #[track_caller]
    fn check(source: &str, value: f64, dimension: Dimension) {
        let q = eval(source);
        assert_eq!(q.dimension, dimension, "`{source}`");
        let tolerance = EPS * value.abs().max(1.0);
        assert!(
            (q.value - value).abs() <= tolerance,
            "`{source}` = {}, expected {value}",
            q.value
        );
    }

    #[test]
    fn examples_from_the_architecture_doc() {
        check("25 mm", 25.0, LENGTH);
        check("width / 2 + 3 mm", 23.0, LENGTH);
        check("height - 2 * wall", 26.0, LENGTH);
        check("2 * pi * r", 10.0 * PI, LENGTH);
        check("atan2(a, b)", (3.0f64).atan2(4.0), ANGLE);
        check("10 mm + 1 in", 35.4, LENGTH);
    }

    #[test]
    fn literals_and_units() {
        check("25", 25.0, NONE);
        check("25mm", 25.0, LENGTH);
        check("1.5e-3 m", 1.5, LENGTH);
        check("1.5e-3m", 1.5, LENGTH);
        check("250 um", 0.25, LENGTH);
        check("2.5 cm", 25.0, LENGTH);
        check("0.5 m", 500.0, LENGTH);
        check("2 in", 50.8, LENGTH);
        check("1 ft", 304.8, LENGTH);
        check("90 deg", PI / 2.0, ANGLE);
        check("0.5 rad", 0.5, ANGLE);
        check(".5", 0.5, NONE);
        check("5.", 5.0, NONE);
        check("pi", PI, NONE);
        check("  \t25\nmm\r\n", 25.0, LENGTH);
    }

    #[test]
    fn operators() {
        check("2 + 3", 5.0, NONE);
        check("2 - 3", -1.0, NONE);
        check("2 * 3", 6.0, NONE);
        check("3 / 2", 1.5, NONE);
        check("3 ^ 2", 9.0, NONE);
        check("2 ^ -2", 0.25, NONE);
        check("2 ^ 0", 1.0, NONE);
        check("-3", -3.0, NONE);
        check("(2 + 3) * 4", 20.0, NONE);
    }

    #[test]
    fn precedence_and_associativity() {
        check("2 - 3 - 4", -5.0, NONE);
        check("2 - (3 - 4)", 3.0, NONE);
        check("1 / 2 / 4", 0.125, NONE);
        check("1 / (2 / 4)", 2.0, NONE);
        check("2 * 3 ^ 2", 18.0, NONE);
        check("-2 ^ 2", -4.0, NONE);
        check("(-2) ^ 2", 4.0, NONE);
        check("2 + 3 * 4", 14.0, NONE);
        check("2 * 3 + 4", 10.0, NONE);
        check("8 / 4 * 2", 4.0, NONE);
        check("8 - 4 + 2", 6.0, NONE);
        check("2 * -3", -6.0, NONE);
        check("2 - -3", 5.0, NONE);
        check("--3", 3.0, NONE);
        check("---3", -3.0, NONE);
        check("-3 * -2", 6.0, NONE);
        check("2 ^ 2 * 3", 12.0, NONE);
        check("-2 ^ 2 + 1", -3.0, NONE);
        check("2 / -4", -0.5, NONE);
        // A unit binds to its number before any operator.
        check("1 / 2 mm", 0.5, Dimension::new(-1, 0));
        check("2 mm ^ 2", 4.0, AREA);
        check("-5 mm", -5.0, LENGTH);
    }

    #[test]
    fn dimensions_follow_the_arithmetic() {
        check("2 * 3 mm", 6.0, LENGTH);
        check("3 mm * 2", 6.0, LENGTH);
        check("10 mm * 2 mm", 20.0, AREA);
        check("1 cm * 1 cm", 100.0, AREA);
        check("10 mm / 4", 2.5, LENGTH);
        check("10 mm / 4 mm", 2.5, NONE);
        check("1 / 4 mm", 0.25, Dimension::new(-1, 0));
        check("(2 mm) ^ 3", 8.0, Dimension::new(3, 0));
        check("(2 mm) ^ -1", 0.5, Dimension::new(-1, 0));
        check("(2 mm) ^ 0", 1.0, NONE);
        check("1 in - 5.4 mm", 20.0, LENGTH);
        check("90 deg + 1 rad", PI / 2.0 + 1.0, ANGLE);
        check("90 deg / 1 rad", PI / 2.0, NONE);
        check("r * tilt", 5.0 * PI / 6.0, Dimension::new(1, 1));
        check("n * wall", 12.0, LENGTH);
    }

    #[test]
    fn trigonometry() {
        check("sin(30 deg)", 0.5, NONE);
        check("cos(60 deg)", 0.5, NONE);
        check("tan(45 deg)", 1.0, NONE);
        check("sin(pi / 2)", 1.0, NONE);
        check("cos(0)", 1.0, NONE);
        check("sin(tilt)", 0.5, NONE);
        check("sin(0.5 rad)", (0.5f64).sin(), NONE);
        check("r * cos(tilt)", 5.0 * (PI / 6.0).cos(), LENGTH);
        check("asin(0.5)", PI / 6.0, ANGLE);
        check("asin(-1)", -PI / 2.0, ANGLE);
        check("acos(0.5)", PI / 3.0, ANGLE);
        check("acos(1)", 0.0, ANGLE);
        check("atan(1)", PI / 4.0, ANGLE);
        check("atan(a / b)", (0.75f64).atan(), ANGLE);
        check("atan2(1, 1)", PI / 4.0, ANGLE);
        check("atan2(1, -1)", 3.0 * PI / 4.0, ANGLE);
        check("atan2(-1 mm, 0 mm)", -PI / 2.0, ANGLE);
        check("atan2(0, 0)", 0.0, ANGLE);
        check("sin(asin(0.25))", 0.25, NONE);
    }

    #[test]
    fn sqrt_abs_min_max() {
        check("sqrt(16)", 4.0, NONE);
        check("sqrt(0)", 0.0, NONE);
        check("sqrt(a ^ 2 + b ^ 2)", 5.0, LENGTH);
        check("sqrt(4 mm * 9 mm)", 6.0, LENGTH);
        check("sqrt(1 / (2 mm) ^ 2)", 0.5, Dimension::new(-1, 0));
        check("sqrt((2 mm) ^ 4)", 4.0, AREA);
        check("abs(-3 mm)", 3.0, LENGTH);
        check("abs(3)", 3.0, NONE);
        check("abs(-90 deg)", PI / 2.0, ANGLE);
        check("min(3, 2)", 2.0, NONE);
        check("max(3, 2)", 3.0, NONE);
        check("min(1 in, 25 mm)", 25.0, LENGTH);
        check("max(1 in, 25 mm)", 25.4, LENGTH);
        check("min(5)", 5.0, NONE);
        check("max(1, 5, 3, -2)", 5.0, NONE);
        check("min(1, 5, 3, -2)", -2.0, NONE);
        check("max(a, b) - min(a, b)", 1.0, LENGTH);
    }

    #[test]
    fn whole_number_functions() {
        check("floor(2.7)", 2.0, NONE);
        check("floor(-2.2)", -3.0, NONE);
        check("ceil(2.2)", 3.0, NONE);
        check("ceil(-2.7)", -2.0, NONE);
        check("trunc(2.7)", 2.0, NONE);
        check("trunc(-2.7)", -2.0, NONE);
        check("round(2.4)", 2.0, NONE);
        check("round(2.6)", 3.0, NONE);
        check("round(-2.6)", -3.0, NONE);
        check("floor(3)", 3.0, NONE);
        check("ceil(-3)", -3.0, NONE);
        check("floor(0)", 0.0, NONE);
        // Lengths through a ratio: the unit cancels, so the document's
        // unit does not matter.
        check("floor(width / 12 mm)", 3.0, NONE);
        check("ceil(width / 12 mm)", 4.0, NONE);
        check("floor(1 in / 1 mm)", 25.0, NONE);
        check("round(n / 4)", 2.0, NONE);
        check("floor(90 deg / 1 deg)", 90.0, NONE);
        // Huge values are whole numbers already.
        check("floor(1e300)", 1e300, NONE);
        check("round(-1e300)", -1e300, NONE);
        check("trunc(4503599627370497)", 4_503_599_627_370_497.0, NONE);
    }

    #[test]
    fn round_ties_go_to_even() {
        check("round(0.5)", 0.0, NONE);
        check("round(1.5)", 2.0, NONE);
        check("round(2.5)", 2.0, NONE);
        check("round(3.5)", 4.0, NONE);
        check("round(-0.5)", 0.0, NONE);
        check("round(-2.5)", -2.0, NONE);
        check("round(-3.5)", -4.0, NONE);
        check("round(2.5 mm, 1 mm)", 2.0, LENGTH);
        check("round(7.5 mm, 5 mm)", 10.0, LENGTH);
        // The finding of the laser box (L1): the old workaround gave 1 for
        // 1.5 and 2 for 2.5, ties going neither one way nor to even. A tie
        // is a tie whichever side of it the arithmetic landed:
        // 0.35 / 0.1 is 3.4999999999999996 in `f64`, 0.15 / 0.1 is
        // 1.4999999999999998.
        check("round(0.35 / 0.1)", 4.0, NONE);
        check("round(-0.35 / 0.1)", -4.0, NONE);
        check("round(0.15 / 0.1)", 2.0, NONE);
        // Not a tie: far from the half in units in the last place.
        check("round(2.4999999)", 2.0, NONE);
        check("round(2.5000001)", 3.0, NONE);
    }

    #[test]
    fn rounding_error_at_a_jump_is_not_a_whole_number_off() {
        // 0.3 / 0.1 is 2.9999999999999996 in `f64`; the person meant 3.
        assert!(eval("0.3 / 0.1").value < 3.0);
        assert!(eval("0.35 / 0.1").value < 3.5);
        check("floor(0.3 / 0.1)", 3.0, NONE);
        check("trunc(0.3 / 0.1)", 3.0, NONE);
        check("floor(0.7 / 0.1)", 7.0, NONE);
        check("floor(-0.3 / 0.1)", -3.0, NONE);
        check("floor(0.3 mm, 0.1 mm)", 0.30000000000000004, LENGTH);
        check("mod(0.3, 0.1)", 0.0, NONE);
        check("mod(1 in, 0.1 in)", 0.0, LENGTH);
        // At every scale: the bound is relative.
        check("floor(3e-9 / 1e-9)", 3.0, NONE);
        check("floor(0.3e12 / 0.1e12)", 3.0, NONE);
        // Farther away than rounding: taken as it is.
        check("floor(2.9999999999)", 2.0, NONE);
        check("ceil(3.0000000001)", 4.0, NONE);
    }

    /// Where four units in the last place reach half the gap between two
    /// jumps (from 2^49 for the whole numbers, 2^48 for the halves), there
    /// is no snap: it could cross a jump that was meant.
    #[test]
    fn no_snap_across_a_jump_near_two_to_the_fifty_two() {
        check("floor(1e15 + 0.5)", 1e15, NONE);
        check("ceil(1e15 + 0.5)", 1e15 + 1.0, NONE);
        check("mod(1e15 + 0.5, 1)", 0.5, NONE);
        // 2^49 + 0.25, 2^50 + 0.25, 2^52 - 0.5: each exact in `f64`.
        check("floor(562949953421312 + 0.25)", 562_949_953_421_312.0, NONE);
        check(
            "ceil(1125899906842624 + 0.25)",
            1_125_899_906_842_625.0,
            NONE,
        );
        check(
            "floor(4503599627370496 - 0.5)",
            4_503_599_627_370_495.0,
            NONE,
        );
        check(
            "trunc(4503599627370496 - 0.5)",
            4_503_599_627_370_495.0,
            NONE,
        );
        // round jumps at the halves: 2^48 + 0.25 is a quarter from both.
        check("round(281474976710656 + 0.25)", 281_474_976_710_656.0, NONE);
        check("round(281474976710656 + 0.5)", 281_474_976_710_656.0, NONE);
        check("round(281474976710656 + 0.75)", 281_474_976_710_657.0, NONE);
        // Below that the snap still works: 2^40 less one unit in the last
        // place is 2^40.
        check(
            "floor(1099511627776 - 0.0001220703125)",
            1_099_511_627_776.0,
            NONE,
        );
        // From 2^52 on a quotient is a whole number: mod gives 0.
        check("mod(4503599627370496, 1)", 0.0, NONE);
        check("mod(1e300, 3)", 0.0, NONE);
    }

    #[test]
    fn no_negative_zero() {
        for source in [
            "round(-0.4)",
            "trunc(-0.5)",
            "ceil(-0.5)",
            "round(-0)",
            "floor(-1e-17)",
            "round(-0.4 mm, 1 mm)",
            "mod(-0, 3)",
            "mod(-6, 3)",
        ] {
            let q = eval(source);
            assert!(
                q.value == 0.0 && q.value.is_sign_positive(),
                "`{source}`: {q:?}"
            );
        }
    }

    #[test]
    fn rounding_to_a_step() {
        check("floor(width, 12 mm)", 36.0, LENGTH);
        check("ceil(width, 12 mm)", 48.0, LENGTH);
        check("round(width, 12 mm)", 36.0, LENGTH);
        check("round(43 mm, 1 cm)", 40.0, LENGTH);
        check("round(1 in, 1 mm)", 25.0, LENGTH);
        check("trunc(-7 mm, 2 mm)", -6.0, LENGTH);
        check("floor(-7 mm, 2 mm)", -8.0, LENGTH);
        check("round(100 deg, 15 deg)", 105.0 * PI / 180.0, ANGLE);
        check("floor(7.5, 2)", 6.0, NONE);
        check("round(width * wall, 1 mm * 1 mm)", 80.0, AREA);
    }

    #[test]
    fn modulo() {
        check("mod(7, 3)", 1.0, NONE);
        check("mod(-7, 3)", 2.0, NONE);
        check("mod(7, -3)", -2.0, NONE);
        check("mod(-7, -3)", -1.0, NONE);
        check("mod(6, 3)", 0.0, NONE);
        check("mod(width, 12 mm)", 4.0, LENGTH);
        check("mod(1 in, 1 cm)", 5.4, LENGTH);
        check("mod(-1 mm, 3 mm)", 2.0, LENGTH);
        check("mod(370 deg, 360 deg)", 10.0 * PI / 180.0, ANGLE);
        check("mod(2.5, 1)", 0.5, NONE);
        // The identity it is defined by, and where the rest lies.
        for (a, b) in [
            (7.25, 2.0),
            (-7.25, 2.0),
            (7.25, -2.0),
            (1e6, 7.0),
            (-1e-3, 3e-4),
        ] {
            let r = eval(&format!("mod({a}, {b})")).value;
            let q = eval(&format!("floor({a} / {b})")).value;
            assert!((a - b * q - r).abs() <= 1e-12 * a.abs().max(1.0), "{a} {b}");
            assert!(r == 0.0 || (r > 0.0) == (b > 0.0), "{a} {b}: {r}");
            assert!(r.abs() < b.abs(), "{a} {b}: {r}");
        }
        // A rest within rounding of the divisor is no rest.
        check("mod(-1e-20, 1)", 0.0, NONE);
    }

    #[test]
    fn whole_number_functions_refuse_what_has_no_answer() {
        let wrong = |source: &str| match err(source) {
            ExprError {
                pos,
                kind:
                    ErrorKind::ArgumentDimension {
                        function, found, ..
                    },
            } => (pos, function, found),
            other => panic!("`{source}`: {other:?}"),
        };
        // A length alone has a whole number only in a unit.
        assert_eq!(wrong("floor(width)"), (0, "floor", LENGTH));
        assert_eq!(wrong("1 + ceil(2.5 mm)"), (4, "ceil", LENGTH));
        assert_eq!(wrong("round(tilt)"), (0, "round", ANGLE));
        assert_eq!(wrong("trunc(width * wall)"), (0, "trunc", AREA));
        assert!(
            err("floor(width)")
                .to_string()
                .contains("floor(length / 1 mm)")
        );
        assert_eq!(err("floor(width, 2)"), mismatch(0, "floor", LENGTH, NONE));
        assert_eq!(err("round(2, 1 mm)"), mismatch(0, "round", NONE, LENGTH));
        assert_eq!(err("mod(width, 2)"), mismatch(0, "mod", LENGTH, NONE));
        assert_eq!(err("mod(1 mm, 1 deg)"), mismatch(0, "mod", LENGTH, ANGLE));
        let step = |source: &str| match err(source) {
            ExprError {
                kind: ErrorKind::InvalidStep { function, value },
                ..
            } => (function, value),
            other => panic!("`{source}`: {other:?}"),
        };
        assert_eq!(step("floor(width, 0 mm)"), ("floor", 0.0));
        assert_eq!(step("round(3, -1)"), ("round", -1.0));
        assert_eq!(step("ceil(3, zero)"), ("ceil", 0.0));
        assert!(
            err("trunc(3, -0.5)")
                .to_string()
                .contains("has to be positive")
        );
        assert_eq!(
            err("mod(3, 0)"),
            ExprError::new(0, ErrorKind::DivisionByZero)
        );
        assert_eq!(
            err("mod(width, 0 mm)"),
            ExprError::new(0, ErrorKind::DivisionByZero)
        );
        // Overflow of the ratio or of the result is an error, never
        // infinity; so is a value that is not a number.
        for source in [
            "floor(1e300, 1e-300)",
            "mod(1e300, 1e-300)",
            "floor(nan)",
            "mod(1, nan)",
        ] {
            assert!(
                matches!(err(source).kind, ErrorKind::NonFinite { .. }),
                "`{source}`"
            );
        }
    }

    /// The laser box (target/work/laserbox, finding L1): the odd finger
    /// count nearest to span / fingerW, at least 3, written plainly; it
    /// was `q - atan(tan(q * 180 deg)) / (180 deg)` with a bias against
    /// ties before there was a `round`.
    #[test]
    fn odd_finger_count_of_the_laser_box() {
        let count = |span: f64, finger: f64| {
            let source = format!("max(3, 2 * round(({span} mm / {finger} mm - 1) / 2) + 1)");
            eval(&source).value
        };
        // spanW = 160 - 2 * 3 = 154 at fingerW 12: 12.83, nearest odd 13.
        assert_eq!(count(154.0, 12.0), 13.0);
        assert_eq!(count(104.0, 12.0), 9.0);
        assert_eq!(count(77.0, 12.0), 7.0);
        assert_eq!(count(10.0, 12.0), 3.0);
        // A tie between two odd counts (an even ratio) goes to the even
        // half, (14 - 1) / 2 = 6.5 to 6, so to 13, whichever way the
        // division rounded: 168 / 12 is 14, 0.14 / 0.01 is
        // 14.000000000000002 in `f64`.
        assert_eq!(count(168.0, 12.0), 13.0);
        assert_eq!(count(0.14, 0.01), 13.0);
        assert_eq!(count(192.0, 12.0), 17.0);
        // With floor: the odd count at or below.
        let below = |q: &str| eval(&format!("2 * floor((({q}) - 1) / 2) + 1")).value;
        assert_eq!(below("12.83"), 11.0);
        assert_eq!(below("13"), 13.0);
        assert_eq!(below("1.3 / 0.1"), 13.0);
        // Odd or even, by mod.
        check("mod(13, 2)", 1.0, NONE);
        check("mod(2 * round(6.4) + 1, 2)", 1.0, NONE);
    }

    fn mismatch(
        pos: usize,
        operation: &'static str,
        left: Dimension,
        right: Dimension,
    ) -> ExprError {
        ExprError::new(
            pos,
            ErrorKind::DimensionMismatch {
                operation,
                left,
                right,
            },
        )
    }

    #[test]
    fn adding_different_dimensions_is_an_error() {
        assert_eq!(err("10 mm + 30 deg"), mismatch(6, "+", LENGTH, ANGLE));
        assert_eq!(err("10 mm - 30 deg"), mismatch(6, "-", LENGTH, ANGLE));
        assert_eq!(err("10 mm + 1"), mismatch(6, "+", LENGTH, NONE));
        assert_eq!(err("1 + 90 deg"), mismatch(2, "+", NONE, ANGLE));
        assert_eq!(err("width + width * wall"), mismatch(6, "+", LENGTH, AREA));
        assert_eq!(err("2 * (width - tilt)"), mismatch(11, "-", LENGTH, ANGLE));
        assert_eq!(err("min(1 mm, 2)"), mismatch(0, "min", LENGTH, NONE));
        assert_eq!(err("1 + max(1, 2, 3 deg)"), mismatch(4, "max", NONE, ANGLE));
        assert_eq!(err("atan2(1 mm, 2)"), mismatch(0, "atan2", LENGTH, NONE));
        assert_eq!(
            err("10 mm + 30 deg").to_string(),
            "at byte 6: `+` needs operands of the same dimension, but the left one is \
             length and the right one is angle"
        );
    }

    #[test]
    fn function_argument_dimensions() {
        let wrong = |source: &str| match err(source) {
            ExprError {
                pos,
                kind:
                    ErrorKind::ArgumentDimension {
                        function, found, ..
                    },
            } => (pos, function, found),
            other => panic!("`{source}`: {other:?}"),
        };
        assert_eq!(wrong("sin(1 mm)"), (0, "sin", LENGTH));
        assert_eq!(wrong("1 + cos(width)"), (4, "cos", LENGTH));
        assert_eq!(wrong("tan(tilt * tilt)"), (0, "tan", Dimension::new(0, 2)));
        assert_eq!(wrong("asin(1 mm)"), (0, "asin", LENGTH));
        assert_eq!(wrong("acos(tilt)"), (0, "acos", ANGLE));
        assert_eq!(wrong("atan(1 mm)"), (0, "atan", LENGTH));
        assert!(
            err("sin(1 mm)")
                .to_string()
                .contains("an angle or a dimensionless")
        );
    }

    #[test]
    fn sqrt_of_an_odd_dimension() {
        assert_eq!(
            err("sqrt(4 mm)"),
            ExprError::new(0, ErrorKind::OddRoot { dimension: LENGTH })
        );
        assert_eq!(
            err("2 * sqrt(width ^ 3)"),
            ExprError::new(
                4,
                ErrorKind::OddRoot {
                    dimension: Dimension::new(3, 0)
                }
            )
        );
        assert!(matches!(
            err("sqrt(90 deg)").kind,
            ErrorKind::OddRoot { .. }
        ));
    }

    #[test]
    fn unknown_variable() {
        assert_eq!(
            err("width + depth"),
            ExprError::new(
                8,
                ErrorKind::UnknownVariable {
                    name: "depth".into()
                }
            )
        );
        assert_eq!(
            err("depth").to_string(),
            "at byte 0: unknown variable `depth`"
        );
        // Names are case-sensitive.
        assert!(matches!(
            err("Width").kind,
            ErrorKind::UnknownVariable { .. }
        ));
    }

    #[test]
    fn division_by_zero() {
        let zero = |pos| ExprError::new(pos, ErrorKind::DivisionByZero);
        assert_eq!(err("1 / 0"), zero(2));
        assert_eq!(err("width / zero"), zero(6));
        assert_eq!(err("1 mm / (2 - 2)"), zero(5));
        assert_eq!(err("0 / 0"), zero(2));
        assert_eq!(err("1 / -0"), zero(2));
        assert_eq!(err("0 ^ -1"), zero(2));
        assert_eq!(err("zero ^ -2"), zero(5));
        check("0 / 5", 0.0, NONE);
        check("0 ^ 0", 1.0, NONE);
    }

    #[test]
    fn out_of_domain() {
        let domain = |source: &str| match err(source) {
            ExprError {
                pos,
                kind: ErrorKind::OutOfDomain { function, value },
            } => (pos, function, value),
            other => panic!("`{source}`: {other:?}"),
        };
        assert_eq!(domain("sqrt(-1)"), (0, "sqrt", -1.0));
        assert_eq!(domain("1 + sqrt(-4 mm * 1 mm)"), (4, "sqrt", -4.0));
        assert_eq!(domain("asin(2)"), (0, "asin", 2.0));
        assert_eq!(domain("asin(-1.5)"), (0, "asin", -1.5));
        assert_eq!(domain("acos(1.0000001)"), (0, "acos", 1.0000001));
        assert_eq!(
            err("sqrt(-1)").to_string(),
            "at byte 0: `sqrt` is not defined for -1"
        );
    }

    #[test]
    fn overflow_is_an_error_not_infinity() {
        let non_finite = |pos, operation| ExprError::new(pos, ErrorKind::NonFinite { operation });
        assert_eq!(err("1e308 * 10"), non_finite(6, "*"));
        assert_eq!(err("1e308 + 1e308"), non_finite(6, "+"));
        assert_eq!(err("-1e308 - 1e308"), non_finite(7, "-"));
        assert_eq!(err("1e308 / 1e-10"), non_finite(6, "/"));
        assert_eq!(err("10 ^ 400"), non_finite(3, "^"));
        assert_eq!(err("1e200 mm * 1e200 mm"), non_finite(9, "*"));
    }

    #[test]
    fn non_finite_variable_values_are_rejected() {
        let non_finite = |pos| {
            ExprError::new(
                pos,
                ErrorKind::NonFinite {
                    operation: "variable",
                },
            )
        };
        assert_eq!(err("1 + nan"), non_finite(4));
        assert_eq!(err("inf"), non_finite(0));
        // Not laundered through an operation that would hide it.
        assert_eq!(err("min(1, nan)"), non_finite(7));
        assert_eq!(err("atan(inf / 1 mm)"), non_finite(5));
    }

    #[test]
    fn dimension_exponent_overflow() {
        assert_eq!(
            err("((1 mm) ^ 2147483647) ^ 2"),
            ExprError::new(22, ErrorKind::DimensionOverflow)
        );
        assert_eq!(
            err("(1 mm) ^ 2147483647 * 1 mm"),
            ExprError::new(20, ErrorKind::DimensionOverflow)
        );
        assert_eq!(
            err("(1 mm) ^ -2147483647 / 1 mm / 1 mm"),
            ExprError::new(28, ErrorKind::DimensionOverflow)
        );
    }

    #[test]
    fn parse_errors_come_through_evaluate() {
        assert!(matches!(err("2 +").kind, ErrorKind::Expected { .. }));
    }

    #[test]
    fn to_unit_on_results() {
        let q = eval("10 mm + 1 in");
        assert!((q.to_unit(Unit::Inch).unwrap() - 35.4 / 25.4).abs() < EPS);
        assert!((eval("asin(1)").to_unit(Unit::Degree).unwrap() - 90.0).abs() < 1e-12);
        assert!(eval("10 mm * 2 mm").to_unit(Unit::Millimetre).is_err());
    }

    #[test]
    fn lookup_may_borrow_mutably() {
        let mut asked = Vec::new();
        let result = evaluate("x + y + x", |name| {
            asked.push(name.to_string());
            Some(Quantity::dimensionless(1.0))
        });
        assert_eq!(result.unwrap().value, 3.0);
        assert_eq!(asked, ["x", "y", "x"]);
    }

    // --- conditions ---

    #[test]
    fn comparisons_give_one_or_zero() {
        check("1 < 2", 1.0, NONE);
        check("2 < 1", 0.0, NONE);
        check("2 <= 2", 1.0, NONE);
        check("3 > 2", 1.0, NONE);
        check("2 >= 3", 0.0, NONE);
        check("2 == 2", 1.0, NONE);
        check("2 != 2", 0.0, NONE);
        check("width > height", 1.0, NONE);
        check("width == 40 mm", 1.0, NONE);
        check("1 in == 25.4 mm", 1.0, NONE);
        check("90 deg == pi / 2 * 1 rad", 1.0, NONE);
        check("width * wall >= 80 mm * 1 mm", 1.0, NONE);
        // A comparison is a number: it can be added and multiplied.
        check("(width > 30 mm) + (height > 30 mm)", 1.0, NONE);
        check("2 + (1 < 2) * 3", 5.0, NONE);
        check("-1 < 0", 1.0, NONE);
    }

    #[test]
    fn comparisons_need_the_same_dimension() {
        assert_eq!(err("width > 3"), mismatch(6, ">", LENGTH, NONE));
        assert_eq!(err("1 mm == 1 deg"), mismatch(5, "==", LENGTH, ANGLE));
        assert_eq!(err("tilt <= width"), mismatch(5, "<=", ANGLE, LENGTH));
        assert_eq!(
            err("width != width * wall"),
            mismatch(6, "!=", LENGTH, AREA)
        );
    }

    /// Equality holds within a relative 1e-12: values that are meant to be
    /// equal but come from different arithmetic compare equal, and the
    /// comparisons stay consistent with it (exactly one of `<`, `==`, `>`).
    #[test]
    fn equality_within_rounding() {
        assert!(eval("0.1 + 0.2").value != 0.3);
        check("0.1 + 0.2 == 0.3", 1.0, NONE);
        check("0.1 + 0.2 != 0.3", 0.0, NONE);
        check("0.1 + 0.2 > 0.3", 0.0, NONE);
        check("0.1 + 0.2 >= 0.3", 1.0, NONE);
        check("0.3 < 0.1 + 0.2", 0.0, NONE);
        check("0.3 <= 0.1 + 0.2", 1.0, NONE);
        check("3 * 0.1 in == 0.3 in", 1.0, NONE);
        check("r * cos(60 deg) == r / 2", 1.0, NONE);
        check("sqrt(2) ^ 2 == 2", 1.0, NONE);
        // At every scale: the bound is relative.
        check("(0.1 + 0.2) * 1e-9 mm == 0.3e-9 mm", 1.0, NONE);
        check("(0.1 + 0.2) * 1e9 mm == 0.3e9 mm", 1.0, NONE);
        // A difference that is a part in 10^10 is a difference.
        check("1 == 1 + 1e-10", 0.0, NONE);
        check("1 < 1 + 1e-10", 1.0, NONE);
        check("1e-6 mm == 1.0000000001e-6 mm", 0.0, NONE);
        // Zero equals only zero: compare the values, not their difference.
        check("sin(pi) == 0", 0.0, NONE);
        check("0 == -0", 1.0, NONE);
        for (a, b) in [(1.0, 1.0 + 1e-13), (1.0, 2.0), (-5.0, 5.0), (0.0, 1e-300)] {
            let count = ["<", "==", ">"]
                .iter()
                .filter(|op| eval(&format!("{a:e} {op} {b:e}")).value == 1.0)
                .count();
            assert_eq!(count, 1, "{a} {b}");
        }
    }

    #[test]
    fn logic() {
        check("1 && 1", 1.0, NONE);
        check("1 && 0", 0.0, NONE);
        check("0 || 1", 1.0, NONE);
        check("0 || 0", 0.0, NONE);
        check("!0", 1.0, NONE);
        check("!5", 0.0, NONE);
        check("!!5", 1.0, NONE);
        check("-2 && 0.5", 1.0, NONE);
        check("width > 30 mm && height < 40 mm", 1.0, NONE);
        check("width > 50 mm || height > 50 mm", 0.0, NONE);
        check("!width > 50 mm", 1.0, NONE);
        check("0 && 1 || 1", 1.0, NONE);
        check("1 || 1 && 0", 1.0, NONE);
        check("(1 || 1) && 0", 0.0, NONE);
        let wrong = |source: &str| match err(source) {
            ExprError {
                pos,
                kind:
                    ErrorKind::ArgumentDimension {
                        function, found, ..
                    },
            } => (pos, function, found),
            other => panic!("`{source}`: {other:?}"),
        };
        assert_eq!(wrong("width && 1"), (6, "&&", LENGTH));
        assert_eq!(wrong("1 || tilt"), (2, "||", ANGLE));
        assert_eq!(wrong("!width"), (0, "!", LENGTH));
        // Not used, but not a condition either.
        assert_eq!(wrong("0 && width"), (2, "&&", LENGTH));
        assert!(err("!width").to_string().contains("a comparison such as"));
    }

    #[test]
    fn logic_decides_on_the_left_when_it_can() {
        check("0 && 1 / zero > 0", 0.0, NONE);
        check("1 || sqrt(-1) > 0", 1.0, NONE);
        check("n != 0 && 12 / n > 1", 1.0, NONE);
        check("zero != 0 && 12 / zero > 1", 0.0, NONE);
        check("0 && depth", 0.0, NONE);
        assert_eq!(
            err("1 && 1 / zero > 0"),
            ExprError::new(7, ErrorKind::DivisionByZero)
        );
        assert_eq!(
            err("0 || depth"),
            ExprError::new(
                5,
                ErrorKind::UnknownVariable {
                    name: "depth".into()
                }
            )
        );
        // An error on the left is always an error.
        assert_eq!(
            err("1 / zero > 0 || 1"),
            ExprError::new(2, ErrorKind::DivisionByZero)
        );
    }

    #[test]
    fn if_takes_one_branch() {
        check("if(1, 2 mm, 3 mm)", 2.0, LENGTH);
        check("if(0, 2 mm, 3 mm)", 3.0, LENGTH);
        check("if(width > 200 mm, 4, 2)", 2.0, NONE);
        check("if(width > 20 mm, 4, 2)", 4.0, NONE);
        check("if(-0.5, 1, 2)", 1.0, NONE);
        check("if(n > 3, if(n > 5, 3, 2), 1)", 3.0, NONE);
        check("if(n > 3, width, height) / 2", 20.0, LENGTH);
        check("max(if(n > 3, width, height), 45 mm)", 45.0, LENGTH);
    }

    /// Only the branch taken has to evaluate: the other may divide by
    /// zero, leave a domain or name a variable that does not exist.
    #[test]
    fn the_branch_not_taken_may_fail() {
        check("if(zero > 0, width / zero, width)", 40.0, LENGTH);
        check("if(zero == 0, width, width / zero)", 40.0, LENGTH);
        check("if(1, 1, sqrt(-1))", 1.0, NONE);
        check("if(1, 1 mm, depth)", 1.0, LENGTH);
        check("if(0, 1e308 * 10, 1)", 1.0, NONE);
        check("if(0, nan, 1)", 1.0, NONE);
        check("if(0, 1 mm + 1 deg, 2)", 2.0, NONE);
        check("if(1, 2, if(1, 1 / zero, 3))", 2.0, NONE);
        // The branch taken fails as it would on its own.
        assert_eq!(
            err("if(zero == 0, width / zero, width)"),
            ExprError::new(20, ErrorKind::DivisionByZero)
        );
        assert_eq!(
            err("if(0, 1, depth)"),
            ExprError::new(
                9,
                ErrorKind::UnknownVariable {
                    name: "depth".into()
                }
            )
        );
        // And so does the condition.
        assert_eq!(
            err("if(1 / zero, 1, 2)"),
            ExprError::new(5, ErrorKind::DivisionByZero)
        );
    }

    #[test]
    fn if_needs_branches_of_one_dimension_and_a_condition() {
        assert_eq!(err("if(1, 1 mm, 2)"), mismatch(0, "if", LENGTH, NONE));
        assert_eq!(err("if(0, 1 mm, 2)"), mismatch(0, "if", LENGTH, NONE));
        assert_eq!(
            err("2 * if(n > 1, tilt, width)"),
            mismatch(4, "if", ANGLE, LENGTH)
        );
        assert!(matches!(
            err("if(width, 1, 2)").kind,
            ErrorKind::ArgumentDimension { function: "if", .. }
        ));
    }

    #[test]
    fn clamp() {
        check("clamp(5, 1, 10)", 5.0, NONE);
        check("clamp(-5, 1, 10)", 1.0, NONE);
        check("clamp(50, 1, 10)", 10.0, NONE);
        check("clamp(wall, 1.2 mm, 5 mm)", 2.0, LENGTH);
        check("clamp(width, 1.2 mm, 5 mm)", 5.0, LENGTH);
        check("clamp(0.5 mm, 1.2 mm, 5 mm)", 1.2, LENGTH);
        check("clamp(width, 1 in, 1 in)", 25.4, LENGTH);
        check("clamp(tilt, 0 deg, 10 deg)", 10.0 * PI / 180.0, ANGLE);
        assert_eq!(
            err("clamp(width, 1, 5 mm)"),
            mismatch(0, "clamp", LENGTH, NONE)
        );
        assert_eq!(err("clamp(1, 1, 5 mm)"), mismatch(0, "clamp", NONE, LENGTH));
        assert_eq!(
            err("clamp(wall, 5 mm, 1 mm)"),
            ExprError::new(
                0,
                ErrorKind::InvalidBounds {
                    function: "clamp",
                    lo: 5.0,
                    hi: 1.0
                }
            )
        );
        assert!(
            err("clamp(1, 2, 1)")
                .to_string()
                .contains("is larger than the upper bound")
        );
    }

    #[test]
    fn choose_counts_from_one() {
        check("choose(1, 2 mm, 4 mm, 8 mm)", 2.0, LENGTH);
        check("choose(3, 2 mm, 4 mm, 8 mm)", 8.0, LENGTH);
        check("choose(n / 3, width, height)", 30.0, LENGTH);
        check("choose(1, 7)", 7.0, NONE);
        // A whole number within rounding is that whole number.
        check("choose(0.3 / 0.1, 1, 2, 3)", 3.0, NONE);
        // Only the choice taken has to evaluate.
        check("choose(1, 2, 1 / zero, depth)", 2.0, NONE);
        let range = |source: &str| match err(source).kind {
            ErrorKind::ChoiceOutOfRange { index, count } => (index, count),
            other => panic!("`{source}`: {other:?}"),
        };
        assert_eq!(range("choose(0, 1, 2)"), (0.0, 2));
        assert_eq!(range("choose(3, 1, 2)"), (3.0, 2));
        assert_eq!(range("choose(-1, 1, 2)"), (-1.0, 2));
        assert_eq!(range("choose(1e300, 1, 2)"), (1e300, 2));
        assert!(
            err("choose(3, 1, 2)")
                .to_string()
                .contains("has 2 choice(s), numbered from 1")
        );
        assert_eq!(
            err("choose(1.5, 1, 2)"),
            ExprError::new(
                0,
                ErrorKind::NotWholeNumber {
                    function: "choose",
                    value: 1.5
                }
            )
        );
        assert!(matches!(
            err("choose(1 mm, 1, 2)").kind,
            ErrorKind::ArgumentDimension {
                function: "choose",
                ..
            }
        ));
        assert_eq!(
            err("choose(2, 1, 2 mm)"),
            mismatch(0, "choose", NONE, LENGTH)
        );
        assert_eq!(
            err("choose(1, 1, 2, 3 mm)"),
            mismatch(0, "choose", NONE, LENGTH)
        );
        assert_eq!(
            err("choose(2, 1, 2 mm / zero)"),
            ExprError::new(18, ErrorKind::DivisionByZero)
        );
    }

    #[test]
    fn exponentials_and_logarithms() {
        check("exp(0)", 1.0, NONE);
        check("exp(1)", std::f64::consts::E, NONE);
        check("ln(exp(2))", 2.0, NONE);
        check("log10(1000)", 3.0, NONE);
        check("log10(width / 4 mm)", 1.0, NONE);
        check("pow(2, 0.5)", 2f64.sqrt(), NONE);
        check("pow(-2, 3)", -8.0, NONE);
        check("pow(0, 0)", 1.0, NONE);
        check("pow(width / 1 mm, 1 / 3)", 40f64.powf(1.0 / 3.0), NONE);
        check("sinh(1)", 1f64.sinh(), NONE);
        check("cosh(0)", 1.0, NONE);
        check("tanh(1)", 1f64.tanh(), NONE);
        // A catenary: y = a * cosh(x / a), a curve from a formula.
        check("r * cosh(width / r) - r", 5.0 * (8f64.cosh() - 1.0), LENGTH);
        let domain = |source: &str| match err(source).kind {
            ErrorKind::OutOfDomain { function, value } => (function, value),
            other => panic!("`{source}`: {other:?}"),
        };
        assert_eq!(domain("ln(0)"), ("ln", 0.0));
        assert_eq!(domain("ln(-1)"), ("ln", -1.0));
        assert_eq!(domain("log10(-2)"), ("log10", -2.0));
        assert_eq!(domain("pow(-8, 1 / 3)"), ("pow", -8.0));
        assert_eq!(
            err("pow(0, -1)"),
            ExprError::new(0, ErrorKind::DivisionByZero)
        );
        for source in ["exp(1000)", "sinh(1000)", "cosh(-1000)", "pow(10, 400)"] {
            assert!(
                matches!(err(source).kind, ErrorKind::NonFinite { .. }),
                "`{source}`"
            );
        }
        for source in [
            "exp(1 mm)",
            "ln(width)",
            "sinh(tilt)",
            "pow(width, 2)",
            "pow(2, 1 mm)",
        ] {
            assert!(
                matches!(err(source).kind, ErrorKind::ArgumentDimension { .. }),
                "`{source}`"
            );
        }
        assert!(err("pow(width, 2)").to_string().contains("`r ^ 2`"));
    }

    #[test]
    fn a_pattern_count_from_a_condition() {
        // The kind of expression a pattern's count gets.
        check("if(width > 200 mm, 4, 2)", 2.0, NONE);
        check("choose(if(width > 30 mm, 2, 1), 3, 6)", 6.0, NONE);
        check("max(2, floor(width / 12 mm)) * (wall < 3 mm)", 3.0, NONE);
    }

    // --- tables ---

    fn belt() -> Vec<Table> {
        use crate::tables::{Cell, Column};
        let column = |name: &str, unit| Column {
            name: name.into(),
            unit,
        };
        vec![Table {
            name: "belt".into(),
            columns: vec![
                column("profile", None),
                column("pitch", Some(Unit::Millimetre)),
                column("angle", Some(Unit::Degree)),
                column("teeth", None),
            ],
            key: 0,
            rows: std::sync::Arc::new(vec![
                vec![
                    Cell::Text("GT2".into()),
                    Cell::Number(2.0),
                    Cell::Number(40.0),
                    Cell::Number(20.0),
                ],
                vec![
                    Cell::Text("HTD3".into()),
                    Cell::Number(0.3),
                    Cell::Number(0.0),
                    Cell::Text("none".into()),
                ],
            ]),
        }]
    }

    fn eval_t(source: &str) -> Result<Quantity, ExprError> {
        let tables = belt();
        Expr::parse(source)?.eval_with(vars, &tables)
    }

    #[test]
    fn lookups_take_the_dimension_of_their_column() {
        let q = eval_t("belt[1].pitch").unwrap();
        assert_eq!(q, Quantity::new(2.0, LENGTH));
        let q = eval_t("belt[\"HTD3\"].pitch * 10").unwrap();
        assert!((q.value - 3.0).abs() < 1e-12 && q.dimension == LENGTH);
        let q = eval_t("belt[n / 3].pitch").unwrap();
        assert_eq!(q.value, 0.3);
        let q = eval_t("belt[1].angle").unwrap();
        assert!((q.value - 40f64.to_radians()).abs() < 1e-15 && q.dimension == ANGLE);
        assert_eq!(
            eval_t("belt[1].teeth").unwrap(),
            Quantity::dimensionless(20.0)
        );
        assert_eq!(eval_t("width + belt[1].pitch").unwrap().value, 42.0);
        assert_eq!(
            eval_t("if(n > 10, belt[9].pitch, belt[1].pitch)")
                .unwrap()
                .value,
            2.0
        );
    }

    #[test]
    fn lookups_that_find_nothing_say_why() {
        let kind = |source: &str| eval_t(source).unwrap_err().kind;
        assert_eq!(
            kind("belt[3].pitch"),
            ErrorKind::RowOutOfRange {
                table: "belt".into(),
                index: 3.0,
                rows: 2
            }
        );
        assert!(matches!(
            kind("belt[0].pitch"),
            ErrorKind::RowOutOfRange { .. }
        ));
        assert!(matches!(
            kind("belt[-1].pitch"),
            ErrorKind::RowOutOfRange { .. }
        ));
        assert!(matches!(
            kind("belt[1.5].pitch"),
            ErrorKind::NotWholeNumber { .. }
        ));
        assert!(matches!(
            kind("belt[1 mm].pitch"),
            ErrorKind::ArgumentDimension { .. }
        ));
        assert_eq!(
            kind("belt[1].width"),
            ErrorKind::UnknownColumn {
                table: "belt".into(),
                column: "width".into()
            }
        );
        assert_eq!(
            kind("belt[\"T5\"].pitch"),
            ErrorKind::UnknownKey {
                table: "belt".into(),
                key: "T5".into()
            }
        );
        assert_eq!(
            kind("belt[2].teeth"),
            ErrorKind::TextCell {
                table: "belt".into(),
                column: "teeth".into()
            }
        );
        assert_eq!(
            kind("belt[1].profile"),
            ErrorKind::TextCell {
                table: "belt".into(),
                column: "profile".into()
            }
        );
        assert_eq!(
            kind("gear[1].m"),
            ErrorKind::UnknownTable {
                name: "gear".into()
            }
        );
        // Without tables at all.
        assert!(matches!(
            err("belt[1].pitch").kind,
            ErrorKind::UnknownTable { .. }
        ));
        assert_eq!(
            eval_t("belt[3].pitch").unwrap_err().to_string(),
            "at byte 0: table `belt` has 2 row(s), numbered from 1, so there is no row 3"
        );
    }
}
