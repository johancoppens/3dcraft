//! `cargo xtask notices [--out FILE]`: the third-party licence notices of
//! 3DCraft's programs, `THIRD-PARTY-NOTICES.txt` (docs/development.md,
//! Packaging). Written from `Cargo.lock` by `cargo metadata`: every crate
//! that goes into the release build of `craft3d` and `craft3d-cli` (for
//! the Linux, macOS and Windows targets the Release workflow builds) and
//! of `craft3d-web` (for `wasm32-unknown-unknown`), through normal and
//! build dependencies, workspace crates left out. Each crate is listed with
//! its version, the licence expression of its manifest and the licence
//! texts its published source carries (read from cargo's registry, so from
//! `CARGO_HOME`; `cargo metadata` may download a crate's source that is not
//! there yet, nothing else is fetched). Identical texts are written once
//! and referenced. The file is not committed: `cargo xtask package` makes
//! it into every package, and the test below makes sure it can be made.
//!
//! The same review refuses a release whose crates break the licence policy:
//! a licence outside [`ALLOWED`] (for an `OR` expression one allowed
//! alternative suffices, for `AND` every part must be allowed), or a crate
//! without a licence text that is not one of the justified [`NO_TEXT`]
//! exceptions. A crate in `NO_TEXT` gets the standard text of its licence
//! from `packaging/licenses` (the licences' published texts, unchanged:
//! `Apache-2.0.txt` from apache.org, `BSL-1.0.txt` from boost.org,
//! `MIT.txt` the SPDX text without its copyright line, which the notice
//! fills in with the crate's authors).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The licence of the examples, Jigs and assemblies built into the
/// programs (MIT, and CC BY 4.0), quoted in the notices' last section.
const EXAMPLES_LICENCE: &str = include_str!("../../docs/examples/published/LICENSE.md");

/// The name of the file in the packages, beside `LICENSE`.
pub const FILE_NAME: &str = "THIRD-PARTY-NOTICES.txt";

/// The release targets and the programs built for each (the Release
/// workflow: Ubuntu, macOS on Apple Silicon, Windows; the web job).
const TARGETS: [(&str, &[&str]); 4] = [
    ("x86_64-unknown-linux-gnu", &["craft3d", "craft3d-cli"]),
    ("aarch64-apple-darwin", &["craft3d", "craft3d-cli"]),
    ("x86_64-pc-windows-msvc", &["craft3d", "craft3d-cli"]),
    ("wasm32-unknown-unknown", &["craft3d-web"]),
];

/// The licences a shipped crate may have (SPDX identifiers): permissive
/// ones, and OFL-1.1 for fonts.
const ALLOWED: &[&str] = &[
    "0BSD",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "CC0-1.0",
    "ISC",
    "MIT",
    "OFL-1.1",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "Zlib",
];

/// `WITH` exceptions allowed on a licence: (licence, exception).
const ALLOWED_EXCEPTIONS: &[(&str, &str)] = &[("Apache-2.0", "LLVM-exception")];

/// Licences allowed but named in the notices' summary and by
/// `cargo xtask notices`, so the developer sees them: each with why.
const NOTED: &[(&str, &str)] = &[
    (
        "MPL-2.0",
        "file-level weak copyleft: the crate's own files stay under the MPL \
         (their source must stay available), the rest of 3DCraft is not affected",
    ),
    (
        "Ubuntu-font-1.0",
        "a font licence like OFL-1.1 (the Ubuntu font that egui embeds as a \
         default font): redistribution with the licence is free, a modified \
         font must be renamed and stay under the same licence",
    ),
];

/// Crates of the release build that ship no licence file in their published
/// source: their notice carries the standard text of their licence
/// (`packaging/licenses`) and names their authors. Every entry says why
/// the absence is genuine; an entry for a crate that is no longer built,
/// or that now ships a text, fails the review, so the list stays honest.
const NO_TEXT: &[(&str, &str)] = &[
    // The egui family (emilk/egui): the licence files are at the root of
    // the repository, and the published crates carry only their README.
    (
        "ecolor",
        "egui family: licence files only at the repository root",
    ),
    (
        "eframe",
        "egui family: licence files only at the repository root",
    ),
    (
        "egui",
        "egui family: licence files only at the repository root",
    ),
    (
        "egui-wgpu",
        "egui family: licence files only at the repository root",
    ),
    (
        "egui-winit",
        "egui family: licence files only at the repository root",
    ),
    (
        "egui_glow",
        "egui family: licence files only at the repository root",
    ),
    (
        "emath",
        "egui family: licence files only at the repository root",
    ),
    (
        "epaint",
        "egui family: licence files only at the repository root",
    ),
    // Its fonts' licences are in `fonts/` ([`EXTRA_FILES`]); the code's
    // MIT OR Apache-2.0 text is at the repository root only.
    (
        "epaint_default_fonts",
        "egui family: the code's licence only at the repository root",
    ),
    (
        "accesskit",
        "AccessKit: licence files only at the repository root",
    ),
    // The objc2 family (madsmtm/objc2), macOS only: licence files at the
    // repository root.
    (
        "block2",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "dispatch2",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc-sys",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2-app-kit",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2-core-data",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2-core-foundation",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2-core-graphics",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2-core-image",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2-encode",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2-foundation",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2-io-surface",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2-metal",
        "objc2 family: licence files only at the repository root",
    ),
    (
        "objc2-quartz-core",
        "objc2 family: licence files only at the repository root",
    ),
    // Single crates whose published source has no licence file.
    (
        "clipboard-win",
        "Windows clipboard (DoumanAsh/clipboard-win): the published crate has no licence file",
    ),
    (
        "dispatch",
        "macOS GCD bindings (SSheldon/rust-dispatch): the published crate has no licence file",
    ),
    (
        "gl_generator",
        "gl-rs (brendanzab/gl-rs): licence file only at the repository root",
    ),
    (
        "khronos_api",
        "gl-rs (brendanzab/gl-rs): licence file only at the repository root",
    ),
    (
        "profiling",
        "aclysma/profiling: licence files only at the repository root",
    ),
    (
        "spirv",
        "rspirv (gfx-rs/rspirv): licence file only at the repository root",
    ),
];

