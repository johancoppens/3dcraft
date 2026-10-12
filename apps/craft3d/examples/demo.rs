//! Builds a small part in a running `craft3d` window, one step at a time,
//! so a person can watch it grow (docs/development.md, "Watching along").
//!
//! ```sh
//! cargo run --release -p craft3d -- --control 7878 \
//!     --control-token-file .private/control.token \
//!     --automation-write-root target/evidence/m4 &
//! cargo run --release -p craft3d --example demo -- --port 7878 \
//!     --token-file .private/control.token --screenshot viewer-demo.png
//! ```
//!
//! Options: `--port PORT`, `--token-file FILE`, `--delay-ms N` (1000),
//! `--part plate|sketch` (plate), `--screenshot PATH` (beneath the app's
//! write root), `--quit` (close the window at the end).
//!
//! It speaks the control protocol and nothing else: every step below is a
//! request any script or agent could send.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::process::ExitCode;
use std::time::Duration;

struct Options {
    port: u16,
    token_file: String,
    delay: Duration,
    part: String,
    screenshot: Option<String>,
    quit: bool,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        port: 7878,
        token_file: ".private/control.token".to_string(),
        delay: Duration::from_millis(1000),
        part: "plate".to_string(),
        screenshot: None,
        quit: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("`{flag}` needs a value"));
        match flag.as_str() {
            "--port" => {
                options.port = value()?
                    .parse()
                    .map_err(|_| "`--port` must be a port number".to_string())?;
            }
            "--token-file" => options.token_file = value()?,
            "--delay-ms" => {
                let ms: u64 = value()?
                    .parse()
                    .map_err(|_| "`--delay-ms` must be a whole number".to_string())?;
                options.delay = Duration::from_millis(ms.min(60_000));
            }
            "--part" => options.part = value()?,
            "--screenshot" => options.screenshot = Some(value()?),
            "--quit" => options.quit = true,
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    Ok(options)
}

struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl Client {
    fn connect(port: u16, token: &str) -> Result<Self, String> {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).map_err(|e| {
            format!(
                "cannot connect to 127.0.0.1:{port}: {e}; is craft3d running with --control {port}?"
            )
        })?;
        let reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|e| format!("cannot use the connection: {e}"))?,
        );
        let mut client = Self {
            reader,
            writer: stream,
            next_id: 1,
        };
        client.call("auth", &json!({ "token": token }))?;
        Ok(client)
    }

    /// Sends one request and returns its result, or its error.
    fn call(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let request = json!({ "id": id, "method": method, "params": params });
        writeln!(self.writer, "{request}")
            .and_then(|()| self.writer.flush())
            .map_err(|e| format!("cannot send {method}: {e}"))?;
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .map_err(|e| format!("no reply to {method}: {e}"))?;
        if read == 0 {
            return Err(format!("the application closed the connection at {method}"));
        }
        let reply: Value =
            serde_json::from_str(&line).map_err(|e| format!("the reply is not JSON: {e}"))?;
        if reply.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(reply.get("result").cloned().unwrap_or(Value::Null))
        } else {
            let error = reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("no error text");
            Err(format!("{method}: {error}"))
        }
    }
}

/// One step of the demo: the caption, the command and its params.
type Step = (&'static str, &'static str, Value);

fn plate_and_post() -> Vec<Step> {
    vec![
        (
            "A new document, in millimetres",
            "file.new",
            json!({"units": "mm"}),
        ),
        (
            "A variable: width = 60 mm",
            "variable.set",
            json!({"name": "width", "expr": "60 mm"}),
        ),
        (
            "A plate, width by two thirds of it, 8 mm thick",
            "feature.block",
            json!({"name": "Plate", "size": ["width", "width * 2 / 3", "8 mm"]}),
        ),
        (
            "A post on the plate, placed and sized by the same variable",
            "feature.cylinder",
            json!({"name": "Post", "position": ["width / 4", "width / 3", "8 mm"],
                   "radius": "width / 10", "height": "30 mm"}),
        ),
        (
            "A sketch on top of the plate",
            "sketch.create",
            json!({"name": "Outline", "plane": "xy", "offset": "8 mm"}),
        ),
        (
            "A circle around the post",
            "sketch.circle",
            json!({"sketch": "Outline", "center": [15, 20], "radius": 10}),
        ),
        (
            "A slot, drawn as a closed polyline",
            "sketch.polyline",
            json!({"sketch": "Outline", "closed": true,
                   "points": [[32, 10], [54, 10], [54, 30], [32, 30]]}),
        ),
        (
            "A construction line through the slot",
            "sketch.line",
            json!({"sketch": "Outline", "from": [32, 20], "to": [54, 20], "construction": true}),
        ),
        (
            "width = 90 mm: everything that depends on it follows",
            "variable.set",
            json!({"name": "width", "expr": "90 mm"}),
        ),
        ("Undo", "edit.undo", json!({})),
    ]
}

