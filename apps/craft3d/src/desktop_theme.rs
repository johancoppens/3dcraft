//! The desktop's light or dark setting where the windowing library does
//! not report one: on Linux winit gives no system theme, so Appearance ›
//! System would always be light. GNOME (and desktops that follow its
//! settings) keep it in `org.gnome.desktop.interface color-scheme`
//! (`'default'`, `'prefer-dark'`, `'prefer-light'`), the value the XDG
//! desktop portal's `org.freedesktop.appearance color-scheme` is made from.
//!
//! It is read with the `gsettings` program, once at the start and then
//! from `gsettings monitor`, on a thread of its own, so the window never
//! waits for it and no crate is added (D-Bus would need one). Where
//! `gsettings` is missing or says nothing, the theme stays unknown and
//! System stays light, as before. The monitor ends with the window: when
//! the window goes ([`Drop`], also while the monitor is being started) and
//! when 3DCraft is killed or crashes without unwinding, through the shell
//! that runs it ([`Watcher`]). Before 2026-11-13 a killed window left its
//! `gsettings monitor` running until the setting next changed.

use eframe::egui;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// The desktop's colour scheme as `gsettings` writes it: dark, light, or
/// `None` for no preference (`'default'`) or what cannot be read.
pub fn parse_color_scheme(text: &str) -> Option<egui::Theme> {
    // `gsettings get` prints `'prefer-dark'`; `gsettings monitor` prints
    // `color-scheme: 'prefer-dark'`.
    let value = text.rsplit(':').next().unwrap_or(text).trim();
    let value = value.trim_matches(|c| c == '\'' || c == '"');
    match value {
        "prefer-dark" => Some(egui::Theme::Dark),
        "prefer-light" => Some(egui::Theme::Light),
        _ => None,
    }
}

/// The desktop's theme as last read, shared with the reading thread.
pub struct DesktopTheme {
    latest: Arc<Mutex<Option<egui::Theme>>>,
    monitor: Arc<Mutex<Monitor>>,
}

/// The `gsettings monitor` being read, and whether the window has gone:
/// set under the same lock, so a monitor started after the window went is
/// ended by the thread that started it.
#[derive(Default)]
struct Monitor {
    watcher: Option<Watcher>,
    stopped: bool,
}

impl DesktopTheme {
    /// Starts reading the desktop's setting; `ctx` is asked for a frame
    /// whenever it changes. On a platform other than Linux it reads
    /// nothing (winit reports the theme there).
    pub fn start(ctx: Option<egui::Context>) -> Self {
        let theme = Self {
            latest: Arc::new(Mutex::new(None)),
            monitor: Arc::new(Mutex::new(Monitor::default())),
        };
        if cfg!(target_os = "linux") {
            let latest = Arc::clone(&theme.latest);
            let monitor = Arc::clone(&theme.monitor);
            let spawned = std::thread::Builder::new()
                .name("craft3d-desktop-theme".to_string())
                .spawn(move || read(&latest, &monitor, ctx.as_ref()));
            if let Err(error) = spawned {
                log::warn!("the desktop's light or dark setting is not read: {error}");
            }
        }
        theme
    }

    /// The desktop's theme as last read; `None` while unknown.
    pub fn get(&self) -> Option<egui::Theme> {
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for DesktopTheme {
    fn drop(&mut self) {
        let watcher = {
            let mut monitor = self.monitor.lock().unwrap_or_else(PoisonError::into_inner);
            monitor.stopped = true;
            monitor.watcher.take()
        };
        // Ended outside the lock: the reading thread may want it meanwhile.
        drop(watcher);
    }
}

const SCHEMA: &str = "org.gnome.desktop.interface";
const KEY: &str = "color-scheme";

/// The thread: the value now, then each change the monitor prints, until
/// it ends (the window ended it, or `gsettings` went away).
fn read(
    latest: &Mutex<Option<egui::Theme>>,
    monitor: &Mutex<Monitor>,
    ctx: Option<&egui::Context>,
) {
    use std::io::{BufRead, Read};
    let set = |theme: Option<egui::Theme>| {
        let mut slot = latest.lock().unwrap_or_else(PoisonError::into_inner);
        if *slot != theme {
            *slot = theme;
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        }
    };
    let Ok(now) = Command::new("gsettings")
        .args(["get", SCHEMA, KEY])
        .stderr(Stdio::null())
        .output()
    else {
        // No gsettings: System stays light.
        return;
    };
    if !now.status.success() {
        return;
    }
    set(parse_color_scheme(&String::from_utf8_lossy(&now.stdout)));
    let Ok((watcher, stdout)) = Watcher::spawn("gsettings", &["monitor", SCHEMA, KEY]) else {
        log::warn!("the desktop's light or dark setting is read once, not followed");
        return;
    };
    {
        let mut slot = monitor.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.stopped {
            // The window went while it was being started: ended here.
            drop(slot);
            drop(watcher);
            return;
        }
        slot.watcher = Some(watcher);
    }
    // One line per change; a line far longer than any value is cut, so a
    // misbehaving program cannot grow it without bound.
    let mut reader = std::io::BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match Read::by_ref(&mut reader).take(4096).read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if line.trim_start().starts_with(KEY) {
                    set(parse_color_scheme(&line));
                }
            }
        }
    }
    // `gsettings monitor` ended by itself (or the window ended it): its
    // shell is let go of here rather than when the window goes.
    let watcher = monitor
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .watcher
        .take();
    drop(watcher);
}

