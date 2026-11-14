//! Embeds the commit this program is built from, for the bug report
//! (`craft3d-buildinfo`), and the release notes it carries, from the
//! workspace's `CHANGELOG.md` (`craft3d-changelog`): What's new and
//! `session.whatsNew`. `main` hands both to the engine. A changelog that
//! does not read fails the build.

fn main() -> std::process::ExitCode {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    craft3d_buildinfo::emit_commit(&workspace);
    if let Err(error) = craft3d_changelog::build::emit(&workspace) {
        eprintln!("error: {error}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
