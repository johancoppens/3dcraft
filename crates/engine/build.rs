//! Embeds the git commit 3DCraft is built from, for the bug report
//! (`src/report.rs`): `CRAFT3D_GIT_COMMIT` when the environment sets it
//! (a build from a source archive), otherwise what `git` says about the
//! checkout. Without either the report says "unknown"; a build never fails
//! for want of git.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Builds again when the checkout moves to another commit: the HEAD file,
/// the branch it names and the packed refs (in a worktree they live in
/// different directories, which `git rev-parse --git-path` resolves).
fn watch_head() {
    let mut paths = vec!["HEAD".to_string(), "packed-refs".to_string()];
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        paths.push(branch);
    }
    for path in paths {
        if let Some(file) = git(&["rev-parse", "--git-path", &path])
            && Path::new(&file).exists()
        {
            println!("cargo:rerun-if-changed={file}");
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=CRAFT3D_GIT_COMMIT");
    let given = std::env::var("CRAFT3D_GIT_COMMIT")
        .ok()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
    let commit = match given {
        Some(commit) => Some(commit),
        None => {
            watch_head();
            git(&["rev-parse", "--short=12", "HEAD"])
        }
    };
    // Only letters and digits get through: the value ends up in a string
    // literal and in a report people paste into an issue.
    if let Some(commit) = commit {
        let clean: String = commit
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(40)
            .collect();
        if !clean.is_empty() {
            println!("cargo:rustc-env=CRAFT3D_GIT_COMMIT={clean}");
        }
    }
}
