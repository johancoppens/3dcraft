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
//! working tree and its other branches do not matter; no network; with
//! `--keep-commits` at the commit the manifest records instead), the
//! built file without its thumbnails ([`without_thumbnails`]: the picture
//! `thumb.png` a file holds for a gallery, its own and that of every
//! document packed in it, which loading ignores and the start screen does
//! not show; nothing else of the file changes, [`check_copy`]), its
//! commit, its licence, title and one-line summary (its `meta.json`), the
//! numbers its repository recorded when it was built (`stats`: the
//! features, assertions, parts, instances and how long it took to open),
//! the size and SHA-256 of the built file and of the copy (`copy`, with
//! how many pictures it left out and their bytes). Its picture on the start
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
//! part, a Jig, an assembly); a file that holds notes pinned to the model,
//! or a part packed in an assembly that does (a published example holds
//! none: docs/decisions.md, 2026-11-17; they are deleted where the file is
//! built, `note.delete {"state": "all"}`, so that the file in its own
//! repository holds none either); a `meta.json` without its id, title, summary
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
/// Where a document packed into an assembly is in its file:
/// `blobs/<sha256>.3dcraft` (`craft3d-format`, docs/file-formats.md).
const PACKED_PREFIX: &str = "blobs/";
const PACKED_SUFFIX: &str = ".3dcraft";
/// The list of an assembly's packed documents in a manifest, as 3DCraft
/// writes it: the key and the bracket its hashes follow.
const PACKED_LIST: &str = "\"packed\": [";
/// What an entry named `thumb.png` takes in a stored archive besides its
/// data: its local header (30 bytes) and its record in the central
/// directory (46), each with the name.
pub const THUMB_ENTRY_OVERHEAD: u64 = (30 + 46 + 2 * craft3d_format::THUMBNAIL_NAME.len()) as u64;

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
    /// The SHA-256 and size of the built file, as its repository has it at
    /// `commit`.
    pub sha256: String,
    pub bytes: u64,
    /// The copy bundled here.
    pub copy: Bundled,
    pub stats: Value,
    pub format_version: Option<u64>,
    pub built_with: String,
}

/// What the manifest says of the copy of a built file that is bundled:
/// its size and SHA-256, and how many pictures (`thumb.png`) were left out
/// of it ([`without_thumbnails`]) and their bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bundled {
    pub sha256: String,
    pub bytes: u64,
    pub left_out: LeftOut,
}

impl Bundled {
    fn json(&self) -> Value {
        json!({
            "bytes": self.bytes,
            "sha256": self.sha256,
            "thumbnails": self.left_out.thumbnails,
            "thumbnailBytes": self.left_out.bytes,
        })
    }

