//! `craft3d-launcher`: starts the 3DCraft window without a console window
//! (docs/decisions.md, 2026-11-13: *On Windows a launcher starts the
//! window*).
//!
//! `craft3d.exe` is a console program, so that `--version`, `--help`, its
//! start-up errors and the control channel's port line reach a terminal and
//! an agent. Started from the Start menu or by a double-clicked file it
//! therefore opened a console window beside its own, and closing that
//! closed 3DCraft. The launcher is a program of the Windows GUI subsystem
//! (no console of its own) that the Start menu and the three file types
//! point to: it starts `craft3d.exe` from its own folder with
//! `CREATE_NO_WINDOW` (a console that is never shown), passes its
//! arguments on unchanged, and waits for it. When the window ends with an
//! error soon after it started, the launcher shows the window's last
//! message in a message box (Windows; on another system on its own
//! standard error), or, with [`LOG_SWITCH`] set to a file, writes it there
//! instead, for a machine without a desktop.
//!
//! The window's standard error goes to a file of its own in the system's
//! temporary folder, read back only once the window has ended, so the
//! window never waits on a full pipe and the launcher holds nothing while
//! it runs, however long that is. Its standard output is the launcher's,
//! so `3DCraft.exe --version | more` still answers.
//!
//! Everything but the start itself and the box is in portable functions
//! with tests here; `main.rs` only joins them.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The window's program, beside the launcher (with the platform's
/// executable suffix).
pub const WINDOW: &str = "craft3d";

/// The switch for a machine without a desktop (a CI runner): set to a
/// file, the launcher appends what it would show in a message box to that
/// file, one line, and shows no box.
pub const LOG_SWITCH: &str = "CRAFT3D_LAUNCHER_LOG";

/// A window that ends with an error within this time after its start is
/// one that could not start, and the launcher says why. One that ends
/// later had started (it was killed, or crashed) and leaves its recovery
/// files; the next start offers them.
pub const START_UP: Duration = Duration::from_secs(60);

/// The most of the window's error output read back: its last message is
/// at its end.
const TAIL: u64 = 64 * 1024;

/// The most lines and characters of a message shown.
const MOST_LINES: usize = 40;
const MOST_CHARS: usize = 4000;

/// How the window's messages begin (`apps/craft3d/src/main.rs`).
const MESSAGE_START: &str = "craft3d:";

/// The window's program beside the launcher at `launcher`; `None` for a
/// path without a folder.
pub fn window_beside(launcher: &Path) -> Option<PathBuf> {
    let folder = launcher.parent()?;
    Some(folder.join(format!("{WINDOW}{}", std::env::consts::EXE_SUFFIX)))
}

/// The command that starts `window` with `args`, unchanged and in order
/// (std quotes each one for the program's command line on Windows), its
/// input closed, its output the launcher's, its errors into `errors` when
/// there is such a file, and on Windows without a console window.
pub fn window_command(
    window: &Path,
    args: impl IntoIterator<Item = OsString>,
    errors: Option<File>,
) -> Command {
    let mut command = Command::new(window);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(errors.map_or_else(Stdio::null, Stdio::from));
    without_console(&mut command);
    command
}

/// `CREATE_NO_WINDOW`: the console program is given a console of its own
/// that is never shown, so neither it nor a console program it starts
/// opens a window, and there is none to close it by.
#[cfg(windows)]
fn without_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Elsewhere a program started from the desktop has no console to hide.
#[cfg(not(windows))]
fn without_console(_: &mut Command) {}

/// The file the window's errors are written to, in the system's
/// temporary folder and named after the launcher's process (and a count,
/// for a process that starts several), with its path; `None` where it
/// cannot be made (the window then starts without one, and a failure is
/// said without its message).
fn error_file() -> Option<(PathBuf, File)> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static STARTED: AtomicU32 = AtomicU32::new(0);
    let count = STARTED.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "3dcraft-launcher-{}-{count}.txt",
        std::process::id()
    ));
    let file = File::create(&path).ok()?;
    Some((path, file))
}

