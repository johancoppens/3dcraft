//! Every expression in the repository's documents, examples and fixtures
//! evaluates as it did before a unit was accepted after a parenthesis, a
//! call or `pi` (2026-10-28).
//!
//! The argument: the parser changed in one place only. After a `)` or a
//! `pi` it now takes a unit that follows and makes a
//! [`NodeKind::WithUnit`] node of it; before, that unit was always a parse
//! error. A source whose tree has no such node is therefore parsed into
//! the tree it was parsed into before, and the evaluator treats every
//! other node as it did. So it is enough to show that no expression of the
//! corpus has a `WithUnit` node, but the ones listed here, which were
//! refused before (examples of the refusal in the docs).
//!
//! The corpus is every string in quotes in the repository's `.json`,
//! `.3dcraft`, `.3dcraftjig` (stored ZIP archives, so their JSON is
//! readable as it is) and `.md` files: names and paths among them parse as
//! a variable or not at all, which is harmless.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use craft3d_expr::{Expr, NodeKind};

/// Sources in the corpus that have a unit after a parenthesis or `pi`:
/// refused before 2026-10-28, written in the docs to show the rule, and
/// what the second field test tried (`docs/field-test/2026-10-22`,
/// `d2-units.json`: refused then, which is what it found).
const NEW_MEANING: &[&str] = &[
    "(pi / 4) rad",
    "pi rad / 4",
    "(a + b) mm",
    "(360 / n) deg",
    "2 * pi rad / n",
    "sin(pi rad / 6)",
];

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !matches!(
                name.as_str(),
                "target" | ".git" | ".claude" | "node_modules" | "log" | "plan"
            ) {
                files(&path, out);
            }
        } else if [".json", ".3dcraft", ".3dcraftjig", ".md"]
            .iter()
            .any(|ext| name.ends_with(ext))
        {
            out.push(path);
        }
    }
}

/// The strings in double quotes in `bytes`, unescaped as JSON writes
/// them (`\"`, `\\`), up to the longest source an expression may be.
fn quoted(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let mut text = Vec::new();
        let mut j = i + 1;
        let mut closed = false;
        while j < bytes.len() && text.len() <= craft3d_expr::MAX_SOURCE_LEN {
            match bytes[j] {
                b'"' => {
                    closed = true;
                    break;
                }
                b'\\' if j + 1 < bytes.len() => {
                    text.push(bytes[j + 1]);
                    j += 2;
                }
                b'\n' => break,
                b => {
                    text.push(b);
                    j += 1;
                }
            }
        }
        if closed && let Ok(text) = String::from_utf8(text) {
            out.push(text);
        }
        i = j + 1;
    }
    out
}

#[test]
fn every_expression_of_the_corpus_keeps_its_meaning() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut paths = Vec::new();
    files(&root, &mut paths);
    let (mut documents, mut parsed, mut with_pi) = (0, 0, 0);
    let mut new_meaning = Vec::new();
    for path in &paths {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if path.to_string_lossy().contains(".3dcraft") {
            documents += 1;
        }
        for source in quoted(&bytes) {
            let Ok(expr) = Expr::parse(&source) else {
                continue;
            };
            parsed += 1;
            if expr.nodes().iter().any(|n| n.kind == NodeKind::Pi) {
                with_pi += 1;
            }
            if expr
                .nodes()
                .iter()
                .any(|n| matches!(n.kind, NodeKind::WithUnit { .. }))
                && !NEW_MEANING.contains(&source.as_str())
            {
                new_meaning.push(format!("{}: `{source}`", path.display()));
            }
        }
    }
    // Not vacuous: the showcase and the fixtures are there and read.
    assert!(documents >= 60, "{documents} documents");
    assert!(parsed >= 10_000, "{parsed} strings parsed");
    assert!(with_pi >= 10, "{with_pi} with pi");
    assert!(
        new_meaning.is_empty(),
        "these sources have a unit after a parenthesis, a call or pi, which was refused before; \
         if they are new examples of the rule, list them in NEW_MEANING: {new_meaning:#?}"
    );
}
