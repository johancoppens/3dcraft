//! Hostile-input tests: random text, deep nesting, long input. They assert
//! that parsing and evaluating always return (a panic or a stack overflow
//! fails the test) and that what they return keeps the crate's promises.
//! Deterministic: a fixed-seed xorshift generator, no external crate.

use crate::parser::parse_with_limits;
use crate::{
    ErrorKind, Expr, ExprError, MAX_DEPTH, MAX_SOURCE_LEN, Quantity, Unit, VariableTable,
    rename_variable,
};

/// Marsaglia's xorshift64 (2003). The seed must not be zero.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

fn lookup(name: &str) -> Option<Quantity> {
    match name {
        "a" => Some(Quantity::from_unit(3.0, Unit::Millimetre)),
        "b" => Some(Quantity::from_unit(-4.5, Unit::Inch)),
        "n" => Some(Quantity::dimensionless(2.0)),
        "t" => Some(Quantity::from_unit(30.0, Unit::Degree)),
        "z" => Some(Quantity::dimensionless(0.0)),
        "big" => Some(Quantity::dimensionless(1e300)),
        _ => None,
    }
}

/// Runs everything the crate does with one source text and checks the
/// promises that hold for any input. Returns whether it evaluated.
fn exercise(source: &str) -> bool {
    let check_error = |e: &ExprError| {
        assert!(
            e.pos <= source.len(),
            "{source:?}: position {} past the end",
            e.pos
        );
        assert!(
            source.is_char_boundary(e.pos),
            "{source:?}: position {} inside a char",
            e.pos
        );
        assert!(!e.to_string().is_empty());
    };
    // Renaming must cope with anything, and renaming back must restore the
    // text byte for byte (unless it already mentioned the temporary name).
    let renamed = rename_variable(source, "a", "renamed_a").unwrap();
    assert!(renamed.len() >= source.len());
    if !source.contains("renamed_a") {
        assert_eq!(rename_variable(&renamed, "renamed_a", "a").unwrap(), source);
    }

    let expr = match Expr::parse(source) {
        Ok(expr) => expr,
        Err(e) => {
            check_error(&e);
            return false;
        }
    };
    assert!(!expr.nodes().is_empty());
    assert!(expr.nodes().len() <= source.len());
    for (name, pos) in expr.variable_refs() {
        assert_eq!(source.get(pos..pos + name.len()), Some(name));
    }
    // With a table to look in as well.
    let table = crate::Table {
        name: "t".into(),
        columns: vec![
            crate::Column {
                name: "k".into(),
                unit: None,
            },
            crate::Column {
                name: "x".into(),
                unit: Some(Unit::Inch),
            },
        ],
        key: 0,
        rows: vec![
            vec![crate::Cell::Text("GT2".into()), crate::Cell::Number(2.5)],
            vec![crate::Cell::Number(7.0), crate::Cell::Number(-1.0)],
        ],
    };
    match expr.eval_with(lookup, std::slice::from_ref(&table)) {
        Ok(q) => assert!(q.value.is_finite(), "{source:?} gave {q:?}"),
        Err(e) => {
            check_error(&e);
            assert!(e.kind != ErrorKind::Internal, "{source:?}");
        }
    }
    match expr.eval(lookup) {
        Ok(q) => {
            assert!(q.value.is_finite(), "{source:?} gave {q:?}");
            true
        }
        Err(e) => {
            check_error(&e);
            assert!(e.kind != ErrorKind::Internal, "{source:?}");
            false
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    // Mostly bytes that mean something to the lexer, so that the parser
    // gets past the first token; plus raw bytes, which become U+FFFD and
    // the occasional real multi-byte character.
    let alphabet = b"0123456789.eE+-*/^(), \t\nabmnpitz_dgrsqxc<>=!&|f[].\"";
    for _ in 0..100_000 {
        let len = rng.below(40);
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                if rng.below(8) == 0 {
                    (rng.next() & 0xFF) as u8
                } else {
                    alphabet[rng.below(alphabet.len())]
                }
            })
            .collect();
        exercise(&String::from_utf8_lossy(&bytes));
    }
}

