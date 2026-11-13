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
//! arguments on unchanged, waits for it, and ends with its exit code. When
//! the window ends with an error within [`START_UP`], the launcher says
//! that 3DCraft could not start, or, once the window had said it started,
//! that it ended unexpectedly, with the exit code and the window's last
//! message ([`start_up_failure`]), in a message box (Windows; on another
//! system on its own standard error), or, with [`LOG_SWITCH`] set to a
//! file, as a line there instead, for a machine without a desktop.
//!
//! The window's standard error goes to a file of its own in the system's
//! temporary folder, read back only once the window has ended, so the
//! window never waits on a full pipe and the launcher holds nothing while
//! it runs, however long that is. Its standard output is the launcher's:
//! where the launcher was given one (a pipe, as in the Release workflow's
//! smoke test, which reads `3DCraft.exe --version`), the window answers
//! there.
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
/// reported: that it could not start, or, when it had started, that it
/// ended unexpectedly; with its exit code either way. One that ends later
/// was killed or crashed after work, and leaves its recovery files, which
/// the next start offers.
pub const START_UP: Duration = Duration::from_secs(60);

/// The most of the window's error output read back: its last message is
/// at its end.
const TAIL: u64 = 64 * 1024;

/// The most lines and characters of a message shown.
const MOST_LINES: usize = 40;
const MOST_CHARS: usize = 4000;

/// How the window's messages begin (`apps/craft3d/src/main.rs`).
const MESSAGE_START: &str = "craft3d:";

/// What the window says once its window exists (`apps/craft3d/src/app.rs`,
/// `viewport renderer: …`): a window that said it had started.
const STARTED: &str = "craft3d: viewport renderer:";

/// The window's news, said on every good start (the renderer, the control
/// channel, documents to recover; `apps/craft3d/src/app.rs`): never the
/// reason it ended.
const NEWS: [&str; 3] = [
    "craft3d: viewport renderer:",
    "craft3d: control channel on ",
    "unsaved document(s) of an earlier session can be recovered",
];

/// Whether `line` is one of the window's [`NEWS`].
fn is_news(line: &str) -> bool {
    line.starts_with(MESSAGE_START) && NEWS.iter().any(|news| line.contains(news))
}

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

/// The file the window's errors are written to, with its path: a new file
/// in the system's temporary folder, named after the launcher's process, a
/// count (for a process that starts several) and the time, and made only
/// if nothing of that name is there (`create_new`: never a file or a link
/// someone put there before). `None` where none can be made (the window
/// then starts without one, and a failure is said without its message). A
/// launcher that is killed leaves its file behind (the window writes its
/// errors there until it ends), for the system's clean-up of the folder.
fn error_file() -> Option<(PathBuf, File)> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static STARTS: AtomicU32 = AtomicU32::new(0);
    for _ in 0..8 {
        let count = STARTS.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "3dcraft-launcher-{}-{count}-{nanos}.txt",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Some((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
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
/// (a crash's). Control characters are left out, and at most
/// `MOST_LINES` lines and `MOST_CHARS` characters are kept, from the
/// end, where the error is: the end of the last line is always in it.
/// Empty when the window said nothing.
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
    let message_lines = lines.get(start..).unwrap_or_default();
    let message = message_lines
        .get(message_lines.len().saturating_sub(MOST_LINES)..)
        .unwrap_or_default()
        .join("\n");
    let count = message.chars().count();
    if count > MOST_CHARS {
        let end: String = message.chars().skip(count - MOST_CHARS).collect();
        return format!("…{end}");
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
/// say (it ended well, or ran for longer than [`START_UP`]). Always with
/// the exit code. A window that had said it started (`STARTED`) *ended
/// unexpectedly* (a driver's crash, End task); one that had not *could not
/// start*. The reason given is its last message ([`last_message`]) without
/// the news of a good start (`NEWS`), which never is one.
pub fn start_up_failure(status: ExitStatus, ran: Duration, errors: &str) -> Option<String> {
    if status.success() || ran >= START_UP {
        return None;
    }
    let code = exit_code_text(status.code());
    let started = errors.lines().any(|line| line.starts_with(STARTED));
    let message = last_message(errors);
    let reason = message
        .lines()
        .skip_while(|line| is_news(line))
        .collect::<Vec<_>>()
        .join("\n");
    let reason = reason.trim_matches('\n');
    Some(match (started, reason.is_empty()) {
        (true, true) => format!("3DCraft ended unexpectedly (exit code {code})."),
        (true, false) => format!("3DCraft ended unexpectedly (exit code {code}).\n\n{reason}"),
        (false, true) => {
            format!("3DCraft could not start (exit code {code}), and the window gave no reason.")
        }
        (false, false) => format!("3DCraft could not start (exit code {code}).\n\n{reason}"),
    })
}

/// How the window the launcher started ended.
#[derive(Debug, PartialEq, Eq)]
pub struct Ended {
    /// The window's exit code, passed on as the launcher's; `None` when it
    /// did not run, or ended by a signal.
    pub code: Option<i32>,
    /// What to say: why the window is missing, could not be started or
    /// could not start, or that it ended unexpectedly
    /// ([`start_up_failure`]).
    pub say: Option<String>,
}

impl Ended {
    fn said(say: String) -> Self {
        Self {
            code: None,
            say: Some(say),
        }
    }
}

/// Starts the window beside `launcher` with `args` and waits for it, as
/// long as it runs, and says how it ended ([`Ended`]).
pub fn launch(launcher: &Path, args: Vec<OsString>) -> Ended {
    let Some(window) = window_beside(launcher) else {
        return Ended::said(format!(
            "3DCraft could not start: the launcher's own folder is not known ({}).",
            launcher.display()
        ));
    };
    if !window.is_file() {
        return Ended::said(format!(
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
        Err(error) => Ended::said(format!(
            "3DCraft could not start {}: {error}",
            window.display()
        )),
        Ok(mut child) => match child.wait() {
            Err(error) => Ended::said(format!(
                "3DCraft could not wait for {}: {error}",
                window.display()
            )),
            Ok(status) => {
                let said = errors_path.as_deref().map(tail_of).unwrap_or_default();
                Ended {
                    code: status.code(),
                    say: start_up_failure(status, started.elapsed(), &said),
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