/// Licence texts a crate keeps below its root, as paths relative to it.
const EXTRA_FILES: &[(&str, &[&str])] = &[(
    // The fonts egui embeds, each with its licence.
    "epaint_default_fonts",
    &[
        "fonts/Hack-Regular.txt",
        "fonts/OFL.txt",
        "fonts/UFL.txt",
        "fonts/emoji-icon-font-mit-license.txt",
    ],
)];

/// The standard texts for the [`NO_TEXT`] crates, by SPDX identifier.
const STANDARD: &[(&str, &str)] = &[
    (
        "Apache-2.0",
        include_str!("../../packaging/licenses/Apache-2.0.txt"),
    ),
    (
        "BSL-1.0",
        include_str!("../../packaging/licenses/BSL-1.0.txt"),
    ),
    ("MIT", include_str!("../../packaging/licenses/MIT.txt")),
];

/// The largest licence file read (a licence is a few kilobytes).
const MAX_TEXT: u64 = 1 << 20;

/// How deep a licence expression may nest.
const MAX_DEPTH: usize = 32;

/// A licence expression (SPDX, plus the old `A/B` for `A OR B`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Expr {
    Leaf { id: String, with: Option<String> },
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in s.chars() {
        if c.is_whitespace() || c == '(' || c == ')' || c == '/' {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

struct Parser {
    tokens: Vec<String>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.at).map(String::as_str)
    }

    fn is(&self, op: &str) -> bool {
        self.peek().is_some_and(|t| t.eq_ignore_ascii_case(op))
    }

    fn next(&mut self) -> Option<String> {
        let t = self.tokens.get(self.at).cloned();
        self.at = self.at.saturating_add(1);
        t
    }

    fn or(&mut self, depth: usize) -> Result<Expr, String> {
        if depth > MAX_DEPTH {
            return Err("the expression nests too deeply".to_string());
        }
        let mut parts = vec![self.and(depth)?];
        while self.is("OR") || self.is("/") {
            self.next();
            parts.push(self.and(depth)?);
        }
        if parts.len() > 1 {
            Ok(Expr::Or(parts))
        } else {
            parts.pop().ok_or_else(|| "an empty expression".to_string())
        }
    }

    fn and(&mut self, depth: usize) -> Result<Expr, String> {
        let mut parts = vec![self.with(depth)?];
        while self.is("AND") {
            self.next();
            parts.push(self.with(depth)?);
        }
        if parts.len() > 1 {
            Ok(Expr::And(parts))
        } else {
            parts.pop().ok_or_else(|| "an empty expression".to_string())
        }
    }

    fn with(&mut self, depth: usize) -> Result<Expr, String> {
        // SPDX: `WITH` follows a simple expression (a licence id), never a
        // parenthesised compound one, even when the parentheses hold one id.
        let grouped = self.is("(");
        let atom = self.atom(depth)?;
        if !self.is("WITH") {
            return Ok(atom);
        }
        self.next();
        let (false, Expr::Leaf { id, with: None }) = (grouped, atom) else {
            return Err("WITH follows something other than a licence".to_string());
        };
        let exception = self.word()?;
        Ok(Expr::Leaf {
            id,
            with: Some(exception),
        })
    }

    fn atom(&mut self, depth: usize) -> Result<Expr, String> {
        if self.is("(") {
            self.next();
            let inner = self.or(depth.saturating_add(1))?;
            if !self.is(")") {
                return Err("a `(` is not closed".to_string());
            }
            self.next();
            return Ok(inner);
        }
        Ok(Expr::Leaf {
            id: self.word()?,
            with: None,
        })
    }

    fn word(&mut self) -> Result<String, String> {
        match self.next() {
            Some(t)
                if !["(", ")", "/"].contains(&t.as_str())
                    && !["AND", "OR", "WITH"]
                        .iter()
                        .any(|o| t.eq_ignore_ascii_case(o)) =>
            {
                Ok(t)
            }
            Some(t) => Err(format!("`{t}` where a licence was expected")),
            None => Err("the expression ends where a licence was expected".to_string()),
        }
    }
}

