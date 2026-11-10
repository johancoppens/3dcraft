//! `cargo xtask bundle-examples --from DIR`: the examples, Jigs and
//! assemblies built into 3DCraft for its start screen (docs/decisions.md,
//! 2026-11-10; docs/development.md, Bundled examples), copied from their
//! own repositories, which are their source.
//!
//! [`MANIFEST`] lists them in the start screen's order. Of each entry the
//! task reads `id`, `section` (`parts`, `jigs` or `assemblies`), `start`
//! (one of the few a newcomer opens first), `repository` and `folder`, and
//! writes the rest: from the folder at the head of the repository's `main`
//! in a local clone (`DIR/<repository>`, read with git, so the clone's
//! working tree and its other branches do not matter; no network), the
//! built file unchanged, its commit, its licence, title and one-line
//! summary (its `meta.json`), the numbers its repository recorded when it
//! was built (`stats`: the features, assertions, parts, instances and how
//! long it took to open), its size and SHA-256. Its picture on the start
//! screen is the folder's `preview.png`, made smaller ([`THUMB`]) and cut
//! to [`THUMB_PALETTE`] colours. From the manifest it then writes the
//! licence of the bundled files ([`LICENCE`], which the installers carry
//! as `EXAMPLES-LICENSE.md`), their rows in `ATTRIBUTION.md` and the start
//! screen's list ([`LIST`], included by `crates/ui-egui/src/shell/start.rs`).
//!
//! It refuses, and writes nothing: a licence other than the one the
//! decision allows for the repository (the examples MIT, the Jigs and
//! assemblies CC BY 4.0; so never the Jig shared under CC BY-SA 4.0), in
//! `meta.json`, in the file's own document information or in that of a
//! part packed in an assembly (`blobs/*.3dcraft`, also in a packed
//! subassembly); a file this build of 3DCraft cannot open (a newer format
//! version than it reads); a file of another kind than its section (a
//! part, a Jig, an assembly); a `meta.json` without its id, title, summary
//! or the `stats` the tests compare with; a missing preview; a repository
//! without the `ATTRIBUTION.md` the licence links to; two entries whose
//! files would have the same name; a file larger than the caps below,
//! whose size is asked of git before it is read. Files of the folder that
//! the manifest no longer lists are removed.

use craft3d_doc::Document;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The manifest, relative to the repository.
pub const MANIFEST: &str = "docs/examples/published/bundle.json";
/// Where the bundled files are, relative to the repository.
pub const FOLDER: &str = "docs/examples/published";
/// Their licence, relative to the repository.
pub const LICENCE: &str = "docs/examples/published/LICENSE.md";
/// The start screen's list, relative to the repository (an expression,
/// included by `start.rs`).
pub const LIST: &str = "crates/ui-egui/src/shell/start_bundle.rs";
const ATTRIBUTION: &str = "ATTRIBUTION.md";
/// The size of a thumbnail, 4:3.
pub const THUMB: (u32, u32) = (240, 180);
/// Most colours of a thumbnail.
pub const THUMB_PALETTE: usize = 256;
/// Largest thumbnail accepted, in bytes.
pub const MAX_THUMB_BYTES: u64 = 40 * 1024;
/// Largest bundled file accepted, in bytes: every one is in the web
/// build's module.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Largest `meta.json` or licence text read from a clone, in bytes.
const MAX_TEXT_BYTES: u64 = 1024 * 1024;
/// Largest `preview.png` read from a clone, in bytes (its pixels are
/// capped when it is decoded, `thumbs::MAX_PICTURE_PIXELS`).
const MAX_PREVIEW_BYTES: u64 = 16 * 1024 * 1024;
/// Most assemblies packed in one another that are looked into for the
/// licences of their parts.
const MAX_PACKED_DEPTH: usize = 8;

/// The repositories the bundled files come from: name, address, the
/// licence their bundled files are under (the decision of 2026-11-10), and
/// how a credit names the collection.
pub const REPOSITORIES: [(&str, &str, &str, &str); 3] = [
    (
        "3dcraft-examples",
        "https://github.com/johancoppens/3dcraft-examples",
        "MIT",
        "the 3DCraft examples",
    ),
    (
        "3dcraft-jigs",
        "https://github.com/johancoppens/3dcraft-jigs",
        "CC-BY-4.0",
        "the 3DCraft Jigs",
    ),
    (
        "3dcraft-assemblies-examples",
        "https://github.com/johancoppens/3dcraft-assemblies-examples",
        "CC-BY-4.0",
        "the 3DCraft assemblies",
    ),
];

/// The MIT licence after its copyright line, as the repositories' `LICENSE`
/// files hold it.
const MIT_TERMS: &str =
    "Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.";

/// A section of the start screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Parts,
    Jigs,
    Assemblies,
}

impl Section {
    fn parse(word: &str) -> Result<Self, String> {
        match word {
            "parts" => Ok(Section::Parts),
            "jigs" => Ok(Section::Jigs),
            "assemblies" => Ok(Section::Assemblies),
            other => Err(format!(
                "section `{other}` is not parts, jigs or assemblies"
            )),
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Section::Parts => "parts",
            Section::Jigs => "jigs",
            Section::Assemblies => "assemblies",
        }
    }

    /// The extension of its files.
    pub fn extension(self) -> &'static str {
        match self {
            Section::Parts => "3dcraft",
            Section::Jigs => "3dcraftjig",
            Section::Assemblies => "3dcraftasm",
        }
    }

    fn rust(self) -> &'static str {
        match self {
            Section::Parts => "ExampleSection::Parts",
            Section::Jigs => "ExampleSection::Jigs",
            Section::Assemblies => "ExampleSection::Assemblies",
        }
    }
}

/// What the manifest says of one bundled file.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub section: Section,
    pub start: bool,
    pub repository: String,
    pub folder: String,
    /// What the task wrote; empty or zero before it first ran.
    pub file: String,
    pub commit: String,
    pub licence: String,
    pub title: String,
    pub summary: String,
    pub author: String,
    pub sha256: String,
    pub bytes: u64,
    pub stats: Value,
    pub format_version: Option<u64>,
    pub built_with: String,
}

