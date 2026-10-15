//! Reading and writing `.3dcraft` files on disk. Not compiled for wasm,
//! where the platform hands the app bytes instead of paths.

use crate::{FormatError, MAX_FILE_BYTES, to_bytes};
use craft3d_doc::Document;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Makes the temporary names of concurrent saves in one process differ.
static SAVE_COUNTER: AtomicU32 = AtomicU32::new(0);

/// How many temporary names are tried before giving up. A name is only
/// taken when an earlier process with the same id crashed in mid-save.
const TEMP_NAME_ATTEMPTS: u32 = 16;

/// Saves the document to `path`, atomically: the bytes go to a temporary
/// file in the same directory, are flushed to disk, and the temporary file
/// is renamed over `path`. Readers see the old file or the new one, never
/// half of either; on any failure the temporary file is removed and an
/// existing file at `path` is untouched.
///
/// The directory must exist. The new file gets default permissions, not
/// those of the file it replaces.
pub fn save(path: &Path, document: &Document) -> Result<(), FormatError> {
    // Serialise first: a document that cannot be saved touches nothing.
    let bytes = to_bytes(document)?;
    write_atomically(path, &bytes).map_err(|error| FormatError::Io {
        action: "save",
        path: path.to_path_buf(),
        error,
    })
}

/// Loads the document at `path`. Files larger than [`MAX_FILE_BYTES`] are
/// refused before they are read.
pub fn load(path: &Path) -> Result<Document, FormatError> {
    load_noted(path).map(|(document, _)| document)
}

/// As [`load`], with what was changed on reading
/// ([`crate::from_manifest_json_noted`]).
pub fn load_noted(path: &Path) -> Result<(Document, Vec<String>), FormatError> {
    let io = |action: &'static str| {
        move |error: io::Error| FormatError::Io {
            action,
            path: path.to_path_buf(),
            error,
        }
    };
    let too_large = |size: u64| FormatError::TooLarge {
        what: "the file",
        size,
        max: MAX_FILE_BYTES,
    };
    let file = File::open(path).map_err(io("open"))?;
    let size = file.metadata().map_err(io("read"))?.len();
    if size > MAX_FILE_BYTES {
        return Err(too_large(size));
    }
    // The size above is a promise the file system need not keep (the file
    // can grow, and some files have no size), so the read is capped too.
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io("read"))?;
    let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if read > MAX_FILE_BYTES {
        return Err(too_large(read));
    }
    crate::from_bytes_noted(&bytes)
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))?;
    // `parent()` of a bare file name is the empty path: the current
    // directory.
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let (temp_path, mut temp) = create_temp(directory, name)?;
    let written = temp
        .write_all(bytes)
        // Data and metadata on disk before the rename, or a crash could
        // leave the new name pointing at an empty file.
        .and_then(|()| temp.sync_all());
    drop(temp);
    let renamed = written.and_then(|()| fs::rename(&temp_path, path));
    if renamed.is_err() {
        // Nothing useful can be done if this fails too; the error that
        // matters is the first one.
        let _ = fs::remove_file(&temp_path);
        return renamed;
    }
    // Make the rename itself durable. Best effort: directories cannot be
    // opened for this on every platform, and the save has happened.
    if let Ok(directory) = File::open(directory) {
        let _ = directory.sync_all();
    }
    Ok(())
}

/// Creates a new, empty file next to the target: `.<name>.<pid>-<n>.tmp`.
/// Hidden on Unix, and never an existing file (`create_new`), so nothing of
/// the user's is overwritten.
fn create_temp(directory: &Path, name: &std::ffi::OsStr) -> io::Result<(PathBuf, File)> {
    let mut last = io::Error::other("no temporary file name was tried");
    for _ in 0..TEMP_NAME_ATTEMPTS {
        let n = SAVE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = OsString::from(".");
        temp_name.push(name);
        temp_name.push(format!(".{}-{n}.tmp", std::process::id()));
        let temp_path = directory.join(temp_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
}
