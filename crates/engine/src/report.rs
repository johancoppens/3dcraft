//! The bug report: a plain text a tester pastes into an issue when a
//! command failed, and the `session.report` command that gives it to an
//! agent. It names the build (version and git commit), the platform, what
//! the frontend says about itself (the renderer and its adapter, set with
//! [`Session::set_host_details`]) and the last command that failed, with
//! its params and the whole error.

use crate::commands::{Run, spec};
use crate::params::Params;
use crate::{CommandSpec, Session};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[spec!("session.report", "Report for a Bug", "{}",
        "Returns a plain-text report to paste into a bug report: the 3DCraft version and git commit, the platform, what the frontend runs on (in the window: the renderer and its graphics adapter) and the last command that failed in this session, with its params and its whole error. Also returns these as fields; lastFailure is null when no command has failed. Running it changes nothing.", doc: false, session_report)];

/// The version of 3DCraft this is.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The commit the running program was built from, as its `main` gave it
/// ([`set_build_commit`]).
static BUILD_COMMIT: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Tells the engine which commit the program was built from, for every
/// report: a program's `main` passes `option_env!("CRAFT3D_GIT_COMMIT")`,
/// which its build script sets (`craft3d-buildinfo`). The value is embedded
/// in the program, not in this library, so a new commit rebuilds only the
/// program. The first call wins; `None` changes nothing.
pub fn set_build_commit(commit: Option<&'static str>) {
    if let Some(commit) = commit.filter(|c| !c.is_empty()) {
        let _ = BUILD_COMMIT.set(commit);
    }
}

/// The commit [`set_build_commit`] was given (letters and digits, up to 64:
/// usually the first 12 hex digits of the object name), or `None` when the
/// program did not know it.
pub fn build_commit() -> Option<&'static str> {
    BUILD_COMMIT.get().copied()
}

/// What `--version` prints for `program`: the version, the commit it was
/// built from when known, and the `.3dcraft` format version it writes.
pub fn version_line(program: &str) -> String {
    let format = craft3d_format::FORMAT_VERSION;
    match build_commit() {
        Some(commit) => format!("{program} {VERSION} ({commit}, format {format})"),
        None => format!("{program} {VERSION} (format {format})"),
    }
}

/// The most characters of params and of an error a report keeps.
pub const MAX_REPORT_FIELD_CHARS: usize = 4000;

/// Most host details kept, and characters of each.
const MAX_HOST_DETAILS: usize = 16;
const MAX_HOST_DETAIL_CHARS: usize = 400;

/// A command that failed, as a report shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedCommand {
    /// The command id as it was asked for (also an unknown one).
    pub command: String,
    /// The params as compact JSON, cut at [`MAX_REPORT_FIELD_CHARS`].
    pub params: String,
    /// The error, cut at [`MAX_REPORT_FIELD_CHARS`].
    pub error: String,
    /// Counts the failures of the session: a frontend that keeps a copy
    /// sees when a new one came.
    pub serial: u64,
}

/// `text`, cut to `max` characters with a note of how long it was.
fn clip(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str(&format!("… ({count} characters in all)"));
    out
}

/// One line, with control characters made spaces.
fn one_line(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    clip(&flat, max)
}

/// The operating system and processor this runs on, as Rust names them:
/// `linux x86_64`, `macos aarch64`, `windows x86_64`; in a browser
/// `web (wasm32)`.
pub fn platform() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    if cfg!(target_arch = "wasm32") {
        format!("web ({arch})")
    } else {
        format!("{os} {arch}")
    }
}