fn parse(expression: &str) -> Result<Expr, String> {
    let mut p = Parser {
        tokens: tokens(expression),
        at: 0,
    };
    let e = p.or(0)?;
    match p.peek() {
        None => Ok(e),
        Some(t) => Err(format!("`{t}` after the end of the expression")),
    }
}

/// Whether an expression may ship: `Ok` with the noted licences it relies
/// on (an `OR` takes the alternative with the fewest), or `Err` with what
/// is refused.
fn judge(e: &Expr) -> Result<Vec<String>, String> {
    match e {
        Expr::Leaf { id, with: Some(w) } => {
            if ALLOWED_EXCEPTIONS.contains(&(id.as_str(), w.as_str())) {
                Ok(Vec::new())
            } else {
                Err(format!("{id} WITH {w}"))
            }
        }
        Expr::Leaf { id, with: None } => {
            if ALLOWED.contains(&id.as_str()) {
                Ok(Vec::new())
            } else if NOTED.iter().any(|(n, _)| n == id) {
                Ok(vec![id.clone()])
            } else {
                Err(id.clone())
            }
        }
        Expr::And(parts) => {
            let mut noted = Vec::new();
            for p in parts {
                noted.extend(judge(p)?);
            }
            Ok(noted)
        }
        Expr::Or(parts) => {
            let mut best: Option<Vec<String>> = None;
            let mut refused = Vec::new();
            for p in parts {
                match judge(p) {
                    Ok(n) if best.as_ref().is_none_or(|b| n.len() < b.len()) => best = Some(n),
                    Ok(_) => {}
                    Err(r) => refused.push(r),
                }
            }
            best.ok_or_else(|| refused.join(" and "))
        }
    }
}

/// The licence a crate whose expression offers a choice (`OR`, at any
/// depth) is used under: the alternative [`judge`] takes (the first with
/// the fewest noted licences), written out. `None` without a choice, or
/// when no alternative is allowed.
fn chosen(e: &Expr) -> Option<String> {
    fn has_or(e: &Expr) -> bool {
        match e {
            Expr::Leaf { .. } => false,
            Expr::Or(_) => true,
            Expr::And(parts) => parts.iter().any(has_or),
        }
    }
    fn pick(e: &Expr) -> Option<String> {
        match e {
            Expr::Leaf { id, with: None } => Some(id.clone()),
            Expr::Leaf { id, with: Some(w) } => Some(format!("{id} WITH {w}")),
            Expr::And(parts) => {
                let picked: Option<Vec<String>> = parts
                    .iter()
                    // A choice made inside leaves a licence or an AND,
                    // which alone needs its parentheses.
                    .map(|p| {
                        pick(p).map(|s| {
                            if s.contains(" AND ") {
                                format!("({s})")
                            } else {
                                s
                            }
                        })
                    })
                    .collect();
                picked.map(|p| p.join(" AND "))
            }
            Expr::Or(parts) => {
                let mut best: Option<(usize, &Expr)> = None;
                for p in parts {
                    if let Ok(n) = judge(p)
                        && best.is_none_or(|(b, _)| n.len() < b)
                    {
                        best = Some((n.len(), p));
                    }
                }
                best.and_then(|(_, p)| pick(p))
            }
        }
    }
    if !has_or(e) {
        return None;
    }
    judge(e).ok()?;
    pick(e)
}

/// The licences an expression names, in order.
fn ids(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Leaf { id, .. } => out.push(id.clone()),
        Expr::And(parts) | Expr::Or(parts) => parts.iter().for_each(|p| ids(p, out)),
    }
}

/// Whether a licence is copyleft (strong or weak), for the summary.
fn copyleft(id: &str) -> bool {
    let id = id.to_ascii_uppercase();
    [
        "GPL", "MPL", "EPL", "CDDL", "EUPL", "OSL", "CPL", "MS-RL", "CECILL",
    ]
    .iter()
    .any(|k| id.contains(k))
}

/// The standard texts a crate without its own takes: an `OR` the first
/// alternative with one, an `AND` every part's.
fn standard_ids(e: &Expr) -> Vec<&'static str> {
    match e {
        Expr::Leaf { id, .. } => STANDARD
            .iter()
            .filter(|(s, _)| s == id)
            .map(|(s, _)| *s)
            .collect(),
        Expr::And(parts) => {
            let mut all: Vec<&'static str> = Vec::new();
            for p in parts {
                for s in standard_ids(p) {
                    if !all.contains(&s) {
                        all.push(s);
                    }
                }
            }
            all
        }
        Expr::Or(parts) => parts
            .iter()
            .map(standard_ids)
            .find(|s| !s.is_empty())
            .unwrap_or_default(),
    }
}

