//! For the build scripts of 3DCraft's programs (`apps/*/build.rs`): the git
//! commit a program is built from, for the bug report
//! (`craft3d_engine::bug_report`, `session.report`).
//!
//! Only the programs embed it, never a library: a value that changes with
//! every commit makes Cargo rebuild the crate that holds it and everything
//! that depends on it, and a program is a leaf. Its `main` hands the value
//! to the engine with `craft3d_engine::set_build_commit`.
//!
//! The commit is `CRAFT3D_GIT_COMMIT` when the environment sets it (a build
//! from a source archive); otherwise what `git` says, but only when the
//! checkout git finds is the workspace itself (sources unpacked inside
//! another repository would report that repository's commit); otherwise
//! none, and the report says "unknown". A build never fails for want of git.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The environment variable a build script sets for its crate (and that a
/// builder may set to override what git says).
pub const ENV: &str = "CRAFT3D_GIT_COMMIT";

/// Most characters of a commit kept: a full SHA-256 object name.
pub const MAX_COMMIT_CHARS: usize = 64;

/// Runs `git` in `dir`; its trimmed output when it succeeded and said
/// something.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Only letters and digits, at most [`MAX_COMMIT_CHARS`]: the value ends
/// up in a string literal and in a report people paste into an issue.
/// `None` when nothing is left.
pub fn clean(commit: &str) -> Option<String> {
    let clean: String = commit
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(MAX_COMMIT_CHARS)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

/// The same directory, whatever way it was written (`..`, links).
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The files git changes when the checkout moves to another commit, which
/// exist: the worktree's HEAD and its reflog (appended at every commit,
/// checkout and reset), the branch's loose ref, the packed refs, the
/// directory of branches and a reftable. A path to a directory makes Cargo
/// look at everything beneath it.
fn watched(dir: &Path) -> Vec<PathBuf> {
    let mut names = vec![
        "HEAD".to_string(),
        "logs/HEAD".to_string(),
        "packed-refs".to_string(),
        "refs/heads".to_string(),
        "reftable".to_string(),
    ];
    if let Some(branch) = git(dir, &["symbolic-ref", "-q", "HEAD"]) {
        names.push(branch);
    }
    let mut paths = Vec::new();
    for name in names {
        if let Some(path) = git(
            dir,
            &["rev-parse", "--path-format=absolute", "--git-path", &name],
        ) {
            let path = PathBuf::from(path);
            if path.exists() && !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

/// What [`emit_commit`] decides, without printing: the commit, and the
/// files whose change should run the build script again.
pub fn commit_of(workspace: &Path, given: Option<&str>) -> (Option<String>, Vec<PathBuf>) {
    if let Some(commit) = given.and_then(clean) {
        return (Some(commit), Vec::new());
    }
    let Some(top) = git(workspace, &["rev-parse", "--show-toplevel"]) else {
        return (None, Vec::new());
    };
    if !same_dir(Path::new(&top), workspace) {
        return (None, Vec::new());
    }
    let commit = git(workspace, &["rev-parse", "--short=12", "HEAD"]).and_then(|c| clean(&c));
    (commit, watched(workspace))
}

/// For a build script: sets `CRAFT3D_GIT_COMMIT` for the crate being built
/// to the commit of `workspace` (the repository's root, normally
/// `CARGO_MANIFEST_DIR/../..`), and tells Cargo when to ask again.
pub fn emit_commit(workspace: &Path) {
    println!("cargo:rerun-if-env-changed={ENV}");
    let given = std::env::var(ENV).ok();
    let (commit, watch) = commit_of(workspace, given.as_deref());
    for path in watch {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    if let Some(commit) = commit {
        println!("cargo:rustc-env={ENV}={commit}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn a_given_commit_wins_and_is_cleaned() {
        let (commit, watch) = commit_of(&workspace(), Some(" abc123\"; evil\n"));
        assert_eq!(commit.as_deref(), Some("abc123evil"));
        assert!(watch.is_empty());
        let long = "a".repeat(200);
        let (commit, _) = commit_of(&workspace(), Some(&long));
        assert_eq!(commit.map(|c| c.len()), Some(MAX_COMMIT_CHARS));
        assert_eq!(clean("--- ;"), None);
    }

    #[test]
    fn a_directory_that_is_not_the_top_of_the_checkout_has_no_commit() {
        // This crate's own directory lies inside the checkout but is not
        // its top: git would answer, and must not be believed.
        let (commit, watch) = commit_of(Path::new(env!("CARGO_MANIFEST_DIR")), None);
        assert_eq!(commit, None);
        assert!(watch.is_empty());
        let nowhere = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-dir");
        assert_eq!(commit_of(&nowhere, None).0, None);
    }

    #[test]
    fn in_a_checkout_the_head_and_its_reflog_are_watched() {
        let root = workspace();
        // Built from an archive there is no checkout to test.
        if git(&root, &["rev-parse", "--show-toplevel"]).is_none() {
            return;
        }
        let (commit, watch) = commit_of(&root, None);
        let commit = commit.unwrap();
        assert_eq!(commit.len(), 12, "{commit}");
        assert!(commit.chars().all(|c| c.is_ascii_hexdigit()), "{commit}");
        assert!(
            watch
                .iter()
                .any(|p| p.ends_with("HEAD") && !p.ends_with("logs/HEAD")),
            "{watch:?}"
        );
        // A clone without reflogs (core.logAllRefUpdates off) has none.
        let reflog = git(
            &root,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "logs/HEAD",
            ],
        );
        if reflog.is_some_and(|r| Path::new(&r).exists()) {
            assert!(watch.iter().any(|p| p.ends_with("logs/HEAD")), "{watch:?}");
        }
        assert!(watch.iter().all(|p| p.is_absolute()), "{watch:?}");
    }
}
