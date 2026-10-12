//! Expression tree + variable values → quantity.
//!
//! Evaluation is one pass over the flat node list (operands always precede
//! their node), so it needs no recursion. Every intermediate result is
//! checked: a quantity that leaves this module is finite.

use crate::error::{ErrorKind, ExprError};
use crate::parser::{BinaryOp, Expr, Function, Node, NodeId, NodeKind};
use crate::units::{Dimension, Quantity};

impl Expr {
    /// Evaluates the expression. `lookup` returns the value of a variable,
    /// or `None` when there is no such variable.
    pub fn eval(
        &self,
        mut lookup: impl FnMut(&str) -> Option<Quantity>,
    ) -> Result<Quantity, ExprError> {
        let nodes = self.nodes();
        let mut values: Vec<Quantity> = Vec::with_capacity(nodes.len());
        for node in nodes {
            let value = eval_node(node, &values, &mut lookup)
                .map_err(|kind| ExprError::new(node.pos, kind))?;
            values.push(value);
        }
        values
            .last()
            .copied()
            .ok_or(ExprError::new(0, ErrorKind::Internal))
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
        NodeKind::Binary { op, .. } => op.symbol(),
        NodeKind::Pow { .. } => "^",
        NodeKind::Call { function, .. } => function.name(),
    }
}

fn eval_node(
    node: &Node,
    values: &[Quantity],
    lookup: &mut impl FnMut(&str) -> Option<Quantity>,
) -> Result<Quantity, ErrorKind> {
    // The parser only refers to earlier nodes; `Internal` is the answer to
    // a tree that breaks that rule, instead of an out-of-bounds panic.
    let operand = |id: NodeId| values.get(id).copied().ok_or(ErrorKind::Internal);
    let result = match &node.kind {
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
        NodeKind::Binary { op, left, right } => binary(*op, operand(*left)?, operand(*right)?)?,
        NodeKind::Pow { base, exponent } => power(operand(*base)?, *exponent)?,
        NodeKind::Call { function, args } => {
            let args = args
                .iter()
                .map(|&id| operand(id))
                .collect::<Result<Vec<Quantity>, ErrorKind>>()?;
            call(*function, &args)?
        }
    };
    if !result.value.is_finite() {
        return Err(ErrorKind::NonFinite {
            operation: operation_name(&node.kind),
        });
    }
    Ok(result)
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
    }
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
}