/// The end of a file, at most [`TAIL`] bytes, as text (invalid UTF-8
/// replaced).
fn tail_of(path: &Path) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    if file
        .seek(SeekFrom::Start(length.saturating_sub(TAIL)))
        .is_err()
    {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.take(TAIL).read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The window's last message in `errors`, its error output: from the last
/// line that starts as the window's messages do (`craft3d: …`, which may
/// go on over several lines, such as the help when no graphics adapter can
/// show the window) to the end; without such a line its last few lines
/// (a crash's). The last line is always in it. Control characters are
/// left out, and at most [`MOST_LINES`] lines and [`MOST_CHARS`]
/// characters are kept, from the start of the message. Empty when the
/// window said nothing.
pub fn last_message(errors: &str) -> String {
    let lines: Vec<String> = errors
        .lines()
        .map(|line| {
            line.chars()
                .filter(|c| !c.is_control() || *c == '\t')
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    let Some(last) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let lines = lines.get(..=last).unwrap_or_default();
    let start = lines
        .iter()
        .rposition(|line| line.starts_with(MESSAGE_START))
        .unwrap_or_else(|| {
            // Without the window's own message: its last five lines that
            // say something.
            let mut start = last;
            let mut said = 0;
            for (at, line) in lines.iter().enumerate().rev() {
                if !line.trim().is_empty() {
                    said += 1;
                    start = at;
                    if said == 5 {
                        break;
                    }
                }
            }
            start
        });
    let mut message = String::new();
    for line in lines
        .get(start..)
        .unwrap_or_default()
        .iter()
        .take(MOST_LINES)
    {
        if !message.is_empty() {
            message.push('\n');
        }
        message.push_str(line);
    }
    if message.chars().count() > MOST_CHARS {
        message = message.chars().take(MOST_CHARS).collect();
        message.push('…');
    }
    message
}

/// An exit code as Windows people read it: a small number as it is, an
/// NTSTATUS (a crash, a missing DLL) in hexadecimal.
pub fn exit_code_text(code: Option<i32>) -> String {
    match code {
        Some(code) if (0..=0xFFFF).contains(&code) => code.to_string(),
        Some(code) => format!("{:#010X}", u32::from_ne_bytes(code.to_ne_bytes())),
        None => "none (ended by a signal)".to_string(),
    }
}

/// What the launcher says when the window it started ended with `status`
/// after `ran`, having written `errors`: `None` when there is nothing to
/// say (it ended well, or it had started and ran for longer than
/// [`START_UP`]).
pub fn start_up_failure(status: ExitStatus, ran: Duration, errors: &str) -> Option<String> {
    if status.success() || ran >= START_UP {
        return None;
    }
    let message = last_message(errors);
    Some(if message.is_empty() {
        format!(
            "3DCraft could not start: the window ended with exit code {} and said nothing.",
            exit_code_text(status.code())
        )
    } else {
        format!("3DCraft could not start.\n\n{message}")
    })
}

/// Starts the window beside `launcher` with `args` and waits for it.
/// `Ok` with the window's exit status when there is nothing to say; `Err`
/// with what to say when the window is missing, cannot be started, or
/// could not start ([`start_up_failure`]). A window that never ends is
/// waited for as long as it runs.
pub fn launch(launcher: &Path, args: Vec<OsString>) -> Result<ExitStatus, String> {
    let window = window_beside(launcher).ok_or_else(|| {
        format!(
            "3DCraft could not start: the launcher's own folder is not known ({}).",
            launcher.display()
        )
    })?;
    if !window.is_file() {
        return Err(format!(
            "3DCraft could not start: {} was not found beside {}. Install 3DCraft again, or \
             unpack the whole zip.",
            window.display(),
            launcher.display()
        ));
    }
    let errors = error_file();
    let (errors_path, errors_file) = match errors {
        Some((path, file)) => (Some(path), Some(file)),
        None => (None, None),
    };
    let started = Instant::now();
    let spawned = window_command(&window, args, errors_file).spawn();
    let result = match spawned {
        Err(error) => Err(format!(
            "3DCraft could not start {}: {error}",
            window.display()
        )),
        Ok(mut child) => match child.wait() {
            Err(error) => Err(format!(
                "3DCraft could not wait for {}: {error}",
                window.display()
            )),
            Ok(status) => {
                let said = errors_path.as_deref().map(tail_of).unwrap_or_default();
                match start_up_failure(status, started.elapsed(), &said) {
                    Some(message) => Err(message),
                    None => Ok(status),
                }
            }
        },
    };
    if let Some(path) = errors_path {
        let _ = std::fs::remove_file(path);
    }
    result
}

/// Says `message`: with the switch (`log`, the value of [`LOG_SWITCH`])
/// set to a file, as one line appended to it (and to standard error when
/// that fails); otherwise in a message box on Windows, and on standard
/// error elsewhere.
pub fn report(message: &str, log: Option<&OsStr>) {
    let line = format!(
        "craft3d-launcher: {}",
        message
            .split('\n')
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" | ")
    );
    if let Some(log) = log.filter(|log| !log.is_empty()) {
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .and_then(|mut file| writeln!(file, "{line}"));
        if written.is_err() {
            let _ = writeln!(std::io::stderr(), "{line}");
        }
        return;
    }
    show(message, &line);
}

/// A message box (rfd's, the system's own on Windows), and the line on
/// standard error too, for a launcher started from a terminal.
#[cfg(windows)]
fn show(message: &str, line: &str) {
    let _ = writeln!(std::io::stderr(), "{line}");
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("3DCraft")
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Elsewhere the launcher is not what the desktop starts: the line on
/// standard error.
#[cfg(not(windows))]
fn show(_: &str, line: &str) {
    let _ = writeln!(std::io::stderr(), "{line}");
}

#[cfg(test)]
mod tests;
