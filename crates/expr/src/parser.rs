//! Tokens → expression tree.
//!
//! Grammar (lowest precedence first):
//!
//! ```text
//! expression = term   { ("+" | "-") term }          left-associative
//! term       = unary  { ("*" | "/") unary }         left-associative
//! unary      = { "-" } power                        so -2 ^ 2 = -(2 ^ 2)
//! power      = primary [ "^" [ "-" ] NUMBER ]       integer literal, not chained
//! primary    = NUMBER [ UNIT ] | "pi" | NAME | NAME "(" [ expression { "," expression } ] ")"
//!            | "(" expression ")"
//! ```
//!
//! The tree is stored flat: a `Vec<Node>` in which every node comes after
//! its operands. Evaluating and dropping it are therefore loops, not
//! recursion, and a long chain such as `1 + 1 + 1 + …` (a tree as deep as it
//! is long) cannot overflow the stack. Only parentheses and calls recurse in
//! the parser, and those are capped.

use crate::error::{ErrorKind, ExprError};
use crate::lexer::{Token, TokenKind, tokenize};
use crate::units::Unit;
use crate::{MAX_DEPTH, MAX_SOURCE_LEN};

/// Index of a node in [`Expr::nodes`].
pub type NodeId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl BinaryOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Sqrt,
    Abs,
    Min,
    Max,
    Floor,
    Ceil,
    Round,
    Trunc,
    Mod,
}

impl Function {
    pub const ALL: [Function; 16] = [
        Function::Sin,
        Function::Cos,
        Function::Tan,
        Function::Asin,
        Function::Acos,
        Function::Atan,
        Function::Atan2,
        Function::Sqrt,
        Function::Abs,
        Function::Min,
        Function::Max,
        Function::Floor,
        Function::Ceil,
        Function::Round,
        Function::Trunc,
        Function::Mod,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Function::Sin => "sin",
            Function::Cos => "cos",
            Function::Tan => "tan",
            Function::Asin => "asin",
            Function::Acos => "acos",
            Function::Atan => "atan",
            Function::Atan2 => "atan2",
            Function::Sqrt => "sqrt",
            Function::Abs => "abs",
            Function::Min => "min",
            Function::Max => "max",
            Function::Floor => "floor",
            Function::Ceil => "ceil",
            Function::Round => "round",
            Function::Trunc => "trunc",
            Function::Mod => "mod",
        }
    }

    /// Inverse of [`Function::name`]. Case-sensitive.
    pub fn from_name(name: &str) -> Option<Function> {
        Function::ALL.into_iter().find(|f| f.name() == name)
    }

    /// Checks an argument count; on failure returns the accepted count as
    /// prose for the error message.
    fn check_argument_count(self, count: usize) -> Result<(), &'static str> {
        match self {
            Function::Atan2 | Function::Mod if count != 2 => Err("2"),
            Function::Atan2 | Function::Mod => Ok(()),
            // `floor(x)` of a plain number, `floor(x, step)` to a multiple
            // of a step of any dimension.
            Function::Floor | Function::Ceil | Function::Round | Function::Trunc
                if count == 1 || count == 2 =>
            {
                Ok(())
            }
            Function::Floor | Function::Ceil | Function::Round | Function::Trunc => Err("1 or 2"),
            Function::Min | Function::Max if count == 0 => Err("at least 1"),
            Function::Min | Function::Max => Ok(()),
            _ if count != 1 => Err("1"),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// A literal as written: `25`, `25 mm`.
    Number {
        value: f64,
        unit: Option<Unit>,
    },
    Pi,
    Variable(String),
    Neg(NodeId),
    Binary {
        op: BinaryOp,
        left: NodeId,
        right: NodeId,
    },
    Pow {
        base: NodeId,
        exponent: i32,
    },
    Call {
        function: Function,
        args: Vec<NodeId>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    /// Byte offset in the source of the token this node was made from: the
    /// operator of a binary operation, the name of a variable or function.
    pub pos: usize,
}

/// A parsed expression. Made by [`Expr::parse`], which is the only way to
/// make one; it keeps the invariant that operands precede their node.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    nodes: Vec<Node>,
}

