//! The native `.3dcraft` file: the document, saved as the user wrote it.
//!
//! A `.3dcraft` file is a ZIP archive of stored (uncompressed) entries
//! (`craft3d-zipstore`) with one entry that matters, `manifest.json`:
//!
//! ```json
//! { "format": "3dcraft", "format_version": 1, "document": { ... } }
//! ```
//!
//! pretty-printed, so that it can be read, diffed and, in an emergency,
//! written by a person or an agent (docs/file-formats.md). Expressions are
//! stored as their source text, never as evaluated numbers, and lists are
//! stored in the order the document has them: a loaded file regenerates to
//! the bodies it was saved from.
//!
//! - **The file has its own types** ([`manifest`]), separate from
//!   `craft3d-doc`, and `convert.rs` maps between them by destructuring
//!   every document struct exhaustively. A field added to the document
//!   model does not compile until it is saved and loaded (AGENTS.md §8).
//! - **Nothing is dropped silently.** A field this version does not know is
//!   an error; a file of a newer format version is an error that says a
//!   newer 3DCraft is needed; an older one is brought up to date by
//!   [`migrate`].
//! - **Imported files are stored beside the manifest** (format version
//!   25): the bytes of each STEP file an `import` feature reads, once per
//!   content, as the entry `blobs/<sha256>.step`, named by the SHA-256 of
//!   its bytes, which the manifest's feature names. A blob the manifest
//!   names must be there and hash to its name, or the file is refused; a
//!   blob nobody names is not read.
//! - **A thumbnail may be stored beside the manifest** (format version
//!   32): the entry `thumb.png`, a small picture of the model for a
//!   gallery or a file browser, given by the caller
//!   ([`to_bytes_with_thumbnail`]; the engine renders it for a Jig). It is
//!   derived data: loading ignores it, as an older build does, and
//!   [`thumbnail`] reads it.
//! - **Other ZIP entries are ignored on load and not preserved on save.**
//!   Re-saving a file that has extra entries writes a file without them.
//! - **A loaded document obeys the model's rules:** it has passed
//!   `Document::validate`, so a file edited by hand cannot smuggle in
//!   duplicate ids or a dangling rollback marker. Saving validates too, so
//!   a file this crate wrote is a file this crate loads.
//! - **Saving is atomic** ([`save`]): a crash or a full disk leaves the
//!   previous file as it was.
//! - **Input is hostile.** Sizes are capped ([`MAX_FILE_BYTES`],
//!   [`MAX_MANIFEST_BYTES`]) before anything is parsed, and no input makes
//!   this crate panic.
//!
//! Not here yet: the directory form of a document (docs/file-formats.md).

mod convert;
pub mod manifest;
pub mod migrate;

use craft3d_doc::{DocError, Document};
use craft3d_zipstore::ZipError;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

/// The format version this build writes, and the newest it reads.
pub const FORMAT_VERSION: u32 = 64;

/// The version that made `floor`, `ceil`, `round`, `trunc` and `mod`
/// function names: a variable of an older file with such a name is
/// renamed on reading (`migrate::v16_to_v17`).
const FORMAT_VERSION_OF_WHOLE_NUMBER_FUNCTIONS: u32 = 17;

/// The version that made the names of the condition functions (`if`,
/// `clamp`, `choose`) and of `exp`, `ln`, `log10`, `pow`, `sinh`, `cosh`
/// and `tanh` function names: a variable of an older file with such a
/// name is renamed on reading (`migrate::v30_to_v31`).
const FORMAT_VERSION_OF_CONDITION_FUNCTIONS: u32 = 31;

/// The version that made `asinh`, `acosh`, `atanh`, `log2`, `hypot` and
/// `sign` function names (`migrate::v36_to_v37`).
const FORMAT_VERSION_OF_INVERSE_HYPERBOLIC_FUNCTIONS: u32 = 37;

/// The version that renamed a sheet-metal part's edges and rims
/// (`migrate::v48_to_v49`): a stored name of one in an
/// older file is reported on reading.
const FORMAT_VERSION_OF_SHEET_METAL_EDGE_NAMES: u32 = 49;

