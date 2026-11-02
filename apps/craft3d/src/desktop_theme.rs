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
//! System stays light, as before. The monitor is stopped when the window
//! goes ([`Drop`]).

use eframe::egui;
use std::sync::{Arc, Mutex, PoisonError};

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
    monitor: Arc<Mutex<Option<std::process::Child>>>,
}

impl DesktopTheme {
    /// Starts reading the desktop's setting; `ctx` is asked for a frame
    /// whenever it changes. On a platform other than Linux it reads
    /// nothing (winit reports the theme there).
    pub fn start(ctx: Option<egui::Context>) -> Self {
        let theme = Self {
            latest: Arc::new(Mutex::new(None)),
            monitor: Arc::new(Mutex::new(None)),
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
        let mut monitor = self.monitor.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(mut child) = monitor.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

const SCHEMA: &str = "org.gnome.desktop.interface";
const KEY: &str = "color-scheme";

/// The thread: the value now, then each change the monitor prints, until
/// it ends (the window killed it, or `gsettings` went away).
fn read(
    latest: &Mutex<Option<egui::Theme>>,
    monitor: &Mutex<Option<std::process::Child>>,
    ctx: Option<&egui::Context>,
) {
    use std::io::{BufRead, Read};
    use std::process::{Command, Stdio};
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
    let Ok(mut child) = Command::new("gsettings")
        .args(["monitor", SCHEMA, KEY])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
    else {
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    *monitor.lock().unwrap_or_else(PoisonError::into_inner) = Some(child);
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
}