fn l_profile() -> Vec<Step> {
    let constrain = |caption: &'static str, kind: &str, entity: &str, value: Option<&str>| {
        let mut params = json!({"sketch": "Profile", "type": kind, "entities": [entity]});
        if let (Some(value), Some(map)) = (value, params.as_object_mut()) {
            map.insert("value".to_string(), value.into());
        }
        (caption, "sketch.constrain", params)
    };
    vec![
        ("A new document", "file.new", json!({"units": "mm"})),
        (
            "An empty sketch on the XY plane",
            "sketch.create",
            json!({"name": "Profile"}),
        ),
        (
            "An L, drawn by eye",
            "sketch.polyline",
            json!({"sketch": "Profile", "closed": true,
                   "points": [[0, 0], [38, 1.5], [41, 6], [4, 4.5], [6.5, 28], [-1, 31]]}),
        ),
        constrain("Fix the corner at the origin", "fixed", "e1", None),
        constrain("Bottom edge horizontal", "horizontal", "e7", None),
        constrain("Right edge vertical", "vertical", "e8", None),
        constrain("Inner step horizontal", "horizontal", "e9", None),
        constrain("Inner edge vertical", "vertical", "e10", None),
        constrain("Top edge horizontal", "horizontal", "e11", None),
        constrain("Left edge vertical", "vertical", "e12", None),
        constrain("Width 40 mm", "distance", "e7", Some("40 mm")),
        constrain("Height 30 mm", "distance", "e12", Some("30 mm")),
        constrain("Thickness 5 mm", "distance", "e8", Some("5 mm")),
        constrain(
            "Thickness 5 mm, fully constrained",
            "distance",
            "e11",
            Some("5 mm"),
        ),
    ]
}

fn run(options: &Options) -> Result<(), String> {
    let token = std::fs::read_to_string(&options.token_file)
        .map_err(|e| format!("cannot read the token file `{}`: {e}", options.token_file))?;
    let mut client = Client::connect(options.port, token.trim())?;
    let (steps, view) = match options.part.as_str() {
        "plate" => (plate_and_post(), "iso"),
        "sketch" => (l_profile(), "top"),
        other => return Err(format!("unknown part `{other}`; use plate or sketch")),
    };
    client.call(
        "ui.set",
        &json!({ "view": view, "camera": { "autoFit": true } }),
    )?;
    let total = steps.len();
    for (index, (caption, command, params)) in steps.into_iter().enumerate() {
        let number = index.saturating_add(1);
        client.call(
            "ui.notice",
            &json!({ "text": format!("{number}/{total}  {caption}") }),
        )?;
        let result = client.call(
            "engine.execute",
            &json!({ "command": command, "params": params }),
        )?;
        let mut shown = result.to_string();
        if shown.len() > 100 {
            shown = shown.chars().take(100).collect::<String>() + "…";
        }
        println!("{number:>2}/{total} {command:<18} {shown}");
        std::thread::sleep(options.delay);
    }
    let measured = client.call("engine.execute", &json!({ "command": "document.measure" }))?;
    if let (Some(volume), Some(unit)) = (
        measured.get("volume").and_then(Value::as_f64),
        measured.get("volumeUnit").and_then(Value::as_str),
    ) {
        println!("volume: {volume:.3} {unit}");
    }
    client.call("ui.notice", &json!({ "text": "Done" }))?;
    if let Some(path) = &options.screenshot {
        let shot = client.call("ui.screenshot", &json!({ "path": path }))?;
        println!("screenshot: {shot}");
    }
    if options.quit {
        client.call("ui.quit", &json!({}))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match options().and_then(|options| run(&options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "demo: {error}");
            ExitCode::FAILURE
        }
    }
}