/// The format version since which `name` is reserved in expressions, for
/// the warning that a variable of an older file was renamed.
fn reserved_since(name: &str) -> u32 {
    match name {
        "floor" | "ceil" | "round" | "trunc" | "mod" => FORMAT_VERSION_OF_WHOLE_NUMBER_FUNCTIONS,
        "asinh" | "acosh" | "atanh" | "log2" | "hypot" | "sign" => {
            FORMAT_VERSION_OF_INVERSE_HYPERBOLIC_FUNCTIONS
        }
        _ => FORMAT_VERSION_OF_CONDITION_FUNCTIONS,
    }
}

/// The file extension, without the dot.
pub const EXTENSION: &str = "3dcraft";

/// The extension of a published Jig, without the dot: the same format as
/// a `.3dcraft` file, opened to be used (its parameters set) rather than
/// edited (docs/decisions.md, 2026-10-23).
pub const JIG_EXTENSION: &str = "3dcraftjig";

/// Whether `path` names a published Jig: its file name ends in
/// `.3dcraftjig`, in any case, trailing dots and spaces left out (Windows
/// drops them from a name, so `box.3dcraftjig.` is that file).
pub fn is_jig_path(path: &str) -> bool {
    let trimmed = path.trim_end_matches(['.', ' ']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case(JIG_EXTENSION))
}

/// The extension of an assembly, without the dot: the same container and
/// format versions as a `.3dcraft` file, with a document whose `assembly`
/// is set (docs/decisions.md, 2026-10-29). A `.3dcraft` file never holds
/// an assembly.
pub const ASSEMBLY_EXTENSION: &str = "3dcraftasm";

/// Whether `path` names an assembly: its file name ends in `.3dcraftasm`,
/// in any case, trailing dots and spaces left out (as [`is_jig_path`]).
pub fn is_assembly_path(path: &str) -> bool {
    let trimmed = path.trim_end_matches(['.', ' ']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case(ASSEMBLY_EXTENSION))
}

/// Name of the ZIP entry that holds the document.
pub const MANIFEST_NAME: &str = "manifest.json";

/// Name of the ZIP entry that holds the thumbnail, a PNG picture.
pub const THUMBNAIL_NAME: &str = "thumb.png";

/// Largest thumbnail that is written or read: 4 MiB. The engine's are a
/// few tens of kilobytes.
pub const MAX_THUMBNAIL_BYTES: usize = 4 * 1024 * 1024;

/// Where the bytes of imported files are: `blobs/<sha256>.step`.
pub const BLOB_DIR: &str = "blobs/";
const BLOB_EXTENSION: &str = ".step";
/// Where the source documents packed into an assembly are:
/// `blobs/<sha256>.3dcraft` (format version 52).
const PACKED_EXTENSION: &str = ".3dcraft";
/// Where the fonts a document keeps are: `blobs/<sha256>.font` (format
/// version 55).
const FONT_EXTENSION: &str = ".font";

/// Largest `.3dcraft` file that is read: 256 MiB. Checked before the file
/// is read or parsed.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Largest `manifest.json` that is read or written: 64 MiB, a few hundred
/// bytes for each of the 100 000 features a document may hold. Parsing
/// JSON takes several times its size in memory, so this is what bounds the
/// memory a hostile file can claim.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;