    fn read(value: Option<&Value>) -> Self {
        let number = |key: &str| {
            value
                .and_then(|v| v.get(key))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        Bundled {
            sha256: value.map(|v| text(v, "sha256")).unwrap_or_default(),
            bytes: number("bytes"),
            left_out: LeftOut {
                thumbnails: number("thumbnails"),
                bytes: number("thumbnailBytes"),
            },
        }
    }
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
            "copy": self.copy.json(),
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
            copy: Bundled::read(item.get("copy")),
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

/// The commit `rev` names in `clone`: the head of `main`, or a commit the
/// manifest records (which must be one of the clone's).
fn commit_of(clone: &Path, rev: &str) -> Result<String, String> {
    if rev.is_empty() || rev.starts_with('-') {
        return Err(format!("{}: no commit `{rev}`", clone.display()));
    }
    let out = git(
        clone,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )?;
    let commit = String::from_utf8_lossy(&out).trim().to_string();
    if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "{}: `{rev}` is `{commit}`, not a commit",
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

/// How many pictures [`without_thumbnails`] left out of a file, its own
/// and those of the documents packed in it, and the bytes of their data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeftOut {
    pub thumbnails: u64,
    pub bytes: u64,
}

impl LeftOut {
    fn add(&mut self, other: LeftOut) {
        self.thumbnails = self.thumbnails.saturating_add(other.thumbnails);
        self.bytes = self.bytes.saturating_add(other.bytes);
    }

    /// How much smaller the file is for it: each picture's data and the
    /// headers of its entry ([`THUMB_ENTRY_OVERHEAD`]). A packed document
    /// is stored as it is, so what it loses its file loses, and its new
    /// name, a SHA-256 as the old one was, is as long.
    pub fn saved(&self) -> u64 {
        self.bytes
            .saturating_add(self.thumbnails.saturating_mul(THUMB_ENTRY_OVERHEAD))
    }
}

/// `bytes`, a `.3dcraft` file (a part, a Jig or an assembly), without the
/// picture `thumb.png` it holds for a gallery, which loading ignores
/// (`craft3d-format`), and without those of the documents packed in it
/// (an assembly's `blobs/<sha256>.3dcraft`, at any depth). Everything
/// else stays byte for byte and in its place, but what names a packed
/// document whose bytes changed: its entry takes the name of its new
/// SHA-256, each string of the manifest that was the old one becomes the
/// new one, the manifest's list `packed` is put back in the ascending
/// order of the hashes in which a document keeps it, and the stored files
/// (`blobs/…`) in the order of their names in which 3DCraft writes them.
/// So the copy of a file 3DCraft wrote is the file it writes for the same
/// document, its packed documents without their pictures.
///
/// Refused: an archive that 3DCraft's writer would not write as it is (so
/// that nothing else could change on the way), a manifest that names a
/// packed document other than by its whole hash or holds not one list
/// `packed`, and two documents packed in one file that differ only by
/// their pictures.
pub fn without_thumbnails(bytes: &[u8]) -> Result<(Vec<u8>, LeftOut), String> {
    without_thumbnails_at(bytes, 0)
}

fn without_thumbnails_at(bytes: &[u8], depth: usize) -> Result<(Vec<u8>, LeftOut), String> {
    if depth > MAX_PACKED_DEPTH {
        return Err(format!(
            "its assemblies are packed in one another more than {MAX_PACKED_DEPTH} deep"
        ));
    }
    let entries = craft3d_zipstore::read_stored(bytes)
        .map_err(|e| format!("its archive cannot be read: {e}"))?;
    let pairs: Vec<(&str, &[u8])> = entries.iter().map(|e| (e.name, e.data)).collect();
    if craft3d_zipstore::write_stored(&pairs).ok().as_deref() != Some(bytes) {
        return Err(
            "its archive is not as 3DCraft writes one, so its other entries could not be kept byte for byte"
                .to_string(),
        );
    }
    let mut left = LeftOut::default();
    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut kept: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in &entries {
        if entry.name == craft3d_format::THUMBNAIL_NAME {
            left.add(LeftOut {
                thumbnails: 1,
                bytes: u64::try_from(entry.data.len()).unwrap_or(u64::MAX),
            });
            continue;
        }
        if let Some(old) = packed_hash(entry.name) {
            let (inner, inner_left) = without_thumbnails_at(entry.data, depth + 1)
                .map_err(|e| format!("the document packed as {}: {e}", entry.name))?;
            left.add(inner_left);
            if inner != entry.data {
                let new = hex(&craft3d_doc::sha256(&inner));
                kept.push((format!("{PACKED_PREFIX}{new}{PACKED_SUFFIX}"), inner));
                renamed.push((old.to_string(), new));
                continue;
            }
        }
        kept.push((entry.name.to_string(), entry.data.to_vec()));
    }
    for (n, (name, _)) in kept.iter().enumerate() {
        if kept.iter().skip(n + 1).any(|(other, _)| other == name) {
            return Err(format!(
                "two documents packed in it differ only by their pictures (both are {name} without them)"
            ));
        }
    }
    if !renamed.is_empty() {
        let manifest = kept
            .iter_mut()
            .find(|(name, _)| name == craft3d_format::MANIFEST_NAME)
            .ok_or_else(|| format!("it has no {}", craft3d_format::MANIFEST_NAME))?;
        manifest.1 = renamed_manifest(&manifest.1, &renamed)?;
        // The stored files (`blobs/…`) in the order of their names again,
        // as 3DCraft writes them, in the places they had.
        let mut places = Vec::new();
        let mut blobs = Vec::new();
        for (n, entry) in kept.iter_mut().enumerate() {
            if entry.0.starts_with(PACKED_PREFIX) {
                places.push(n);
                blobs.push(std::mem::take(entry));
            }
        }
        // By name: no two have one.
        blobs.sort();
        for (n, blob) in places.into_iter().zip(blobs) {
            if let Some(entry) = kept.get_mut(n) {
                *entry = blob;
            }
        }
    }
    let pairs: Vec<(&str, &[u8])> = kept
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_slice()))
        .collect();
    let out = craft3d_zipstore::write_stored(&pairs)
        .map_err(|e| format!("its copy cannot be written: {e}"))?;
    Ok((out, left))
}