impl Entry {
    /// Seconds its repository measured it took to open (`stats.load_s`).
    pub fn seconds(&self) -> f64 {
        self.stats
            .get("load_s")
            .and_then(Value::as_f64)
            .filter(|s| s.is_finite() && *s >= 0.0)
            .unwrap_or(0.0)
    }

    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "section": self.section.word(),
            "start": self.start,
            "repository": self.repository,
            "folder": self.folder,
            "file": self.file,
            "commit": self.commit,
            "licence": self.licence,
            "title": self.title,
            "summary": self.summary,
            "author": self.author,
            "bytes": self.bytes,
            "sha256": self.sha256,
            "formatVersion": self.format_version,
            "builtWith": self.built_with,
            "stats": self.stats,
        })
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The entries of a manifest.
pub fn entries(manifest: &Value) -> Result<Vec<Entry>, String> {
    let list = manifest
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("the manifest has no `entries` list")?;
    let mut out: Vec<Entry> = Vec::new();
    for (n, item) in list.iter().enumerate() {
        let id = text(item, "id");
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "entry {n}: `id` must be lower-case letters, digits and dashes"
            ));
        }
        if out.iter().any(|e| e.id == id) {
            return Err(format!("entry {n}: `{id}` is listed twice"));
        }
        let section = Section::parse(&text(item, "section")).map_err(|e| format!("{id}: {e}"))?;
        let repository = text(item, "repository");
        if !REPOSITORIES.iter().any(|(name, ..)| *name == repository) {
            return Err(format!("{id}: no repository `{repository}`"));
        }
        let folder = text(item, "folder");
        // No part starts with `-`, which git could take for an option.
        let safe = !folder.is_empty()
            && !folder.starts_with('/')
            && folder.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && !part.starts_with('-')
                    && part
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            });
        if !safe {
            return Err(format!("{id}: `folder` `{folder}` is not a plain path"));
        }
        out.push(Entry {
            id,
            section,
            start: item.get("start").and_then(Value::as_bool).unwrap_or(false),
            repository,
            folder,
            file: text(item, "file"),
            commit: text(item, "commit"),
            licence: text(item, "licence"),
            title: text(item, "title"),
            summary: text(item, "summary"),
            author: text(item, "author"),
            sha256: text(item, "sha256"),
            bytes: item.get("bytes").and_then(Value::as_u64).unwrap_or(0),
            stats: item.get("stats").cloned().unwrap_or(Value::Null),
            format_version: item.get("formatVersion").and_then(Value::as_u64),
            built_with: text(item, "builtWith"),
        });
    }
    Ok(out)
}

/// What the manifest says of a repository: its commit and the copyright
/// line of its licence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub commit: String,
    pub copyright: String,
}

/// The repositories of a manifest, in [`REPOSITORIES`]' order, those it
/// uses only.
#[cfg(test)]
pub fn sources(manifest: &Value, entries: &[Entry]) -> Vec<(&'static str, Source)> {
    REPOSITORIES
        .iter()
        .filter(|(name, ..)| entries.iter().any(|e| e.repository == *name))
        .map(|(name, ..)| {
            let at = manifest.get("repositories").and_then(|r| r.get(*name));
            let get = |key: &str| at.map(|v| text(v, key)).unwrap_or_default();
            (
                *name,
                Source {
                    commit: get("commit"),
                    copyright: get("copyright"),
                },
            )
        })
        .collect()
}

fn repository(
    name: &str,
) -> Option<&'static (&'static str, &'static str, &'static str, &'static str)> {
    REPOSITORIES.iter().find(|(n, ..)| *n == name)
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs git in `clone` and returns what it wrote.
fn git(clone: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`git -C {} {}` failed: {}",
            clone.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

/// The commit at the head of `main` in `clone`.
fn head(clone: &Path) -> Result<String, String> {
    let out = git(clone, &["rev-parse", "--verify", "main^{commit}"])?;
    let commit = String::from_utf8_lossy(&out).trim().to_string();
    if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "{}: `main` is `{commit}`, not a commit",
            clone.display()
        ));
    }
    Ok(commit)
}

/// The size of `path` at `commit`, in bytes, as git says it without
/// reading the file.
fn blob_size(clone: &Path, commit: &str, path: &str) -> Result<u64, String> {
    let out = git(clone, &["cat-file", "-s", &format!("{commit}:{path}")])?;
    let text = String::from_utf8_lossy(&out);
    text.trim()
        .parse::<u64>()
        .map_err(|_| format!("git gives the size of {path} as `{}`", text.trim()))
}

/// The bytes of `path` at `commit`, when it holds at most `max` of them:
/// its size is asked first, so a larger file is never read.
fn blob(clone: &Path, commit: &str, path: &str, max: u64) -> Result<Vec<u8>, String> {
    let size = blob_size(clone, commit, path)?;
    if size > max {
        return Err(format!(
            "{path} at {commit} is {size} bytes, more than {max}"
        ));
    }
    git(clone, &["cat-file", "blob", &format!("{commit}:{path}")])
}

/// Whether `path` is a file at `commit`.
fn exists(clone: &Path, commit: &str, path: &str) -> Result<bool, String> {
    let out = git(clone, &["ls-tree", "--name-only", commit, "--", path])?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .any(|line| line == path))
}

/// The names of the files directly in `folder` at `commit`.
fn names(clone: &Path, commit: &str, folder: &str) -> Result<Vec<String>, String> {
    let out = git(
        clone,
        &[
            "ls-tree",
            "--name-only",
            commit,
            "--",
            &format!("{folder}/"),
        ],
    )?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| line.strip_prefix(&format!("{folder}/")))
        .map(str::to_string)
        .collect())
}

/// The copyright line of an MIT `LICENSE` whose terms are [`MIT_TERMS`]
/// (an MIT licence may say more before the terms, as the Jigs' says what
/// it covers), else why not.
pub fn mit_copyright(licence: &str) -> Result<String, String> {
    let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    if !words(licence).ends_with(&words(MIT_TERMS)) || !licence.starts_with("MIT License") {
        return Err("its LICENSE is not the MIT licence".to_string());
    }
    copyright_line(licence).ok_or_else(|| "its LICENSE has no copyright line".to_string())
}

/// The first copyright line of a licence text.
fn copyright_line(licence: &str) -> Option<String> {
    licence
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("Copyright (c) "))
        .map(str::to_string)
}

/// The copyright line of a repository's `LICENSE-CC-BY-4.0.md`: the
/// licensor's notice that CC BY 4.0 (section 3(a)(1)(A)(ii)) asks a copy
/// to keep. Else why not.
pub fn cc_by_copyright(licence: &str) -> Result<String, String> {
    let words = licence.split_whitespace().collect::<Vec<_>>().join(" ");
    if !words.contains("Creative Commons Attribution 4.0 International") {
        return Err("its LICENSE-CC-BY-4.0.md does not name CC BY 4.0".to_string());
    }
    copyright_line(licence)
        .ok_or_else(|| "its LICENSE-CC-BY-4.0.md has no copyright line".to_string())
}

/// The copyright line of a repository at `commit` for a section under
/// `licence`: from its MIT `LICENSE` for the examples; from its
/// `LICENSE-CC-BY-4.0.md` for CC BY 4.0 (from its `LICENSE` when it has
/// no such file).
fn source_copyright(clone: &Path, commit: &str, licence: &str) -> Result<String, String> {
    let read = |path: &str| {
        String::from_utf8(blob(clone, commit, path, MAX_TEXT_BYTES)?)
            .map_err(|_| format!("its {path} is not UTF-8"))
    };
    if licence == "MIT" {
        return mit_copyright(&read("LICENSE")?);
    }
    if exists(clone, commit, "LICENSE-CC-BY-4.0.md")? {
        return cc_by_copyright(&read("LICENSE-CC-BY-4.0.md")?);
    }
    copyright_line(&read("LICENSE")?).ok_or_else(|| "its LICENSE has no copyright line".to_string())
}

/// Who a copyright line names: `Copyright (c) 2026 A B` is `A B`.
pub fn holder(copyright: &str) -> &str {
    copyright
        .trim_start_matches("Copyright (c) ")
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == ' ' || c == ',' || c == '-')
}

/// `width` source pixels onto `to`: per output pixel, the source pixels
/// it covers and how much of each (an area-weighted box).
fn spans(width: usize, to: usize) -> Vec<Vec<(usize, f64)>> {
    let scale = width as f64 / to.max(1) as f64;
    (0..to)
        .map(|x| {
            let (a, b) = (x as f64 * scale, (x + 1) as f64 * scale);
            let (first, last) = (a.floor() as usize, (b.ceil() as usize).min(width));
            (first..last)
                .map(|i| {
                    let (lo, hi) = (i as f64, (i + 1) as f64);
                    (i, (hi.min(b) - lo.max(a)).max(0.0))
                })
                .filter(|(_, w)| *w > 0.0)
                .collect()
        })
        .collect()
}