/// Why a document could not be saved or loaded. The messages are written
/// for the person or agent who has to act on them.
#[derive(Debug)]
pub enum FormatError {
    /// The file system refused. `action` is what was being done, as a verb
    /// phrase ("write", "open").
    Io {
        action: &'static str,
        path: PathBuf,
        error: std::io::Error,
    },
    /// The file or its manifest is larger than this crate reads or writes.
    TooLarge {
        what: &'static str,
        size: u64,
        max: u64,
    },
    /// The file is not a ZIP archive this crate can read.
    Zip(ZipError),
    /// The file is not a ZIP archive but a file of another format, told
    /// by how it starts (a STEP or STL file opened as a document).
    Foreign {
        /// What it is, e.g. "a STEP file (ISO 10303-21)".
        kind: &'static str,
        /// What to do with it instead.
        hint: &'static str,
    },
    /// The archive has no `manifest.json`.
    MissingManifest,
    /// `manifest.json` is not JSON. The message has the line and column.
    Json(String),
    /// `manifest.json` is JSON but not an object.
    NotAnObject,
    /// The `format` field is missing (`None`) or is not `"3dcraft"`.
    WrongFormat { found: Option<String> },
    /// The `format_version` field is missing or not a positive whole
    /// number. The string says which.
    BadVersion(String),
    /// The file was written by a newer 3DCraft.
    NewerVersion { file: u64, supported: u32 },
    /// The step from version `from` to `from + 1` could not make sense of
    /// the file.
    Migration { from: u32, reason: String },
    /// The manifest has a missing field, an unknown field or a value of
    /// the wrong type. `path` is where, such as `document.features[2].kind`;
    /// it is empty for the top level of the manifest.
    Shape { path: String, message: String },
    /// The file parses, but describes a document that breaks the model's
    /// rules.
    InvalidDocument(DocError),
    /// The document to save breaks the model's rules; writing it would
    /// give a file that does not load.
    CannotSave(DocError),
    /// The manifest names an imported file the archive does not hold.
    MissingBlob { sha256: String },
    /// An imported file's bytes do not hash to the name of their entry.
    DamagedBlob { name: String },
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Io {
                action,
                path,
                error,
            } => write!(f, "cannot {action} `{}`: {error}", path.display()),
            FormatError::TooLarge { what, size, max } => write!(
                f,
                "{what} is {size} bytes; 3DCraft reads and writes at most {max} bytes"
            ),
            FormatError::Zip(error) => write!(
                f,
                "this is not a readable .3dcraft file: {error}. A .3dcraft file is a ZIP archive \
                 of uncompressed entries; if it was repacked by hand, store the entries without \
                 compression (`zip -0`)"
            ),
            FormatError::Foreign { kind, hint } => {
                write!(f, "this is not a .3dcraft file but {kind}; {hint}")
            }
            FormatError::MissingManifest => write!(
                f,
                "this is not a .3dcraft file: the archive has no `{MANIFEST_NAME}` entry"
            ),
            FormatError::Json(error) => {
                write!(f, "`{MANIFEST_NAME}` is not valid JSON: {error}")
            }
            FormatError::NotAnObject => write!(
                f,
                "`{MANIFEST_NAME}` must be a JSON object with the fields `format`, \
                 `format_version` and `document`"
            ),
            FormatError::WrongFormat { found: None } => write!(
                f,
                "this is not a .3dcraft file: `{MANIFEST_NAME}` has no `format` field (expected \
                 \"format\": \"{}\")",
                manifest::FORMAT_NAME
            ),
            FormatError::WrongFormat { found: Some(found) } => write!(
                f,
                "this is not a .3dcraft file: the `format` field of `{MANIFEST_NAME}` is {found}, \
                 expected \"{}\"",
                manifest::FORMAT_NAME
            ),
            FormatError::BadVersion(problem) => write!(
                f,
                "the `format_version` field of `{MANIFEST_NAME}` is not usable: {problem} (this \
                 build of 3DCraft reads format versions up to {FORMAT_VERSION})"
            ),
            FormatError::NewerVersion { file, supported } => write!(
                f,
                "this file has format version {file}, and this build of 3DCraft reads up to \
                 version {supported}: a newer 3DCraft is needed to open it"
            ),
            FormatError::Migration { from, reason } => write!(
                f,
                "cannot bring the file from format version {from} to {}: {reason}",
                from.saturating_add(1)
            ),
            FormatError::Shape { path, message } if path.is_empty() => write!(
                f,
                "`{MANIFEST_NAME}` does not have the expected content: {message}"
            ),
            FormatError::Shape { path, message } => write!(
                f,
                "`{MANIFEST_NAME}` does not have the expected content at `{path}`: {message}"
            ),
            FormatError::InvalidDocument(error) => write!(
                f,
                "the file describes a document that breaks the model's rules: {error}. Correct \
                 `{MANIFEST_NAME}`, or open an earlier copy of the file"
            ),
            FormatError::CannotSave(error) => write!(
                f,
                "the document cannot be saved because it breaks the model's rules: {error}. The \
                 file on disk was not changed"
            ),
            FormatError::MissingBlob { sha256 } => write!(
                f,
                "the file has an imported STEP file, a packed source document or a font whose \
                 bytes are missing: there is no entry `{BLOB_DIR}{sha256}{BLOB_EXTENSION}` (or \
                 `{PACKED_EXTENSION}`, `{FONT_EXTENSION}`); open an earlier copy of the file"
            ),
            FormatError::DamagedBlob { name } => write!(
                f,
                "the imported file `{name}` is damaged: its bytes do not match the SHA-256 it is \
                 named by; open an earlier copy of the file"
            ),
        }
    }
}

