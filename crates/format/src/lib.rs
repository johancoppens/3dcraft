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
//! - **Other ZIP entries are ignored on load and not preserved on save.**
//!   Later versions add `thumb.png` and `blobs/`, which are derived data.
//!   Until then, re-saving a file that has extra entries writes a file
//!   without them.
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
//! Not here yet: the directory form of a document, the thumbnail and the
//! blob store (docs/file-formats.md).

mod convert;
pub mod manifest;
pub mod migrate;

use craft3d_doc::{DocError, Document};
use craft3d_zipstore::ZipError;
use serde::Deserialize;
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;

/// The format version this build writes, and the newest it reads.
pub const FORMAT_VERSION: u32 = 5;

/// The file extension, without the dot.
pub const EXTENSION: &str = "3dcraft";

/// Name of the ZIP entry that holds the document.
pub const MANIFEST_NAME: &str = "manifest.json";

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
            | FormatError::MissingManifest
            | FormatError::Json(_)
            | FormatError::NotAnObject
            | FormatError::WrongFormat { .. }
            | FormatError::BadVersion(_)
            | FormatError::NewerVersion { .. }
            | FormatError::Migration { .. }
            | FormatError::Shape { .. } => None,
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
    let json = to_manifest_json(document)?;
    craft3d_zipstore::write_stored(&[(MANIFEST_NAME, json.as_bytes())]).map_err(FormatError::Zip)
}

/// Reads the bytes of a `.3dcraft` file. Entries other than
/// `manifest.json` are ignored.
pub fn from_bytes(bytes: &[u8]) -> Result<Document, FormatError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(too_large("the file", bytes.len(), MAX_FILE_BYTES));
    }
    let entries = craft3d_zipstore::read_stored(bytes).map_err(FormatError::Zip)?;
    let manifest = entries
        .iter()
        .find(|entry| entry.name == MANIFEST_NAME)
        .ok_or(FormatError::MissingManifest)?;
    from_manifest_json(manifest.data)
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

    let value = migrate::migrate(value, version)?;
    let manifest =
        manifest::Manifest::deserialize(&value).map_err(|error| shape_error(&value, &error))?;
    let manifest::Manifest {
        format: _,
        format_version: _,
        document,
    } = manifest;
    let document = convert::to_doc(document);
    document.validate().map_err(FormatError::InvalidDocument)?;
    Ok(document)
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
pub use disk::{load, save};