/// An RGB picture of `width` × `height` made `to_w` × `to_h` by averaging
/// the pixels each covers (a box filter weighted by area).
pub fn shrink(
    width: u32,
    height: u32,
    rgb: &[u8],
    to_w: u32,
    to_h: u32,
) -> Result<Vec<u8>, String> {
    let size = |v: u32| usize::try_from(v).map_err(|e| e.to_string());
    let (w, h, tw, th) = (size(width)?, size(height)?, size(to_w)?, size(to_h)?);
    if w == 0 || h == 0 || tw == 0 || th == 0 || tw > w || th > h {
        return Err(format!(
            "cannot make {width} × {height} into {to_w} × {to_h}"
        ));
    }
    if Some(rgb.len()) != w.checked_mul(h).and_then(|n| n.checked_mul(3)) {
        return Err("the pixels are not width × height × 3".to_string());
    }
    let across = spans(w, tw);
    let down = spans(h, th);
    // Across each row first, then down each column.
    let mut rows = vec![0.0_f64; tw * h * 3];
    for (y, row) in rgb.chunks_exact(w * 3).enumerate() {
        for (x, span) in across.iter().enumerate() {
            let total: f64 = span.iter().map(|(_, wt)| wt).sum();
            for c in 0..3 {
                let sum: f64 = span
                    .iter()
                    .map(|(i, wt)| f64::from(*row.get(i * 3 + c).unwrap_or(&0)) * wt)
                    .sum();
                if let Some(out) = rows.get_mut((y * tw + x) * 3 + c) {
                    *out = sum / total.max(f64::MIN_POSITIVE);
                }
            }
        }
    }
    let mut out = vec![0_u8; tw * th * 3];
    for (y, span) in down.iter().enumerate() {
        let total: f64 = span.iter().map(|(_, wt)| wt).sum();
        for x in 0..tw {
            for c in 0..3 {
                let sum: f64 = span
                    .iter()
                    .map(|(j, wt)| rows.get((j * tw + x) * 3 + c).unwrap_or(&0.0) * wt)
                    .sum();
                let value = (sum / total.max(f64::MIN_POSITIVE))
                    .round()
                    .clamp(0.0, 255.0);
                if let Some(px) = out.get_mut((y * tw + x) * 3 + c) {
                    *px = value as u8;
                }
            }
        }
    }
    Ok(out)
}

/// The thumbnail of a preview picture: [`THUMB`], in [`THUMB_PALETTE`]
/// colours, as an indexed PNG.
pub fn thumbnail(preview: &[u8]) -> Result<Vec<u8>, String> {
    let (width, height, rgb) = crate::thumbs::read_rgb(preview)?;
    // 4:3, as the cards are.
    if u64::from(width) * 3 != u64::from(height) * 4 {
        return Err(format!("the preview is {width} × {height}, not 4:3"));
    }
    let small = shrink(width, height, &rgb, THUMB.0, THUMB.1)?;
    let (palette, indices) = crate::thumbs::quantize(&small, THUMB_PALETTE);
    crate::thumbs::write_indexed(THUMB.0, THUMB.1, &palette, &indices)
}

/// A bundled file read from its repository, ready to be written.
struct Copied {
    entry: Entry,
    bytes: Vec<u8>,
    thumb: Vec<u8>,
}

/// What `meta.json` says of a bundled file, checked.
#[derive(Debug, PartialEq)]
pub struct Meta {
    pub licence: String,
    pub title: String,
    pub summary: String,
    pub stats: Value,
}

/// The numbers of `stats` the tests compare a bundled file with
/// (`crates/ui-egui/tests/bundled_examples.rs`), for every section and
/// per section.
const STATS_ALL: [&str; 2] = ["assertions", "warnings"];
const STATS_PART: [&str; 1] = ["features"];
const STATS_ASSEMBLY: [&str; 3] = ["instances", "mates", "dof"];

/// Checks the `meta.json` of `entry` (as `meta`): its id, its licence
/// (`licence`, the one bundled from its repository), a title and summary
/// of one line each, and its `stats`, the numbers its repository recorded
/// when it built the file: how long it took to open and the counts the
/// tests compare with.
pub fn check_meta(entry: &Entry, meta: &Value, licence: &str) -> Result<Meta, String> {
    let (id, folder) = (&entry.id, &entry.folder);
    if text(meta, "id") != *id {
        return Err(format!(
            "{id}: {folder}/meta.json has the id `{}`",
            text(meta, "id")
        ));
    }
    let meta_licence = text(meta, "licence");
    if meta_licence != licence {
        return Err(format!(
            "{id}: {folder}/meta.json says the licence is `{meta_licence}`; from {} only {licence} is bundled",
            entry.repository
        ));
    }
    let (title, summary) = (text(meta, "title"), text(meta, "summary"));
    for (what, value) in [("title", &title), ("summary", &summary)] {
        if value.trim().is_empty() || value.contains('\n') || value.chars().count() > 120 {
            return Err(format!(
                "{id}: {folder}/meta.json's {what} must be one line of at most 120 characters"
            ));
        }
    }
    let stats = meta
        .get("stats")
        .filter(|stats| stats.is_object())
        .ok_or_else(|| {
            format!("{id}: {folder}/meta.json has no `stats`, which the tests compare with")
        })?;
    if !stats
        .get("load_s")
        .and_then(Value::as_f64)
        .is_some_and(|s| s.is_finite() && s >= 0.0)
    {
        return Err(format!(
            "{id}: {folder}/meta.json's stats have no `load_s` (seconds to open)"
        ));
    }
    let per_section: &[&str] = match entry.section {
        Section::Parts | Section::Jigs => &STATS_PART,
        Section::Assemblies => &STATS_ASSEMBLY,
    };
    for key in STATS_ALL.iter().chain(per_section) {
        if stats.get(*key).and_then(Value::as_u64).is_none() {
            return Err(format!(
                "{id}: {folder}/meta.json's stats have no count `{key}`"
            ));
        }
    }
    Ok(Meta {
        licence: meta_licence,
        title,
        summary,
        stats: stats.clone(),
    })
}

/// Checks that every part and subassembly packed in `document` (an
/// assembly's `blobs/*.3dcraft`, and theirs in turn) is under `licence`
/// in its own document information, naming the first that is not (its
/// title, the instances that use it, its blob).
pub fn check_packed_licences(document: &Document, licence: &str) -> Result<(), String> {
    packed_licences(document, licence, 0)
}

fn packed_licences(document: &Document, licence: &str, depth: usize) -> Result<(), String> {
    let Some(assembly) = &document.assembly else {
        return Ok(());
    };
    if depth >= MAX_PACKED_DEPTH {
        return Err(format!(
            "its assemblies are packed in one another more than {MAX_PACKED_DEPTH} deep"
        ));
    }
    for blob in &assembly.packed {
        let hash = blob.sha256();
        let short = hash.get(..12).unwrap_or(&hash);
        let users: Vec<&str> = assembly
            .instances
            .iter()
            .filter(|i| matches!(&i.source, craft3d_doc::Source::Packed { hash: h } if *h == hash))
            .map(|i| i.name.as_str())
            .collect();
        let part = craft3d_format::from_bytes(blob.bytes())
            .map_err(|e| format!("the part packed as blobs/{short}… cannot be read: {e}"))?;
        if part.info.licence != licence {
            let title = if part.info.title.is_empty() {
                "untitled".to_string()
            } else {
                format!("\"{}\"", part.info.title)
            };
            let used = if users.is_empty() {
                String::new()
            } else {
                format!(", used by {}", users.join(", "))
            };
            return Err(format!(
                "the part {title} packed in it (blobs/{short}…{used}) says the licence is `{}`, not {licence}",
                part.info.licence
            ));
        }
        packed_licences(&part, licence, depth + 1)?;
    }
    Ok(())
}