impl std::error::Error for FormatError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FormatError::Io { error, .. } => Some(error),
            FormatError::Zip(error) => Some(error),
            FormatError::InvalidDocument(error) | FormatError::CannotSave(error) => Some(error),
            FormatError::TooLarge { .. }
            | FormatError::Foreign { .. }
            | FormatError::MissingManifest
            | FormatError::Json(_)
            | FormatError::NotAnObject
            | FormatError::WrongFormat { .. }
            | FormatError::BadVersion(_)
            | FormatError::NewerVersion { .. }
            | FormatError::Migration { .. }
            | FormatError::Shape { .. }
            | FormatError::MissingBlob { .. }
            | FormatError::DamagedBlob { .. } => None,
        }
    }
}

fn too_large(what: &'static str, size: usize, max: u64) -> FormatError {
    FormatError::TooLarge {
        what,
        size: u64::try_from(size).unwrap_or(u64::MAX),
        max,
    }
}

/// The text of `manifest.json` for a document: pretty-printed JSON, with a
/// final newline. The same document always gives the same text.
///
/// Fails when the document breaks its own rules (`Document::validate`) or
/// the text would be larger than [`MAX_MANIFEST_BYTES`].
pub fn to_manifest_json(document: &Document) -> Result<String, FormatError> {
    // Also what keeps non-finite tolerances, which JSON cannot hold, out
    // of the file.
    document.validate().map_err(FormatError::CannotSave)?;
    let manifest = manifest::Manifest {
        format: manifest::FORMAT_NAME.to_string(),
        format_version: FORMAT_VERSION,
        document: convert::to_manifest(document),
    };
    // Cannot fail for these types (string keys, no custom serialisers), but
    // that is serde's promise, not ours.
    let mut json =
        serde_json::to_string_pretty(&manifest).map_err(|e| FormatError::Json(e.to_string()))?;
    json.push('\n');
    if json.len() > MAX_MANIFEST_BYTES {
        return Err(too_large(
            "the document's `manifest.json`",
            json.len(),
            MAX_MANIFEST_BYTES as u64,
        ));
    }
    Ok(json)
}

/// The bytes of a `.3dcraft` file for a document. Deterministic: the same
/// document gives the same bytes, on every platform.
pub fn to_bytes(document: &Document) -> Result<Vec<u8>, FormatError> {
    to_bytes_with_thumbnail(document, None)
}

/// [`to_bytes`] with a thumbnail, the bytes of a PNG picture stored as
/// the entry [`THUMBNAIL_NAME`] right after the manifest. Deterministic
/// as `to_bytes` is, for the same thumbnail bytes; whether the picture
/// itself is the same on every platform is the renderer's matter (the
/// engine promises it on one platform and build only).
pub fn to_bytes_with_thumbnail(
    document: &Document,
    thumbnail: Option<&[u8]>,
) -> Result<Vec<u8>, FormatError> {
    let json = to_manifest_json(document)?;
    let blobs = blobs_of(document);
    if let Some(png) = thumbnail
        && png.len() > MAX_THUMBNAIL_BYTES
    {
        return Err(too_large(
            "the thumbnail",
            png.len(),
            MAX_THUMBNAIL_BYTES as u64,
        ));
    }
    // A file larger than this crate reads would be a document that cannot
    // be opened again: refused before anything is written.
    let size = file_size(
        json.len(),
        thumbnail
            .map(|png| (THUMBNAIL_NAME.len(), png.len()))
            .into_iter()
            .chain(blobs.iter().map(|(name, blob)| (name.len(), blob.len()))),
    );
    if size > MAX_FILE_BYTES {
        return Err(FormatError::TooLarge {
            what: "the document with its imported files",
            size,
            max: MAX_FILE_BYTES,
        });
    }
    let mut entries: Vec<(&str, &[u8])> = vec![(MANIFEST_NAME, json.as_bytes())];
    if let Some(png) = thumbnail {
        entries.push((THUMBNAIL_NAME, png));
    }
    entries.extend(
        blobs
            .iter()
            .map(|(name, blob)| (name.as_str(), blob.bytes())),
    );
    craft3d_zipstore::write_stored(&entries).map_err(FormatError::Zip)
}

