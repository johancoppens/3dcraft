//! `cargo xtask stats`: lines of Rust and tests per crate.

use crate::layers;
use std::path::Path;

/// Lines and `#[test]` attributes in the `.rs` files under `dir`.
fn count(dir: &Path, depth: u32) -> (usize, usize) {
    // Source trees are shallow; the cap stops a symlink loop.
    if depth > 32 {
        return (0, 0);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    let (mut lines, mut tests) = (0, 0);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            let (l, t) = count(&path, depth + 1);
            lines += l;
            tests += t;
        } else if path.extension().is_some_and(|e| e == "rs")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            lines += text.lines().count();
            tests += text.lines().filter(|l| l.trim() == "#[test]").count();
        }
    }
    (lines, tests)
}

/// Lines of Rust and `#[test]` attributes per package of the workspace,
/// sorted by name.
pub fn rows() -> Result<Vec<(String, usize, usize)>, String> {
    let mut rows = Vec::new();
    for package in layers::packages()? {
        let name = package
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_string();
        let manifest = package
            .get("manifest_path")
            .and_then(|p| p.as_str())
            .unwrap_or_default();
        let Some(dir) = Path::new(manifest).parent() else {
            continue;
        };
        let (lines, tests) = count(dir, 0);
        rows.push((name, lines, tests));
    }
    rows.sort();
    Ok(rows)
}

pub fn run() -> Result<(), String> {
    let rows = rows()?;
    println!("{:<22} {:>7} {:>6}", "crate", "lines", "tests");
    for (name, lines, tests) in &rows {
        println!("{name:<22} {lines:>7} {tests:>6}");
    }
    let (lines, tests) = rows.iter().fold((0, 0), |(l, t), r| (l + r.1, t + r.2));
    println!("{:<22} {lines:>7} {tests:>6}", "total");
    Ok(())
}