/// Two entries whose files would have the same name in the folder (one
/// would overwrite the other; told apart without case, as a file system
/// may).
pub fn check_unique_files(entries: &[Entry]) -> Result<(), String> {
    for (n, e) in entries.iter().enumerate() {
        if let Some(other) = entries
            .iter()
            .skip(n + 1)
            .find(|o| o.file.eq_ignore_ascii_case(&e.file))
        {
            return Err(format!(
                "{} ({}) and {} ({}) would both be bundled as `{}`",
                e.id, e.repository, other.id, other.repository, e.file
            ));
        }
    }
    Ok(())
}

/// Reads `entry` from the clone at `commit` and checks it.
fn copy(entry: &Entry, clone: &Path, commit: &str) -> Result<Copied, String> {
    let id = &entry.id;
    let (_, _, licence, _) =
        repository(&entry.repository).ok_or_else(|| format!("{id}: no repository"))?;
    let folder = &entry.folder;
    let meta_bytes = blob(
        clone,
        commit,
        &format!("{folder}/meta.json"),
        MAX_TEXT_BYTES,
    )
    .map_err(|e| format!("{id}: {e}"))?;
    let meta: Value = serde_json::from_slice(&meta_bytes)
        .map_err(|e| format!("{id}: {folder}/meta.json is not JSON: {e}"))?;
    let Meta {
        licence: meta_licence,
        title,
        summary,
        stats,
    } = check_meta(entry, &meta, licence)?;
    let extension = entry.section.extension();
    let built: Vec<String> = names(clone, commit, folder)?
        .into_iter()
        .filter(|name| {
            name.rsplit_once('.')
                .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case(extension))
        })
        .collect();
    let [file] = built.as_slice() else {
        return Err(format!(
            "{id}: {folder} has {} .{extension} files at {commit}, not one: {built:?}",
            built.len()
        ));
    };
    let bytes = blob(clone, commit, &format!("{folder}/{file}"), MAX_FILE_BYTES)
        .map_err(|e| format!("{id}: {e}"))?;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let (document, _) = craft3d_format::from_bytes_noted(&bytes)
        .map_err(|e| format!("{id}: this build of 3DCraft cannot open {file}: {e}"))?;
    let kind_ok = match entry.section {
        Section::Parts => !document.is_assembly(),
        Section::Jigs => !document.is_assembly() && document.is_jig(),
        Section::Assemblies => document.is_assembly(),
    };
    if !kind_ok {
        return Err(format!(
            "{id}: {file} does not hold what the section {} holds",
            entry.section.word()
        ));
    }
    if document.info.licence != *licence {
        return Err(format!(
            "{id}: the document information of {file} says the licence is `{}`, not {licence}",
            document.info.licence
        ));
    }
    check_packed_licences(&document, licence).map_err(|e| format!("{id}: {file}: {e}"))?;
    let preview = blob(
        clone,
        commit,
        &format!("{folder}/preview.png"),
        MAX_PREVIEW_BYTES,
    )
    .map_err(|e| format!("{id}: {e}"))?;
    let thumb = thumbnail(&preview).map_err(|e| format!("{id}: {folder}/preview.png: {e}"))?;
    let thumb_size = u64::try_from(thumb.len()).unwrap_or(u64::MAX);
    if thumb_size > MAX_THUMB_BYTES {
        return Err(format!(
            "{id}: its thumbnail is {thumb_size} bytes, more than {MAX_THUMB_BYTES}"
        ));
    }
    let craft3d = meta.get("craft3d");
    let mut made = entry.clone();
    made.file = file.clone();
    made.commit = commit.to_string();
    made.licence = meta_licence;
    made.title = title;
    made.summary = summary;
    made.author = document.info.author.clone();
    made.sha256 = hex(&craft3d_doc::sha256(&bytes));
    made.bytes = size;
    made.stats = stats;
    made.format_version = craft3d
        .and_then(|c| c.get("format_version"))
        .and_then(Value::as_u64);
    made.built_with = craft3d.map(|c| text(c, "commit")).unwrap_or_default();
    Ok(Copied {
        entry: made,
        bytes,
        thumb,
    })
}

/// The manifest's text for `entries` from `sources`, after `about`.
pub fn manifest_text(about: &Value, sources: &[(&str, Source)], entries: &[Entry]) -> String {
    let mut repositories = serde_json::Map::new();
    for (name, source) in sources {
        let url = repository(name).map(|r| r.1).unwrap_or_default();
        repositories.insert(
            (*name).to_string(),
            json!({ "url": url, "commit": source.commit, "copyright": source.copyright }),
        );
    }
    let value = json!({
        "about": about,
        "repositories": repositories,
        "entries": entries.iter().map(Entry::json).collect::<Vec<_>>(),
    });
    let mut out = serde_json::to_string_pretty(&value).unwrap_or_default();
    out.push('\n');
    out
}

/// A link to `folder` of `repo` at `commit`.
fn tree(repo: &str, commit: &str, folder: &str) -> String {
    let url = repository(repo).map(|r| r.1).unwrap_or_default();
    format!("[{folder}]({url}/tree/{commit}/{folder})")
}

/// The address of a repository's `ATTRIBUTION.md` at `commit`, which
/// credits the data and designs from elsewhere that its files use.
fn attribution_link(url: &str, commit: &str) -> String {
    format!("{url}/blob/{commit}/ATTRIBUTION.md")
}