/// The plain-text report. `host` are the frontend's own lines (name,
/// value). `status` is the failure the frontend shows now, as (label,
/// text), and `current` says whether `failure` is that failure (the engine
/// command behind it, perhaps under another label such as a batch or an MCP
/// tool). A status that is not `failure` (a failure outside the engine, such
/// as a file chooser that could not be shown) gets lines of its own, and
/// `failure` is then marked as an earlier one.
pub fn bug_report(
    failure: Option<&FailedCommand>,
    current: bool,
    host: &[(String, String)],
    status: Option<(&str, &str)>,
) -> String {
    let mut out = String::from("3DCraft bug report\n");
    out.push_str(&format!("version: {VERSION}\n"));
    out.push_str(&format!(
        "commit: {}\n",
        build_commit().unwrap_or("unknown")
    ));
    out.push_str(&format!("platform: {}\n", platform()));
    for (name, value) in host {
        out.push_str(&format!("{name}: {value}\n"));
    }
    let current = current && failure.is_some();
    if let Some((label, text)) = status
        && !current
    {
        out.push_str(&format!("failed: {label}\n"));
        out.push_str(&format!("message: {text}\n"));
    }
    match failure {
        Some(failure) => {
            if status.is_some() && !current {
                out.push_str("earlier failed command (not the failure above):\n");
            }
            out.push_str(&format!("command: {}\n", failure.command));
            out.push_str(&format!("params: {}\n", failure.params));
            out.push_str(&format!("error: {}\n", failure.error));
        }
        None if status.is_none() => out.push_str("command: (none failed)\n"),
        None => {}
    }
    out
}

impl Session {
    /// Remembers a command that failed, for the report.
    pub(crate) fn record_failure(&mut self, id: &str, params: &Value, error: &str) {
        let serial = self
            .last_failure
            .as_ref()
            .map_or(1, |f| f.serial.saturating_add(1));
        self.last_failure = Some(FailedCommand {
            command: one_line(id, 200),
            params: clip(&params.to_string(), MAX_REPORT_FIELD_CHARS),
            error: clip(error, MAX_REPORT_FIELD_CHARS),
            serial,
        });
    }

    /// The last command that failed in this session.
    pub fn last_failure(&self) -> Option<&FailedCommand> {
        self.last_failure.as_ref()
    }

    /// What the frontend says about itself in a report, as (name, value)
    /// lines: the window gives its renderer and graphics adapter. Replaces
    /// what was set before; at most 16 lines of 400 characters are kept.
    pub fn set_host_details(&mut self, details: Vec<(String, String)>) {
        self.host_details = details
            .into_iter()
            .take(MAX_HOST_DETAILS)
            .map(|(name, value)| {
                (
                    one_line(&name, MAX_HOST_DETAIL_CHARS),
                    one_line(&value, MAX_HOST_DETAIL_CHARS),
                )
            })
            .collect();
    }

    /// What [`Self::set_host_details`] set.
    pub fn host_details(&self) -> &[(String, String)] {
        &self.host_details
    }

    /// The report of this session: the build, the platform, the host's
    /// details and the last command that failed.
    pub fn bug_report(&self) -> String {
        bug_report(self.last_failure.as_ref(), true, &self.host_details, None)
    }
}

