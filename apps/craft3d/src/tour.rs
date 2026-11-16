//! `CRAFT3D_GPU_TOUR` (docs/guide/troubleshooting.md, docs/design/
//! rendering.md › *Diagnostics of the graphics driver*): one start of the
//! window that asks the graphics driver for everything the viewport can ask
//! of it, so a driver that crashes on a pipeline built only for a see-through
//! or cut-open picture or for the studio look is found in one run, and
//! the trace (`CRAFT3D_GPU_TRACE`, `craft3d_gpu::diag`) names the pipeline.
//!
//! The tour drives the window the way an agent does, through its own
//! request channel (the requests of the control channel, without a port or
//! a token: `origin` is `None`, the window's own): it makes a small body by
//! commands (`file.new`, then a `batch` of `feature.block`,
//! `feature.cylinder`, `feature.boolean` and `feature.sphere`; nothing is
//! read from disk), fits the view, and then sets in turn the plain
//! picture, see-through bodies, a section, both together, the studio look
//! and the studio look cut open (`ui.set`). For each step it takes the
//! window's own picture (`ui.screenshot {window: true}`: the window drawn
//! by its renderer on its own device and read back, once the viewport has
//! settled) and writes it as `tour-<step>.png` beside the trace file, and
//! asks `ui.get` whether the viewport is still drawn by the GPU on the
//! window's device. Device errors nothing else catches are counted (in a
//! tour only, the window's device gets a handler that records them instead
//! of wgpu's default, which panics). Every line of the tour goes to
//! standard error and into the trace; the last is `tour: done, N errors`.
//! With `1` the window then closes (`ui.quit {save: "discard"}`: the
//! process exits 0, also with errors, which the last line counts); with
//! `stay` it stays open for a person to look; `quick` is `1` without the
//! studio look's steps, whose shaders DirectX 12's compiler (FXC) takes
//! minutes over: what the Release workflow runs on its Windows runner.

use crate::control::Wake;
use crate::host::Request;
use craft3d_gpu::diag;
use eframe::wgpu;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

/// The environment variable that starts the tour: `1` (the window closes
/// after it), `stay` (it stays open) or `quick` (as `1`, without the studio
/// look). Unset or empty: no tour.
pub const TOUR_VAR: &str = "CRAFT3D_GPU_TOUR";

/// How long the tour waits for one answer of the window.
const ANSWER_WAIT: Duration = Duration::from_secs(120);

/// What the tour does when it is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The window closes; the process exits 0.
    Close,
    /// The window stays open.
    Stay,
    /// As [`Mode::Close`], without the studio look's steps.
    Quick,
}

/// The tour `value` asks for: `None` for an empty value (as if it were not
/// set), an error for anything but `1`, `stay` or `quick` (any case,
/// blanks around ignored).
pub fn parse(value: &str) -> Result<Option<Mode>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed == "1" {
        return Ok(Some(Mode::Close));
    }
    if trimmed.eq_ignore_ascii_case("stay") {
        return Ok(Some(Mode::Stay));
    }
    if trimmed.eq_ignore_ascii_case("quick") {
        return Ok(Some(Mode::Quick));
    }
    let shown: String = trimmed.chars().take(80).collect();
    Err(format!(
        "{TOUR_VAR}=`{shown}` is not 1, stay or quick; there is no tour"
    ))
}

/// The tour [`TOUR_VAR`] asks for, read through `env`; a bad value is
/// said on standard error and means no tour.
pub fn from_env(env: &dyn Fn(&str) -> Option<String>) -> Option<Mode> {
    let value = env(TOUR_VAR)?;
    match parse(&value) {
        Ok(mode) => mode,
        Err(why) => {
            diag::say(&why);
            None
        }
    }
}

/// Says `text` as a line of the tour: on standard error and in the trace.
fn say(text: &str) {
    diag::say(&format!("tour: {text}"));
    diag::note("tour", text);
}

/// Counts and says the errors of `device` that no error scope caught, in
/// place of wgpu's default handler (which panics). For the tour only.
pub fn count_device_errors(device: &wgpu::Device, errors: Arc<AtomicU64>) {
    device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        errors.fetch_add(1, Ordering::Relaxed);
        let text = error.to_string();
        let text: String = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(2000)
            .collect();
        say(&format!("device error: {text}"));
    }));
}