/// `LICENSE.md` of the bundled files.
pub fn licence_text(sources: &[(&str, Source)], entries: &[Entry]) -> String {
    let mut out = String::from(
        "# Licence of the bundled examples, Jigs and assemblies\n\n\
         The files in this folder are built into 3DCraft and opened from its start screen \
         (docs/decisions.md,\n2026-11-10). They are copied unchanged from three repositories, \
         which are their source; they are not edited\nhere. `bundle.json` lists each with its \
         repository, folder, commit and licence, and\n`cargo xtask bundle-examples` copies them \
         again (docs/development.md, Bundled examples). The pictures in\n`thumbs/` are the \
         `preview.png` of each file's folder, made smaller, under the same licence as the file.\n\
         This file is written by that task: do not edit it by hand.\n",
    );
    for (name, url, licence, collection) in REPOSITORIES {
        let listed: Vec<&Entry> = entries.iter().filter(|e| e.repository == name).collect();
        let Some((_, source)) = sources.iter().find(|(n, _)| *n == name) else {
            continue;
        };
        if listed.is_empty() {
            continue;
        }
        let heading = if licence == "MIT" { "MIT" } else { "CC BY 4.0" };
        out.push_str(&format!(
            "\n## From {collection} ({heading})\n\n[johancoppens/{name}]({url}) at commit `{}`:\n\n\
             | File | Title | Author (`document.info.author`) | Folder |\n|---|---|---|---|\n",
            source.commit
        ));
        for e in &listed {
            out.push_str(&format!(
                "| `{}` | {} | {} | {} |\n",
                e.file,
                e.title.replace('|', "\\|"),
                e.author.replace('|', "\\|"),
                tree(name, &e.commit, &e.folder)
            ));
        }
        if licence == "MIT" {
            out.push_str(&format!(
                "\nEach file's `meta.json` and its own document information say `MIT` (checked when it is \
                 copied). They are\nlicensed under that repository's MIT licence:\n\n```text\nMIT License\n\n{}\n\n{MIT_TERMS}\n```\n",
                source.copyright
            ));
        } else {
            let holder = holder(&source.copyright);
            let example = listed.first().map(|e| e.title.as_str()).unwrap_or_default();
            // Only the Jigs' repository shares one under CC BY-SA 4.0;
            // only an assembly packs parts.
            let mut checked = String::new();
            if listed.iter().any(|e| e.section == Section::Jigs) {
                checked.push_str("\nNone is a Jig shared under CC BY-SA 4.0.");
            }
            if listed.iter().any(|e| e.section == Section::Assemblies) {
                checked.push_str(
                    "\nSo does the document information of every part packed in an assembly \
                     (`blobs/*.3dcraft`, checked the same way).",
                );
            }
            out.push_str(&format!(
                "\nEach file's `meta.json` and its own document information say `CC-BY-4.0` (checked when it \
                 is copied).{checked}\n\nThe licensor's copyright notice, which the licence asks a copy to keep:\n\n\
                 > {}\n\n\
                 Each file is licensed under the **Creative Commons Attribution 4.0 International** licence \
                 (CC BY 4.0):\n<https://creativecommons.org/licenses/by/4.0/>, legal code at\n\
                 <https://creativecommons.org/licenses/by/4.0/legalcode>. You may share and adapt them for any \
                 purpose,\nalso commercially, provided you give appropriate credit, link to the licence and \
                 indicate if changes were\nmade. The repository asks to be credited with the file's title, \
                 as in:\n\n> \"{example}\" by {holder}, from {collection} (CC BY 4.0), {url}\n\n\
                 No changes were made to these copies.\n",
                source.copyright
            ));
        }
        out.push_str(&format!(
            "\nData and designs from elsewhere that these files use are credited in that repository's \
             `ATTRIBUTION.md` at\nthat commit: <{}>.\n",
            attribution_link(url, &source.commit)
        ));
    }
    out
}

/// The rows of `ATTRIBUTION.md` for the bundled files, one per repository.
pub fn attribution_rows(sources: &[(&str, Source)], entries: &[Entry]) -> Vec<String> {
    let mut rows = Vec::new();
    for (name, url, licence, collection) in REPOSITORIES {
        let listed: Vec<&Entry> = entries.iter().filter(|e| e.repository == name).collect();
        let Some((_, source)) = sources.iter().find(|(n, _)| *n == name) else {
            continue;
        };
        if listed.is_empty() {
            continue;
        }
        let files: Vec<String> = listed.iter().map(|e| format!("`{}`", e.file)).collect();
        let titles: Vec<&str> = listed.iter().map(|e| e.title.as_str()).collect();
        let licence = if licence == "MIT" { "MIT" } else { "CC BY 4.0" };
        rows.push(format!(
            "| `{FOLDER}/`: {}; their pictures in `thumbs/` (built into the program as start-screen examples) | {} | {}, {collection} | [johancoppens/{name}]({url}) at commit `{}`; each file's folder and licence in `{LICENCE}`; data and designs from elsewhere that they use are credited in that repository's [`ATTRIBUTION.md`]({}) at that commit | {licence} |",
            files.join(", "),
            titles.join(", ").replace('|', "\\|"),
            holder(&source.copyright),
            source.commit,
            attribution_link(url, &source.commit)
        ));
    }
    rows
}

/// The rows of `ATTRIBUTION.md` that are the bundled files'.
fn is_bundled_row(line: &str) -> bool {
    line.starts_with(&format!("| `{FOLDER}/"))
}

