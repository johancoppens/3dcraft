//! For the build scripts of 3DCraft's programs (`apps/*/build.rs`): the
//! release notes a program carries, taken from the workspace's
//! `CHANGELOG.md` when the program is built. Nothing is fetched, then or
//! later.
//!
//! Only the programs embed the notes, never a library, as with the commit
//! (`craft3d-buildinfo`): the changelog changes with nearly every piece of
//! work, and a library that held it would be rebuilt, with everything
//! above it, each time. A program's `main` hands the text to the engine
//! (`craft3d_engine::set_release_notes`).
//!
//! A changelog the build cannot read is a build error, not a program
//! without notes: [`emit`] returns the reason and the build script ends
//! with it.

use std::path::Path;

/// The file a build script writes into its `OUT_DIR`, for
/// `include_str!(concat!(env!("OUT_DIR"), "/release-notes.md"))`.
pub const FILE: &str = "release-notes.md";

/// The changelog's name at the root of the workspace.
pub const CHANGELOG: &str = "CHANGELOG.md";

/// What a program of version `running` carries of the changelog at
/// `workspace/CHANGELOG.md` ([`crate::embedded`]). The error names the
/// file.
pub fn notes(workspace: &Path, running: &str) -> Result<String, String> {
    let path = workspace.join(CHANGELOG);
    let shown = path.display();
    let size = std::fs::metadata(&path)
        .map_err(|e| format!("cannot read `{shown}`, the release notes this program carries: {e}"))?
        .len();
    if size > crate::MAX_BYTES as u64 {
        return Err(format!(
            "`{shown}` is larger than {} bytes",
            crate::MAX_BYTES
        ));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!("cannot read `{shown}`, the release notes this program carries: {e}")
    })?;
    crate::embedded(&text, running).map_err(|e| format!("`{shown}`: {e}"))
}

/// For a build script: writes the notes of the program being built
/// (`CARGO_PKG_VERSION`) into `OUT_DIR/release-notes.md` and tells Cargo
/// to run the script again when the changelog changes. `workspace` is the
/// repository's root, normally `CARGO_MANIFEST_DIR/../..`.
pub fn emit(workspace: &Path) -> Result<(), String> {
    println!(
        "cargo:rerun-if-changed={}",
        workspace.join(CHANGELOG).display()
    );
    let version = std::env::var("CARGO_PKG_VERSION")
        .map_err(|_| "CARGO_PKG_VERSION is not set: this runs in a build script".to_string())?;
    let out = std::env::var_os("OUT_DIR")
        .ok_or_else(|| "OUT_DIR is not set: this runs in a build script".to_string())?;
    let text = notes(workspace, &version)?;
    let file = Path::new(&out).join(FILE);
    std::fs::write(&file, text).map_err(|e| format!("cannot write `{}`: {e}", file.display()))
}