/// A crate of the release build, from `cargo metadata`.
#[derive(Debug, Clone)]
struct Crate {
    name: String,
    version: String,
    licence: Option<String>,
    licence_file: Option<String>,
    repository: Option<String>,
    authors: Vec<String>,
    dir: PathBuf,
}

fn metadata(repo: &Path, target: &str) -> Result<Value, String> {
    let output = Command::new(crate::cargo_path())
        .args(["metadata", "--format-version", "1", "--locked"])
        .arg("--manifest-path")
        .arg(repo.join("Cargo.toml"))
        .args(["--filter-platform", target])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(crate::package::failure(
            &format!("cargo metadata --filter-platform {target}"),
            &output.status.to_string(),
            "",
            &String::from_utf8_lossy(&output.stderr),
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("cargo metadata wrote no JSON: {e}"))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// The crates reached from `roots` through normal and build dependencies.
fn reached(m: &Value, roots: &[&str]) -> Result<Vec<Crate>, String> {
    let empty = Vec::new();
    let packages = m
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata lists no packages")?;
    let by_id: BTreeMap<&str, &Value> = packages
        .iter()
        .filter_map(|p| Some((p.get("id")?.as_str()?, p)))
        .collect();
    let members: Vec<&str> = m
        .get("workspace_members")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let nodes: BTreeMap<&str, &Value> = m
        .get("resolve")
        .and_then(|r| r.get("nodes"))
        .and_then(Value::as_array)
        .ok_or("cargo metadata has no dependency graph")?
        .iter()
        .filter_map(|n| Some((n.get("id")?.as_str()?, n)))
        .collect();
    let mut stack: Vec<&str> = members
        .iter()
        .copied()
        .filter(|id| {
            by_id
                .get(id)
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .is_some_and(|n| roots.contains(&n))
        })
        .collect();
    if stack.len() != roots.len() {
        return Err(format!(
            "cargo metadata does not have all of {}",
            roots.join(", ")
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let deps = nodes
            .get(id)
            .and_then(|n| n.get("deps"))
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        for dep in deps {
            let shipped = dep
                .get("dep_kinds")
                .and_then(Value::as_array)
                .unwrap_or(&empty)
                .iter()
                .any(|k| matches!(k.get("kind").and_then(Value::as_str), None | Some("build")));
            if let (true, Some(pkg)) = (shipped, dep.get("pkg").and_then(Value::as_str)) {
                stack.push(pkg);
            }
        }
    }
    let mut out = Vec::new();
    for id in seen {
        if members.contains(&id) {
            continue;
        }
        let p = by_id
            .get(id)
            .ok_or_else(|| format!("cargo metadata has no package {id}"))?;
        let manifest = text(p, "manifest_path").ok_or_else(|| format!("{id} has no manifest"))?;
        let dir = Path::new(&manifest)
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("{id}: a manifest without a folder"))?;
        out.push(Crate {
            name: text(p, "name").ok_or_else(|| format!("{id} has no name"))?,
            version: text(p, "version").ok_or_else(|| format!("{id} has no version"))?,
            licence: text(p, "license"),
            licence_file: text(p, "license_file"),
            repository: text(p, "repository"),
            authors: p
                .get("authors")
                .and_then(Value::as_array)
                .unwrap_or(&empty)
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            dir,
        });
    }
    Ok(out)
}

/// Every crate of the release build, all targets together, sorted by
/// name and version.
fn shipped(repo: &Path) -> Result<Vec<Crate>, String> {
    let mut all: BTreeMap<(String, String), Crate> = BTreeMap::new();
    for (target, roots) in TARGETS {
        for c in reached(&metadata(repo, target)?, roots)? {
            all.entry((c.name.clone(), c.version.clone())).or_insert(c);
        }
    }
    Ok(all.into_values().collect())
}

/// Whether a file or folder at a crate's root holds licence texts.
fn is_licence_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "LICENSE",
        "LICENCE",
        "COPYING",
        "COPYRIGHT",
        "NOTICE",
        "UNLICENSE",
    ]
    .iter()
    .any(|p| upper.starts_with(p))
}

fn read_text(path: &Path) -> Result<String, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .len();
    if len > MAX_TEXT {
        return Err(format!("{} is too large for a licence", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let s = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    Ok(s.trim_end().to_string())
}

/// The licence files of a crate: (name relative to its root, text), sorted.
fn licence_files(c: &Crate) -> Result<Vec<(String, String)>, String> {
    let mut paths: Vec<String> = Vec::new();
    let entries =
        std::fs::read_dir(&c.dir).map_err(|e| format!("cannot list {}: {e}", c.dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot list {}: {e}", c.dir.display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_licence_name(&name) {
            continue;
        }
        let path = entry.path();
        if path.is_file() {
            paths.push(name);
        } else if path.is_dir() {
            // A `LICENSES/` folder: its files, one level.
            let inner = std::fs::read_dir(&path)
                .map_err(|e| format!("cannot list {}: {e}", path.display()))?;
            for f in inner.flatten() {
                if f.path().is_file() {
                    paths.push(format!("{name}/{}", f.file_name().to_string_lossy()));
                }
            }
        }
    }
    if let Some(f) = &c.licence_file {
        paths.push(f.clone());
    }
    for (name, files) in EXTRA_FILES {
        if *name == c.name {
            paths.extend(files.iter().map(|f| (*f).to_string()));
        }
    }
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|p| Ok((p.clone(), read_text(&c.dir.join(&p))?)))
        .collect()
}

/// One crate's notice.
struct Entry {
    krate: Crate,
    /// Its own licence files: (name, text).
    files: Vec<(String, String)>,
    /// The standard texts it takes instead ([`NO_TEXT`]).
    standard: Vec<&'static str>,
    /// The noted licences it relies on.
    noted: Vec<String>,
    /// For a licence with a choice (`OR`), the alternative it is used
    /// under, as [`judge`] chooses it.
    used_under: Option<String>,
}

/// The review of the release build.
pub(crate) struct Review {
    entries: Vec<Entry>,
    /// What breaks the licence policy: empty for a release.
    problems: Vec<String>,
}

fn review(crates: Vec<Crate>) -> Review {
    let mut problems = Vec::new();
    let mut entries = Vec::new();
    for c in crates {
        let label = format!("{} {}", c.name, c.version);
        let expr = match c.licence.as_deref().map(parse) {
            Some(Ok(e)) => Some(e),
            Some(Err(why)) => {
                problems.push(format!(
                    "{label}: cannot read its licence expression: {why}"
                ));
                None
            }
            None => {
                problems.push(format!("{label}: its manifest declares no licence"));
                None
            }
        };
        let noted = match expr.as_ref().map(judge) {
            Some(Ok(n)) => n,
            Some(Err(why)) => {
                problems.push(format!(
                    "{label}: licence `{}` is not allowed ({why})",
                    c.licence.as_deref().unwrap_or_default()
                ));
                Vec::new()
            }
            None => Vec::new(),
        };
        let files = match licence_files(&c) {
            Ok(f) => f,
            Err(why) => {
                problems.push(format!("{label}: {why}"));
                Vec::new()
            }
        };
        let excepted = NO_TEXT.iter().any(|(n, _)| *n == c.name);
        let mut standard = Vec::new();
        if files.is_empty() && !excepted {
            problems.push(format!(
                "{label}: no licence text in {} (if the crate genuinely ships none, \
                 add it to NO_TEXT in xtask/src/notices.rs with the reason)",
                c.dir.display()
            ));
        } else if excepted {
            if files
                .iter()
                .any(|(f, _)| !EXTRA_FILES.iter().any(|(_, x)| x.contains(&f.as_str())))
            {
                problems.push(format!(
                    "{label}: now ships a licence text, remove it from NO_TEXT"
                ));
            }
            standard = expr.as_ref().map(standard_ids).unwrap_or_default();
            if standard.is_empty() {
                problems.push(format!(
                    "{label}: no standard text in packaging/licenses for `{}`",
                    c.licence.as_deref().unwrap_or_default()
                ));
            }
        }
        let used_under = expr.as_ref().and_then(chosen);
        entries.push(Entry {
            krate: c,
            files,
            standard,
            noted,
            used_under,
        });
    }
    for (name, _) in NO_TEXT {
        if !entries.iter().any(|e| e.krate.name == *name) {
            problems.push(format!(
                "{name} is in NO_TEXT but not in the release build: remove it"
            ));
        }
    }
    Review { entries, problems }
}

/// Reviews the release build of the repository at `repo`.
pub(crate) fn collect(repo: &Path) -> Result<Review, String> {
    Ok(review(shipped(repo)?))
}

impl Review {
    /// The crates whose expression names a copyleft licence (taken or
    /// not), with the expression.
    fn copyleft(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|e| {
                let expr = parse(e.krate.licence.as_deref()?).ok()?;
                let mut names = Vec::new();
                ids(&expr, &mut names);
                names.iter().any(|n| copyleft(n)).then(|| {
                    format!(
                        "{} {}: {}",
                        e.krate.name,
                        e.krate.version,
                        e.krate.licence.as_deref().unwrap_or_default()
                    )
                })
            })
            .collect()
    }

    /// The crates relying on a noted licence.
    fn noted(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| !e.noted.is_empty())
            .map(|e| {
                format!(
                    "{} {}: {}",
                    e.krate.name,
                    e.krate.version,
                    e.noted.join(", ")
                )
            })
            .collect()
    }

    /// Crates per licence expression, most first.
    fn counts(&self) -> Vec<(usize, String)> {
        let mut by: BTreeMap<String, usize> = BTreeMap::new();
        for e in &self.entries {
            let key = e.krate.licence.clone().unwrap_or_else(|| "(none)".into());
            *by.entry(key).or_default() += 1;
        }
        let mut v: Vec<(usize, String)> = by.into_iter().map(|(k, n)| (n, k)).collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        v
    }

    /// The summary `cargo xtask notices` prints.
    pub(crate) fn summary(&self) -> String {
        let mut s = format!("notices: {} third-party crates\n", self.entries.len());
        for (n, licence) in self.counts() {
            s.push_str(&format!("  {n:4}  {licence}\n"));
        }
        for line in self.noted() {
            s.push_str(&format!("  noted: {line}\n"));
        }
        for line in self.copyleft() {
            s.push_str(&format!("  names a copyleft licence: {line}\n"));
        }
        let excepted = self.entries.iter().filter(|e| !e.standard.is_empty());
        s.push_str(&format!(
            "  without a licence file of their own (NO_TEXT): {}\n",
            excepted.count()
        ));
        s
    }

    /// The text of `THIRD-PARTY-NOTICES.txt`.
    pub(crate) fn render(&self, version: &str) -> String {
        // Texts by content, numbered in order of first use.
        let mut numbers: BTreeMap<&str, usize> = BTreeMap::new();
        let mut texts: Vec<&str> = Vec::new();
        let mut out = String::new();
        let title = format!("Third-party notices for 3DCraft {version}");
        out.push_str(&format!("{title}\n{}\n\n", "=".repeat(title.len())));
        out.push_str(&format!(
            "3DCraft itself is under the MIT licence (LICENSE, beside this file). Its\n\
             programs (craft3d, craft3d-cli and the web build) are built from the\n\
             {} third-party Rust crates below, each under the licence its manifest\n\
             declares; where a crate offers a choice (OR), it is used under any one of\n\
             the alternatives (\"Used under\" names the one taken). Each entry names its licence texts by number; the texts\n\
             follow the list, each once.\n\n\
             Generated by `cargo xtask notices` from Cargo.lock, with the texts read\n\
             from the crates' published sources, for the targets\n  {}\n\n",
            self.entries.len(),
            TARGETS.map(|(t, _)| t).join("\n  ")
        ));
        out.push_str("Crates\n------\n");
        for e in &self.entries {
            let c = &e.krate;
            out.push_str(&format!("\n{} {}\n", c.name, c.version));
            out.push_str(&format!(
                "  Licence: {}\n",
                c.licence.as_deref().unwrap_or("(none declared)")
            ));
            if let Some(r) = &c.repository {
                out.push_str(&format!("  Repository: {r}\n"));
            }
            let mut refs = Vec::new();
            for (name, body) in &e.files {
                let n = text_number(body, &mut numbers, &mut texts);
                refs.push(format!("{name} [{n}]"));
            }
            if !refs.is_empty() {
                out.push_str(&format!("  Texts: {}\n", refs.join(", ")));
            }
            if !e.standard.is_empty() {
                let std_refs: Vec<String> = e
                    .standard
                    .iter()
                    .filter_map(|id| {
                        let body = STANDARD.iter().find(|(s, _)| s == id)?.1.trim_end();
                        let n = text_number(body, &mut numbers, &mut texts);
                        Some(format!("{id} [{n}]"))
                    })
                    .collect();
                out.push_str(&format!(
                    "  The crate ships no licence file of its own; the standard text applies: {}\n",
                    std_refs.join(", ")
                ));
                let holders = if c.authors.is_empty() {
                    format!("the authors of {}", c.name)
                } else {
                    c.authors.join(", ")
                };
                out.push_str(&format!("  Copyright (c) {holders}\n"));
            }
            if let Some(used) = &e.used_under {
                out.push_str(&format!("  Used under: {used}\n"));
            }
            if !e.noted.is_empty() {
                out.push_str(&format!("  Noted: {}\n", e.noted.join(", ")));
            }
        }
        out.push_str("\nBundled fonts (SIL Open Font Licence 1.1)\n------------------------------------------\n\n");
        out.push_str(
            "The programs carry these font files in fonts/ (the web page fetches\n\
             them from fonts/ beside it), each under the SIL Open Font Licence 1.1\n\
             with its copyright and licence text in the file named beside it:\n\n",
        );
        for font in craft3d_font::shipped() {
            out.push_str(&format!(
                "  fonts/{}  (licence: fonts/{}; from {})\n",
                font.file, font.licence, font.url
            ));
        }
        out.push_str("\nBundled examples (MIT; Jigs and assemblies CC BY 4.0)\n-----------------------------------------------------\n\n");
        out.push_str(
            "The programs, the web page's too, carry built in the examples, Jigs and\n\
             assemblies below, from their own repositories. The examples are under\n\
             the MIT licence of theirs, the Jigs and assemblies not under an MIT\n\
             licence but under CC BY 4.0, as EXAMPLES-LICENSE.md (beside this file)\n\
             says:\n\n",
        );
        out.push_str(EXAMPLES_LICENCE.trim_end());
        out.push('\n');
        out.push_str("\nLicence texts\n-------------\n");
        for (i, body) in texts.iter().enumerate() {
            out.push_str(&format!("\n[{}]\n\n{body}\n", i.saturating_add(1)));
        }
        out
    }
}