/// `ATTRIBUTION.md` with the bundled files' rows replaced by `rows`, where
/// the first of the old ones was (else at the end of the table). An error
/// when it has no table: the rows would be dropped.
pub fn with_rows(attribution: &str, rows: &[String]) -> Result<String, String> {
    let mut out: Vec<String> = Vec::new();
    let mut placed = false;
    let lines: Vec<&str> = attribution.lines().collect();
    let last_row = lines
        .iter()
        .rposition(|l| l.starts_with('|'))
        .ok_or_else(|| format!("{ATTRIBUTION} has no table for the bundled files' rows"))?;
    for (i, line) in lines.iter().enumerate() {
        if is_bundled_row(line) {
            if !placed {
                out.extend(rows.iter().cloned());
                placed = true;
            }
            continue;
        }
        out.push((*line).to_string());
        if !placed && i == last_row {
            out.extend(rows.iter().cloned());
            placed = true;
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    Ok(text)
}

/// The start screen's list: an expression of an array of `bundled!`.
pub fn list_text(entries: &[Entry]) -> String {
    let mut out = String::from(
        "// The start screen's examples, as docs/examples/published/bundle.json lists\n\
         // them: written by `cargo xtask bundle-examples`, do not edit by hand.\n[\n",
    );
    for e in entries {
        out.push_str(&format!(
            "    bundled!(\n        {},\n        {:?},\n        {:?},\n        {:?},\n        {:?},\n        {:?},\n        {}\n    ),\n",
            e.section.rust(),
            e.id,
            e.file,
            e.title,
            e.summary,
            e.seconds(),
            e.start
        ));
    }
    out.push_str("]\n");
    out
}

/// What the command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    from: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--from" => {
                o.from = Some(PathBuf::from(it.next().ok_or("--from needs a folder")?));
            }
            other => {
                return Err(format!(
                    "bundle-examples: unknown argument `{other}`; usage: cargo xtask bundle-examples --from DIR \
                     (DIR holds the clones 3dcraft-examples, 3dcraft-jigs and 3dcraft-assemblies-examples)"
                ));
            }
        }
    }
    Ok(o)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Copies every listed file again from the clones in `--from` (else
/// `CRAFT3D_EXAMPLE_REPOS`), then writes the manifest, the licence, the
/// rows of `ATTRIBUTION.md` and the start screen's list: all or nothing.
pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let from = options
        .from
        .or_else(|| std::env::var_os("CRAFT3D_EXAMPLE_REPOS").map(PathBuf::from))
        .ok_or(
            "bundle-examples: give --from DIR, the folder that holds the clones 3dcraft-examples, \
             3dcraft-jigs and 3dcraft-assemblies-examples (or set CRAFT3D_EXAMPLE_REPOS)",
        )?;
    let repo = root();
    let manifest_path = repo.join(MANIFEST);
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("cannot read {}: {e}", manifest_path.display()))?,
    )
    .map_err(|e| format!("{MANIFEST} is not JSON: {e}"))?;
    let listed = entries(&manifest)?;
    // Each repository's head and the copyright of its licence.
    let mut sources: Vec<(&'static str, Source)> = Vec::new();
    for (name, _, licence, _) in REPOSITORIES {
        if !listed.iter().any(|e| e.repository == name) {
            continue;
        }
        let clone = from.join(name);
        let commit = head(&clone)?;
        let copyright =
            source_copyright(&clone, &commit, licence).map_err(|e| format!("{name}: {e}"))?;
        // The licence links to it for the credits of what is from
        // elsewhere.
        if !exists(&clone, &commit, "ATTRIBUTION.md")? {
            return Err(format!(
                "{name}: no ATTRIBUTION.md at {commit}, which the licence of its files links to"
            ));
        }
        eprintln!("bundle-examples: {name} at {commit}");
        sources.push((name, Source { commit, copyright }));
    }
    let mut copied = Vec::new();
    let mut failed = Vec::new();
    for entry in &listed {
        let commit = sources
            .iter()
            .find(|(n, _)| *n == entry.repository)
            .map(|(_, s)| s.commit.clone())
            .unwrap_or_default();
        match copy(entry, &from.join(&entry.repository), &commit) {
            Ok(c) => {
                eprintln!(
                    "{}: {} ({} bytes, thumbnail {} bytes)",
                    c.entry.id,
                    c.entry.file,
                    c.bytes.len(),
                    c.thumb.len()
                );
                copied.push(c);
            }
            Err(e) => failed.push(e),
        }
    }
    if !failed.is_empty() {
        return Err(format!(
            "bundle-examples: nothing was written:\n  {}",
            failed.join("\n  ")
        ));
    }
    let made: Vec<Entry> = copied.iter().map(|c| c.entry.clone()).collect();
    check_unique_files(&made).map_err(|e| format!("bundle-examples: nothing was written: {e}"))?;
    let attribution_path = repo.join(ATTRIBUTION);
    let attribution = std::fs::read_to_string(&attribution_path)
        .map_err(|e| format!("cannot read {ATTRIBUTION}: {e}"))?;
    let attribution = with_rows(&attribution, &attribution_rows(&sources, &made))
        .map_err(|e| format!("bundle-examples: nothing was written: {e}"))?;
    let folder = repo.join(FOLDER);
    // What the manifest no longer lists goes.
    let kept: Vec<String> = made.iter().map(|e| e.file.clone()).collect();
    for dir_entry in std::fs::read_dir(&folder)
        .map_err(|e| format!("cannot read {}: {e}", folder.display()))?
        .flatten()
    {
        let name = dir_entry.file_name().to_string_lossy().to_string();
        let bundled = [".3dcraft", ".3dcraftjig", ".3dcraftasm"]
            .iter()
            .any(|ext| name.ends_with(ext));
        if bundled && !kept.contains(&name) {
            std::fs::remove_file(dir_entry.path())
                .map_err(|e| format!("cannot remove {name}: {e}"))?;
        }
    }
    let thumbs = folder.join("thumbs");
    if thumbs.is_dir() {
        for dir_entry in std::fs::read_dir(&thumbs)
            .map_err(|e| format!("cannot read {}: {e}", thumbs.display()))?
            .flatten()
        {
            let name = dir_entry.file_name().to_string_lossy().to_string();
            if !made.iter().any(|e| format!("{}.png", e.id) == name) {
                std::fs::remove_file(dir_entry.path())
                    .map_err(|e| format!("cannot remove thumbs/{name}: {e}"))?;
            }
        }
    }
    for c in &copied {
        write(&folder.join(&c.entry.file), &c.bytes)?;
        write(&thumbs.join(format!("{}.png", c.entry.id)), &c.thumb)?;
    }
    let about = manifest.get("about").cloned().unwrap_or(Value::Null);
    write(
        &manifest_path,
        manifest_text(&about, &sources, &made).as_bytes(),
    )?;
    write(
        &repo.join(LICENCE),
        licence_text(&sources, &made).as_bytes(),
    )?;
    write(&attribution_path, attribution.as_bytes())?;
    write(&repo.join(LIST), list_text(&made).as_bytes())?;
    let total: u64 = made.iter().map(|e| e.bytes).sum();
    eprintln!(
        "bundle-examples: {} files, {total} bytes; wrote {MANIFEST}, {LICENCE}, {ATTRIBUTION} and {LIST}",
        made.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file of the repository as text, its line ends as LF (a checkout
    /// on Windows may have made them CRLF).
    fn read(path: &str) -> String {
        std::fs::read_to_string(root().join(path))
            .unwrap()
            .replace("\r\n", "\n")
    }

    fn manifest() -> Value {
        serde_json::from_str(&std::fs::read_to_string(root().join(MANIFEST)).unwrap()).unwrap()
    }

    /// The bundled files are the ones the manifest names, as they were
    /// copied (size and SHA-256), with their thumbnails; the licence, the
    /// rows of ATTRIBUTION.md and the start screen's list are what the
    /// manifest makes, so a refresh left nothing behind.
    #[test]
    fn the_bundle_is_what_its_manifest_says() {
        let manifest = manifest();
        let listed = entries(&manifest).unwrap();
        assert_eq!(listed.len(), 20, "the set of 2026-11-10");
        let sources = sources(&manifest, &listed);
        assert_eq!(sources.len(), 3);
        for (name, source) in &sources {
            assert_eq!(source.commit.len(), 40, "{name}");
            assert!(source.copyright.starts_with("Copyright (c) "), "{name}");
        }
        let folder = root().join(FOLDER);
        for e in &listed {
            let (_, _, licence, _) = repository(&e.repository).unwrap();
            assert_eq!(e.licence, *licence, "{}", e.id);
            assert!(
                e.file.ends_with(&format!(".{}", e.section.extension())),
                "{}",
                e.id
            );
            assert_eq!(
                Some(&e.commit),
                sources
                    .iter()
                    .find(|(n, _)| *n == e.repository)
                    .map(|(_, s)| &s.commit),
                "{}",
                e.id
            );
            assert!(!e.title.is_empty() && !e.summary.is_empty(), "{}", e.id);
            let bytes = std::fs::read(folder.join(&e.file)).unwrap();
            assert_eq!(bytes.len() as u64, e.bytes, "{}", e.id);
            assert_eq!(hex(&craft3d_doc::sha256(&bytes)), e.sha256, "{}", e.id);
            let thumb = std::fs::read(folder.join("thumbs").join(format!("{}.png", e.id))).unwrap();
            assert!(thumb.len() as u64 <= MAX_THUMB_BYTES, "{}", e.id);
            let (w, h, _) = crate::thumbs::read_rgb(&thumb).unwrap();
            assert_eq!((w, h), THUMB, "{}", e.id);
            assert!(e.stats.is_object(), "{}", e.id);
        }
        // Nothing else in the folder.
        let mut files: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .flatten()
            .map(|d| d.file_name().to_string_lossy().to_string())
            .collect();
        files.sort();
        let mut expected: Vec<String> = listed.iter().map(|e| e.file.clone()).collect();
        expected.extend(["LICENSE.md", "bundle.json", "thumbs"].map(str::to_string));
        expected.sort();
        assert_eq!(files, expected);
        assert_eq!(
            std::fs::read_dir(folder.join("thumbs")).unwrap().count(),
            listed.len()
        );
        // What the manifest makes.
        assert_eq!(read(LICENCE), licence_text(&sources, &listed));
        assert_eq!(read(LIST), list_text(&listed));
        let attribution = read(ATTRIBUTION);
        let rows = attribution_rows(&sources, &listed);
        let found: Vec<&str> = attribution.lines().filter(|l| is_bundled_row(l)).collect();
        assert_eq!(found, rows.iter().map(String::as_str).collect::<Vec<_>>());
        let about = manifest.get("about").cloned().unwrap();
        assert_eq!(read(MANIFEST), manifest_text(&about, &sources, &listed));
    }

    #[test]
    fn the_examples_are_mit_and_the_jigs_and_assemblies_cc_by() {
        let licence = |name: &str| repository(name).unwrap().2;
        assert_eq!(licence("3dcraft-examples"), "MIT");
        assert_eq!(licence("3dcraft-jigs"), "CC-BY-4.0");
        assert_eq!(licence("3dcraft-assemblies-examples"), "CC-BY-4.0");
    }

    #[test]
    fn a_bad_manifest_is_refused() {
        for (text, says) in [
            (r#"{}"#, "no `entries`"),
            (r#"{"entries": [{"id": "A b"}]}"#, "lower-case"),
            (
                r#"{"entries": [{"id": "a", "section": "jigs", "repository": "3dcraft-jigs", "folder": "x"}, {"id": "a"}]}"#,
                "twice",
            ),
            (
                r#"{"entries": [{"id": "a", "section": "tools", "repository": "3dcraft-jigs", "folder": "x"}]}"#,
                "not parts",
            ),
            (
                r#"{"entries": [{"id": "a", "section": "jigs", "repository": "elsewhere", "folder": "x"}]}"#,
                "no repository",
            ),
            (
                r#"{"entries": [{"id": "a", "section": "jigs", "repository": "3dcraft-jigs", "folder": "../x"}]}"#,
                "plain path",
            ),
            (
                r#"{"entries": [{"id": "a", "section": "jigs", "repository": "3dcraft-jigs", "folder": "/x"}]}"#,
                "plain path",
            ),
            // git could take it for an option.
            (
                r#"{"entries": [{"id": "a", "section": "jigs", "repository": "3dcraft-jigs", "folder": "-x"}]}"#,
                "plain path",
            ),
            (
                r#"{"entries": [{"id": "a", "section": "jigs", "repository": "3dcraft-jigs", "folder": "jigs/--output=x"}]}"#,
                "plain path",
            ),
        ] {
            let value: Value = serde_json::from_str(text).unwrap();
            let err = entries(&value).unwrap_err();
            assert!(err.contains(says), "{text}: {err}");
        }
    }

    #[test]
    fn only_an_mit_licence_gives_its_copyright() {
        let mit = format!("MIT License\n\nCopyright (c) 2026 Someone\n\n{MIT_TERMS}\n");
        assert_eq!(mit_copyright(&mit).unwrap(), "Copyright (c) 2026 Someone");
        // Words before the terms (what the licence covers) are fine.
        let noted = format!(
            "MIT License\n\nCopyright (c) 2026 Someone\n\nThis covers the tools.\n\n{MIT_TERMS}"
        );
        assert!(mit_copyright(&noted).is_ok());
        assert!(mit_copyright("Apache License\nVersion 2.0").is_err());
        let changed = mit.replace("free of charge", "for a fee");
        assert!(mit_copyright(&changed).is_err());
        assert!(mit_copyright(&format!("MIT License\n\n{MIT_TERMS}")).is_err());
    }

    #[test]
    fn shrinking_averages_by_area() {
        // A 4 × 2 picture of two halves made 2 × 1: each pixel the mean of
        // its half.
        let px = |v: u8| [v, v, v];
        let rgb: Vec<u8> = [
            px(0),
            px(100),
            px(200),
            px(255),
            px(50),
            px(150),
            px(250),
            px(255),
        ]
        .concat();
        let small = shrink(4, 2, &rgb, 2, 1).unwrap();
        assert_eq!(small, [75, 75, 75, 240, 240, 240]);
        // Not a whole factor: 3 pixels into 2, the middle one shared.
        let small = shrink(3, 1, &[0, 0, 0, 90, 90, 90, 180, 180, 180], 2, 1).unwrap();
        assert_eq!(small, [30, 30, 30, 150, 150, 150]);
        assert!(shrink(2, 2, &[0; 12], 3, 1).is_err(), "never larger");
        assert!(shrink(2, 2, &[0; 11], 1, 1).is_err());
        assert!(shrink(0, 2, &[], 1, 1).is_err());
    }

    #[test]
    fn the_attribution_rows_are_replaced_in_place() {
        let text = "# A\n\n| Path | B |\n|---|---|\n| `x` | y |\n| `docs/examples/published/old.3dcraftjig` | z |\n| `w` | v |\n\nAfter.\n";
        let rows = vec!["| `docs/examples/published/new.3dcraft` | n |".to_string()];
        let out = with_rows(text, &rows).unwrap();
        assert_eq!(
            out,
            "# A\n\n| Path | B |\n|---|---|\n| `x` | y |\n| `docs/examples/published/new.3dcraft` | n |\n| `w` | v |\n\nAfter.\n"
        );
        // None yet: after the table's last row.
        let first = "# A\n\n| Path | B |\n|---|---|\n| `x` | y |\n\nAfter.\n";
        assert_eq!(
            with_rows(first, &rows).unwrap(),
            "# A\n\n| Path | B |\n|---|---|\n| `x` | y |\n| `docs/examples/published/new.3dcraft` | n |\n\nAfter.\n"
        );
        // No table: an error, never the rows dropped.
        let error = with_rows("# A\n\nNo table here.\n", &rows).unwrap_err();
        assert!(error.contains("no table"), "{error}");
    }

    /// An entry as the manifest gives it, before the task wrote the rest.
    fn entry(id: &str, section: Section, repository: &str) -> Entry {
        Entry {
            id: id.to_string(),
            section,
            start: false,
            repository: repository.to_string(),
            folder: format!("x/{id}"),
            file: format!("{id}.{}", section.extension()),
            commit: String::new(),
            licence: String::new(),
            title: String::new(),
            summary: String::new(),
            author: String::new(),
            sha256: String::new(),
            bytes: 0,
            stats: Value::Null,
            format_version: None,
            built_with: String::new(),
        }
    }

    #[test]
    fn two_files_of_one_name_are_refused() {
        let a = entry("bin", Section::Parts, "3dcraft-examples");
        let mut b = entry("other-bin", Section::Jigs, "3dcraft-jigs");
        assert!(check_unique_files(&[a.clone(), b.clone()]).is_ok());
        // Told apart without case, as a file system may.
        b.file = "BIN.3dcraft".to_string();
        let error = check_unique_files(&[a, b]).unwrap_err();
        assert!(
            error.contains("bin (3dcraft-examples)") && error.contains("other-bin (3dcraft-jigs)"),
            "{error}"
        );
    }

    #[test]
    fn a_meta_json_without_the_stats_the_tests_compare_with_is_refused() {
        let part = entry("gear", Section::Parts, "3dcraft-examples");
        let asm = entry("clamp", Section::Assemblies, "3dcraft-assemblies-examples");
        let meta = |id: &str, licence: &str, stats: Value| json!({"id": id, "licence": licence, "title": "A title", "summary": "What it is", "stats": stats});
        let part_stats = json!({"load_s": 0.2, "features": 4, "assertions": 2, "warnings": 0});
        let ok = check_meta(&part, &meta("gear", "MIT", part_stats.clone()), "MIT").unwrap();
        assert_eq!(ok.stats, part_stats);
        assert_eq!((ok.title.as_str(), ok.licence.as_str()), ("A title", "MIT"));
        let asm_stats = json!({"load_s": 1.5, "instances": 9, "mates": 8, "dof": 1, "assertions": 3, "warnings": 0});
        assert!(check_meta(&asm, &meta("clamp", "CC-BY-4.0", asm_stats), "CC-BY-4.0").is_ok());
        for (entry, value, licence, says) in [
            (
                &part,
                json!({"id": "gear", "licence": "MIT", "title": "A", "summary": "B"}),
                "MIT",
                "no `stats`",
            ),
            (&part, meta("gear", "MIT", json!(3)), "MIT", "no `stats`"),
            (
                &part,
                meta(
                    "gear",
                    "MIT",
                    json!({"features": 4, "assertions": 2, "warnings": 0}),
                ),
                "MIT",
                "load_s",
            ),
            (
                &part,
                meta(
                    "gear",
                    "MIT",
                    json!({"load_s": 0.2, "assertions": 2, "warnings": 0}),
                ),
                "MIT",
                "`features`",
            ),
            (
                &part,
                meta(
                    "gear",
                    "MIT",
                    json!({"load_s": -1, "features": 4, "assertions": 2, "warnings": 0}),
                ),
                "MIT",
                "load_s",
            ),
            (
                &asm,
                meta("clamp", "CC-BY-4.0", part_stats.clone()),
                "CC-BY-4.0",
                "`instances`",
            ),
            (
                &part,
                meta("gear", "CC-BY-SA-4.0", part_stats.clone()),
                "MIT",
                "licence",
            ),
            (
                &part,
                meta("cog", "MIT", part_stats.clone()),
                "MIT",
                "the id",
            ),
        ] {
            let error = check_meta(entry, &value, licence).unwrap_err();
            assert!(error.contains(says), "{value}: {error}");
        }
    }

    /// A part document with `licence` and `title` in its information, as
    /// the bytes of its file.
    fn part_bytes(title: &str, licence: &str) -> Vec<u8> {
        let mut part = Document::new(craft3d_doc::Units::default());
        part.info.title = title.to_string();
        part.info.licence = licence.to_string();
        craft3d_format::to_bytes(&part).unwrap()
    }

    /// An assembly that packs `parts` (the bytes of their files).
    fn assembly(parts: Vec<Vec<u8>>) -> Document {
        let mut document = Document::new(craft3d_doc::Units::default());
        let mut assembly = craft3d_doc::Assembly::default();
        for bytes in parts {
            assembly.pack(craft3d_doc::Blob::new(bytes));
        }
        document.assembly = Some(assembly);
        document
    }

    #[test]
    fn a_part_packed_in_an_assembly_under_another_licence_is_refused_by_name() {
        let by = "CC-BY-4.0";
        let fine = assembly(vec![part_bytes("Jaw", by), part_bytes("Lever", by)]);
        assert!(check_packed_licences(&fine, by).is_ok());
        // A part is no assembly: nothing packed.
        assert!(check_packed_licences(&Document::new(craft3d_doc::Units::default()), by).is_ok());
        let mixed = assembly(vec![part_bytes("Jaw", by), part_bytes("Bolt", "MIT")]);
        let error = check_packed_licences(&mixed, by).unwrap_err();
        assert!(
            error.contains("\"Bolt\"") && error.contains("`MIT`") && error.contains("blobs/"),
            "{error}"
        );
        // Also in a subassembly packed in it.
        let mut inner = assembly(vec![part_bytes("Pin", "MIT")]);
        inner.info.licence = by.to_string();
        let outer = assembly(vec![craft3d_format::to_bytes(&inner).unwrap()]);
        let error = check_packed_licences(&outer, by).unwrap_err();
        assert!(error.contains("\"Pin\""), "{error}");
        // Bytes that are no document are refused, not skipped.
        let broken = assembly(vec![b"not a document".to_vec()]);
        assert!(check_packed_licences(&broken, by).is_err());
    }

    #[test]
    fn the_cc_by_licence_gives_its_copyright() {
        let text = "# Licence\n\nCopyright (c) 2026 Someone\n\nThe files are licensed under the Creative Commons\nAttribution 4.0 International licence (CC BY 4.0).\n";
        assert_eq!(cc_by_copyright(text).unwrap(), "Copyright (c) 2026 Someone");
        assert!(cc_by_copyright("Copyright (c) 2026 Someone\n\nMIT License").is_err());
        assert!(
            cc_by_copyright("Creative Commons Attribution 4.0 International, no notice").is_err()
        );
    }

    /// The licence of the bundled files keeps the CC BY licensor's notice,
    /// says CC BY-SA only of the Jigs and the packed parts only of the
    /// assemblies, and links every repository's credits of what is from
    /// elsewhere; so do the rows of ATTRIBUTION.md.
    #[test]
    fn the_licence_keeps_the_notices_and_links_the_credits() {
        let mut part = entry("gear", Section::Parts, "3dcraft-examples");
        let mut jig = entry("box", Section::Jigs, "3dcraft-jigs");
        let mut asm = entry("clamp", Section::Assemblies, "3dcraft-assemblies-examples");
        for e in [&mut part, &mut jig, &mut asm] {
            e.title = format!("The {}", e.id);
            e.commit = "c".repeat(40);
        }
        let source = |n: char| Source {
            commit: n.to_string().repeat(40),
            copyright: format!("Copyright (c) 2026 Holder {n}"),
        };
        let sources = [
            ("3dcraft-examples", source('a')),
            ("3dcraft-jigs", source('b')),
            ("3dcraft-assemblies-examples", source('d')),
        ];
        let text = licence_text(&sources, &[part.clone(), jig.clone(), asm.clone()]);
        let section = |from: &str| {
            let start = text.find(&format!("## From {from}")).unwrap();
            let rest = &text[start + 3..];
            rest[..rest.find("\n## ").unwrap_or(rest.len())].to_string()
        };
        let (examples, jigs, assemblies) = (
            section("the 3DCraft examples"),
            section("the 3DCraft Jigs"),
            section("the 3DCraft assemblies"),
        );
        assert!(jigs.contains("> Copyright (c) 2026 Holder b"), "{jigs}");
        assert!(
            assemblies.contains("> Copyright (c) 2026 Holder d"),
            "{assemblies}"
        );
        assert!(
            examples.contains("Copyright (c) 2026 Holder a"),
            "{examples}"
        );
        assert!(jigs.contains("CC BY-SA"), "{jigs}");
        assert!(!assemblies.contains("CC BY-SA"), "{assemblies}");
        assert!(assemblies.contains("blobs/*.3dcraft"), "{assemblies}");
        assert!(!jigs.contains("blobs/"), "{jigs}");
        for (body, repo, n) in [
            (&examples, "3dcraft-examples", 'a'),
            (&jigs, "3dcraft-jigs", 'b'),
            (&assemblies, "3dcraft-assemblies-examples", 'd'),
        ] {
            let link = format!(
                "https://github.com/johancoppens/{repo}/blob/{}/ATTRIBUTION.md",
                n.to_string().repeat(40)
            );
            assert!(body.contains(&link), "{body}");
        }
        let rows = attribution_rows(&sources, &[part, jig, asm]);
        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert!(row.contains("/ATTRIBUTION.md)"), "{row}");
        }
    }

    #[test]
    fn arguments_parse() {
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse(&args(&["--from", "/x"])).unwrap().from,
            Some(PathBuf::from("/x"))
        );
        assert!(parse(&args(&["--from"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
    }
}