/// Each imported file of the document once, by its entry name, in order.
fn blobs_of(document: &Document) -> BTreeMap<String, &craft3d_doc::Blob> {
    let mut blobs: BTreeMap<String, &craft3d_doc::Blob> = BTreeMap::new();
    for feature in &document.features {
        if let craft3d_doc::FeatureKind::Import(import) = &feature.kind {
            blobs
                .entry(format!(
                    "{BLOB_DIR}{}{BLOB_EXTENSION}",
                    import.data.sha256()
                ))
                .or_insert(&import.data);
        }
    }
    if let Some(assembly) = &document.assembly {
        for blob in &assembly.packed {
            blobs
                .entry(format!("{BLOB_DIR}{}{PACKED_EXTENSION}", blob.sha256()))
                .or_insert(blob);
        }
    }
    // The fonts a document may keep (never one only this session has).
    for font in &document.fonts {
        if let Some(blob) = &font.blob {
            blobs
                .entry(format!("{BLOB_DIR}{}{FONT_EXTENSION}", blob.sha256()))
                .or_insert(blob);
        }
    }
    blobs
}

/// Bytes a stored ZIP of a manifest of `manifest` bytes and the given
/// entries (name length, data length) takes: the data, and per entry its
/// name twice and the headers (30 + 46 bytes), and the end record (22).
fn file_size(manifest: usize, entries: impl Iterator<Item = (usize, usize)>) -> u64 {
    let entry = |name: usize, data: usize| {
        (data as u64)
            .saturating_add(2 * name as u64)
            .saturating_add(76)
    };
    entries.fold(
        entry(MANIFEST_NAME.len(), manifest).saturating_add(22),
        |total, (n, d)| total.saturating_add(entry(n, d)),
    )
}

/// How large the `.3dcraft` file of `document` would be with one more
/// imported file of `extra` bytes (not counted when the document holds
/// those bytes already) and `features` more features of some 300 bytes:
/// for a command to refuse an import that would make a document that
/// cannot be saved and opened again ([`MAX_FILE_BYTES`]).
pub fn size_with(
    document: &Document,
    extra: &craft3d_doc::Blob,
    features: usize,
) -> Result<u64, FormatError> {
    let json = to_manifest_json(document)?;
    let mut blobs = blobs_of(document);
    let name = format!("{BLOB_DIR}{}{BLOB_EXTENSION}", extra.sha256());
    blobs.entry(name).or_insert(extra);
    let manifest = json.len().saturating_add(features.saturating_mul(300));
    Ok(file_size(
        manifest,
        blobs.iter().map(|(n, b)| (n.len(), b.len())),
    ))
}

/// Reads the bytes of a `.3dcraft` file. Entries other than
/// `manifest.json` are ignored.
pub fn from_bytes(bytes: &[u8]) -> Result<Document, FormatError> {
    from_bytes_noted(bytes).map(|(document, _)| document)
}

/// As [`from_bytes`], with what was changed on reading
/// ([`from_manifest_json_noted`]).
pub fn from_bytes_noted(bytes: &[u8]) -> Result<(Document, Vec<String>), FormatError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(too_large("the file", bytes.len(), MAX_FILE_BYTES));
    }
    let entries = craft3d_zipstore::read_stored(bytes)
        .map_err(|error| foreign(bytes).unwrap_or(FormatError::Zip(error)))?;
    let manifest = entries
        .iter()
        .find(|entry| entry.name == MANIFEST_NAME)
        .ok_or(FormatError::MissingManifest)?;
    // The bytes of the entries that may be imported files, by hash; only
    // those the manifest names are hashed and read (`read_manifest`).
    let mut raw: BTreeMap<String, (&str, &[u8])> = BTreeMap::new();
    for entry in &entries {
        if let Some(hash) = entry.name.strip_prefix(BLOB_DIR).and_then(|rest| {
            rest.strip_suffix(BLOB_EXTENSION)
                .or_else(|| rest.strip_suffix(PACKED_EXTENSION))
                .or_else(|| rest.strip_suffix(FONT_EXTENSION))
        }) {
            raw.insert(hash.to_string(), (entry.name, entry.data));
        }
    }
    read_manifest(manifest.data, &raw)
}