const TOKENS: &[&str] = &[
    "0",
    "1",
    "2",
    "3",
    "7",
    "0.5",
    "1.5e-3",
    "1e308",
    "1e-320",
    "25",
    ".5",
    "4.",
    "1e999",
    "2147483647",
    "2147483648",
    "mm",
    "um",
    "cm",
    "m",
    "in",
    "ft",
    "deg",
    "rad",
    "a",
    "b",
    "n",
    "t",
    "z",
    "big",
    "unknown",
    "pi",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sqrt",
    "abs",
    "min",
    "max",
    "if",
    "clamp",
    "choose",
    "exp",
    "ln",
    "pow",
    "cosh",
    "<",
    "<=",
    ">",
    ">=",
    "==",
    "!=",
    "&&",
    "||",
    "[",
    "]",
    ".",
    "t[1].x",
    "\"GT2\"",
    "\"",
    "!",
    "&",
    "|",
    "+",
    "-",
    "*",
    "/",
    "^",
    "(",
    ")",
    ",",
    " ",
    "  ",
    "\t",
    "é",
    "µm",
    "×",
    "\u{1F600}",
    "$",
    "=",
    "e",
    "E",
    "_",
    "..",
    "\0",
];

#[test]
fn random_token_sequences_never_panic() {
    let mut rng = XorShift(0x2545_F491_4F6C_DD1D);
    let mut evaluated = 0usize;
    for _ in 0..100_000 {
        let count = rng.below(24);
        let mut source = String::new();
        for _ in 0..count {
            source.push_str(rng.pick(TOKENS));
            if rng.below(2) == 0 {
                source.push(' ');
            }
        }
        if exercise(&source) {
            evaluated += 1;
        }
    }
    // The generator must reach the evaluator, not only the error paths.
    assert!(evaluated > 1_000, "only {evaluated} sources evaluated");
}

/// A random expression that is valid by construction (it may still fail to
/// evaluate: dimensions, domains, overflow).
fn random_expression(rng: &mut XorShift, depth: usize, out: &mut String) {
    let leaf = depth == 0 || rng.below(3) == 0;
    if leaf {
        match rng.below(4) {
            0 => out.push_str(rng.pick(&["a", "b", "n", "t", "z", "pi"])),
            1 => {
                out.push_str(rng.pick(&["0", "1", "2", "0.25", "1e3", "12.5"]));
                out.push_str(rng.pick(&[" mm", "mm", " in", " deg", " cm", " rad"]));
            }
            _ => out.push_str(rng.pick(&["0", "1", "2", "3", "0.5", "10"])),
        }
        return;
    }
    match rng.below(11) {
        8 => {
            out.push_str("if(");
            random_expression(rng, depth - 1, out);
            out.push_str(rng.pick(&[" > ", " <= ", " == ", " != ", " < ", ">="]));
            random_expression(rng, depth - 1, out);
            out.push_str(", ");
            random_expression(rng, depth - 1, out);
            out.push_str(", ");
            random_expression(rng, depth - 1, out);
            out.push(')');
        }
        9 => {
            out.push_str(rng.pick(&["clamp(", "choose(n, ", "choose(1, "]));
            random_expression(rng, depth - 1, out);
            out.push_str(", ");
            random_expression(rng, depth - 1, out);
            out.push_str(", ");
            random_expression(rng, depth - 1, out);
            out.push(')');
        }
        10 => {
            // Parenthesised whole: `!` cannot be the operand of `*`.
            out.push_str(rng.pick(&["(!(", "(("]));
            random_expression(rng, depth - 1, out);
            out.push_str(rng.pick(&[" && ", " || ", " < ", " == "]));
            random_expression(rng, depth - 1, out);
            out.push_str("))");
        }
        0 => {
            out.push('(');
            random_expression(rng, depth - 1, out);
            out.push(')');
        }
        1 => {
            out.push('-');
            random_expression(rng, depth - 1, out);
        }
        2 => {
            out.push('(');
            random_expression(rng, depth - 1, out);
            out.push_str(rng.pick(&[") ^ 2", ")^-1", ") ^ 0", ")^3"]));
        }
        3 => {
            let function = rng.pick(&[
                "sin", "cos", "tan", "asin", "acos", "atan", "sqrt", "abs", "exp", "ln", "log10",
                "sinh", "cosh", "tanh",
            ]);
            out.push_str(function);
            out.push('(');
            random_expression(rng, depth - 1, out);
            out.push(')');
        }
        4 => {
            out.push_str(rng.pick(&["atan2(", "min(", "max(", "pow("]));
            random_expression(rng, depth - 1, out);
            out.push_str(", ");
            random_expression(rng, depth - 1, out);
            out.push(')');
        }
        _ => {
            random_expression(rng, depth - 1, out);
            out.push_str(rng.pick(&[" + ", " - ", " * ", " / ", "+", "*"]));
            random_expression(rng, depth - 1, out);
        }
    }
}

