//! Tokens → expression tree.
//!
//! Grammar (lowest precedence first):
//!
//! ```text
//! expression = and        { "||" and }              left-associative
//! and        = not        { "&&" not }              left-associative
//! not        = { "!" } comparison                   so !a < b = !(a < b)
//! comparison = sum [ CMP sum ]                      not chained: a < b < c is an error
//! sum        = term   { ("+" | "-") term }          left-associative
//! term       = unary  { ("*" | "/") unary }         left-associative
//! unary      = { "-" } power                        so -2 ^ 2 = -(2 ^ 2)
//! power      = primary [ "^" [ "-" ] NUMBER ]       integer literal, not chained
//! primary    = NUMBER [ UNIT ] | "pi" | NAME | NAME "(" [ expression { "," expression } ] ")"
//!            | NAME "[" ( expression | TEXT ) "]" "." NAME      a table lookup
//!            | "(" expression ")"
//! CMP        = "<" | "<=" | ">" | ">=" | "==" | "!="
//! ```
//!
//! `!` binds looser than a comparison and tighter than `&&`, as `not` does
//! in Python: a condition is negated whole. `2 * !c` is therefore a parse
//! error; write `2 * (!c)`.
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
    /// The comparisons: operands of the same dimension, result 1 or 0.
    Less,
    LessEq,
    Greater,
    GreaterEq,
    Equal,
    NotEqual,
    /// `&&` and `||`: dimensionless operands, result 1 or 0. The right
    /// operand only counts when the left one does not decide.
    And,
    Or,
}

impl BinaryOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Less => "<",
            BinaryOp::LessEq => "<=",
            BinaryOp::Greater => ">",
            BinaryOp::GreaterEq => ">=",
            BinaryOp::Equal => "==",
            BinaryOp::NotEqual => "!=",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
        }
    }

    /// The comparison a token stands for, if it is one.
    fn comparison(kind: TokenKind) -> Option<BinaryOp> {
        Some(match kind {
            TokenKind::Less => BinaryOp::Less,
            TokenKind::LessEq => BinaryOp::LessEq,
            TokenKind::Greater => BinaryOp::Greater,
            TokenKind::GreaterEq => BinaryOp::GreaterEq,
            TokenKind::EqEq => BinaryOp::Equal,
            TokenKind::NotEq => BinaryOp::NotEqual,
            _ => return None,
        })
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
    Exp,
    Ln,
    Log10,
    Pow,
    Sinh,
    Cosh,
    Tanh,
    If,
    Clamp,
    Choose,
}