/// The shell script that runs a watching program (its arguments) so that
/// it ends with 3DCraft. The shell starts the program in the background,
/// lets go of its own copy of the program's output pipe (so the reader
/// sees the end when the program ends), and waits on its input: a pipe of
/// which only 3DCraft holds the other end (std opens pipes close-on-exec,
/// so no other child inherits it). That end is closed when 3DCraft lets
/// go of it ([`Watcher`]'s `Drop`) and when 3DCraft dies in any way, a
/// SIGKILL included, since the kernel closes a dead process's files. The
/// read then returns and the shell kills its process group, the program
/// and itself. The group is the shell's own ([`in_own_group`]): `-$$`
/// names it and nothing of 3DCraft's, and where the shell were not its
/// leader no such group would exist and the kill would do nothing. All of
/// it POSIX `sh`, without `unsafe` on our side (no `pre_exec`, no
/// `prctl`).
const WRAPPER: &str = r#""$@" & exec >/dev/null; read -r line; kill -s KILL -- -$$"#;

/// How long the shell is waited for once its pipe is closed: it takes
/// milliseconds, a kill and an exit.
const SHELL_ENDS: Duration = Duration::from_millis(500);

/// A watching program run by [`WRAPPER`], which ends when this is dropped
/// or when this process dies.
struct Watcher {
    /// The shell, its input the pipe whose end is held here.
    shell: Child,
}

impl Watcher {
    /// Starts `program` with `args`, and returns it with its output.
    fn spawn(program: &str, args: &[&str]) -> std::io::Result<(Self, ChildStdout)> {
        let mut command = Command::new("sh");
        command
            .args(["-c", WRAPPER, "sh", program])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        in_own_group(&mut command)?;
        let mut watcher = Self {
            shell: command.spawn()?,
        };
        let stdout = watcher
            .shell
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("the watcher's output is not a pipe"))?;
        Ok((watcher, stdout))
    }

    /// The shell's process id (its group's too).
    #[cfg(all(test, target_os = "linux"))]
    fn id(&self) -> u32 {
        self.shell.id()
    }
}

impl Drop for Watcher {
    /// Closes the shell's input, so it ends the program and itself, and
    /// collects it, waiting at most [`SHELL_ENDS`]. A shell that has not
    /// ended by then is left to end by itself (it has seen its pipe
    /// close) and to be collected by the system once 3DCraft has gone:
    /// killing it instead could leave the program behind.
    fn drop(&mut self) {
        drop(self.shell.stdin.take());
        let start = std::time::Instant::now();
        while start.elapsed() < SHELL_ENDS {
            match self.shell.try_wait() {
                Ok(None) => std::thread::sleep(Duration::from_millis(2)),
                Ok(Some(_)) | Err(_) => return,
            }
        }
    }
}

/// Starts the shell as the leader of a process group of its own
/// (`setpgid(0, 0)` in the child, by std's safe `process_group`).
#[cfg(unix)]
fn in_own_group(command: &mut Command) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    Ok(())
}