/// The thumbnail of a `.3dcraft` file ([`THUMBNAIL_NAME`]): the bytes of
/// its PNG picture, or `None` when the file has none (or one larger than
/// [`MAX_THUMBNAIL_BYTES`]). The manifest is not read, so this works on a
/// file of any format version.
pub fn thumbnail(bytes: &[u8]) -> Result<Option<Vec<u8>>, FormatError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(too_large("the file", bytes.len(), MAX_FILE_BYTES));
    }
    let entries = craft3d_zipstore::read_stored(bytes)
        .map_err(|error| foreign(bytes).unwrap_or(FormatError::Zip(error)))?;
    Ok(entries
        .iter()
        .find(|entry| entry.name == THUMBNAIL_NAME)
        .filter(|entry| entry.data.len() <= MAX_THUMBNAIL_BYTES)
        .map(|entry| entry.data.to_vec()))
}

/// What a file that is not a ZIP archive is instead, by how it starts.
fn foreign(bytes: &[u8]) -> Option<FormatError> {
    let start = bytes.get(..bytes.len().min(512))?;
    let text = String::from_utf8_lossy(start);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with("ISO-10303-21") {
        return Some(FormatError::Foreign {
            kind: "a STEP file (ISO 10303-21)",
            hint: "import it into a document with `file.import`, which reads its solids \
                   as bodies; 3DCraft writes STEP with `file.export`",
        });
    }
    if text.starts_with("solid") && text.contains("facet") {
        return Some(FormatError::Foreign {
            kind: "an STL mesh",
            hint: "reading meshes into a document is not built yet; \
                   3DCraft writes STL with `file.export`",
        });
    }
    None
}

/// A short description of a JSON value for an error message: hostile files
/// do not get to put megabytes into it.
fn describe(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) if s.chars().count() <= 40 => format!("\"{s}\""),
        Value::String(s) => format!("\"{}...\"", s.chars().take(40).collect::<String>()),
        Value::Array(_) => "a list".to_string(),
        Value::Object(_) => "an object".to_string(),
    }
}

/// Reads the text of a `manifest.json`: checks that it is a 3DCraft
/// manifest, migrates it from an older format version, reads it into a
/// document and validates the document.
pub fn from_manifest_json(json: &[u8]) -> Result<Document, FormatError> {
    from_manifest_json_noted(json).map(|(document, _)| document)
}

/// As [`from_manifest_json`], with a warning for each thing that was
/// changed on reading. Two such things: a positive, finite linear
/// tolerance outside the range a document may have (which a build before
/// 2026-10-15 could save) is read as the nearer end of the range
/// (`doc::Tolerance::in_range`); and a variable of a file older than
/// version 17 whose name is now a function (`floor`) is renamed with
/// every reference to it (`migrate::v16_to_v17`).
pub fn from_manifest_json_noted(json: &[u8]) -> Result<(Document, Vec<String>), FormatError> {
    read_manifest(json, &BTreeMap::new())
}

