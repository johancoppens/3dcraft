//! The built-in fonts at run time (docs/design/text.md › *Built-in fonts*).
//!
//! Font files are not in the program: the desktop app and the CLI find
//! them where the installers put them, and the web page fetches them over
//! HTTP when a text first asks for one. A built-in font's bytes must have
//! the SHA-256 `craft3d_font::shipped` pins (the built-in fonts and the
//! interface's).
//!
//! Where the desktop and the CLI look, in order: `$CRAFT3D_FONTS`; the
//! directory `fonts` beside the program; `../share/craft3d/fonts` from it
//! (Linux packages); `../Resources/fonts` from it (a macOS bundle); and
//! the repository's `assets/fonts` this crate was built from (development,
//! tests: `cargo xtask fonts` fetches them there).
//!
//! In the browser nothing is on disk: a text in a built-in font that is
//! not here yet fails with a message that says it is being fetched, and
//! the font is listed by [`wanted`]; the page fetches `fonts/<file>` and
//! hands it to [`register`], and the command is run again.
//!
//! The interface's own font (`craft3d_font::UI_FONTS`, Inter) is found
//! and fetched the same way, by [`load`], but a text cannot name it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

static LOADED: Mutex<BTreeMap<&'static str, Arc<[u8]>>> = Mutex::new(BTreeMap::new());
static WANTED: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn checked(font: &'static craft3d_font::BuiltinFont, bytes: &[u8]) -> Result<Arc<[u8]>, String> {
    let sum = hex(&craft3d_doc::sha256(bytes));
    if sum != font.sha256 {
        return Err(format!(
            "the font `{}` ({}) has the SHA-256 {sum}, not the pinned {}",
            font.name, font.file, font.sha256
        ));
    }
    Ok(Arc::from(bytes))
}

/// Hands the bytes of shipped font `name` over (the web page, once it
/// fetched them); checked against the pinned SHA-256.
pub fn register(name: &str, bytes: &[u8]) -> Result<(), String> {
    let font =
        craft3d_font::shipped_font(name).ok_or_else(|| format!("no shipped font `{name}`"))?;
    let bytes = checked(font, bytes)?;
    LOADED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(font.name, bytes);
    WANTED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(font.name);
    Ok(())
}

/// The shipped fonts asked for that are not here: name and file
/// (for the web page to fetch `fonts/<file>`).
pub fn wanted() -> Vec<(&'static str, &'static str)> {
    let wanted = WANTED.lock().unwrap_or_else(PoisonError::into_inner);
    craft3d_font::shipped()
        .filter(|f| wanted.contains(f.name))
        .map(|f| (f.name, f.file))
        .collect()
}

/// Whether shipped font `name` is here (loaded, or registered by the
/// page).
pub fn ready(name: &str) -> bool {
    LOADED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(name)
}

#[cfg(not(target_arch = "wasm32"))]
fn places() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut out = Vec::new();
    if let Some(dir) = std::env::var_os("CRAFT3D_FONTS").filter(|d| !d.is_empty()) {
        out.push(PathBuf::from(dir));
    }
    if let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
    {
        out.push(exe.join("fonts"));
        out.push(exe.join("../share/craft3d/fonts"));
        out.push(exe.join("../Resources/fonts"));
    }
    out.push(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/fonts"
    )));
    out
}

/// The bytes of shipped font `name` (built-in or the interface's), for
/// the interface: read from where the fonts are installed, or in the
/// browser listed by [`wanted`] for the page to fetch (an `Err` until it
/// is here).
pub fn load(name: &str) -> Result<Arc<[u8]>, String> {
    bytes(name)
}

/// The bytes of shipped font `name`.
pub(crate) fn bytes(name: &str) -> Result<Arc<[u8]>, String> {
    let font =
        craft3d_font::shipped_font(name).ok_or_else(|| format!("no shipped font `{name}`"))?;
    if let Some(found) = LOADED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(font.name)
    {
        return Ok(Arc::clone(found));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut tried = Vec::new();
        for dir in places() {
            let path = dir.join(font.file);
            let limit = u64::try_from(craft3d_font::MAX_FONT_BYTES).unwrap_or(u64::MAX);
            match std::fs::metadata(&path) {
                Ok(meta) if meta.len() <= limit => {}
                _ => {
                    tried.push(dir.display().to_string());
                    continue;
                }
            }
            let Ok(read) = std::fs::read(&path) else {
                tried.push(dir.display().to_string());
                continue;
            };
            let bytes = checked(font, &read)?;
            LOADED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(font.name, Arc::clone(&bytes));
            return Ok(bytes);
        }
        Err(format!(
            "the font `{name}` ({}) is not installed: it was looked for in {}; set CRAFT3D_FONTS to the directory that holds it (a source checkout gets it with `cargo xtask fonts`)",
            font.file,
            tried.join(", ")
        ))
    }
    #[cfg(target_arch = "wasm32")]
    {
        WANTED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(font.name);
        Err(format!(
            "the font `{name}` is being fetched; run the command again in a moment"
        ))
    }
}

/// Marks built-in font `name` as wanted (what the web build does when a
/// text asks for one that is not here): for the page's test of the lazy
/// fetch on a native build.
pub fn want(name: &str) -> Result<(), String> {
    let font = craft3d_font::builtin(name).ok_or_else(|| format!("no built-in font `{name}`"))?;
    if !ready(font.name) {
        WANTED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(font.name);
    }
    Ok(())
}