/// No process groups: the watcher is not run (nor needed: winit reports
/// the theme on the other platforms).
#[cfg(not(unix))]
fn in_own_group(_: &mut Command) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the watcher needs a Unix process group",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_color_scheme_reads_as_gsettings_writes_it() {
        assert_eq!(
            parse_color_scheme("'prefer-dark'\n"),
            Some(egui::Theme::Dark)
        );
        assert_eq!(
            parse_color_scheme("'prefer-light'"),
            Some(egui::Theme::Light)
        );
        assert_eq!(parse_color_scheme("'default'"), None);
        assert_eq!(
            parse_color_scheme("color-scheme: 'prefer-dark'\n"),
            Some(egui::Theme::Dark)
        );
        assert_eq!(parse_color_scheme("color-scheme: 'default'"), None);
        assert_eq!(parse_color_scheme(""), None);
        assert_eq!(parse_color_scheme("nonsense"), None);
    }

    /// The watcher of a window that is killed (SIGKILL: no `Drop`, no
    /// unwinding) and of one that ends normally, seen from outside in
    /// `/proc`.
    #[cfg(target_os = "linux")]
    mod ends_with_its_window {
        use super::super::*;
        use std::io::{BufRead, Read};
        use std::time::Instant;

        /// The environment variable that turns [`helper`] into a stand-in
        /// window: the program to watch and its arguments, separated by
        /// spaces.
        const HELPER: &str = "CRAFT3D_TEST_WATCHER_HELPER";

        /// What [`helper`] prints before its shell's id. Searched for
        /// anywhere in a line: libtest writes `test <name> ... ` before
        /// the test's own output on the same line.
        const SAYS: &str = "craft3d-watcher-shell=";

        /// How long a process is given to go after its parent went.
        const BOUND: Duration = Duration::from_secs(10);

        /// A process as `/proc/<pid>/stat` has it: its parent, its state
        /// and its start time (to tell it from a later process given the
        /// same id).
        #[derive(Clone, Copy, Debug, PartialEq)]
        struct Stat {
            ppid: u32,
            state: char,
            start: u64,
        }

        fn stat(pid: u32) -> Option<Stat> {
            let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            // `pid (comm) state ppid ...`: the name may hold spaces and
            // parentheses, so the fields are counted after its last `)`.
            let rest = text.get(text.rfind(')')? + 1..)?;
            let fields: Vec<&str> = rest.split_whitespace().collect();
            Some(Stat {
                state: fields.first()?.chars().next()?,
                ppid: fields.get(1)?.parse().ok()?,
                // Field 22 of proc(5), the 20th after the name.
                start: fields.get(19)?.parse().ok()?,
            })
        }

        /// The process `pid` that started at `start` still runs (not
        /// gone, not a zombie waiting to be collected).
        fn runs(pid: u32, start: u64) -> bool {
            stat(pid).is_some_and(|s| s.start == start && s.state != 'Z')
        }

        /// The children of `parent`, waited for up to [`BOUND`].
        fn child_of(parent: u32) -> Option<(u32, Stat)> {
            let begin = Instant::now();
            while begin.elapsed() < BOUND {
                let found = std::fs::read_dir("/proc")
                    .ok()?
                    .flatten()
                    .find_map(|entry| {
                        let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
                        let s = stat(pid)?;
                        (s.ppid == parent && s.state != 'Z').then_some((pid, s))
                    });
                if found.is_some() {
                    return found;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            None
        }

        /// Waits up to [`BOUND`] for `pid` (started at `start`) to go.
        fn goes(pid: u32, start: u64) -> bool {
            let begin = Instant::now();
            while begin.elapsed() < BOUND {
                if !runs(pid, start) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        }

        /// Ends, when dropped, what a test started that still runs: the
        /// stand-in window, and each process recorded by its id and start
        /// time (a later process given the same id is spared). So a test
        /// whose assertion fails leaves nothing behind.
        #[derive(Default)]
        struct Leftovers {
            window: Option<Child>,
            processes: Vec<(u32, u64)>,
        }

        impl Drop for Leftovers {
            fn drop(&mut self) {
                if let Some(mut window) = self.window.take() {
                    let _ = window.kill();
                    let _ = window.wait();
                }
                for &(pid, start) in &self.processes {
                    if runs(pid, start) {
                        let _ = Command::new("sh")
                            .args(["-c", "kill -s KILL \"$1\"", "sh", &pid.to_string()])
                            .status();
                    }
                }
            }
        }

        /// Whether `sh` and `sleep` are there to run the tests with.
        fn has_sh_and_sleep() -> bool {
            let there = Command::new("sh").args(["-c", "exit 0"]).status().is_ok()
                && Command::new("sleep").arg("0").status().is_ok();
            if !there {
                println!("SKIPPED: no sh or sleep");
            }
            there
        }

        /// Not a test of its own: the stand-in window that
        /// [`a_killed_window_takes_its_watcher_along`] starts and kills.
        /// It starts the watcher as the window does, says the shell's id,
        /// and waits to be killed (a minute at most, so none is left).
        #[test]
        fn helper() {
            let Ok(command) = std::env::var(HELPER) else {
                return;
            };
            let mut words = command.split(' ');
            let program = words.next().unwrap();
            let args: Vec<&str> = words.collect();
            let (watcher, _stdout) = Watcher::spawn(program, &args).unwrap();
            println!("{SAYS}{}", watcher.id());
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            std::thread::sleep(Duration::from_secs(60));
        }

        /// Starts the stand-in window watching `command`, finds the
        /// watched program, kills the window with SIGKILL and returns
        /// whether the program and its shell went within [`BOUND`]
        /// (`None`: the program was not seen running, so nothing was
        /// shown).
        fn killed_window_watching(command: &str) -> Option<bool> {
            let mut left = Leftovers::default();
            let exe = std::env::current_exe().unwrap();
            let window = left.window.insert(
                Command::new(exe)
                    .args([
                        "desktop_theme::tests::ends_with_its_window::helper",
                        "--exact",
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env(HELPER, command)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let window_id = window.id();
            let stdout = window.stdout.take().unwrap();
            let shell = std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
                .find_map(|line| {
                    let (_, id) = line.split_once(SAYS)?;
                    id.split_whitespace().next()?.parse::<u32>().ok()
                });
            let shell = shell.expect("the stand-in window never said its watcher's id");
            let shell_stat = stat(shell).unwrap();
            left.processes.push((shell, shell_stat.start));
            assert_eq!(shell_stat.ppid, window_id);
            let watched = child_of(shell);
            if let Some((pid, s)) = watched {
                left.processes.push((pid, s.start));
            }
            // Killed as a crash or `kill -9` would: no Drop runs.
            let mut window = left.window.take().unwrap();
            window.kill().unwrap();
            window.wait().unwrap();
            let (watched, watched_stat) = watched?;
            println!("{command}: pid {watched} under shell {shell}; the window killed");
            Some(goes(watched, watched_stat.start) && goes(shell, shell_stat.start))
        }

        /// The orphans of 2026-11-12: a window killed with SIGKILL left
        /// its `gsettings monitor` running. Now its shell sees the pipe
        /// close and ends the program. Shown with `sleep` everywhere, and
        /// with the real `gsettings monitor` where GNOME's schema is.
        #[test]
        fn a_killed_window_takes_its_watcher_along() {
            if !has_sh_and_sleep() {
                return;
            }
            assert_eq!(
                killed_window_watching("sleep 600"),
                Some(true),
                "the watched sleep outlived the killed window"
            );
            let gnome = Command::new("gsettings")
                .args(["get", SCHEMA, KEY])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if !gnome {
                println!("SKIPPED the gsettings part: no gsettings, or no {SCHEMA} schema");
                return;
            }
            match killed_window_watching(&format!("gsettings monitor {SCHEMA} {KEY}")) {
                Some(gone) => assert!(gone, "gsettings monitor outlived the killed window"),
                None => println!("SKIPPED the gsettings part: gsettings monitor ended at once"),
            }
        }

        /// A window that ends normally drops its watcher: the program and
        /// the shell go.
        #[test]
        fn a_dropped_watcher_ends_its_program() {
            if !has_sh_and_sleep() {
                return;
            }
            let mut left = Leftovers::default();
            let (watcher, _stdout) = Watcher::spawn("sleep", &["600"]).unwrap();
            let shell = watcher.id();
            let shell_stat = stat(shell).unwrap();
            left.processes.push((shell, shell_stat.start));
            let (watched, watched_stat) = child_of(shell).expect("the watched sleep never ran");
            left.processes.push((watched, watched_stat.start));
            drop(watcher);
            assert!(goes(watched, watched_stat.start));
            assert!(goes(shell, shell_stat.start));
        }

        /// What the program prints reaches the reader, and the reader sees
        /// the end of it when the program ends, while the shell still
        /// waits on its pipe (it let go of its copy of the output).
        #[test]
        fn the_reader_sees_the_watcher_out() {
            if !has_sh_and_sleep() {
                return;
            }
            // A failing assertion drops the watcher while unwinding, which
            // ends it.
            let (watcher, stdout) =
                Watcher::spawn("printf", &["color-scheme: 'prefer-dark'\\n"]).unwrap();
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut text = String::new();
                let read = std::io::BufReader::new(stdout).read_to_string(&mut text);
                let _ = send.send(read.map(|_| text));
            });
            let text = receive
                .recv_timeout(BOUND)
                .expect("the reader never saw the end of the output")
                .unwrap();
            assert_eq!(text, "color-scheme: 'prefer-dark'\n");
            assert_eq!(parse_color_scheme(&text), Some(egui::Theme::Dark));
            drop(watcher);
        }
    }
}