/// Where the pictures of the tour go: beside the trace file, or in the
/// current directory without one.
fn picture_folder(env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    env(diag::TRACE_VAR)
        .filter(|path| !path.trim().is_empty())
        .and_then(|path| {
            Path::new(&path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Starts the tour on a thread of its own: it sends its requests to
/// `requests` and wakes the window with `wake`; `errors` is shared with
/// [`count_device_errors`].
pub fn start(
    mode: Mode,
    requests: Sender<Request>,
    wake: Wake,
    errors: Arc<AtomicU64>,
    env: &dyn Fn(&str) -> Option<String>,
) {
    let folder = picture_folder(env);
    let started = std::thread::Builder::new()
        .name("craft3d-gpu-tour".to_string())
        .spawn(move || {
            let mut tour = Tour {
                requests,
                wake,
                errors,
                folder,
                next_id: 1,
            };
            tour.run(mode);
        });
    if let Err(e) = started {
        say(&format!("the tour could not start: {e}"));
    }
}

/// The steps after the body is made: a name (also of its picture) and
/// what `ui.set` is given.
fn steps() -> Vec<(&'static str, Value)> {
    // The plane y = 15, through the hole and the sphere; flipped, so the
    // half nearer the camera is cut away and the cap is in sight.
    let section = json!({"on": true, "plane": "zx", "offset": 15, "flip": true});
    let off = json!({"on": false});
    let plain = json!({"mode": "plain"});
    let studio = json!({"mode": "studio"});
    // The studio look's options stay closed: they would cover the picture.
    let closed = json!({"lookOptions": false});
    vec![
        (
            "plain",
            json!({"transparency": 0.0, "section": off, "look": plain}),
        ),
        (
            "see-through",
            json!({"transparency": 0.5, "section": off, "look": plain}),
        ),
        (
            "section",
            json!({"transparency": 0.0, "section": section, "look": plain}),
        ),
        (
            "see-through-section",
            json!({"transparency": 0.5, "section": section, "look": plain}),
        ),
        (
            "studio",
            json!({"transparency": 0.0, "section": off, "look": studio, "panels": closed}),
        ),
        (
            "studio-section",
            json!({"transparency": 0.0, "section": section, "look": studio, "panels": closed}),
        ),
    ]
}

struct Tour {
    requests: Sender<Request>,
    wake: Wake,
    errors: Arc<AtomicU64>,
    folder: PathBuf,
    next_id: u64,
}

impl Tour {
    fn run(&mut self, mode: Mode) {
        say(&format!(
            "begin ({}); pictures in {}",
            match mode {
                Mode::Close => "the window closes after it",
                Mode::Stay => "the window stays open after it",
                Mode::Quick => "without the studio look; the window closes after it",
            },
            self.folder.display()
        ));
        say("body begin");
        let body = self.body();
        self.count(&body);
        say(&format!("body end{}", outcome(&body)));
        for (name, set) in steps()
            .into_iter()
            .filter(|(name, _)| mode != Mode::Quick || !name.starts_with("studio"))
        {
            say(&format!("{name} begin"));
            let done = self.step(name, &set);
            self.count(&done);
            say(&format!("{name} end{}", outcome(&done)));
        }
        let errors = self.errors.load(Ordering::Relaxed);
        say(&format!("done, {errors} errors"));
        if mode != Mode::Stay
            && let Err(why) = self.ask("ui.quit", json!({"save": "discard"}))
        {
            say(&format!("the window did not close: {why}"));
        }
    }

    /// Adds one to the errors for a step that failed.
    fn count(&self, done: &Result<(), String>) {
        if done.is_err() {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The small body: a block with a hole through it, and a sphere beside
    /// it, framed.
    fn body(&mut self) -> Result<(), String> {
        self.ask(
            "engine.execute",
            json!({"command": "file.new", "params": {"units": "mm"}}),
        )?;
        self.ask(
            "batch",
            json!({"steps": [
                {"command": "feature.block",
                 "params": {"name": "Block", "size": [40, 30, 20]}},
                {"command": "feature.cylinder",
                 "params": {"name": "Bore", "radius": 7, "height": 40,
                            "position": [20, 15, -10]}},
                {"command": "feature.boolean",
                 "params": {"name": "Holed", "op": "subtract",
                            "target": "Block", "tool": "Bore"}},
                {"command": "feature.sphere",
                 "params": {"name": "Ball", "radius": 12, "position": [62, 15, 10]}},
            ]}),
        )?;
        self.ask("ui.fit", json!({}))?;
        Ok(())
    }

    /// One step: the window set to `set`, its picture taken and written,
    /// and whether the GPU still draws the viewport on the window's device.
    fn step(&mut self, name: &str, set: &Value) -> Result<(), String> {
        self.ask("ui.set", set.clone())?;
        let shot = self.ask("ui.screenshot", json!({"window": true}))?;
        let picture = shot
            .get("base64")
            .and_then(Value::as_str)
            .and_then(unbase64)
            .ok_or("the picture of the window came without its PNG")?;
        let path = self.folder.join(format!("tour-{name}.png"));
        std::fs::write(&path, &picture)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        say(&format!(
            "{name}: picture {} ({} bytes)",
            path.display(),
            picture.len()
        ));
        let state = self.ask("ui.get", json!({}))?;
        let renderer = state.get("renderer").cloned().unwrap_or(Value::Null);
        let render = state.get("render").cloned().unwrap_or(Value::Null);
        say(&format!(
            "{name}: renderer {renderer}; last frame {}",
            json!({
                "renderer": render.get("renderer"),
                "msaa": render.get("msaa"),
                "count": render.get("count"),
            })
        ));
        let direct = renderer.get("direct").and_then(Value::as_bool) == Some(true);
        let fell_back = ["fallback", "leftGpu"]
            .iter()
            .any(|key| renderer.get(*key).is_some_and(|v| !v.is_null()));
        if !direct || fell_back {
            return Err(
                "the viewport is not drawn by the GPU on the window's device any more".to_string(),
            );
        }
        Ok(())
    }

    /// Sends one request to the window and waits for its answer: the
    /// result, or the error.
    fn ask(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        let (reply, answer) = mpsc::channel();
        self.requests
            .send(Request {
                line,
                reply,
                origin: None,
            })
            .map_err(|_| format!("{method}: the window takes no more requests"))?;
        (self.wake)();
        let answer = match answer.recv_timeout(ANSWER_WAIT) {
            Ok(Some(answer)) => answer,
            Ok(None) => return Err(format!("{method}: no answer")),
            Err(_) => {
                return Err(format!(
                    "{method}: no answer within {} s",
                    ANSWER_WAIT.as_secs()
                ));
            }
        };
        let answer: Value = serde_json::from_str(&answer)
            .map_err(|e| format!("{method}: an answer that is not JSON: {e}"))?;
        if answer.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(answer.get("result").cloned().unwrap_or(Value::Null))
        } else {
            let error = answer
                .get("error")
                .map_or_else(|| "failed".to_string(), Value::to_string);
            Err(format!("{method}: {error}"))
        }
    }
}

/// How a step ended, for its `end` line.
fn outcome(done: &Result<(), String>) -> String {
    match done {
        Ok(()) => String::new(),
        Err(why) => {
            let why: String = why.chars().take(2000).collect();
            format!(": error: {why}")
        }
    }
}

/// The bytes of standard base64 text (padding optional, no line breaks),
/// or `None` for anything else.
fn unbase64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let digits = text.trim_end_matches('=').as_bytes();
    if digits.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(digits.len() / 4 * 3 + 2);
    for chunk in digits.chunks(4) {
        let mut bits = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            bits |= value(*c)? << (18 - 6 * i);
        }
        let bytes = bits.to_be_bytes();
        let take = chunk.len().saturating_sub(1);
        out.extend(bytes.iter().skip(1).take(take));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tour_takes_one_stay_or_quick_and_nothing_else() {
        assert_eq!(parse("1"), Ok(Some(Mode::Close)));
        assert_eq!(parse(" Quick"), Ok(Some(Mode::Quick)));
        assert_eq!(parse(" 1 "), Ok(Some(Mode::Close)));
        assert_eq!(parse("stay"), Ok(Some(Mode::Stay)));
        assert_eq!(parse("STAY"), Ok(Some(Mode::Stay)));
        assert_eq!(parse(""), Ok(None));
        assert_eq!(parse(" \t"), Ok(None));
        for bad in [
            "0", "2", "yes", "true", "01", "1 1", "stay!", "\u{0}", "１", "-1",
        ] {
            let said = parse(bad).expect_err(bad);
            assert!(said.contains(TOUR_VAR), "{said}");
        }
        let huge = "x".repeat(1 << 20);
        assert!(parse(&huge).expect_err("huge").len() < 300);
        let env = |name: &str| (name == TOUR_VAR).then(|| "garbage".to_string());
        assert_eq!(from_env(&env), None);
        let env = |name: &str| (name == TOUR_VAR).then(|| "stay".to_string());
        assert_eq!(from_env(&env), Some(Mode::Stay));
        assert_eq!(from_env(&|_: &str| None), None);
    }

    #[test]
    fn base64_comes_back_as_it_went() {
        for bytes in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            &[0, 255, 128, 7, 254],
        ] {
            let text = craft3d_engine::encode_base64(bytes);
            assert_eq!(unbase64(&text).as_deref(), Some(bytes), "{text}");
        }
        assert_eq!(unbase64("Zm9vYmFy").as_deref(), Some(&b"foobar"[..]));
        for bad in ["Z", "Zm9v!", "Zm 9v", "Zm9vY", "\u{e9}"] {
            assert_eq!(unbase64(bad), None, "{bad}");
        }
    }

    #[test]
    fn pictures_go_beside_the_trace() {
        let env = |name: &str| (name == diag::TRACE_VAR).then(|| "/tmp/x/trace.txt".to_string());
        assert_eq!(picture_folder(&env), PathBuf::from("/tmp/x"));
        let env = |name: &str| (name == diag::TRACE_VAR).then(|| "trace.txt".to_string());
        assert_eq!(picture_folder(&env), PathBuf::from("."));
        assert_eq!(picture_folder(&|_: &str| None), PathBuf::from("."));
    }

    #[test]
    fn the_steps_go_from_plain_to_the_studio_look_cut_open() {
        let names: Vec<&str> = steps().iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "plain",
                "see-through",
                "section",
                "see-through-section",
                "studio",
                "studio-section"
            ]
        );
    }
}