fn session_report(session: &mut Session, params: &Value) -> Run {
    Params::new("session.report", params, &[])?;
    let failure = session.last_failure().map(|f| {
        json!({
            "command": f.command,
            "params": f.params,
            "error": f.error,
        })
    });
    let host: serde_json::Map<String, Value> = session
        .host_details()
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    Ok(json!({
        "text": session.bug_report(),
        "version": VERSION,
        "commit": build_commit(),
        "platform": platform(),
        "host": host,
        "lastFailure": failure,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Workspace;

    #[test]
    fn a_failed_command_is_in_the_report_with_its_params() {
        let mut session = Session::new(Workspace::memory());
        assert!(session.last_failure().is_none());
        let params = json!({"size": ["40 mm", "oops", "10 mm"]});
        let error = session.execute("feature.block", &params).unwrap_err();
        let failure = session.last_failure().unwrap().clone();
        assert_eq!(failure.command, "feature.block");
        assert_eq!(failure.params, params.to_string());
        assert_eq!(failure.error, error.message());
        assert_eq!(failure.serial, 1);

        session.set_host_details(vec![("renderer".into(), "gpu (Test (Vulkan))".into())]);
        let report = session.execute("session.report", &json!({})).unwrap();
        let text = report["text"].as_str().unwrap();
        assert!(text.starts_with("3DCraft bug report\n"), "{text}");
        assert!(text.contains(&format!("version: {VERSION}\n")), "{text}");
        assert!(text.contains("renderer: gpu (Test (Vulkan))\n"), "{text}");
        assert!(text.contains("command: feature.block\n"), "{text}");
        assert!(text.contains(&format!("params: {params}\n")), "{text}");
        assert!(
            text.contains(&format!("error: {}\n", error.message())),
            "{text}"
        );
        assert_eq!(report["lastFailure"]["command"], "feature.block");
        assert_eq!(report["version"], VERSION);
        assert_eq!(report["host"]["renderer"], "gpu (Test (Vulkan))");

        // Success leaves the failure; so does a report that is refused.
        session.execute("file.new", &json!({})).unwrap();
        session
            .execute("session.report", &json!({"x": 1}))
            .unwrap_err();
        assert_eq!(session.last_failure().unwrap().serial, 1);
        session.execute("no.such", &json!(null)).unwrap_err();
        let failure = session.last_failure().unwrap();
        assert_eq!((failure.command.as_str(), failure.serial), ("no.such", 2));
    }

    #[test]
    fn a_report_without_a_failure_says_so() {
        let session = Session::new(Workspace::memory());
        let text = session.bug_report();
        assert!(text.contains("command: (none failed)\n"), "{text}");
        assert!(
            text.contains(&format!("platform: {}\n", platform())),
            "{text}"
        );
        // Which commit is the concern of `the_commit_is_what_the_program_gave`.
        assert!(text.contains("\ncommit: "), "{text}");
    }

    #[test]
    fn long_and_hostile_fields_are_cut() {
        let mut session = Session::new(Workspace::memory());
        let huge = json!({"data": "x".repeat(100_000)});
        session.execute("feature.block", &huge).unwrap_err();
        let failure = session.last_failure().unwrap();
        assert!(failure.params.chars().count() < MAX_REPORT_FIELD_CHARS + 50);
        assert!(failure.params.ends_with("characters in all)"));
        session.set_host_details(
            (0..100)
                .map(|i| (format!("line\n{i}"), "v\u{0}".repeat(1000)))
                .collect(),
        );
        assert_eq!(session.host_details().len(), MAX_HOST_DETAILS);
        let (name, value) = &session.host_details()[0];
        assert_eq!(name, "line 0");
        assert!(!value.contains('\u{0}'));
        assert!(value.chars().count() < MAX_HOST_DETAIL_CHARS + 50);
    }

    #[test]
    fn a_status_line_is_added_when_it_is_another_failure() {
        let failure = FailedCommand {
            command: "file.open".into(),
            params: "{}".into(),
            error: "file.open: no such file".into(),
            serial: 1,
        };
        // The status bar shows this failure, under its own label or that
        // of a batch: one block, no status lines.
        for label in ["file.open", "batch of 3"] {
            let same = bug_report(Some(&failure), true, &[], Some((label, "no such file")));
            assert!(!same.contains("failed: "), "{same}");
            assert!(!same.contains("earlier"), "{same}");
            assert!(same.contains("command: file.open\n"), "{same}");
        }
        // Another failure: its lines, then the engine's as an earlier one.
        let other = bug_report(Some(&failure), false, &[], Some(("download", "blocked")));
        assert!(
            other.contains("failed: download\nmessage: blocked\n"),
            "{other}"
        );
        assert!(
            other.contains("earlier failed command (not the failure above):\ncommand: file.open\n"),
            "{other}"
        );
        let only = bug_report(None, false, &[], Some(("file", "no chooser")));
        assert!(
            only.contains("failed: file\nmessage: no chooser\n"),
            "{only}"
        );
        assert!(!only.contains("command:"), "{only}");
        // Without a status (session.report) the failure is simply named.
        let plain = bug_report(Some(&failure), false, &[], None);
        assert!(!plain.contains("earlier"), "{plain}");
        assert!(plain.contains("command: file.open\n"), "{plain}");
    }

    #[test]
    fn the_commit_is_what_the_program_gave() {
        set_build_commit(None);
        set_build_commit(Some(""));
        // Other tests of this binary may have set it; it never changes.
        let first = build_commit();
        set_build_commit(Some("abc123def456"));
        let now = build_commit();
        assert!(now.is_some());
        if let Some(first) = first {
            assert_eq!(now, Some(first));
        }
        assert_eq!(
            version_line("x"),
            format!(
                "x {VERSION} ({}, format {})",
                now.unwrap(),
                craft3d_format::FORMAT_VERSION
            )
        );
        let text = Session::new(Workspace::memory()).bug_report();
        assert!(
            text.contains(&format!("commit: {}\n", now.unwrap())),
            "{text}"
        );
    }
}