#[test]
fn random_valid_expressions_always_parse() {
    let mut rng = XorShift(0xD1B5_4A32_D192_ED03);
    let mut evaluated = 0usize;
    for _ in 0..100_000 {
        let mut source = String::new();
        random_expression(&mut rng, 4, &mut source);
        if let Err(e) = Expr::parse(&source) {
            panic!("`{source}` should parse: {e}");
        }
        if exercise(&source) {
            evaluated += 1;
        }
    }
    assert!(evaluated > 10_000, "only {evaluated} sources evaluated");
}

#[test]
fn random_variable_tables_never_panic() {
    let mut rng = XorShift(0x0123_4567_89AB_CDEF);
    let names = ["a", "b", "c", "d", "e", "f"];
    let pieces = [
        "a", "b", "c", "d", "e", "f", "1", "2 mm", "3 deg", "+", "-", "*", "/", "(", ")", "sin(",
        "sqrt(", ",", "^ 2", "é", "nope", "if(", "choose(", ">", "==", "&&", "!",
    ];
    for _ in 0..20_000 {
        let mut table = VariableTable::new();
        for _ in 0..rng.below(8) {
            let mut source = String::new();
            for _ in 0..rng.below(6) {
                source.push_str(rng.pick(&pieces));
                source.push(' ');
            }
            table.set(rng.pick(&names), &source).unwrap();
        }
        if rng.below(4) == 0 {
            // Either outcome is fine; it must not panic.
            let _ = table.rename(rng.pick(&names), rng.pick(&["g", "a", "mm", "b2"]));
        }
        if rng.below(4) == 0 {
            let _ = table.remove(rng.pick(&names));
        }
        let order = table.evaluation_order();
        assert_eq!(order.len(), table.len());
        let mut sorted = order.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), table.len(), "order is not a permutation");

        let evaluation = table.evaluate();
        assert_eq!(evaluation.entries().len(), table.len());
        // Determinism: the same table gives the same answer again.
        assert_eq!(table.clone().evaluate(), evaluation);
        for (i, entry) in evaluation.entries().iter().enumerate() {
            let variable = table.get(&entry.name).unwrap();
            match &entry.result {
                Ok(q) => {
                    assert!(q.value.is_finite());
                    // A value means every dependency came earlier; one
                    // that only a branch not taken refers to may have
                    // failed.
                    for dep in variable.parsed().unwrap().variables() {
                        let at = order.iter().position(|n| *n == dep).unwrap();
                        assert!(at < i, "{dep} must come before {}", entry.name);
                    }
                }
                Err(e) => {
                    assert!(e.pos <= variable.source().len());
                    assert!(e.kind != ErrorKind::Internal);
                }
            }
        }
    }
}

#[test]
fn a_hundred_thousand_nested_parentheses() {
    let depth = 100_000;
    let source = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
    // Through the public entry point the length cap answers first.
    assert_eq!(
        Expr::parse(&source).unwrap_err().kind,
        ErrorKind::SourceTooLong {
            len: 2 * depth + 1,
            max: MAX_SOURCE_LEN
        }
    );
    // With the length cap out of the way the depth cap must stop the
    // recursion long before the stack runs out.
    assert_eq!(
        parse_with_limits(&source, usize::MAX, MAX_DEPTH).unwrap_err(),
        ExprError::new(MAX_DEPTH, ErrorKind::TooDeep { max: MAX_DEPTH })
    );
    // Unclosed, and through function calls.
    let open = "(".repeat(depth);
    assert!(matches!(
        parse_with_limits(&open, usize::MAX, MAX_DEPTH)
            .unwrap_err()
            .kind,
        ErrorKind::TooDeep { .. }
    ));
    let calls = "sqrt(".repeat(depth);
    assert!(matches!(
        parse_with_limits(&calls, usize::MAX, MAX_DEPTH)
            .unwrap_err()
            .kind,
        ErrorKind::TooDeep { .. }
    ));
    let closing = ")".repeat(depth);
    assert!(matches!(
        parse_with_limits(&closing, usize::MAX, MAX_DEPTH)
            .unwrap_err()
            .kind,
        ErrorKind::Expected { .. }
    ));
}

