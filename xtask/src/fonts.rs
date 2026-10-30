//! `cargo xtask fonts`: fetches the built-in fonts and their licences
//! (`craft3d_font::BUILTIN`, each pinned to a URL and a SHA-256) into
//! `$CRAFT3D_FONTS`, or `assets/fonts/` of the repository when that is not
//! set. A file that is there with the right sum is kept; one with another
//! sum is fetched again. Font files are never committed (AGENTS.md): this
//! is how the tests, the CI workflows and the installers get them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the fonts go: `$CRAFT3D_FONTS`, or `assets/fonts` of the repository.
pub(crate) fn dir() -> PathBuf {
    match std::env::var_os("CRAFT3D_FONTS") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => crate::package::repo_root().join("assets").join("fonts"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sum_of(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(hex(&craft3d_doc::sha256(&bytes)))
}

fn fetch(dir: &Path, file: &str, url: &str, sha256: &str) -> Result<(), String> {
    let path = dir.join(file);
    if sum_of(&path).as_deref() == Some(sha256) {
        return Ok(());
    }
    eprintln!("fonts: fetching {file}");
    let part = dir.join(format!("{file}.part"));
    let status = Command::new("curl")
        .args(["-fsSL", "--retry", "3", "-o"])
        .arg(&part)
        .arg(url)
        .status()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    if !status.success() {
        return Err(format!("fetching {url} failed ({status})"));
    }
    let got = sum_of(&part).unwrap_or_default();
    if got != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "{file}: the download's SHA-256 is {got}, not the pinned {sha256}"
        ));
    }
    std::fs::rename(&part, &path).map_err(|e| format!("cannot keep {file}: {e}"))
}

pub(crate) fn run() -> Result<(), String> {
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    for font in craft3d_font::BUILTIN {
        fetch(&dir, font.file, font.url, font.sha256)?;
        fetch(&dir, font.licence, font.licence_url, font.licence_sha256)?;
    }
    eprintln!("fonts: in {}", dir.display());
    Ok(())
}