/// [`from_manifest_json_noted`] with the imported files of the archive.
fn read_manifest(
    json: &[u8],
    raw: &BTreeMap<String, (&str, &[u8])>,
) -> Result<(Document, Vec<String>), FormatError> {
    if json.len() > MAX_MANIFEST_BYTES {
        return Err(too_large(
            "`manifest.json`",
            json.len(),
            MAX_MANIFEST_BYTES as u64,
        ));
    }
    // serde_json limits nesting depth, so deep input is an error and not a
    // stack overflow, here and in everything that walks the value later.
    let value: Value =
        serde_json::from_slice(json).map_err(|e| FormatError::Json(e.to_string()))?;
    let object = value.as_object().ok_or(FormatError::NotAnObject)?;

    // Identity and version first, by hand: a file from another program or
    // from a newer 3DCraft should say so, not "unknown field".
    match object.get("format") {
        Some(Value::String(name)) if name == manifest::FORMAT_NAME => {}
        other => {
            return Err(FormatError::WrongFormat {
                found: other.map(describe),
            });
        }
    }
    let version = match object.get("format_version") {
        None => return Err(FormatError::BadVersion("it is missing".to_string())),
        Some(found) => match found.as_u64() {
            Some(version) => match u32::try_from(version) {
                Ok(version) if version <= FORMAT_VERSION => version,
                _ => {
                    return Err(FormatError::NewerVersion {
                        file: version,
                        supported: FORMAT_VERSION,
                    });
                }
            },
            None => {
                return Err(FormatError::BadVersion(format!(
                    "it is {}, not a positive whole number",
                    describe(found)
                )));
            }
        },
    };

    // Names the version can no longer read the old way are found before
    // the migration (which leaves them as they are) and reported below.
    let renamed = if version < FORMAT_VERSION_OF_SHEET_METAL_EDGE_NAMES {
        migrate::sheet_metal_edge_names(&value)
    } else {
        Vec::new()
    };
    let value = migrate::migrate(value, version)?;
    let manifest =
        manifest::Manifest::deserialize(&value).map_err(|error| shape_error(&value, &error))?;
    let manifest::Manifest {
        format: _,
        format_version: _,
        document,
    } = manifest;
    // Each imported file the manifest names, hashed once and checked
    // against its name; an entry no feature names is not read.
    let mut blobs = convert::Blobs::new();
    for feature in &document.features {
        if let manifest::FeatureKind::Import { sha256, .. } = &feature.kind
            && !blobs.contains_key(sha256)
        {
            let Some((name, data)) = raw.get(sha256) else {
                return Err(FormatError::MissingBlob {
                    sha256: sha256.clone(),
                });
            };
            let blob = craft3d_doc::Blob::new(data.to_vec());
            if blob.sha256() != *sha256 {
                return Err(FormatError::DamagedBlob {
                    name: (*name).to_string(),
                });
            }
            blobs.insert(sha256.clone(), blob);
        }
    }
    // Each document packed into an assembly, the same way.
    for sha256 in document.assembly.iter().flat_map(|a| &a.packed) {
        if blobs.contains_key(sha256) {
            continue;
        }
        let Some((name, data)) = raw.get(sha256) else {
            return Err(FormatError::MissingBlob {
                sha256: sha256.clone(),
            });
        };
        let blob = craft3d_doc::Blob::new(data.to_vec());
        if blob.sha256() != *sha256 {
            return Err(FormatError::DamagedBlob {
                name: (*name).to_string(),
            });
        }
        blobs.insert(sha256.clone(), blob);
    }
    // Each font the file keeps, the same way.
    for font in document.fonts.iter().filter(|f| f.embedded) {
        if blobs.contains_key(&font.sha256) {
            continue;
        }
        let Some((name, data)) = raw.get(&font.sha256) else {
            return Err(FormatError::MissingBlob {
                sha256: font.sha256.clone(),
            });
        };
        let blob = craft3d_doc::Blob::new(data.to_vec());
        if blob.sha256() != font.sha256 {
            return Err(FormatError::DamagedBlob {
                name: (*name).to_string(),
            });
        }
        blobs.insert(font.sha256.clone(), blob);
    }
    let blobs = &blobs;
    // An output is computed from a measure or from an expression, one of
    // the two: a file with both or neither (edited by hand) is refused by
    // the output's name rather than read as one of them.
    for output in &document.outputs {
        if output.measure.is_some() == output.expr.is_some() {
            return Err(FormatError::InvalidDocument(DocError::Invalid(format!(
                "output `{}` must have either a measure or an expression, {}",
                output.name,
                if output.measure.is_some() {
                    "not both"
                } else {
                    "and has neither"
                }
            ))));
        }
    }
    let mut document = convert::to_doc(document, blobs);
    let mut warnings = Vec::new();
    for (at, name) in renamed {
        let shown = manifest::Name::deserialize(&name).map_or_else(
            |_| name.to_string(),
            |name| convert::name_to_doc(name).to_string(),
        );
        warnings.push(format!(
            "sheet-metal edge names changed in format {FORMAT_VERSION_OF_SHEET_METAL_EDGE_NAMES}: pick the edge again: {at} refers to {shown}, which now names nothing or another edge"
        ));
    }
    let read = document.tolerance.in_range();
    if read.linear != document.tolerance.linear {
        let unit = document.units.length.symbol();
        warnings.push(format!(
            "tolerance {:e} {unit} read as {:e} {unit} (outside {:e}..{:e})",
            document.tolerance.linear,
            read.linear,
            craft3d_doc::Tolerance::LEAST_LINEAR,
            craft3d_doc::Tolerance::GREATEST_LINEAR
        ));
        document.tolerance = read;
    }
    if version < FORMAT_VERSION_OF_INVERSE_HYPERBOLIC_FUNCTIONS {
        // Tables first (tables came with version 33, after every other
        // reserved name but those of version 37).
        for (old, new) in document
            .rename_reserved_tables()
            .map_err(FormatError::InvalidDocument)?
        {
            let since = reserved_since(&old);
            warnings.push(format!(
                "table `{old}` renamed `{new}`: `{old}` is a function in expressions since format version {since}"
            ));
        }
        for (old, new) in document
            .rename_reserved_variables()
            .map_err(FormatError::InvalidDocument)?
        {
            let since = reserved_since(&old);
            warnings.push(format!(
                "variable `{old}` renamed `{new}`: `{old}` is a function in expressions since format version {since}"
            ));
        }
    }
    // Where image points lay is where the next solve starts carrying them
    // from, not part of the design: a stale or malformed place is dropped
    // (that image is then not carried), not a reason to refuse the file.
    for feature in &mut document.features {
        if let craft3d_doc::FeatureKind::Sketch(sketch) = &mut feature.kind {
            let dropped = sketch.tidy_image_places();
            if dropped > 0 {
                warnings.push(format!(
                    "{}: {dropped} image place{} dropped (not an image point, repeated or not finite)",
                    feature.id,
                    if dropped == 1 { "" } else { "s" }
                ));
            }
        }
    }
    document.validate().map_err(FormatError::InvalidDocument)?;
    Ok((document, warnings))
}