#[test]
fn very_long_input() {
    // 10 MB is rejected by its length, before any work is done on it.
    let huge = "1 + ".repeat(2_500_000) + "1";
    assert_eq!(
        Expr::parse(&huge).unwrap_err(),
        ExprError::new(
            MAX_SOURCE_LEN,
            ErrorKind::SourceTooLong {
                len: huge.len(),
                max: MAX_SOURCE_LEN
            }
        )
    );
    let non_ascii = "é".repeat(1_000_000);
    assert!(matches!(
        Expr::parse(&non_ascii).unwrap_err().kind,
        ErrorKind::SourceTooLong { .. }
    ));
    let mut table = VariableTable::new();
    assert!(table.set("a", &huge).is_err());
    assert!(table.set(&"a".repeat(1_000_000), "1").is_err());
    assert!(table.is_empty());
}

#[test]
fn long_flat_chains_do_not_recurse() {
    // A chain makes a tree as deep as it is long. Without the length cap,
    // parsing, evaluating and dropping it must still be loops.
    let terms = 200_000;
    let sum = "1 + ".repeat(terms) + "1";
    let expr = parse_with_limits(&sum, usize::MAX, MAX_DEPTH).unwrap();
    assert_eq!(expr.eval(lookup).unwrap().value, (terms + 1) as f64);

    let minuses = "-".repeat(terms + 1) + "1";
    let expr = parse_with_limits(&minuses, usize::MAX, MAX_DEPTH).unwrap();
    assert_eq!(expr.eval(lookup).unwrap().value, -1.0);

    let product = "2 * ".repeat(5_000) + "1";
    let expr = parse_with_limits(&product, usize::MAX, MAX_DEPTH).unwrap();
    assert_eq!(
        expr.eval(lookup).unwrap_err().kind,
        ErrorKind::NonFinite { operation: "*" }
    );

    // The longest chain the public entry point accepts.
    let longest = "1+".repeat(MAX_SOURCE_LEN / 2 - 1) + "1";
    assert!(longest.len() <= MAX_SOURCE_LEN);
    assert_eq!(
        Expr::parse(&longest).unwrap().eval(lookup).unwrap().value,
        (MAX_SOURCE_LEN / 2) as f64
    );
}

#[test]
fn deep_and_long_conditions() {
    // `if` nested as deep as calls may be, in both branches.
    let nested = |depth: usize| format!("{}1{}", "if(n > 1, ".repeat(depth), ", 2)".repeat(depth));
    assert_eq!(
        Expr::parse(&nested(MAX_DEPTH))
            .unwrap()
            .eval(lookup)
            .unwrap()
            .value,
        1.0
    );
    assert!(matches!(
        Expr::parse(&nested(MAX_DEPTH + 1)).unwrap_err().kind,
        ErrorKind::TooDeep { .. }
    ));
    // Long runs of `!`, `&&` and `||` are loops, not recursion.
    let terms = 200_000;
    let bangs = "!".repeat(terms) + "1";
    let expr = parse_with_limits(&bangs, usize::MAX, MAX_DEPTH).unwrap();
    assert_eq!(expr.eval(lookup).unwrap().value, 1.0);
    let ands = "1 && ".repeat(terms) + "z";
    let expr = parse_with_limits(&ands, usize::MAX, MAX_DEPTH).unwrap();
    assert_eq!(expr.eval(lookup).unwrap().value, 0.0);
    let ors = "z || ".repeat(terms) + "1 / z";
    let expr = parse_with_limits(&ors, usize::MAX, MAX_DEPTH).unwrap();
    assert_eq!(
        expr.eval(lookup).unwrap_err().kind,
        ErrorKind::DivisionByZero
    );
    // An error deep in a branch not taken, under a long chain.
    let chain = format!("if(1, 1, {}1 / z)", "1 + ".repeat(1000));
    assert_eq!(
        Expr::parse(&chain).unwrap().eval(lookup).unwrap().value,
        1.0
    );
}