impl Expr {
    /// Parses `source`. Does not look at variables: an unknown name is an
    /// evaluation error, not a parse error.
    pub fn parse(source: &str) -> Result<Expr, ExprError> {
        parse_with_limits(source, MAX_SOURCE_LEN, MAX_DEPTH)
    }

    /// The tree in evaluation order: every node comes after its operands,
    /// and the last node is the root. Never empty.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Every reference to a variable, with its byte offset, in source
    /// order. A name used twice appears twice.
    pub fn variable_refs(&self) -> impl Iterator<Item = (&str, usize)> {
        self.nodes.iter().filter_map(|node| match &node.kind {
            NodeKind::Variable(name) => Some((name.as_str(), node.pos)),
            _ => None,
        })
    }

    /// The distinct variable names the expression refers to, in order of
    /// first appearance.
    pub fn variables(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for (name, _) in self.variable_refs() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }
}

/// [`Expr::parse`] with explicit limits, so tests can exercise the depth
/// guard on inputs longer than the source cap.
pub(crate) fn parse_with_limits(
    source: &str,
    max_len: usize,
    max_depth: usize,
) -> Result<Expr, ExprError> {
    if source.len() > max_len {
        return Err(ExprError::new(
            max_len,
            ErrorKind::SourceTooLong {
                len: source.len(),
                max: max_len,
            },
        ));
    }
    let mut parser = Parser {
        source,
        tokens: tokenize(source)?,
        at: 0,
        nodes: Vec::new(),
        depth: 0,
        max_depth,
    };
    parser.expression()?;
    let rest = parser.peek();
    if rest.kind != TokenKind::End {
        return Err(parser.after_operand("an operator or the end of the expression", rest));
    }
    Ok(Expr {
        nodes: parser.nodes,
    })
}