/// Whether `text` is a SHA-256 as 3DCraft writes one: 64 lowercase
/// hexadecimal digits.
fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The SHA-256 in the name of a packed document's entry,
/// `blobs/<sha256>.3dcraft`.
fn packed_hash(name: &str) -> Option<&str> {
    name.strip_prefix(PACKED_PREFIX)?
        .strip_suffix(PACKED_SUFFIX)
        .filter(|hash| is_sha256(hash))
}

/// The first twelve digits of a hash, for a message.
fn short(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

/// The manifest `json` with each packed document of `renamed` (its old
/// SHA-256, its new one) named by its new one: every string that was the
/// old hash, which must never appear but as a whole string, and the list
/// `packed` in ascending order again ([`sorted_packed`]).
fn renamed_manifest(json: &[u8], renamed: &[(String, String)]) -> Result<Vec<u8>, String> {
    let mut text = String::from_utf8(json.to_vec())
        .map_err(|_| format!("its {} is not UTF-8", craft3d_format::MANIFEST_NAME))?;
    for (old, new) in renamed {
        let quoted = format!("\"{old}\"");
        let whole = text.matches(&quoted).count();
        if whole == 0 || whole != text.matches(old.as_str()).count() {
            return Err(format!(
                "its manifest names the packed document {}… other than by its whole SHA-256",
                short(old)
            ));
        }
        text = text.replace(&quoted, &format!("\"{new}\""));
    }
    sorted_packed(&text).map(String::into_bytes)
}

/// `text`, a manifest, with the hashes of its list `packed` in ascending
/// order, each where one of them was, so that nothing else of the text
/// moves (they are all as long). A string of the manifest cannot hold
/// [`PACKED_LIST`], whose quotes it would escape, so the one found is the
/// key.
fn sorted_packed(text: &str) -> Result<String, String> {
    let mut found = text.match_indices(PACKED_LIST).map(|(at, _)| at);
    let (Some(at), None) = (found.next(), found.next()) else {
        return Err("its manifest does not hold one list `packed`".to_string());
    };
    let start = at.saturating_add(PACKED_LIST.len());
    let rest = text.get(start..).unwrap_or_default();
    let end = rest
        .find(']')
        .and_then(|len| start.checked_add(len))
        .ok_or("its manifest's list `packed` does not end")?;
    let list = text.get(start..end).unwrap_or_default();
    let items: Vec<&str> = list.split(',').collect();
    let mut hashes = Vec::new();
    for item in &items {
        let hash = item
            .trim()
            .strip_prefix('"')
            .and_then(|i| i.strip_suffix('"'))
            .filter(|h| is_sha256(h))
            .ok_or_else(|| {
                format!(
                    "its manifest's list `packed` holds `{}`, not a SHA-256",
                    item.trim()
                )
            })?;
        hashes.push(hash);
    }
    let mut sorted = hashes.clone();
    sorted.sort_unstable();
    let list: Vec<String> = items
        .iter()
        .zip(hashes.iter().zip(&sorted))
        .map(|(item, (old, new))| item.replacen(old, new, 1))
        .collect();
    Ok(format!(
        "{}{}{}",
        text.get(..start).unwrap_or_default(),
        list.join(","),
        text.get(end..).unwrap_or_default()
    ))
}

/// Checks that `copy` is `source` without its thumbnails
/// ([`without_thumbnails`]) as a document too: neither holds a picture
/// any more, both load with the same notes, and the copy's document is
/// the source's but for the documents packed in it, each the copy of the
/// source's (checked so in turn, at any depth), and the hashes that name
/// them.
pub fn check_copy(source: &[u8], copy: &[u8]) -> Result<(), String> {
    check_copy_at(source, copy, 0)
}

fn check_copy_at(source: &[u8], copy: &[u8], depth: usize) -> Result<(), String> {
    if depth > MAX_PACKED_DEPTH {
        return Err(format!(
            "its assemblies are packed in one another more than {MAX_PACKED_DEPTH} deep"
        ));
    }
    if craft3d_format::thumbnail(copy)
        .map_err(|e| format!("its copy cannot be read: {e}"))?
        .is_some()
    {
        return Err(format!(
            "its copy still holds {}",
            craft3d_format::THUMBNAIL_NAME
        ));
    }
    let (original, source_notes) =
        craft3d_format::from_bytes_noted(source).map_err(|e| format!("it cannot be read: {e}"))?;
    let (copied, copy_notes) = craft3d_format::from_bytes_noted(copy)
        .map_err(|e| format!("its copy cannot be read: {e}"))?;
    if copy_notes != source_notes {
        return Err(format!(
            "reading its copy notes {copy_notes:?}, reading it {source_notes:?}"
        ));
    }
    let mut expected = original;
    if let Some(assembly) = expected.assembly.as_mut() {
        let theirs = copied
            .assembly
            .as_ref()
            .ok_or("its copy is not an assembly")?;
        let mut names: Vec<(String, String)> = Vec::new();
        let mut packed = Vec::new();
        for blob in &assembly.packed {
            let old = blob.sha256();
            let (bytes, _) = without_thumbnails(blob.bytes())?;
            let new = hex(&craft3d_doc::sha256(&bytes));
            let mine = theirs.packed(&new).ok_or_else(|| {
                format!(
                    "its copy lacks the document packed as blobs/{}… without its pictures",
                    short(&old)
                )
            })?;
            check_copy_at(blob.bytes(), mine.bytes(), depth + 1)
                .map_err(|e| format!("the document packed as blobs/{}…: {e}", short(&old)))?;
            names.push((old, new));
            packed.push(mine.clone());
        }
        assembly.packed.clear();
        for blob in packed {
            assembly.pack(blob);
        }
        let rename = |hash: &mut String| {
            if let Some((_, new)) = names.iter().find(|(old, _)| old.as_str() == hash.as_str()) {
                *hash = new.clone();
            }
        };
        for instance in &mut assembly.instances {
            if let craft3d_doc::Source::Packed { hash } = &mut instance.source {
                rename(hash);
            }
            if let Some(hash) = &mut instance.pinned {
                rename(hash);
            }
        }
    }
    if expected != copied {
        return Err(
            "the document of its copy differs from its own beyond the names of the documents packed in it"
                .to_string(),
        );
    }
    Ok(())
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

/// Checks that `document` holds no notes pinned to the model, nor does a
/// part or subassembly packed in it (`blobs/*.3dcraft`, and theirs in
/// turn): a published example holds none (docs/decisions.md, 2026-11-17).
/// The file is bundled as it was built, so its notes are not stripped
/// here: they are deleted where it is built.
pub fn check_no_notes(document: &Document) -> Result<(), String> {
    no_notes(document, 0)
}

fn no_notes(document: &Document, depth: usize) -> Result<(), String> {
    let count = document.notes.items.len();
    if count > 0 {
        let title = if document.info.title.is_empty() {
            "it".to_string()
        } else {
            format!("\"{}\"", document.info.title)
        };
        let plural = if count == 1 { "" } else { "s" };
        return Err(format!(
            "{title} holds {count} note{plural} pinned to the model, which a published example never does: delete them where the file is built (note.delete with state all) and build it again"
        ));
    }
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
        let part = craft3d_format::from_bytes(blob.bytes())
            .map_err(|e| format!("the part packed as blobs/{short}… cannot be read: {e}"))?;
        no_notes(&part, depth + 1)
            .map_err(|e| format!("the part packed as blobs/{short}…: {e}"))?;
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
    check_no_notes(&document).map_err(|e| format!("{id}: {file}: {e}"))?;
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
    // What is bundled: the file without its pictures, the same document.
    let (copied, left_out) = without_thumbnails(&bytes)
        .map_err(|e| format!("{id}: {file} cannot be copied without its thumbnails: {e}"))?;
    check_copy(&bytes, &copied).map_err(|e| format!("{id}: {file}: {e}"))?;
    let copy_size = u64::try_from(copied.len()).unwrap_or(u64::MAX);
    if size.checked_sub(copy_size) != Some(left_out.saved()) {
        return Err(format!(
            "{id}: the copy of {file} is {copy_size} bytes, not its {size} less the {} of the pictures left out",
            left_out.saved()
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
    made.copy = Bundled {
        sha256: hex(&craft3d_doc::sha256(&copied)),
        bytes: copy_size,
        left_out,
    };
    made.stats = stats;
    made.format_version = craft3d
        .and_then(|c| c.get("format_version"))
        .and_then(Value::as_u64);
    made.built_with = craft3d.map(|c| text(c, "commit")).unwrap_or_default();
    Ok(Copied {
        entry: made,
        bytes: copied,
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
         (docs/decisions.md,\n2026-11-10). They are copied from three repositories, which are \
         their source, with one change: the\npicture `thumb.png` a file holds for a gallery is \
         left out, its own and that of each part packed in an\nassembly (which then takes the \
         SHA-256 of its new bytes as its name in the file). Nothing else is changed,\nand they \
         are not edited here. `bundle.json` lists each with its repository, folder, commit and \
         licence,\nthe size and SHA-256 of the file there and of the copy here, and `cargo xtask \
         bundle-examples` copies them\nagain (docs/development.md, Bundled examples). The \
         pictures in `thumbs/` are the `preview.png` of each\nfile's folder, made smaller, under \
         the same licence as the file. This file is written by that task: do\nnot edit it by \
         hand.\n",
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
                 The one change made to these copies: the picture `thumb.png` is left out of each \
                 file and of each\npart packed in it (above).\n",
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
    /// Each repository at the commit the manifest records, not at the head
    /// of its `main`: the same sources copied again, after a change to how
    /// they are copied.
    keep_commits: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--from" => {
                o.from = Some(PathBuf::from(it.next().ok_or("--from needs a folder")?));
            }
            "--keep-commits" => o.keep_commits = true,
            other => {
                return Err(format!(
                    "bundle-examples: unknown argument `{other}`; usage: cargo xtask bundle-examples --from DIR [--keep-commits] \
                     (DIR holds the clones 3dcraft-examples, 3dcraft-jigs and 3dcraft-assemblies-examples; \
                     --keep-commits reads each at the commit the manifest records instead of the head of its main)"
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
/// `CRAFT3D_EXAMPLE_REPOS`), at the head of their `main` or, with
/// `--keep-commits`, at the commits the manifest records, each without its
/// thumbnails, then writes the manifest, the licence, the rows of
/// `ATTRIBUTION.md` and the start screen's list: all or nothing.
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
    let recorded = sources(&manifest, &listed);
    // Each repository's head (or the commit recorded) and the copyright of
    // its licence.
    let mut sources: Vec<(&'static str, Source)> = Vec::new();
    for (name, _, licence, _) in REPOSITORIES {
        if !listed.iter().any(|e| e.repository == name) {
            continue;
        }
        let clone = from.join(name);
        let commit = if options.keep_commits {
            let at = recorded
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, s)| s.commit.as_str())
                .unwrap_or_default();
            commit_of(&clone, at).map_err(|e| {
                format!("{name}: --keep-commits: the manifest records no commit of the clone: {e}")
            })?
        } else {
            commit_of(&clone, "main")?
        };
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
                    "{}: {} ({} bytes, {} without its {} thumbnails; start-screen picture {} bytes)",
                    c.entry.id,
                    c.entry.file,
                    c.entry.bytes,
                    c.bytes.len(),
                    c.entry.copy.left_out.thumbnails,
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
    let total: u64 = made.iter().map(|e| e.copy.bytes).sum();
    let built: u64 = made.iter().map(|e| e.bytes).sum();
    eprintln!(
        "bundle-examples: {} files, {total} bytes ({built} as built, with their thumbnails); wrote {MANIFEST}, {LICENCE}, {ATTRIBUTION} and {LIST}",
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

    /// The pictures (`thumb.png`) a file holds, its own and those of the
    /// documents packed in it at any depth, read through the format crate
    /// (not [`without_thumbnails`]).
    fn pictures_in(bytes: &[u8], depth: usize) -> usize {
        assert!(depth <= MAX_PACKED_DEPTH);
        let own = usize::from(craft3d_format::thumbnail(bytes).unwrap().is_some());
        let document = craft3d_format::from_bytes(bytes).unwrap();
        own + document
            .assembly
            .iter()
            .flat_map(|a| &a.packed)
            .map(|blob| pictures_in(blob.bytes(), depth + 1))
            .sum::<usize>()
    }

    /// The bundled files are the ones the manifest names, as they were
    /// copied (size and SHA-256), with their start-screen pictures; each
    /// is its built file (whose size and SHA-256 at the commit the
    /// manifest keeps, the size the one its repository recorded when it
    /// built it) without the pictures left out: no `thumb.png` is left in
    /// it nor in a document packed in it, it opens, and it is smaller by
    /// exactly the pictures' entries. The licence, the rows of
    /// ATTRIBUTION.md and the start screen's list are what the manifest
    /// makes, so a refresh left nothing behind.
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
            assert_eq!(bytes.len() as u64, e.copy.bytes, "{}", e.id);
            assert_eq!(hex(&craft3d_doc::sha256(&bytes)), e.copy.sha256, "{}", e.id);
            // Its built file: as large as its repository measured it, and
            // larger than the copy by the pictures' entries alone.
            assert!(is_sha256(&e.sha256), "{}", e.id);
            if let Some(built) = e.stats.get("file_bytes").and_then(Value::as_u64) {
                assert_eq!(built, e.bytes, "{}", e.id);
            }
            assert_eq!(
                e.bytes.checked_sub(e.copy.bytes),
                Some(e.copy.left_out.saved()),
                "{}",
                e.id
            );
            assert_eq!(
                e.copy.left_out.thumbnails == 0,
                e.copy.sha256 == e.sha256,
                "{}",
                e.id
            );
            // No picture left, nor in a part packed in it; it opens.
            assert_eq!(pictures_in(&bytes, 0), 0, "{}", e.id);
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
            copy: Bundled::default(),
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

    /// No document the repository ships for others to open (the bundled
    /// examples, Jigs and assemblies under `docs/examples/published`, the
    /// showcase under `docs/showcase`, anything else under `docs/`) holds
    /// a note pinned to the model, nor does a part packed in it: a
    /// published file holds none (docs/decisions.md, 2026-11-17).
    #[test]
    fn no_document_under_docs_holds_a_note() {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    walk(&path, out);
                } else if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| matches!(e, "3dcraft" | "3dcraftjig" | "3dcraftasm"))
                {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(&root().join("docs"), &mut files);
        assert!(
            files.iter().any(|f| f.starts_with(root().join(FOLDER))),
            "the bundled examples are among them: {files:?}"
        );
        for file in &files {
            let bytes = std::fs::read(file).unwrap();
            let document = craft3d_format::from_bytes(&bytes)
                .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            if let Err(e) = check_no_notes(&document) {
                panic!("{}: {e}", file.display());
            }
        }
    }

    /// A file with notes pinned to the model is not bundled, nor an
    /// assembly that packs a part with notes, at any depth: a published
    /// example holds none.
    #[test]
    fn a_file_or_a_packed_part_with_notes_is_refused() {
        let by = "CC-BY-4.0";
        let noted = |title: &str| {
            let mut part = Document::new(craft3d_doc::Units::default());
            part.info.title = title.to_string();
            part.info.licence = by.to_string();
            part.add_note(
                "2 mm thicker here",
                None,
                [0.0; 3],
                craft3d_doc::NoteAuthor::Unknown,
                0,
            )
            .unwrap();
            part
        };
        let clean = assembly(vec![part_bytes("Jaw", by)]);
        assert!(check_no_notes(&clean).is_ok());
        let error = check_no_notes(&noted("Gear")).unwrap_err();
        assert!(
            error.contains("\"Gear\" holds 1 note ") && error.contains("note.delete"),
            "{error}"
        );
        let packed = assembly(vec![
            part_bytes("Jaw", by),
            craft3d_format::to_bytes(&noted("Lever")).unwrap(),
        ]);
        let error = check_no_notes(&packed).unwrap_err();
        assert!(
            error.contains("blobs/") && error.contains("\"Lever\""),
            "{error}"
        );
        let outer = assembly(vec![craft3d_format::to_bytes(&packed).unwrap()]);
        assert!(check_no_notes(&outer).is_err());
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
            parse(&args(&["--from", "/x"])).unwrap(),
            Options {
                from: Some(PathBuf::from("/x")),
                keep_commits: false
            }
        );
        assert!(
            parse(&args(&["--keep-commits", "--from", "/x"]))
                .unwrap()
                .keep_commits
        );
        assert!(parse(&args(&["--from"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
    }

    /// A small PNG-like picture, different per `seed` (only its bytes
    /// matter: loading never decodes it).
    fn picture(seed: u8, len: usize) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend((0..len).map(|i| seed.wrapping_add(i as u8)));
        png
    }

    /// A part with `title`, saved with the picture `seed` (none for `None`).
    fn part_file(title: &str, seed: Option<u8>) -> (Document, Vec<u8>) {
        let mut part = Document::new(craft3d_doc::Units::default());
        part.info.title = title.to_string();
        part.info.licence = "CC-BY-4.0".to_string();
        let png = seed.map(|s| picture(s, 300 + usize::from(s)));
        let bytes = craft3d_format::to_bytes_with_thumbnail(&part, png.as_deref()).unwrap();
        (part, bytes)
    }

    fn instance(name: &str, source: craft3d_doc::Source) -> craft3d_doc::Instance {
        craft3d_doc::Instance {
            name: name.to_string(),
            source,
            body: None,
            params: Vec::new(),
            pinned: None,
            mode: craft3d_doc::InstanceMode::Rigid,
            visible: true,
            colour: None,
            placement: craft3d_doc::Placement::default(),
        }
    }

    /// An assembly that packs the files `parts` and uses each from one
    /// instance (`P0`, `P1`, …), the second pinned to its bytes.
    fn assembly_of(parts: &[&[u8]]) -> Document {
        let mut document = Document::new(craft3d_doc::Units::default());
        document.info.licence = "CC-BY-4.0".to_string();
        let mut assembly = craft3d_doc::Assembly::default();
        for (n, bytes) in parts.iter().enumerate() {
            let blob = craft3d_doc::Blob::new(bytes.to_vec());
            let hash = blob.sha256();
            assembly.pack(blob);
            let mut placed = instance(
                &format!("P{n}"),
                craft3d_doc::Source::Packed { hash: hash.clone() },
            );
            if n == 1 {
                placed.pinned = Some(hash);
            }
            assembly.instances.push(placed);
        }
        document.assembly = Some(assembly);
        document
    }

    /// The entries of a stored archive, by name.
    fn entries_of(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
        craft3d_zipstore::read_stored(bytes)
            .unwrap()
            .iter()
            .map(|e| (e.name.to_string(), e.data.to_vec()))
            .collect()
    }

    /// A part or a Jig is copied as 3DCraft writes it without the
    /// picture, byte for byte (its manifest untouched); a file without one
    /// is copied unchanged; what was left out makes the size.
    #[test]
    fn a_file_is_copied_without_its_thumbnail_and_nothing_else() {
        let (part, bytes) = part_file("Bracket", Some(7));
        let (copy, left) = without_thumbnails(&bytes).unwrap();
        assert_eq!(copy, craft3d_format::to_bytes(&part).unwrap());
        let png = picture(7, 307);
        assert_eq!(
            left,
            LeftOut {
                thumbnails: 1,
                bytes: png.len() as u64
            }
        );
        assert_eq!((bytes.len() - copy.len()) as u64, left.saved());
        assert_eq!(craft3d_format::thumbnail(&copy).unwrap(), None);
        check_copy(&bytes, &copy).unwrap();
        // Nothing to leave out: the same bytes.
        assert_eq!(
            without_thumbnails(&copy).unwrap(),
            (copy.clone(), LeftOut::default())
        );
        check_copy(&copy, &copy).unwrap();
    }

    /// An assembly loses its picture and those of the parts packed in it,
    /// also in a packed subassembly; each changed part takes the name of
    /// its new bytes, in the manifest too (a pinned instance included),
    /// and the list `packed` is in order again: the copy holds exactly the
    /// entries 3DCraft writes for the same assembly packed from the parts
    /// without their pictures, and loads to that document.
    #[test]
    fn an_assembly_is_copied_without_the_thumbnails_of_its_packed_parts() {
        // The pictures give the parts' hashes an order the copies do not
        // keep (seeded until they swap, so the list must be sorted again).
        fn before(a: &[u8], b: &[u8]) -> bool {
            craft3d_doc::sha256(a) < craft3d_doc::sha256(b)
        }
        let (jaw, jaw_bytes) = part_file("Jaw", Some(1));
        let (lever, mut lever_bytes) = part_file("Lever", Some(2));
        let mut seed: u8 = 2;
        let bare = |p: &Document| craft3d_format::to_bytes(p).unwrap();
        while before(&jaw_bytes, &lever_bytes) == before(&bare(&jaw), &bare(&lever)) {
            seed += 1;
            lever_bytes = part_file("Lever", Some(seed)).1;
        }
        let (pin, pin_bytes) = part_file("Pin", Some(200));
        let (washer, washer_bytes) = part_file("Washer", None);
        let sub = assembly_of(&[&pin_bytes, &washer_bytes]);
        let sub_bytes =
            craft3d_format::to_bytes_with_thumbnail(&sub, Some(&picture(9, 50))).unwrap();
        let top = assembly_of(&[&jaw_bytes, &lever_bytes, &sub_bytes]);
        let source = craft3d_format::to_bytes_with_thumbnail(&top, Some(&picture(5, 80))).unwrap();

        let (copy, left) = without_thumbnails(&source).unwrap();
        assert_eq!(
            left.thumbnails, 5,
            "the assembly's, three parts', the subassembly's"
        );
        assert_eq!((source.len() - copy.len()) as u64, left.saved());
        check_copy(&source, &copy).unwrap();
        // The same assembly packed from the parts without their pictures.
        let sub_bare = assembly_of(&[&bare(&pin), &bare(&washer)]);
        let expected = assembly_of(&[&bare(&jaw), &bare(&lever), &bare(&sub_bare)]);
        assert_eq!(entries_of(&copy), entries_of(&bare(&expected)));
        assert_eq!(craft3d_format::from_bytes(&copy).unwrap(), expected);
        // The part without a picture kept its name.
        let washer_name = format!("blobs/{}.3dcraft", hex(&craft3d_doc::sha256(&washer_bytes)));
        let sub_entries = entries_of(
            &entries_of(&copy)[&format!(
                "blobs/{}.3dcraft",
                hex(&craft3d_doc::sha256(&bare(&sub_bare)))
            )],
        );
        assert!(sub_entries.contains_key(&washer_name), "{sub_entries:?}");
        // Copying the copy changes nothing.
        assert_eq!(
            without_thumbnails(&copy).unwrap(),
            (copy.clone(), LeftOut::default())
        );
    }

    /// What the copy must not be: another document, a picture kept, a part
    /// lost; and what cannot be copied so: an archive 3DCraft would not
    /// write as it is, a hash named otherwise than whole.
    #[test]
    fn a_copy_that_is_not_its_source_without_its_pictures_is_refused() {
        let (_, jaw) = part_file("Jaw", Some(1));
        let (_, other) = part_file("Other", Some(1));
        let (other_copy, _) = without_thumbnails(&other).unwrap();
        assert!(check_copy(&jaw, &other_copy).is_err(), "another document");
        assert!(check_copy(&jaw, &jaw).unwrap_err().contains("thumb.png"));
        let top = craft3d_format::to_bytes(&assembly_of(&[&jaw])).unwrap();
        let (top_copy, _) = without_thumbnails(&top).unwrap();
        check_copy(&top, &top_copy).unwrap();
        // The packed part left as it was (with its picture): refused.
        let error = check_copy(&top, &top).unwrap_err();
        assert!(error.contains("lacks the document packed"), "{error}");

        // Entries in any order the writer writes are kept; an archive with
        // what the writer never writes (a comment) is refused.
        let mut entries: Vec<(String, Vec<u8>)> = entries_of(&jaw).into_iter().collect();
        entries.reverse();
        let pairs: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        let reordered = craft3d_zipstore::write_stored(&pairs).unwrap();
        assert!(without_thumbnails(&reordered).is_ok());
        let mut commented = jaw.clone();
        commented.truncate(jaw.len() - 2);
        commented.extend_from_slice(&[3, 0, b'a', b'b', b'c']);
        assert!(craft3d_zipstore::read_stored(&commented).is_ok());
        let error = without_thumbnails(&commented).unwrap_err();
        assert!(error.contains("not as 3DCraft writes"), "{error}");

        // A hash named inside a longer string.
        let hash = "ab".repeat(32);
        let manifest = format!("{{\"packed\": [\n  \"{hash}\"\n], \"note\": \"x{hash}\"}}");
        let error = renamed_manifest(manifest.as_bytes(), &[(hash, "cd".repeat(32))]).unwrap_err();
        assert!(error.contains("whole SHA-256"), "{error}");
    }

    /// The list `packed` is sorted where it stands, its layout kept; a
    /// manifest without one list, or with something else in it, is refused.
    #[test]
    fn the_packed_list_is_sorted_in_place() {
        let (a, b, c) = ("aa".repeat(32), "bb".repeat(32), "cc".repeat(32));
        let text = format!(
            "{{\n  \"assembly\": {{\n    \"instances\": [{{\"source\": {{\"packed\": {{\"hash\": \"{c}\"}}}}}}],\n    \"packed\": [\n      \"{c}\",\n      \"{a}\",\n      \"{b}\"\n    ]\n  }}\n}}\n"
        );
        let sorted = sorted_packed(&text).unwrap();
        assert_eq!(
            sorted,
            format!(
                "{{\n  \"assembly\": {{\n    \"instances\": [{{\"source\": {{\"packed\": {{\"hash\": \"{c}\"}}}}}}],\n    \"packed\": [\n      \"{a}\",\n      \"{b}\",\n      \"{c}\"\n    ]\n  }}\n}}\n"
            )
        );
        assert!(sorted_packed("{\"instances\": []}").is_err());
        let twice = format!("\"packed\": [\"{a}\"], \"packed\": [\"{b}\"]");
        assert!(sorted_packed(&twice).is_err());
        let not_a_hash = format!("\"packed\": [\"{a}\", 3]");
        assert!(sorted_packed(&not_a_hash).is_err());
        let unended = format!("\"packed\": [\"{a}\"");
        assert!(sorted_packed(&unended).is_err());
    }
}