/// Turns "the manifest does not fit its types" into an error that says
/// where. serde_json reports what is wrong but, for a parsed value, not
/// the place; so the parts are read again one by one, innermost first, and
/// the first one that fails names the place. Only runs on the error path.
fn shape_error(manifest: &Value, whole: &serde_json::Error) -> FormatError {
    fn probe<'a, T: Deserialize<'a>>(value: Option<&'a Value>) -> Option<String> {
        T::deserialize(value?).err().map(|e| e.to_string())
    }
    fn items(list: Option<&Value>) -> impl Iterator<Item = (usize, &Value)> {
        list.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
    }
    let located = || {
        let document = manifest.get("document")?;
        for (index, feature) in items(document.get("features")) {
            if let Some(message) = probe::<manifest::FeatureKind>(feature.get("kind")) {
                return Some((format!("document.features[{index}].kind"), message));
            }
            if let Some(message) = probe::<manifest::Feature>(Some(feature)) {
                return Some((format!("document.features[{index}]"), message));
            }
        }
        for (index, variable) in items(document.get("variables")) {
            if let Some(message) = probe::<manifest::Variable>(Some(variable)) {
                return Some((format!("document.variables[{index}]"), message));
            }
        }
        if let Some(message) = probe::<manifest::Units>(document.get("units")) {
            return Some(("document.units".to_string(), message));
        }
        if let Some(message) = probe::<manifest::Tolerance>(document.get("tolerance")) {
            return Some(("document.tolerance".to_string(), message));
        }
        probe::<manifest::Document>(Some(document)).map(|m| ("document".to_string(), m))
    };
    let (path, message) = located().unwrap_or_else(|| (String::new(), whole.to_string()));
    FormatError::Shape { path, message }
}

#[cfg(not(target_arch = "wasm32"))]
mod disk;
#[cfg(not(target_arch = "wasm32"))]
pub use disk::{load, load_noted, save, save_with_thumbnail};