/// The number of a text in `texts`, adding it the first time.
fn text_number<'a>(
    body: &'a str,
    numbers: &mut BTreeMap<&'a str, usize>,
    texts: &mut Vec<&'a str>,
) -> usize {
    if let Some(n) = numbers.get(body) {
        return *n;
    }
    texts.push(body);
    let n = texts.len();
    numbers.insert(body, n);
    n
}

/// Writes the notices of the release build to `to`, refusing when the
/// licence policy is broken (a package never carries incomplete notices).
pub(crate) fn write(repo: &Path, to: &Path) -> Result<Review, String> {
    let review = collect(repo)?;
    if !review.problems.is_empty() {
        return Err(format!(
            "the release build breaks the licence policy:\n  {}",
            review.problems.join("\n  ")
        ));
    }
    crate::package::write(to, review.render(env!("CARGO_PKG_VERSION")).as_bytes())?;
    Ok(review)
}

/// `cargo xtask notices [--out FILE]`; by default into the target folder.
pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let repo = crate::package::repo_root();
    let to = match args {
        [] => crate::package::target_dir(&repo).join(FILE_NAME),
        [flag, file] if flag == "--out" => PathBuf::from(file),
        _ => return Err("usage: cargo xtask notices [--out FILE]".to_string()),
    };
    let review = write(&repo, &to)?;
    eprint!("{}", review.summary());
    eprintln!("notices: wrote {}", to.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: &str) -> Expr {
        Expr::Leaf {
            id: id.to_string(),
            with: None,
        }
    }

    #[test]
    fn expressions_are_parsed_with_and_binding_tighter() {
        assert_eq!(parse("MIT").unwrap(), leaf("MIT"));
        assert_eq!(
            parse("MIT OR Apache-2.0 AND Zlib").unwrap(),
            Expr::Or(vec![
                leaf("MIT"),
                Expr::And(vec![leaf("Apache-2.0"), leaf("Zlib")])
            ])
        );
        assert_eq!(
            parse("(MIT OR Apache-2.0) AND Unicode-3.0").unwrap(),
            Expr::And(vec![
                Expr::Or(vec![leaf("MIT"), leaf("Apache-2.0")]),
                leaf("Unicode-3.0")
            ])
        );
        // The old cargo syntax.
        assert_eq!(
            parse("MIT/Apache-2.0").unwrap(),
            parse("MIT OR Apache-2.0").unwrap()
        );
        assert_eq!(
            parse("MIT / Apache-2.0").unwrap(),
            parse("MIT OR Apache-2.0").unwrap()
        );
        assert_eq!(
            parse("Apache-2.0 WITH LLVM-exception").unwrap(),
            Expr::Leaf {
                id: "Apache-2.0".into(),
                with: Some("LLVM-exception".into())
            }
        );
        for junk in [
            "",
            "MIT OR",
            "(MIT",
            "MIT)",
            "AND MIT",
            "MIT WITH",
            "(MIT) WITH X",
        ] {
            assert!(parse(junk).is_err(), "{junk:?} parsed");
        }
        let deep = format!("{}MIT{}", "(".repeat(100), ")".repeat(100));
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn or_needs_one_allowed_alternative_and_needs_all() {
        let ok = |s: &str| judge(&parse(s).unwrap());
        assert_eq!(ok("MIT OR Apache-2.0").unwrap(), Vec::<String>::new());
        assert!(ok("Apache-2.0 OR GPL-2.0-only").is_ok());
        assert!(ok("Unlicense OR MIT").is_ok());
        assert!(ok("Apache-2.0 WITH LLVM-exception OR MIT").is_ok());
        assert!(ok("GPL-3.0-only").is_err());
        assert!(ok("LGPL-2.1-or-later OR AGPL-3.0-only").is_err());
        assert!(ok("MIT AND GPL-2.0-only").is_err());
        assert!(ok("GPL-2.0-only WITH LLVM-exception").is_err());
        assert!(ok("Unlicense").is_err());
        // Noted licences ship, and are named; an OR prefers a plain one.
        assert_eq!(ok("MPL-2.0").unwrap(), vec!["MPL-2.0".to_string()]);
        assert_eq!(ok("MPL-2.0 OR MIT").unwrap(), Vec::<String>::new());
        assert_eq!(
            ok("(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0").unwrap(),
            vec!["Ubuntu-font-1.0".to_string()]
        );
    }

    #[test]
    fn copyleft_is_recognised() {
        for id in [
            "GPL-2.0-only",
            "LGPL-3.0",
            "AGPL-3.0-or-later",
            "MPL-2.0",
            "EPL-2.0",
        ] {
            assert!(copyleft(id), "{id}");
        }
        for id in ["MIT", "Apache-2.0", "OFL-1.1", "BSD-3-Clause"] {
            assert!(!copyleft(id), "{id}");
        }
    }

    #[test]
    fn crates_without_a_text_take_the_standard_one() {
        let s = |e: &str| standard_ids(&parse(e).unwrap());
        assert_eq!(s("MIT OR Apache-2.0"), vec!["MIT"]);
        assert_eq!(s("Zlib OR Apache-2.0 OR MIT"), vec!["Apache-2.0"]);
        assert_eq!(s("BSL-1.0"), vec!["BSL-1.0"]);
        assert_eq!(s("Apache-2.0 AND MIT"), vec!["Apache-2.0", "MIT"]);
        assert!(s("ISC").is_empty());
    }

    #[test]
    fn licence_files_are_recognised_by_name() {
        for n in [
            "LICENSE",
            "LICENSE-MIT",
            "license-apache.md",
            "LICENCE",
            "COPYING",
            "UNLICENSE",
            "NOTICE",
        ] {
            assert!(is_licence_name(n), "{n}");
        }
        for n in ["README.md", "Cargo.toml", "src", "CHANGELOG.md"] {
            assert!(!is_licence_name(n), "{n}");
        }
    }

    #[test]
    fn the_policy_lists_are_well_formed() {
        for list in [
            NO_TEXT.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            EXTRA_FILES.iter().map(|(n, _)| *n).collect(),
        ] {
            let mut sorted = list.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), list.len(), "a crate is listed twice");
        }
        for (_, why) in NO_TEXT.iter().chain(NOTED) {
            assert!(!why.trim().is_empty());
        }
        for (_, body) in STANDARD {
            assert!(body.len() > 500);
        }
    }

    /// The MSI installs the notices beside LICENSE.txt, from the folder
    /// the package stages them in.
    #[test]
    fn the_windows_installer_carries_the_notices() {
        let wxs = std::fs::read_to_string(
            crate::package::repo_root().join("packaging/windows/craft3d.wxs"),
        )
        .unwrap();
        assert!(wxs.contains(&format!("Source=\"$(var.BinDir)\\{FILE_NAME}\"")));
        let examples = crate::package::EXAMPLES_LICENCE;
        assert!(wxs.contains(&format!("Source=\"$(var.BinDir)\\{examples}\"")));
    }

    /// A crate with a choice names the alternative it is used under: the
    /// first allowed one with the fewest noted licences.
    #[test]
    fn the_alternative_an_or_crate_is_used_under_is_named() {
        let used = |s: &str| chosen(&parse(s).unwrap());
        assert_eq!(
            used("Apache-2.0 OR GPL-2.0-only").as_deref(),
            Some("Apache-2.0")
        );
        assert_eq!(used("GPL-2.0-only OR MIT").as_deref(), Some("MIT"));
        assert_eq!(used("MPL-2.0 OR MIT").as_deref(), Some("MIT"));
        assert_eq!(
            used("(MIT OR Apache-2.0) AND Unicode-3.0").as_deref(),
            Some("MIT AND Unicode-3.0")
        );
        assert_eq!(used("MIT"), None);
        assert_eq!(used("MIT AND Apache-2.0"), None);
        assert_eq!(used("GPL-3.0-only OR AGPL-3.0-only"), None);
    }

    /// The release build: every crate has an allowed licence and a text,
    /// and the notices can be written (what `cargo xtask package` does).
    #[test]
    fn every_shipped_crate_has_an_allowed_licence_and_its_text() {
        let review = collect(&crate::package::repo_root()).unwrap();
        eprint!("{}", review.summary());
        assert!(
            review.problems.is_empty(),
            "the release build breaks the licence policy:\n  {}",
            review.problems.join("\n  ")
        );
        assert!(
            review.entries.len() > 50,
            "too few crates: {}",
            review.entries.len()
        );
        let text = review.render("0.0.0");
        for e in &review.entries {
            let head = format!("\n{} {}\n  Licence: ", e.krate.name, e.krate.version);
            assert!(text.contains(&head), "{} is not listed", e.krate.name);
        }
        assert!(text.contains("\nLicence texts\n-------------\n\n[1]\n"));
        // The examples built into the programs are named, with their
        // licence.
        assert!(text.contains("\nBundled examples (MIT; Jigs and assemblies CC BY 4.0)\n"));
        assert!(text.contains("steel-bookend.3dcraft"));
        assert!(text.contains("radial-engine.3dcraftasm"));
        assert!(text.contains("creativecommons.org/licenses/by/4.0"));
        // The font files beside the programs, the interface's Inter too.
        assert!(text.contains("\nBundled fonts (SIL Open Font Licence 1.1)\n"));
        for font in [
            "NotoSans-Regular.otf",
            "JetBrainsMono-Regular.ttf",
            "InterVariable.ttf",
        ] {
            assert!(
                text.contains(&format!("fonts/{font}")),
                "{font} is not named"
            );
        }
        // self_cell (Apache-2.0 OR GPL-2.0-only) says what it is used under.
        if let Some(e) = review.entries.iter().find(|e| e.krate.name == "self_cell") {
            assert_eq!(e.used_under.as_deref(), Some("Apache-2.0"));
        }
        assert!(
            !review
                .entries
                .iter()
                .any(|e| e.krate.name.starts_with("craft3d"))
        );
    }
}