/// The unit a word that is not one most likely means: a symbol in
/// another case (`MM`, `Deg`) or a unit spelled out (`inch`, `degrees`).
fn spelled_unit(word: &str) -> Option<Unit> {
    let lower = word.to_ascii_lowercase();
    if let Some(unit) = Unit::from_symbol(&lower) {
        return Some(unit);
    }
    let unit = match lower.trim_end_matches('s') {
        "micron" | "micrometre" | "micrometer" => Unit::Micrometre,
        "millimetre" | "millimeter" => Unit::Millimetre,
        "centimetre" | "centimeter" => Unit::Centimetre,
        "metre" | "meter" => Unit::Metre,
        "inch" | "inche" => Unit::Inch,
        "foot" | "feet" => Unit::Foot,
        "degree" => Unit::Degree,
        "radian" => Unit::Radian,
        _ => return None,
    };
    Some(unit)
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    at: usize,
    nodes: Vec<Node>,
    depth: usize,
    max_depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Token {
        // The token list always ends with `End` and `at` never moves past
        // it, so the fallback is only there to avoid indexing.
        self.tokens.get(self.at).copied().unwrap_or(Token {
            kind: TokenKind::End,
            pos: self.source.len(),
            end: self.source.len(),
        })
    }

    fn advance(&mut self) {
        if self.at + 1 < self.tokens.len() {
            self.at += 1;
        }
    }

    fn text(&self, token: Token) -> &str {
        self.source.get(token.pos..token.end).unwrap_or("")
    }

    fn push(&mut self, kind: NodeKind, pos: usize) -> NodeId {
        self.nodes.push(Node { kind, pos });
        self.nodes.len() - 1
    }

    fn expected(&self, expected: &'static str, found: Token) -> ExprError {
        let found_text = match found.kind {
            TokenKind::End => "the end of the expression".to_string(),
            _ => format!("`{}`", self.text(found)),
        };
        ExprError::new(
            found.pos,
            ErrorKind::Expected {
                expected,
                found: found_text,
            },
        )
    }

    /// [`Parser::expected`] for a token met where an operator could follow
    /// an operand: a word there is most often a unit in the wrong place
    /// (`(pi / 4) rad`) or a unit that does not exist (`2 inch`), and the
    /// error says so.
    fn after_operand(&self, expected: &'static str, found: Token) -> ExprError {
        if found.kind == TokenKind::Ident {
            let word = self.text(found);
            let before = self
                .at
                .checked_sub(1)
                .and_then(|before| self.tokens.get(before))
                .map(|before| (before.kind, self.text(*before)));
            // `(pi / 4) rad`, `width mm`; not `2 mm mm`, which is only one
            // unit too many.
            let after_operand = match before {
                Some((TokenKind::RParen, _)) => true,
                Some((TokenKind::Ident, text)) => Unit::from_symbol(text).is_none(),
                _ => false,
            };
            if let (true, Some(unit)) = (after_operand, Unit::from_symbol(word)) {
                return ExprError::new(found.pos, ErrorKind::UnitAfterExpression { unit });
            }
            // `2 inch`, `3 MM`, `5 degrees`: a unit spelled otherwise.
            if let (Some((TokenKind::Number(_), _)), Some(meant)) = (before, spelled_unit(word)) {
                return ExprError::new(
                    found.pos,
                    ErrorKind::UnknownUnit {
                        name: word.to_string(),
                        meant,
                    },
                );
            }
        }
        self.expected(expected, found)
    }

    /// One more level of parentheses or call. `pos` is the opening token.
    fn enter(&mut self, pos: usize) -> Result<(), ExprError> {
        if self.depth >= self.max_depth {
            return Err(ExprError::new(
                pos,
                ErrorKind::TooDeep {
                    max: self.max_depth,
                },
            ));
        }
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn expression(&mut self) -> Result<NodeId, ExprError> {
        let mut left = self.term()?;
        loop {
            let token = self.peek();
            let op = match token.kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.term()?;
            left = self.push(NodeKind::Binary { op, left, right }, token.pos);
        }
    }

    fn term(&mut self) -> Result<NodeId, ExprError> {
        let mut left = self.unary()?;
        loop {
            let token = self.peek();
            let op = match token.kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.unary()?;
            left = self.push(NodeKind::Binary { op, left, right }, token.pos);
        }
    }

    /// A run of minus signs is consumed in a loop, not by recursion: `----x`
    /// may be as long as the source allows.
    fn unary(&mut self) -> Result<NodeId, ExprError> {
        let mut minus_positions = Vec::new();
        while self.peek().kind == TokenKind::Minus {
            minus_positions.push(self.peek().pos);
            self.advance();
        }
        let mut node = self.power()?;
        for pos in minus_positions.into_iter().rev() {
            node = self.push(NodeKind::Neg(node), pos);
        }
        Ok(node)
    }

    fn power(&mut self) -> Result<NodeId, ExprError> {
        let base = self.primary()?;
        let caret = self.peek();
        if caret.kind != TokenKind::Caret {
            return Ok(base);
        }
        self.advance();
        let negative = self.peek().kind == TokenKind::Minus;
        if negative {
            self.advance();
        }
        let literal = self.peek();
        let TokenKind::Number(magnitude) = literal.kind else {
            return Err(ExprError::new(literal.pos, ErrorKind::InvalidExponent));
        };
        self.advance();
        let after = self.peek();
        let has_unit =
            after.kind == TokenKind::Ident && Unit::from_symbol(self.text(after)).is_some();
        // The range check comes before the cast: an `as` cast would
        // silently saturate `1e30`.
        if has_unit || magnitude.fract() != 0.0 || magnitude > f64::from(i32::MAX) {
            return Err(ExprError::new(literal.pos, ErrorKind::InvalidExponent));
        }
        if after.kind == TokenKind::Caret {
            return Err(ExprError::new(after.pos, ErrorKind::ChainedPower));
        }
        let magnitude = magnitude as i32;
        let exponent = if negative { -magnitude } else { magnitude };
        Ok(self.push(NodeKind::Pow { base, exponent }, caret.pos))
    }

    fn primary(&mut self) -> Result<NodeId, ExprError> {
        let token = self.peek();
        match token.kind {
            TokenKind::Number(value) => {
                self.advance();
                let next = self.peek();
                let unit = match next.kind {
                    TokenKind::Ident => Unit::from_symbol(self.text(next)),
                    _ => None,
                };
                if let Some(unit) = unit {
                    self.advance();
                    // `1e308 m` is finite as written but not in millimetres.
                    if !(value * unit.factor()).is_finite() {
                        return Err(ExprError::new(token.pos, ErrorKind::NumberOutOfRange));
                    }
                }
                Ok(self.push(NodeKind::Number { value, unit }, token.pos))
            }
            TokenKind::Ident => {
                self.advance();
                if self.peek().kind == TokenKind::LParen {
                    return self.call(token);
                }
                let name = self.text(token);
                let kind = if name == "pi" {
                    NodeKind::Pi
                } else if let Some(unit) = Unit::from_symbol(name) {
                    return Err(ExprError::new(
                        token.pos,
                        ErrorKind::UnitWithoutNumber { unit },
                    ));
                } else if Function::from_name(name).is_some() {
                    return Err(self.expected("`(` after the function name", self.peek()));
                } else {
                    NodeKind::Variable(name.to_string())
                };
                Ok(self.push(kind, token.pos))
            }
            TokenKind::LParen => {
                self.enter(token.pos)?;
                self.advance();
                let inner = self.expression()?;
                let close = self.peek();
                if close.kind != TokenKind::RParen {
                    return Err(self.after_operand("an operator or `)`", close));
                }
                self.advance();
                self.leave();
                Ok(inner)
            }
            _ => Err(self.expected("a number, a variable, a function or `(`", token)),
        }
    }

    /// `name` has been consumed and the next token is `(`.
    fn call(&mut self, name: Token) -> Result<NodeId, ExprError> {
        let Some(function) = Function::from_name(self.text(name)) else {
            return Err(ExprError::new(
                name.pos,
                ErrorKind::UnknownFunction {
                    name: self.text(name).to_string(),
                },
            ));
        };
        self.enter(self.peek().pos)?;
        self.advance();
        let mut args = Vec::new();
        if self.peek().kind == TokenKind::RParen {
            self.advance();
        } else {
            loop {
                args.push(self.expression()?);
                let separator = self.peek();
                self.advance();
                match separator.kind {
                    TokenKind::Comma => {}
                    TokenKind::RParen => break,
                    _ => return Err(self.expected("an operator, `,` or `)`", separator)),
                }
            }
        }
        self.leave();
        if let Err(expected) = function.check_argument_count(args.len()) {
            return Err(ExprError::new(
                name.pos,
                ErrorKind::ArgumentCount {
                    function: function.name(),
                    expected,
                    found: args.len(),
                },
            ));
        }
        Ok(self.push(NodeKind::Call { function, args }, name.pos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(source: &str) -> ExprError {
        Expr::parse(source).unwrap_err()
    }

    fn expected(pos: usize, expected: &'static str, found: &str) -> ExprError {
        ExprError::new(
            pos,
            ErrorKind::Expected {
                expected,
                found: found.to_string(),
            },
        )
    }

    const OPERAND: &str = "a number, a variable, a function or `(`";
    const END: &str = "the end of the expression";

    #[test]
    fn tree_is_flat_and_operands_come_first() {
        let expr = Expr::parse("width / 2 + 3 mm").unwrap();
        let kinds: Vec<&NodeKind> = expr.nodes().iter().map(|n| &n.kind).collect();
        assert_eq!(
            kinds,
            [
                &NodeKind::Variable("width".into()),
                &NodeKind::Number {
                    value: 2.0,
                    unit: None
                },
                &NodeKind::Binary {
                    op: BinaryOp::Div,
                    left: 0,
                    right: 1
                },
                &NodeKind::Number {
                    value: 3.0,
                    unit: Some(Unit::Millimetre)
                },
                &NodeKind::Binary {
                    op: BinaryOp::Add,
                    left: 2,
                    right: 3
                },
            ]
        );
        let positions: Vec<usize> = expr.nodes().iter().map(|n| n.pos).collect();
        assert_eq!(positions, [0, 8, 6, 12, 10]);
    }

    #[test]
    fn variables_are_listed_once_in_order_of_appearance() {
        let expr = Expr::parse("b + a * b - max(c, a) + pi + sin(1) + 2 mm").unwrap();
        assert_eq!(expr.variables(), ["b", "a", "c"]);
        let refs: Vec<(&str, usize)> = expr.variable_refs().collect();
        assert_eq!(refs, [("b", 0), ("a", 4), ("b", 8), ("c", 16), ("a", 19)]);
        assert!(Expr::parse("2 * pi").unwrap().variables().is_empty());
    }

    #[test]
    fn names_that_only_look_reserved_are_variables() {
        // Units, functions and `pi` are case-sensitive, and a longer name
        // that starts with one is an ordinary name.
        let expr = Expr::parse("MM + Pi + mm2 + sine + _x1 + e").unwrap();
        assert_eq!(expr.variables(), ["MM", "Pi", "mm2", "sine", "_x1", "e"]);
    }

    #[test]
    fn empty_and_blank_sources() {
        assert_eq!(err(""), expected(0, OPERAND, END));
        assert_eq!(err("  \t\n"), expected(4, OPERAND, END));
    }

    #[test]
    fn missing_operand() {
        assert_eq!(err("2 +"), expected(3, OPERAND, END));
        assert_eq!(err("2 * / 3"), expected(4, OPERAND, "`/`"));
        assert_eq!(err("(2 + )"), expected(5, OPERAND, "`)`"));
        assert_eq!(err("+2"), expected(0, OPERAND, "`+`"));
        assert_eq!(err("max(1,)"), expected(6, OPERAND, "`)`"));
        assert_eq!(err("max(,1)"), expected(4, OPERAND, "`,`"));
    }

    #[test]
    fn missing_operator() {
        let operator = "an operator or the end of the expression";
        assert_eq!(err("2 3"), expected(2, operator, "`3`"));
        assert_eq!(err("2 width"), expected(2, operator, "`width`"));
        assert_eq!(err("2 mm mm"), expected(5, operator, "`mm`"));
        assert_eq!(err("a b"), expected(2, operator, "`b`"));
        assert_eq!(err("2 pi"), expected(2, operator, "`pi`"));
        assert_eq!(err("1.2.3"), expected(3, operator, "`.3`"));
        assert_eq!(err("(1) (2)"), expected(4, operator, "`(`"));
        assert_eq!(err("1, 2"), expected(1, operator, "`,`"));
    }

    #[test]
    fn unbalanced_parentheses() {
        assert_eq!(err("(1 + 2"), expected(6, "an operator or `)`", END));
        assert_eq!(
            err("1 + 2)"),
            expected(5, "an operator or the end of the expression", "`)`")
        );
        assert_eq!(err("()"), expected(1, OPERAND, "`)`"));
        assert_eq!(err("max(1, 2"), expected(8, "an operator, `,` or `)`", END));
        assert_eq!(
            err("max(1 2)"),
            expected(6, "an operator, `,` or `)`", "`2`")
        );
    }

    #[test]
    fn functions_need_a_call() {
        assert_eq!(
            err("sin + 1"),
            expected(4, "`(` after the function name", "`+`")
        );
        assert_eq!(err("sqrt"), expected(4, "`(` after the function name", END));
    }

    #[test]
    fn the_unknown_function_message_lists_every_function() {
        let message = err("sinh(2)").to_string();
        for function in Function::ALL {
            assert!(
                message.contains(&format!(" {},", function.name()))
                    || message.contains(&format!(" {} ", function.name()))
                    || message.ends_with(&format!(" {}", function.name())),
                "{function:?} missing in: {message}"
            );
        }
    }

    #[test]
    fn unknown_function() {
        assert_eq!(
            err("1 + sinh(2)"),
            ExprError::new(
                4,
                ErrorKind::UnknownFunction {
                    name: "sinh".into()
                }
            )
        );
        // A variable or unit name followed by `(` is not implicit
        // multiplication.
        assert!(matches!(
            err("a(2)").kind,
            ErrorKind::UnknownFunction { .. }
        ));
        assert!(matches!(
            err("mm(2)").kind,
            ErrorKind::UnknownFunction { .. }
        ));
    }

    #[test]
    fn argument_counts() {
        let count = |source: &str| match err(source) {
            ExprError {
                pos,
                kind:
                    ErrorKind::ArgumentCount {
                        function,
                        expected,
                        found,
                    },
            } => (pos, function, expected, found),
            other => panic!("{source}: {other:?}"),
        };
        assert_eq!(count("sin()"), (0, "sin", "1", 0));
        assert_eq!(count("1 + sqrt(1, 2)"), (4, "sqrt", "1", 2));
        assert_eq!(count("atan2(1)"), (0, "atan2", "2", 1));
        assert_eq!(count("atan2(1, 2, 3)"), (0, "atan2", "2", 3));
        assert_eq!(count("min()"), (0, "min", "at least 1", 0));
        assert_eq!(count("max()"), (0, "max", "at least 1", 0));
        for function in Function::ALL {
            let one = Expr::parse(&format!("{}(1)", function.name()));
            let two_only = matches!(function, Function::Atan2 | Function::Mod);
            assert_eq!(one.is_ok(), !two_only, "{function:?}");
        }
        assert!(Expr::parse("min(1, 2, 3, 4)").is_ok());
    }

    #[test]
    fn unit_without_number() {
        assert_eq!(
            err("width * mm"),
            ExprError::new(
                8,
                ErrorKind::UnitWithoutNumber {
                    unit: Unit::Millimetre
                }
            )
        );
        // A unit after a parenthesis is still refused, now with its own
        // error that says how to give an expression a unit (the second
        // field test; this pins wording, not mathematics).
        assert_eq!(
            err("(1 + 2) mm").kind,
            ErrorKind::UnitAfterExpression {
                unit: Unit::Millimetre
            }
        );
        for unit in Unit::ALL {
            assert_eq!(
                err(unit.symbol()),
                ExprError::new(0, ErrorKind::UnitWithoutNumber { unit })
            );
        }
    }

    #[test]
    fn exponent_must_be_an_integer_literal() {
        let invalid = |pos| ExprError::new(pos, ErrorKind::InvalidExponent);
        assert_eq!(err("2 ^ x"), invalid(4));
        assert_eq!(err("2 ^ 0.5"), invalid(4));
        assert_eq!(err("2 ^ (2)"), invalid(4));
        assert_eq!(err("2 ^ 2 mm"), invalid(4));
        assert_eq!(err("2 ^ - x"), invalid(6));
        assert_eq!(err("2 ^"), invalid(3));
        assert_eq!(err("2 ^ 1e30"), invalid(4));
        assert_eq!(err("2 ^ 2147483648"), invalid(4));
        assert_eq!(err("2 ^ --2"), invalid(5));
        assert!(Expr::parse("2 ^ 2147483647").is_ok());
        assert!(Expr::parse("2 ^ -2147483647").is_ok());
        assert!(Expr::parse("2 ^ 2.0").is_ok());
        assert!(Expr::parse("2 ^ 2 * x").is_ok());
    }

    #[test]
    fn chained_power_is_rejected() {
        assert_eq!(err("2 ^ 3 ^ 2"), ExprError::new(6, ErrorKind::ChainedPower));
        assert!(Expr::parse("(2 ^ 3) ^ 2").is_ok());
    }

    #[test]
    fn literal_too_large_once_converted() {
        assert!(Expr::parse("1e308").is_ok());
        assert_eq!(
            err("1 + 1e308 m"),
            ExprError::new(4, ErrorKind::NumberOutOfRange)
        );
    }

    #[test]
    fn source_length_is_capped() {
        let at_limit = format!("{}1", " ".repeat(MAX_SOURCE_LEN - 1));
        assert!(Expr::parse(&at_limit).is_ok());
        let over = format!("{at_limit} ");
        assert_eq!(
            err(&over),
            ExprError::new(
                MAX_SOURCE_LEN,
                ErrorKind::SourceTooLong {
                    len: MAX_SOURCE_LEN + 1,
                    max: MAX_SOURCE_LEN
                }
            )
        );
    }

    #[test]
    fn nesting_depth_is_capped() {
        let nested = |depth: usize| format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
        assert!(Expr::parse(&nested(MAX_DEPTH)).is_ok());
        assert_eq!(
            err(&nested(MAX_DEPTH + 1)),
            ExprError::new(MAX_DEPTH, ErrorKind::TooDeep { max: MAX_DEPTH })
        );
        // Calls count too, and sibling groups do not add up.
        let calls = |depth: usize| format!("{}1{}", "abs(".repeat(depth), ")".repeat(depth));
        assert!(Expr::parse(&calls(MAX_DEPTH)).is_ok());
        assert!(matches!(
            err(&calls(MAX_DEPTH + 1)).kind,
            ErrorKind::TooDeep { .. }
        ));
        let siblings = "(1) + ".repeat(MAX_DEPTH * 2) + "1";
        assert!(Expr::parse(&siblings).is_ok());
    }

    #[test]
    fn a_unit_in_the_wrong_place_or_a_unit_that_does_not_exist_is_named() {
        // A unit after a parenthesis or a name: the error says a unit
        // belongs to a number, and how to give an expression a unit.
        for source in ["(pi / 4) rad", "pi rad / 4", "(a + b) mm", "width mm"] {
            let e = err(source);
            assert!(
                matches!(e.kind, ErrorKind::UnitAfterExpression { .. }),
                "{source}: {e}"
            );
            assert!(e.to_string().contains("* 1 "), "{source}: {e}");
        }
        // A word after a number that is not a unit (upper case, spelled
        // out): the error lists the units.
        for (source, word, meant) in [
            ("2 inch", "inch", Unit::Inch),
            ("3 MM", "MM", Unit::Millimetre),
            ("5 degrees", "degrees", Unit::Degree),
            ("1 feet", "feet", Unit::Foot),
        ] {
            let e = err(source);
            assert_eq!(
                e.kind,
                ErrorKind::UnknownUnit {
                    name: word.to_string(),
                    meant
                },
                "{source}"
            );
            assert!(e.to_string().contains("um, mm, cm, m, in, ft, deg and rad"));
        }
        // A name after a name, or after a number, is still only a missing
        // operator (see `missing_operator`).
        for source in ["a b", "2 width", "2 mm mm"] {
            assert!(
                matches!(err(source).kind, ErrorKind::Expected { .. }),
                "{source}"
            );
        }
        // What was right stays right.
        assert!(Expr::parse("pi / 4 * 1 rad").is_ok());
        assert!(Expr::parse("(pi / 4) * 1 rad").is_ok());
    }

    #[test]
    fn error_messages_say_what_to_do() {
        assert_eq!(
            err("2 +").to_string(),
            "at byte 3: expected a number, a variable, a function or `(`, \
             found the end of the expression"
        );
        assert!(err("width * mm").to_string().contains("as in `1 mm`"));
        assert!(
            err("sinh(1)")
                .to_string()
                .contains("unknown function `sinh`")
        );
        assert!(
            err("atan2(1)")
                .to_string()
                .contains("takes 2 argument(s), but 1")
        );
    }
}