impl Function {
    pub const ALL: [Function; 26] = [
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
        Function::Exp,
        Function::Ln,
        Function::Log10,
        Function::Pow,
        Function::Sinh,
        Function::Cosh,
        Function::Tanh,
        Function::If,
        Function::Clamp,
        Function::Choose,
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
            Function::Exp => "exp",
            Function::Ln => "ln",
            Function::Log10 => "log10",
            Function::Pow => "pow",
            Function::Sinh => "sinh",
            Function::Cosh => "cosh",
            Function::Tanh => "tanh",
            Function::If => "if",
            Function::Clamp => "clamp",
            Function::Choose => "choose",
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
            Function::Atan2 | Function::Mod | Function::Pow if count != 2 => Err("2"),
            Function::Atan2 | Function::Mod | Function::Pow => Ok(()),
            Function::If | Function::Clamp if count != 3 => Err("3"),
            Function::If | Function::Clamp => Ok(()),
            Function::Choose if count < 2 => Err("at least 2"),
            Function::Choose => Ok(()),
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
    /// `!x`: 1 when `x` is 0, else 0.
    Not(NodeId),
    /// `table[row].column`: a cell of a table.
    Lookup {
        table: String,
        row: LookupRow,
        column: String,
    },
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

/// How a lookup names its row.
#[derive(Clone, Debug, PartialEq)]
pub enum LookupRow {
    /// An expression whose value is the row's number, counted from 1.
    Number(NodeId),
    /// The row's key, written in quotes.
    Key(String),
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

    /// Every lookup's table, with the byte offset of its name, in source
    /// order.
    pub fn table_refs(&self) -> impl Iterator<Item = (&str, usize)> {
        self.nodes.iter().filter_map(|node| match &node.kind {
            NodeKind::Lookup { table, .. } => Some((table.as_str(), node.pos)),
            _ => None,
        })
    }

    /// The distinct tables the expression looks values up in, in order of
    /// first appearance.
    pub fn tables(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for (name, _) in self.table_refs() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names
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
        let mut left = self.and()?;
        loop {
            let token = self.peek();
            if token.kind != TokenKind::OrOr {
                return Ok(left);
            }
            self.advance();
            let right = self.and()?;
            left = self.push(
                NodeKind::Binary {
                    op: BinaryOp::Or,
                    left,
                    right,
                },
                token.pos,
            );
        }
    }

    fn and(&mut self) -> Result<NodeId, ExprError> {
        let mut left = self.not()?;
        loop {
            let token = self.peek();
            if token.kind != TokenKind::AndAnd {
                return Ok(left);
            }
            self.advance();
            let right = self.not()?;
            left = self.push(
                NodeKind::Binary {
                    op: BinaryOp::And,
                    left,
                    right,
                },
                token.pos,
            );
        }
    }

    /// A run of `!` is consumed in a loop, as a run of minus signs is.
    fn not(&mut self) -> Result<NodeId, ExprError> {
        let mut bang_positions = Vec::new();
        while self.peek().kind == TokenKind::Bang {
            bang_positions.push(self.peek().pos);
            self.advance();
        }
        let mut node = self.comparison()?;
        for pos in bang_positions.into_iter().rev() {
            node = self.push(NodeKind::Not(node), pos);
        }
        Ok(node)
    }

    fn comparison(&mut self) -> Result<NodeId, ExprError> {
        let left = self.sum()?;
        let token = self.peek();
        let Some(op) = BinaryOp::comparison(token.kind) else {
            return Ok(left);
        };
        self.advance();
        let right = self.sum()?;
        let after = self.peek();
        if BinaryOp::comparison(after.kind).is_some() {
            return Err(ExprError::new(after.pos, ErrorKind::ChainedComparison));
        }
        Ok(self.push(NodeKind::Binary { op, left, right }, token.pos))
    }

    fn sum(&mut self) -> Result<NodeId, ExprError> {
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
                if self.peek().kind == TokenKind::LBracket {
                    return self.lookup(token);
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

    /// `name` has been consumed and the next token is `[`: a lookup
    /// `name[row].column`.
    fn lookup(&mut self, name: Token) -> Result<NodeId, ExprError> {
        let table = self.text(name).to_string();
        if crate::is_reserved_name(&table) {
            return Err(self.expected("a table name before `[`", name));
        }
        self.enter(self.peek().pos)?;
        self.advance();
        let first = self.peek();
        let row = if first.kind == TokenKind::Text {
            self.advance();
            let key = self
                .source
                .get(first.pos + 1..first.end.saturating_sub(1))
                .unwrap_or("");
            LookupRow::Key(key.to_string())
        } else {
            LookupRow::Number(self.expression()?)
        };
        let close = self.peek();
        if close.kind != TokenKind::RBracket {
            return Err(self.expected("an operator or `]`", close));
        }
        self.advance();
        self.leave();
        let dot = self.peek();
        if dot.kind != TokenKind::Dot {
            return Err(self.expected("`.` and a column name after `]`", dot));
        }
        self.advance();
        let column = self.peek();
        if column.kind != TokenKind::Ident {
            return Err(self.expected("a column name after `.`", column));
        }
        self.advance();
        let column = self.text(column).to_string();
        Ok(self.push(NodeKind::Lookup { table, row, column }, name.pos))
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
        assert_eq!(err("1 . 2"), expected(2, operator, "`.`"));
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
        let message = err("sine(2)").to_string();
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
            err("1 + sine(2)"),
            ExprError::new(
                4,
                ErrorKind::UnknownFunction {
                    name: "sine".into()
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
        assert_eq!(count("pow(2)"), (0, "pow", "2", 1));
        assert_eq!(count("if(1, 2)"), (0, "if", "3", 2));
        assert_eq!(count("if(1, 2, 3, 4)"), (0, "if", "3", 4));
        assert_eq!(count("clamp(1, 2)"), (0, "clamp", "3", 2));
        assert_eq!(count("choose(1)"), (0, "choose", "at least 2", 1));
        for function in Function::ALL {
            let one = Expr::parse(&format!("{}(1)", function.name()));
            let more_than_one = matches!(
                function,
                Function::Atan2
                    | Function::Mod
                    | Function::Pow
                    | Function::If
                    | Function::Clamp
                    | Function::Choose
            );
            assert_eq!(one.is_ok(), !more_than_one, "{function:?}");
        }
        assert!(Expr::parse("min(1, 2, 3, 4)").is_ok());
        assert!(Expr::parse("choose(2, 1, 2, 3, 4)").is_ok());
    }

    /// The tree of `source` written out with every operation in
    /// parentheses, to read precedence and associativity off it.
    fn tree(source: &str) -> String {
        fn show(expr: &Expr, id: NodeId) -> String {
            match &expr.nodes()[id].kind {
                NodeKind::Binary { op, left, right } => {
                    format!(
                        "({} {} {})",
                        show(expr, *left),
                        op.symbol(),
                        show(expr, *right)
                    )
                }
                NodeKind::Not(inner) => format!("!{}", show(expr, *inner)),
                NodeKind::Neg(inner) => format!("-{}", show(expr, *inner)),
                NodeKind::Pow { base, exponent } => format!("({} ^ {exponent})", show(expr, *base)),
                NodeKind::Variable(name) => name.clone(),
                NodeKind::Number { value, .. } => value.to_string(),
                NodeKind::Call { function, args } => {
                    let args: Vec<String> = args.iter().map(|&a| show(expr, a)).collect();
                    format!("{}({})", function.name(), args.join(", "))
                }
                NodeKind::Pi => "pi".to_string(),
                NodeKind::Lookup { table, row, column } => match row {
                    LookupRow::Number(id) => format!("{table}[{}].{column}", show(expr, *id)),
                    LookupRow::Key(key) => format!("{table}[{key:?}].{column}"),
                },
            }
        }
        let expr = Expr::parse(source).unwrap();
        show(&expr, expr.nodes().len() - 1)
    }

    #[test]
    fn precedence_of_comparisons_and_logic() {
        assert_eq!(tree("a + 1 < b * 2"), "((a + 1) < (b * 2))");
        assert_eq!(tree("a < b && c > d"), "((a < b) && (c > d))");
        assert_eq!(tree("a || b && c"), "(a || (b && c))");
        assert_eq!(tree("a && b || c"), "((a && b) || c)");
        assert_eq!(tree("a || b || c"), "((a || b) || c)");
        assert_eq!(tree("a && b && c"), "((a && b) && c)");
        assert_eq!(tree("!a < b"), "!(a < b)");
        assert_eq!(tree("!a && b"), "(!a && b)");
        assert_eq!(tree("!!a"), "!!a");
        assert_eq!(tree("-a == b"), "(-a == b)");
        assert_eq!(tree("a != b ^ 2"), "(a != (b ^ 2))");
        assert_eq!(tree("(a < b) < c"), "((a < b) < c)");
        assert_eq!(tree("2 * (!c)"), "(2 * !c)");
        assert_eq!(tree("if(a >= b, a, b)"), "if((a >= b), a, b)");
        for op in ["<", "<=", ">", ">=", "==", "!="] {
            assert_eq!(tree(&format!("a {op} b")), format!("(a {op} b)"));
        }
    }

    #[test]
    fn table_lookups() {
        assert_eq!(tree("belt[2].pitch"), "belt[2].pitch");
        assert_eq!(tree("belt[\"GT2\"].pitch"), "belt[\"GT2\"].pitch");
        assert_eq!(
            tree("2 * belt[n + 1].pitch ^ 2"),
            "(2 * (belt[(n + 1)].pitch ^ 2))"
        );
        assert_eq!(tree("t[t[1].i].x"), "t[t[1].i].x");
        // A column may share a variable's or a unit's name.
        assert_eq!(tree("t[1].mm + mm_x"), "(t[1].mm + mm_x)");
        let expr = Expr::parse("belt[profile].pitch + gear[1].m * belt[2].x").unwrap();
        assert_eq!(expr.tables(), ["belt", "gear"]);
        assert_eq!(expr.variables(), ["profile"]);
        let refs: Vec<(&str, usize)> = expr.table_refs().collect();
        assert_eq!(refs, [("belt", 0), ("gear", 22), ("belt", 34)]);
        for (source, pos) in [
            ("belt[1]", 7),
            ("belt[1].", 8),
            ("belt[1].2", 7),
            ("belt[1 .x", 7),
            ("belt[].x", 5),
            ("sin[1].x", 0),
            ("mm[1].x", 0),
        ] {
            assert_eq!(err(source).pos, pos, "{source}");
        }
        assert!(Expr::parse("belt[1].x").is_ok());
        // Nesting counts against the depth.
        let deep = format!(
            "{}1{}",
            "t[".repeat(MAX_DEPTH + 1),
            "].x".repeat(MAX_DEPTH + 1)
        );
        assert!(matches!(err(&deep).kind, ErrorKind::TooDeep { .. }));
    }

    #[test]
    fn comparisons_do_not_chain() {
        assert_eq!(
            err("a < b < c"),
            ExprError::new(6, ErrorKind::ChainedComparison)
        );
        assert_eq!(
            err("1 == 1 != 0"),
            ExprError::new(7, ErrorKind::ChainedComparison)
        );
        assert!(err("a < b < c").to_string().contains("a < b && b < c"));
    }

    #[test]
    fn logic_needs_operands() {
        assert_eq!(err("a &&"), expected(4, OPERAND, END));
        assert_eq!(err("|| a"), expected(0, OPERAND, "`||`"));
        assert_eq!(err("a <"), expected(3, OPERAND, END));
        assert_eq!(err("2 * !c"), expected(4, OPERAND, "`!`"));
        assert_eq!(err("!"), expected(1, OPERAND, END));
        assert!(matches!(err("a = b").kind, ErrorKind::LoneOperator { .. }));
    }

    #[test]
    fn variables_of_every_branch_are_listed() {
        let expr = Expr::parse("if(a > 0, b, c) + choose(n, d, e) + (f || g) * 0").unwrap();
        assert_eq!(expr.variables(), ["a", "b", "c", "n", "d", "e", "f", "g"]);
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
            err("sine(1)")
                .to_string()
                .contains("unknown function `sine`")
        );
        assert!(
            err("atan2(1)")
                .to_string()
                .contains("takes 2 argument(s), but 1")
        );
    }
}
