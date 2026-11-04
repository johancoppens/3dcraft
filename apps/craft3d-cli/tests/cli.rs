//! The built `craft3d-cli` binary, run as a user or an agent client runs
//! it: arguments in, stdout, stderr and an exit code out.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SUBCOMMANDS: [&str; 10] = [
    "commands",
    "run",
    "info",
    "render",
    "convert",
    "replay",
    "score",
    "check-range",
    "serve",
    "mcp",
];

/// What one run of the binary produced.
struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Ran {
    /// The run succeeded and said nothing on stderr.
    fn ok(self) -> Self {
        assert_eq!(
            self.code, 0,
            "stdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
        assert_eq!(self.stderr, "");
        self
    }

    /// The run failed with this exit code and an error on stderr, and not
    /// by panicking.
    fn failed(self, code: i32) -> Self {
        assert_eq!(
            self.code, code,
            "stdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
        assert!(self.stderr.starts_with("craft3d-cli: "), "{}", self.stderr);
        assert!(
            !self.stderr.contains("panicked") && !self.stderr.contains("RUST_BACKTRACE"),
            "{}",
            self.stderr
        );
        self
    }

    /// stdout as one JSON value per line.
    fn json_lines(&self) -> Vec<Value> {
        self.stdout
            .lines()
            .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
            .collect()
    }
}

fn cli_in(dir: &Path, args: &[&str], stdin: Option<&str>) -> Ran {
    let mut child = Command::new(env!("CARGO_BIN_EXE_craft3d-cli"))
        .args(args)
        .current_dir(dir)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary starts");
    if let Some(input) = stdin {
        // Dropping the handle closes the pipe, which ends a server.
        let mut pipe = child.stdin.take().expect("a piped stdin");
        pipe.write_all(input.as_bytes())
            .expect("the child reads its input");
    }
    let output = child.wait_with_output().expect("the binary exits");
    Ran {
        code: output.status.code().expect("an exit code, not a signal"),
        stdout: String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr is UTF-8"),
    }
}

fn cli(args: &[&str]) -> Ran {
    cli_in(Path::new(env!("CARGO_TARGET_TMPDIR")), args, None)
}

/// [`cli`] with environment variables set (`Some`) or removed (`None`).
fn cli_env(args: &[&str], env: &[(&str, Option<&str>)]) -> Ran {
    let mut command = Command::new(env!("CARGO_BIN_EXE_craft3d-cli"));
    command
        .args(args)
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
        .stdin(Stdio::null());
    for (name, value) in env {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    let output = command.output().expect("the binary runs");
    Ran {
        code: output.status.code().expect("an exit code, not a signal"),
        stdout: String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr is UTF-8"),
    }
}

/// A fresh directory for one test.
fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn path(dir: &Path, name: &str) -> String {
    dir.join(name).to_str().expect("a UTF-8 path").to_string()
}

/// Volumes come from integrating over the faces in floating point: they are
/// compared with a relative tolerance of 1e-9, as in the engine's tests.
fn close(actual: &Value, expected: f64) -> bool {
    let actual = actual.as_f64().expect("a number");
    (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0)
}

/// Builds the part most tests work on: a 40 x 20 x 5 plate with a pin.
fn build_part(dir: &Path) -> String {
    let part = path(dir, "part.3dcraft");
    let ran = cli(&[
        "run",
        "--new",
        "{}",
        "--cmd",
        "variable.set",
        "--params",
        r#"{"name": "width", "expr": "40 mm"}"#,
        "--cmd",
        "feature.block",
        "--params",
        r#"{"name": "Plate", "size": ["width", "width / 2", "5 mm"]}"#,
        "--cmd",
        "feature.cylinder",
        "--params",
        r#"{"name": "Pin", "radius": 4, "height": 20, "position": [20, 10, 5]}"#,
        "--cmd",
        "document.measure",
        "--out",
        &part,
    ])
    .ok();
    let lines = ran.json_lines();
    assert_eq!(
        lines.len(),
        4,
        "one line of JSON per command: {}",
        ran.stdout
    );
    assert_eq!(lines[0]["value"], 40.0);
    assert_eq!(lines[1]["status"], "ok");
    assert_eq!(lines[2]["name"], "Pin");
    // Closed form: 40 x 20 x 5 + pi 4^2 20.
    let expected = 4000.0 + std::f64::consts::PI * 16.0 * 20.0;
    assert!(close(&lines[3]["volume"], expected), "{}", lines[3]);
    part
}

// ------------------------------------------------------ version and help

#[test]
fn version_prints_the_package_version() {
    for flag in ["--version", "-V"] {
        let ran = cli(&[flag]).ok();
        assert_eq!(
            ran.stdout,
            format!("craft3d-cli {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn help_lists_the_subcommands_and_each_describes_itself() {
    let top = cli(&["--help"]).ok().stdout;
    assert!(top.contains("Usage: craft3d-cli <subcommand>"), "{top}");
    for name in SUBCOMMANDS {
        assert!(
            top.lines()
                .any(|line| line.trim_start().starts_with(&format!("{name} "))),
            "`{name}` is listed in:\n{top}"
        );
    }
    // The same text without arguments, with -h and with `help`.
    for args in [&[][..], &["-h"], &["help"]] {
        assert_eq!(cli(args).ok().stdout, top);
    }

    for name in SUBCOMMANDS {
        let help = cli(&[name, "--help"]).ok().stdout;
        assert!(
            help.starts_with(&format!("Usage: craft3d-cli {name}")),
            "{help}"
        );
        assert_ne!(help, top);
        assert!(
            help.lines().count() >= 3,
            "{name} --help says what it does:\n{help}"
        );
        assert_eq!(cli(&["help", name]).ok().stdout, help);
        assert_eq!(cli(&[name, "-h"]).ok().stdout, help);
    }
    // Help wins wherever it stands, and nothing is run or read.
    let help = cli(&["run", "missing.3dcraft", "--cmd", "file.new", "--help"])
        .ok()
        .stdout;
    assert!(help.starts_with("Usage: craft3d-cli run"));
    // Every flag a subcommand's help names is one it accepts.
    let serve = cli(&["serve", "--help"]).ok().stdout;
    assert!(
        serve.contains("--automation-read-root DIR")
            && serve.contains("--automation-write-root DIR")
    );
    let run = cli(&["run", "--help"]).ok().stdout;
    for flag in ["--new", "--cmd", "--params", "--script", "--out"] {
        assert!(run.contains(flag), "{flag} in:\n{run}");
    }
}

#[test]
fn an_unknown_subcommand_is_a_usage_error() {
    let ran = cli(&["frobnicate", "x"]).failed(2);
    assert_eq!(ran.stdout, "");
    assert!(
        ran.stderr.contains("unknown subcommand `frobnicate`"),
        "{}",
        ran.stderr
    );
    // It says what does exist.
    for name in SUBCOMMANDS {
        assert!(ran.stderr.contains(name), "{}", ran.stderr);
    }
    assert!(
        cli(&["help", "frobnicate"])
            .failed(2)
            .stderr
            .contains("`frobnicate`")
    );
}

#[test]
fn an_unknown_flag_is_a_usage_error_that_names_the_flag() {
    let ran = cli(&["--bogus"]).failed(2);
    assert!(
        ran.stderr.contains("unknown flag `--bogus`"),
        "{}",
        ran.stderr
    );
    for name in SUBCOMMANDS {
        // stdin is closed, so a server that wrongly started would exit 0.
        let ran = cli(&[name, "--bogus", "value"]).failed(2);
        assert_eq!(ran.stdout, "");
        assert!(
            ran.stderr.contains("unknown flag `--bogus`"),
            "{name}: {}",
            ran.stderr
        );
        assert!(
            ran.stderr.contains(&format!("craft3d-cli {name} --help")),
            "{name} points at its help: {}",
            ran.stderr
        );
        let ran = cli(&[name, "-z"]).failed(2);
        assert!(
            ran.stderr.contains("unknown flag `-z`"),
            "{name}: {}",
            ran.stderr
        );
    }
}

#[test]
fn other_usage_errors_exit_with_2() {
    let dir = workdir("usage");
    let part = build_part(&dir);
    for (args, expected) in [
        (vec!["info"], "missing FILE"),
        (vec!["info", &part, "extra"], "unexpected argument `extra`"),
        (vec!["render", &part], "missing `--out PNG`"),
        (vec!["render", "--out", "x.png"], "missing FILE"),
        (vec!["render", &part, "--out"], "`--out` needs a value"),
        (
            vec!["render", &part, "--out", "x.png", "--size", "big"],
            "`--size` must be WIDTHxHEIGHT",
        ),
        (
            vec!["render", &part, "--out", "x.png", "--out", "y.png"],
            "`--out` was given twice",
        ),
        // No window: nothing selected or clicked to focus on.
        (
            vec!["render", &part, "--out", "x.png", "--focus", "selection"],
            "--focus selection: render has no window",
        ),
        (
            vec!["render", &part, "--out", "x.png", "--focus", "picked"],
            "--focus picked: render has no window",
        ),
        (vec!["convert", &part], "IN OUT"),
        // How a drawing of the model as seen looks at it: only for such
        // a drawing.
        (
            vec!["convert", &part, "a.stl", "--view", "top"],
            "`--view`, `--projection` and `--hidden` are for a drawing of the model as seen",
        ),
        (
            vec![
                "convert", &part, "a.dxf", "--sketch", "S", "--hidden", "dashed",
            ],
            "without `--sketch`, `--face` or `--set`",
        ),
        (
            vec!["convert", &part, "a.svg", "--feature", "Pin"],
            "give it with `--face`",
        ),
        (
            vec!["convert", &part, "a.stl", "b.stl"],
            "unexpected argument `b.stl`",
        ),
        (
            vec!["convert", &part, "a.stl", "--tolerance", "0"],
            "`--tolerance` must be a positive number",
        ),
        (
            vec!["convert", &part, "a.stl", "--tolerance", "fine"],
            "`--tolerance` must be a positive number",
        ),
        (
            vec!["convert", &part, "a.3dcraft", "--tolerance", "0.1"],
            "not to .3dcraft",
        ),
        (vec!["run", "--params", "{}"], "`--params` must follow"),
        (
            vec!["run", "--cmd", "file.new", "--params", "not json"],
            "`--params` is not valid JSON",
        ),
        (vec!["run", &part, "--new", "{}"], "not both"),
        (vec!["commands", "fillet"], "unexpected argument `fillet`"),
        (vec!["serve", "extra"], "unexpected argument `extra`"),
        (
            vec!["mcp", "--automation-read-root"],
            "`--automation-read-root` needs a value",
        ),
        (vec!["--version=1"], "`--version` takes no value"),
    ] {
        let ran = cli_in(&dir, &args, None).failed(2);
        assert!(ran.stderr.contains(expected), "{args:?}: {}", ran.stderr);
        assert_eq!(ran.stdout, "", "{args:?}");
    }
    assert!(
        !dir.join("x.png").exists()
            && !dir.join("a.stl").exists()
            && !dir.join("a.3dcraft").exists()
            && !dir.join("a.dxf").exists()
    );
}

/// `mcp --help` names every tool the server lists, and the one a window
/// adds, from the same list (it named five of six until 2026-10-22).
#[test]
fn mcp_help_lists_every_tool_the_server_serves() {
    let dir = workdir("mcp-help");
    let input = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string();
    let ran = cli_in(&dir, &["mcp"], Some(&input)).ok();
    let tools = ran.json_lines()[0]["result"]["tools"].clone();
    let names: Vec<String> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"selection_get".to_string()), "{names:?}");
    let help = cli(&["mcp", "--help"]).ok().stdout;
    for name in names.iter().map(String::as_str).chain(["control_call"]) {
        assert!(
            help.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| word == name),
            "{name} in:\n{help}"
        );
    }
}

// --------------------------------------------------------------- commands

#[test]
fn commands_prints_the_registry_and_filters_it() {
    let all = cli(&["commands"]).ok().stdout;
    let blocks: Vec<&str> = all.trim_end().split("\n\n").collect();
    // One block per command of the registry, whatever their number.
    let registered = craft3d_engine::Session::commands().len();
    assert_eq!(blocks.len(), registered, "one block per command:\n{all}");
    for block in &blocks {
        let lines: Vec<&str> = block.lines().collect();
        assert_eq!(lines.len(), 3, "{block}");
        assert!(
            lines[0].contains('.') && !lines[0].starts_with(' '),
            "an id: {block}"
        );
        assert!(lines[1].starts_with("  params:  {"), "{block}");
        assert!(lines[2].starts_with("  summary: "), "{block}");
    }
    assert!(blocks[0].starts_with("file.new\n"));
    assert!(all.contains("feature.block\n  params:  {\"size\": [expr, expr, expr]"));

    let filtered = cli(&["commands", "--filter", "VARIABLE."]).ok().stdout;
    let ids: Vec<&str> = filtered
        .lines()
        .filter(|line| !line.starts_with(' ') && !line.is_empty())
        .collect();
    assert_eq!(
        ids,
        [
            "variable.set",
            "variable.delete",
            "variable.rename",
            "variable.measure",
            "variable.output",
            "variable.publish",
            "variable.unpublish"
        ]
    );
    // By label: "Roll Back".
    assert!(
        cli(&["commands", "--filter=roll b"])
            .ok()
            .stdout
            .starts_with("history.rollback\n")
    );
    // (A word no command has: until 2026-10-14 this was `fillet`.)
    let none = cli(&["commands", "--filter", "zeppelin"]).ok().stdout;
    assert_eq!(none, "No command matches `zeppelin`.\n");
}

// ------------------------------------------- run, info, render, convert

#[test]
fn run_builds_a_part_that_info_render_and_convert_read() {
    let dir = workdir("pipeline");
    let part = build_part(&dir);
    let bytes = std::fs::read(&part).unwrap();
    assert!(
        bytes.starts_with(b"PK\x03\x04"),
        "a .3dcraft file is a zip container"
    );

    // info: pretty JSON of document.inspect.
    let ran = cli(&["info", &part]).ok();
    assert!(
        ran.stdout.lines().count() > 20,
        "pretty-printed:\n{}",
        ran.stdout
    );
    let info: Value = serde_json::from_str(&ran.stdout).unwrap();
    assert_eq!(info["units"]["length"], "mm");
    assert_eq!(info["variables"][0]["name"], "width");
    assert_eq!(info["features"].as_array().unwrap().len(), 2);
    assert_eq!(info["ok"], true);
    assert_eq!(info["dirty"], false);
    let plate = &info["bodies"][0];
    assert_eq!(plate["name"], "Plate");
    assert!(close(&plate["volume"], 4000.0), "{plate}");
    assert_eq!(
        (plate["faces"].as_u64(), plate["edges"].as_u64()),
        (Some(6), Some(12))
    );

    // render: a PNG of the size asked for.
    let shot = path(&dir, "shot.png");
    let ran = cli(&[
        "render", &part, "--out", &shot, "--size", "320x240", "--view", "top", "--mode", "ids",
    ])
    .ok();
    let drawn = &ran.json_lines()[0];
    assert_eq!(drawn["view"], "top");
    assert!(drawn["coverage"].as_f64().unwrap() > 0.05, "{drawn}");
    assert!(drawn.get("base64").is_none(), "the image goes to the file");
    let png = std::fs::read(&shot).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 320);
    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 240);
    // A lens on the model's centre, given by name.
    let lens_shot = path(&dir, "lens.png");
    let ran = cli(&[
        "render", &part, "--out", &lens_shot, "--size", "96x72", "--fstop", "4", "--focus",
        "centre",
    ])
    .ok();
    let drawn = &ran.json_lines()[0];
    assert_eq!(drawn["look"]["dof"]["focus"], "centre", "{drawn}");
    assert_eq!(drawn["lens"]["fallback"], Value::Null, "{drawn}");
    assert!(std::fs::read(&lens_shot).is_ok());
    // The default size.
    let default_shot = path(&dir, "default.png");
    cli(&["render", &part, "--out", &default_shot]).ok();
    let png = std::fs::read(&default_shot).unwrap();
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 800);
    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 600);

    // convert: each mesh format, by extension.
    let stl = path(&dir, "part.stl");
    let exported = cli(&["convert", &part, &stl]).ok().json_lines().remove(0);
    assert_eq!(exported["bodies"], 2);
    let triangles = exported["triangles"].as_u64().unwrap();
    let bytes = std::fs::read(&stl).unwrap();
    // Binary STL has no magic number; its layout is the check: an 80-byte
    // header, a little-endian triangle count, 50 bytes per triangle.
    assert_eq!(
        u64::from(u32::from_le_bytes(bytes[80..84].try_into().unwrap())),
        triangles
    );
    assert_eq!(bytes.len() as u64, 84 + 50 * triangles);

    let three_mf = path(&dir, "part.3mf");
    cli(&["convert", &part, &three_mf]).ok();
    let bytes = std::fs::read(&three_mf).unwrap();
    assert!(
        bytes.starts_with(b"PK\x03\x04"),
        "a 3MF file is a zip container"
    );
    assert!(
        bytes.windows(19).any(|w| w == b"[Content_Types].xml"),
        "an OPC package names its content types part"
    );

    let obj = path(&dir, "part.obj");
    let exported = cli(&["convert", &part, &obj]).ok().json_lines().remove(0);
    let text = std::fs::read_to_string(&obj).unwrap();
    let faces = text.lines().filter(|line| line.starts_with("f ")).count() as u64;
    assert_eq!(
        Some(faces),
        exported["triangles"].as_u64(),
        "one `f` line per triangle"
    );
    assert!(text.lines().any(|line| line.starts_with("v ")));

    // A finer tolerance gives more triangles on the pin.
    let fine = path(&dir, "fine.stl");
    let exported = cli(&["convert", &part, &fine, "--tolerance", "0.001"])
        .ok()
        .json_lines()
        .remove(0);
    assert_eq!(exported["tolerance"], 0.001);
    assert!(
        exported["triangles"].as_u64().unwrap() > triangles,
        "{exported}"
    );
    // The quality and the angle (2026-10-23): fine has more triangles
    // than coarse; the angle is what was given.
    let count = |quality: &str| {
        let out = path(&dir, &format!("{quality}.stl"));
        let exported = cli(&["convert", &part, &out, "--quality", quality])
            .ok()
            .json_lines()
            .remove(0);
        assert_eq!(exported["quality"], quality, "{exported}");
        exported["triangles"].as_u64().unwrap()
    };
    assert!(count("coarse") < count("fine"));
    let angled = path(&dir, "angled.stl");
    let exported = cli(&["convert", &part, &angled, "--angle", "30"])
        .ok()
        .json_lines()
        .remove(0);
    assert_eq!(exported["angle"], 30.0);
    cli(&["convert", &part, &angled, "--angle", "0.5"]).failed(1);
    cli(&["convert", &part, &angled, "--angle", "steep"]).failed(2);
    cli(&["convert", &part, &path(&dir, "q.step"), "--quality", "fine"]).failed(2);

    // convert to STEP: the exact geometry, as text.
    let step = path(&dir, "part.step");
    let exported = cli(&["convert", &part, &step]).ok().json_lines().remove(0);
    assert_eq!(exported["format"], "step", "{exported}");
    assert_eq!(exported["bodies"], exported["solids"], "{exported}");
    let text = std::fs::read_to_string(&step).unwrap();
    assert!(text.starts_with("ISO-10303-21;\nHEADER;"));
    assert!(text.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"));
    assert_eq!(
        text.matches("MANIFOLD_SOLID_BREP(").count() as u64,
        exported["solids"].as_u64().unwrap()
    );
    assert_eq!(
        exported["bytes"].as_u64().unwrap() as usize,
        text.len(),
        "{exported}"
    );
    let ran = cli(&["convert", &part, &step, "--tolerance", "0.1"]).failed(2);
    assert!(
        ran.stderr.contains("not to .3dcraft or STEP"),
        "{}",
        ran.stderr
    );

    // convert to DXF and SVG: the outline of a flat face, at true size.
    // The plate's top is 40 × 20; the pin's is a circle of radius 4.
    let dxf = path(&dir, "top.dxf");
    let exported = cli(&["convert", &part, &dxf, "--face", "F1:face(1)"])
        .ok()
        .json_lines()
        .remove(0);
    assert_eq!(exported["format"], "dxf", "{exported}");
    assert_eq!(exported["lines"], 4, "{exported}");
    assert!(close(&exported["length"], 120.0), "{exported}");
    let text = std::fs::read_to_string(&dxf).unwrap();
    assert!(text.contains("$INSUNITS\n 70\n4\n") && text.ends_with("  0\nEOF\n"));
    assert_eq!(text.matches("  0\nLINE\n").count(), 4);
    let svg = path(&dir, "pin.svg");
    let exported = cli(&[
        "convert",
        &part,
        &svg,
        "--face",
        "F2:face(2)",
        "--feature",
        "Pin",
    ])
    .ok()
    .json_lines()
    .remove(0);
    assert_eq!(exported["circles"], 1, "{exported}");
    assert!(
        close(&exported["length"], 8.0 * std::f64::consts::PI),
        "{exported}"
    );
    let text = std::fs::read_to_string(&svg).unwrap();
    assert!(text.contains("width=\"8mm\" height=\"8mm\""), "{text}");
    // Without a sketch, a face or a set a drawing is of the model as
    // seen, as file.export makes it (until 2026-10-23 convert refused it
    // as a usage error): seen from the front, orthographic, the plate
    // (40 wide, 5 high) with the pin (20 high) on it is 40 × 25 at true
    // size, nothing behind a face drawn.
    let front = path(&dir, "front.dxf");
    let exported = cli(&["convert", &part, &front, "--view", "front"])
        .ok()
        .json_lines()
        .remove(0);
    assert_eq!(exported["view"], "front", "{exported}");
    assert_eq!(exported["projection"], "orthographic", "{exported}");
    assert_eq!(exported["hiddenLines"], 0, "{exported}");
    let bbox: Vec<f64> = exported["bbox"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert!(
        (bbox[2] - bbox[0] - 40.0).abs() < 1e-6 && (bbox[3] - bbox[1] - 25.0).abs() < 1e-6,
        "{exported}"
    );
    assert!(
        std::fs::read_to_string(&front)
            .unwrap()
            .ends_with("  0\nEOF\n")
    );
    // From the default iso view with what lies behind dashed: the plate's
    // edges at the back are drawn, on the layer hidden.
    let iso = path(&dir, "iso.svg");
    let exported = cli(&[
        "convert",
        &part,
        &iso,
        "--hidden",
        "dashed",
        "--projection",
        "perspective",
    ])
    .ok()
    .json_lines()
    .remove(0);
    assert_eq!(exported["view"], "iso", "{exported}");
    assert_eq!(exported["projection"], "perspective", "{exported}");
    assert!(exported["hiddenLines"].as_u64().unwrap() > 0, "{exported}");
    assert!(
        exported["layers"]
            .as_array()
            .unwrap()
            .contains(&json!("hidden"))
    );
    // A view the engine does not know is its error (exit 1).
    let ran = cli(&[
        "convert",
        &part,
        &path(&dir, "none.dxf"),
        "--view",
        "sideways",
    ])
    .failed(1);
    assert!(ran.stderr.contains("sideways"), "{}", ran.stderr);
    let ran = cli(&[
        "convert",
        &part,
        &path(&dir, "none.stl"),
        "--face",
        "F1:face(1)",
    ])
    .failed(1);
    assert!(ran.stderr.contains(".dxf or .svg"), "{}", ran.stderr);
    let ran = cli(&[
        "convert",
        &part,
        &path(&dir, "none.3dcraft"),
        "--sketch",
        "S",
    ])
    .failed(2);
    assert!(ran.stderr.contains("--sketch"), "{}", ran.stderr);
    assert!(!dir.join("none.dxf").exists() && !dir.join("none.stl").exists());

    // convert to .3dcraft: the same document again.
    let copy = path(&dir, "copy.3dcraft");
    cli(&["convert", &part, &copy]).ok();
    assert!(std::fs::read(&copy).unwrap().starts_with(b"PK\x03\x04"));
    let mut copied: Value = serde_json::from_str(&cli(&["info", &copy]).ok().stdout).unwrap();
    let mut original = info;
    // Only the path differs.
    assert_eq!(copied["path"], copy);
    copied["path"] = Value::Null;
    original["path"] = Value::Null;
    assert_eq!(copied, original);
}

#[test]
fn run_opens_a_file_edits_it_and_saves_a_new_one() {
    let dir = workdir("edit");
    let part = build_part(&dir);
    let wider = path(&dir, "wider.3dcraft");
    let ran = cli(&[
        "run",
        &part,
        "--cmd",
        "variable.set",
        "--params",
        r#"{"name": "width", "expr": "60 mm"}"#,
        "--out",
        &wider,
    ])
    .ok();
    assert_eq!(ran.json_lines()[0]["value"], 60.0);
    let info: Value = serde_json::from_str(&cli(&["info", &wider]).ok().stdout).unwrap();
    // 60 x 30 x 5.
    assert!(close(&info["bodies"][0]["volume"], 9000.0), "{info}");
    // The file that was opened is untouched.
    let info: Value = serde_json::from_str(&cli(&["info", &part]).ok().stdout).unwrap();
    assert!(close(&info["bodies"][0]["volume"], 4000.0), "{info}");
}

#[test]
fn run_takes_a_script_and_runs_it_before_the_cmd_flags() {
    let dir = workdir("script");
    let script = path(&dir, "steps.json");
    std::fs::write(
        &script,
        r#"[
            ["variable.set", {"name": "r", "expr": "3 mm"}],
            {"command": "feature.sphere", "params": {"radius": "r", "name": "Ball"}},
            ["document.measure"]
        ]"#,
    )
    .unwrap();
    // `--cmd` comes first on the command line and still runs last.
    let ran = cli(&[
        "run",
        "--cmd",
        "feature.rename",
        "--params",
        r#"{"feature": "Ball", "name": "Orb"}"#,
        "--script",
        &script,
        "--new",
        r#"{"units": "cm"}"#,
    ])
    .ok();
    let lines = ran.json_lines();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0]["lengthUnit"], "cm", "--new reached file.new");
    assert_eq!(lines[1]["name"], "Ball");
    // Closed form: 4/3 pi r^3, with 3 mm = 0.3 cm.
    assert!(
        close(
            &lines[2]["volume"],
            4.0 / 3.0 * std::f64::consts::PI * 0.027
        ),
        "{}",
        lines[2]
    );
    assert_eq!(lines[3]["name"], "Orb");

    // A script that cannot be used fails before anything runs.
    std::fs::write(&script, r#"[["file.new"], "document.measure"]"#).unwrap();
    let ran = cli(&[
        "run",
        "--script",
        &script,
        "--out",
        &path(&dir, "never.3dcraft"),
    ])
    .failed(1);
    assert!(ran.stderr.contains("step 1 must be"), "{}", ran.stderr);
    assert_eq!(ran.stdout, "");
    assert!(!dir.join("never.3dcraft").exists());
    let ran = cli(&["run", "--script", &path(&dir, "missing.json")]).failed(1);
    assert!(ran.stderr.contains("cannot read script"), "{}", ran.stderr);
}

#[test]
fn run_stops_at_the_first_failing_command_with_exit_code_1() {
    let dir = workdir("failing");
    let never = path(&dir, "never.3dcraft");
    let ran = cli(&[
        "run",
        "--cmd",
        "feature.block",
        "--params",
        r#"{"size": [1, 2, 3]}"#,
        "--cmd",
        "feature.blok",
        "--cmd",
        "document.measure",
        "--out",
        &never,
    ])
    .failed(1);
    assert_eq!(
        ran.json_lines().len(),
        1,
        "only the first command ran: {}",
        ran.stdout
    );
    assert_eq!(
        ran.stderr,
        "craft3d-cli: unknown command `feature.blok`; did you mean `feature.block`?\n"
    );
    assert!(
        !Path::new(&never).exists(),
        "nothing is saved after a failure"
    );

    // Bad params of a known command fail the same way.
    let ran = cli(&[
        "run",
        "--cmd",
        "feature.block",
        "--params",
        r#"{"size": [1, 2]}"#,
    ])
    .failed(1);
    assert!(
        ran.stderr.contains("feature.block") && ran.stderr.contains("size"),
        "{}",
        ran.stderr
    );
    // So do bad params of the new document.
    let ran = cli(&["run", "--new", r#"{"units": "furlong"}"#]).failed(1);
    assert!(ran.stderr.contains("furlong"), "{}", ran.stderr);
}

/// A `variable.set` whose expression the engine keeps but cannot work out
/// stops a run like a refused command (a Jig build, 2026-10-23: the steps
/// after it failed one by one, each for the same reason); with
/// `--keep-going` it is a failed line.
#[test]
fn run_stops_at_a_variable_that_cannot_be_worked_out() {
    let dir = workdir("failing-variable");
    let never = path(&dir, "never.3dcraft");
    let ran = cli(&[
        "run",
        "--cmd",
        "variable.set",
        "--params",
        r#"{"name": "w", "expr": "40 mm"}"#,
        "--cmd",
        "variable.set",
        "--params",
        r#"{"name": "d", "expr": "w + 1"}"#,
        "--cmd",
        "feature.block",
        "--params",
        r#"{"size": ["w", "d", "1 mm"]}"#,
        "--out",
        &never,
    ])
    .failed(1);
    assert_eq!(ran.json_lines().len(), 1, "{}", ran.stdout);
    assert!(
        ran.stderr
            .contains("variable.set: `d` cannot be worked out"),
        "{}",
        ran.stderr
    );
    assert!(!Path::new(&never).exists());

    let ran = cli(&[
        "run",
        "--keep-going",
        "--cmd",
        "variable.set",
        "--params",
        r#"{"name": "d", "expr": "nowhere * 2"}"#,
        "--cmd",
        "variable.set",
        "--params",
        r#"{"name": "w", "expr": "40 mm"}"#,
    ])
    .failed(1);
    let lines = ran.json_lines();
    assert_eq!(lines.len(), 2, "{}", ran.stdout);
    assert_eq!(lines[0]["failed"], "variable.set", "{}", ran.stdout);
    assert_eq!(lines[1]["name"], "w", "{}", ran.stdout);
    assert!(
        ran.stderr.contains("1 of 2 commands failed"),
        "{}",
        ran.stderr
    );
}

/// With `--keep-going` a failing command does not throw away what the
/// others built (showcase U1, 2026-10-15: a failing last command lost a
/// ten-minute build): it is reported on its line, the rest run, `--out`
/// saves the document, and the exit code still says something failed.
#[test]
fn run_keep_going_runs_past_a_failure_and_saves_what_was_built() {
    let dir = workdir("keep-going");
    let kept = path(&dir, "kept.3dcraft");
    let ran = cli(&[
        "run",
        "--keep-going",
        "--cmd",
        "feature.block",
        "--params",
        r#"{"size": [1, 2, 3]}"#,
        "--cmd",
        "feature.blok",
        "--cmd",
        "feature.cylinder",
        "--params",
        r#"{"radius": 1, "height": 1, "position": [5, 0, 0]}"#,
        "--cmd",
        "feature.block",
        "--params",
        r#"{"size": [1, 2]}"#,
        "--out",
        &kept,
    ])
    .failed(1);
    let lines = ran.json_lines();
    assert_eq!(lines.len(), 4, "a line per command: {}", ran.stdout);
    assert_eq!(lines[1]["failed"], "feature.blok", "{}", ran.stdout);
    assert_eq!(lines[1]["step"], 1);
    assert!(
        lines[1]["error"]
            .as_str()
            .unwrap()
            .contains("unknown command")
    );
    assert_eq!(lines[2]["status"], "ok", "{}", ran.stdout);
    assert_eq!(lines[3]["failed"], "feature.block");
    assert!(
        ran.stderr
            .starts_with("craft3d-cli: 2 of 4 commands failed, the first step 1 `feature.blok`")
            && ran.stderr.contains(&kept),
        "{}",
        ran.stderr
    );
    // The two features that were built are in the file.
    let info = cli(&["info", &kept]).ok();
    let info: Value = serde_json::from_str(&info.stdout).unwrap();
    assert_eq!(info["features"].as_array().unwrap().len(), 2, "{info}");

    // Without a failure it is a run like any other.
    cli(&[
        "run",
        "--keep-going",
        "--cmd",
        "feature.block",
        "--params",
        r#"{"size": [1, 2, 3]}"#,
    ])
    .ok();
    // Given twice, or with a value, it is a usage error.
    cli(&["run", "--keep-going", "--keep-going"]).failed(2);
    cli(&["run", "--keep-going=yes"]).failed(2);
}

#[test]
fn files_that_cannot_be_read_or_written_fail_with_exit_code_1() {
    let dir = workdir("files");
    let part = build_part(&dir);
    let missing = path(&dir, "missing.3dcraft");
    for args in [
        vec!["info", missing.as_str()],
        vec!["run", &missing],
        vec!["render", &missing, "--out", "x.png"],
        vec!["convert", &missing, "x.stl"],
    ] {
        let ran = cli_in(&dir, &args, None).failed(1);
        assert!(
            ran.stderr.contains("missing.3dcraft"),
            "{args:?}: {}",
            ran.stderr
        );
        assert_eq!(ran.stdout, "");
    }
    // Not a .3dcraft file at all.
    let junk = path(&dir, "junk.3dcraft");
    std::fs::write(&junk, b"PK\x03\x04 this is not a document").unwrap();
    cli(&["info", &junk]).failed(1);
    // An extension no exporter knows: the engine names the ones it does.
    let ran = cli(&["convert", &part, &path(&dir, "part.dwg")]).failed(1);
    assert!(ran.stderr.contains(".stl"), "{}", ran.stderr);
    assert!(ran.stderr.contains(".step"), "{}", ran.stderr);
    assert!(ran.stderr.contains(".dxf"), "{}", ran.stderr);
    assert!(!dir.join("part.dwg").exists());
    // A directory that does not exist.
    cli(&["convert", &part, &path(&dir, "no/such/dir/part.stl")]).failed(1);
    // Values the engine refuses.
    let ran = cli(&[
        "render",
        &part,
        "--out",
        &path(&dir, "x.png"),
        "--size",
        "8x8",
    ])
    .failed(1);
    assert!(ran.stderr.contains("width"), "{}", ran.stderr);
    let ran = cli(&[
        "render",
        &part,
        "--out",
        &path(&dir, "x.png"),
        "--view",
        "sideways",
    ])
    .failed(1);
    assert!(ran.stderr.contains("sideways"), "{}", ran.stderr);
    assert!(!dir.join("x.png").exists());
}

// ---------------------------------------------------------- serve and mcp

#[test]
fn serve_answers_json_lines_on_stdin() {
    let dir = workdir("serve");
    let input = [
        r#"{"id": 1, "method": "engine.execute", "params": {"command": "file.new"}}"#,
        "this is not json",
        r#"{"id": 2, "method": "batch", "params": {"steps": [{"command": "feature.block", "params": {"size": [2, 3, 4]}}, {"command": "document.measure"}]}}"#,
        r#"{"id": 3, "method": "engine.commands", "params": {"filter": "undo"}}"#,
        r#"{"id": 4, "method": "nope"}"#,
        r#"{"id": 5, "method": "methods"}"#,
    ]
    .join("\n");
    let ran = cli_in(&dir, &["serve"], Some(&input)).ok();
    let replies = ran.json_lines();
    assert_eq!(replies.len(), 6, "{}", ran.stdout);
    assert_eq!(
        replies[0],
        json!({"id": 1, "ok": true, "result": {"document": 0, "units": "mm"}})
    );
    assert_eq!(replies[1]["id"], Value::Null);
    assert_eq!(replies[1]["ok"], false);
    assert_eq!(replies[2]["result"]["completed"], 2);
    assert!(close(
        &replies[2]["result"]["results"][1]["result"]["volume"],
        24.0
    ));
    assert_eq!(replies[3]["result"]["commands"][0]["id"], "edit.undo");
    assert_eq!(replies[4]["ok"], false);
    assert_eq!(replies[5]["result"]["methods"].as_array().unwrap().len(), 4);
    // An empty input is a session with nothing to do.
    assert_eq!(cli_in(&dir, &["serve"], Some("")).ok().stdout, "");
}

/// The requests of a session that builds a block and tries to save it.
fn save_session(path: &str) -> String {
    [
        json!({"id": 1, "method": "engine.execute", "params": {"command": "file.new"}}),
        json!({"id": 2, "method": "engine.execute", "params": {"command": "feature.block", "params": {"size": [1, 2, 3]}}}),
        json!({"id": 3, "method": "engine.execute", "params": {"command": "file.save", "params": {"path": path}}}),
        json!({"id": 4, "method": "engine.execute", "params": {"command": "file.open", "params": {"path": path}}}),
    ]
    .map(|request| request.to_string())
    .join("\n")
}

#[test]
fn serve_has_no_file_access_without_roots() {
    let dir = workdir("serve-no-roots");
    // Relative (to the directory the server runs in) and absolute.
    for target in ["saved.3dcraft".to_string(), path(&dir, "saved.3dcraft")] {
        let ran = cli_in(&dir, &["serve"], Some(&save_session(&target))).ok();
        let replies = ran.json_lines();
        assert_eq!(replies.len(), 4);
        assert_eq!(replies[1]["ok"], true);
        assert_eq!(replies[2]["ok"], false, "{}", replies[2]);
        assert!(
            replies[2]["error"]
                .as_str()
                .unwrap()
                .contains("no write access"),
            "{}",
            replies[2]
        );
        assert_eq!(replies[3]["ok"], false);
        assert!(
            replies[3]["error"]
                .as_str()
                .unwrap()
                .contains("no read access"),
            "{}",
            replies[3]
        );
    }
    assert!(!dir.join("saved.3dcraft").exists());
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "nothing was written"
    );
}

#[test]
fn serve_writes_beneath_the_write_root_and_reads_beneath_the_read_root() {
    let dir = workdir("serve-roots");
    let (inbox, outbox) = (dir.join("in"), dir.join("out"));
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::create_dir_all(&outbox).unwrap();

    // A write root only: saving works, opening does not. The root is given
    // relative to the working directory and still resolves.
    let ran = cli_in(
        &dir,
        &["serve", "--automation-write-root", "out"],
        Some(&save_session("saved.3dcraft")),
    )
    .ok();
    let replies = ran.json_lines();
    assert_eq!(replies[2]["ok"], true, "{}", replies[2]);
    assert!(
        std::fs::read(outbox.join("saved.3dcraft"))
            .unwrap()
            .starts_with(b"PK")
    );
    assert!(
        !dir.join("saved.3dcraft").exists(),
        "not in the working directory"
    );
    assert_eq!(replies[3]["ok"], false, "the write root grants no reading");
    assert!(
        replies[3]["error"]
            .as_str()
            .unwrap()
            .contains("no read access")
    );

    // Outside the root: refused, by `..` and by an absolute path.
    for escape in [
        "../escaped.3dcraft".to_string(),
        path(&inbox, "escaped.3dcraft"),
    ] {
        let ran = cli_in(
            &dir,
            &["serve", "--automation-write-root", "out"],
            Some(&save_session(&escape)),
        )
        .ok();
        let replies = ran.json_lines();
        assert_eq!(replies[2]["ok"], false, "{escape}");
        assert!(
            replies[2]["error"].as_str().unwrap().contains("write root"),
            "{}",
            replies[2]
        );
    }
    assert!(!dir.join("escaped.3dcraft").exists() && !inbox.join("escaped.3dcraft").exists());

    // A read root only: opening works, saving does not.
    std::fs::copy(outbox.join("saved.3dcraft"), inbox.join("saved.3dcraft")).unwrap();
    let ran = cli_in(
        &dir,
        &[
            "serve",
            &format!("--automation-read-root={}", inbox.display()),
        ],
        Some(&save_session("saved.3dcraft")),
    )
    .ok();
    let replies = ran.json_lines();
    assert_eq!(replies[2]["ok"], false);
    assert!(
        replies[2]["error"]
            .as_str()
            .unwrap()
            .contains("no write access")
    );
    assert_eq!(replies[3]["ok"], true, "{}", replies[3]);
    assert_eq!(replies[3]["result"]["features"], 1);

    // Both, each in its own direction.
    let ran = cli_in(
        &dir,
        &[
            "serve",
            "--automation-read-root",
            "in",
            "--automation-write-root",
            "out",
        ],
        Some(&save_session("both.3dcraft")),
    )
    .ok();
    let replies = ran.json_lines();
    assert_eq!(replies[2]["ok"], true);
    assert_eq!(
        replies[3]["ok"], false,
        "saved under `out`, so not found under `in`: {}",
        replies[3]
    );
    assert!(outbox.join("both.3dcraft").exists() && !inbox.join("both.3dcraft").exists());
}

#[test]
fn a_root_must_be_an_existing_directory() {
    let dir = workdir("bad-roots");
    std::fs::write(dir.join("file.txt"), "x").unwrap();
    for subcommand in ["serve", "mcp"] {
        for flag in ["--automation-read-root", "--automation-write-root"] {
            let ran = cli_in(&dir, &[subcommand, flag, "no-such-dir"], Some("")).failed(1);
            assert!(
                ran.stderr.contains(flag) && ran.stderr.contains("no-such-dir"),
                "{}",
                ran.stderr
            );
            let ran = cli_in(&dir, &[subcommand, flag, "file.txt"], Some("")).failed(1);
            assert!(ran.stderr.contains("not a directory"), "{}", ran.stderr);
        }
        let ran = cli_in(
            &dir,
            &[
                subcommand,
                "--automation-read-root",
                ".",
                "--automation-read-root",
                ".",
            ],
            Some(""),
        )
        .failed(2);
        assert!(ran.stderr.contains("given twice"), "{}", ran.stderr);
    }
}

#[test]
fn mcp_answers_json_rpc_on_stdin() {
    let dir = workdir("mcp");
    std::fs::create_dir_all(dir.join("out")).unwrap();
    let call = |id: u64, name: &str, arguments: Value| {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}}).to_string()
    };
    let input = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "cli-test", "version": "1"}}})
        .to_string(),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string(),
        "{ this is not json".to_string(),
        call(3, "command_run", json!({"id": "file.new"})),
        call(4, "command_run", json!({"id": "feature.block", "params": {"size": [40, 20, 5]}})),
        call(5, "doc_inspect", json!({})),
        call(6, "command_run", json!({"id": "file.export", "params": {"path": "part.3mf"}})),
        call(7, "command_run", json!({"id": "file.export", "params": {"path": "../escaped.3mf"}})),
        call(8, "doc_render_preview", json!({"max_side": 64})),
        call(9, "no_such_tool", json!({})),
        json!({"jsonrpc": "2.0", "id": 10, "method": "ping"}).to_string(),
    ]
    .join("\n");
    let ran = cli_in(
        &dir,
        &["mcp", "--automation-write-root", "out"],
        Some(&input),
    )
    .ok();
    let replies = ran.json_lines();
    // Eleven requests and one notification.
    assert_eq!(replies.len(), 11, "{}", ran.stdout);
    assert!(replies.iter().all(|reply| reply["jsonrpc"] == "2.0"));

    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(
        replies[0]["result"]["serverInfo"],
        json!({"name": "craft3d", "version": env!("CARGO_PKG_VERSION")})
    );
    assert_eq!(replies[1]["id"], 2);
    assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 6);
    assert_eq!(replies[2]["id"], Value::Null);
    assert_eq!(replies[2]["error"]["code"], -32700);
    assert_eq!(replies[3]["result"]["isError"], false);
    let inspect: Value =
        serde_json::from_str(replies[5]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(close(&inspect["bodies"][0]["volume"], 4000.0), "{inspect}");
    assert_eq!(replies[6]["result"]["isError"], false, "{}", replies[6]);
    assert!(
        std::fs::read(dir.join("out/part.3mf"))
            .unwrap()
            .starts_with(b"PK")
    );
    assert_eq!(replies[7]["result"]["isError"], true);
    assert!(!dir.join("escaped.3mf").exists());
    let image = &replies[8]["result"]["content"][0];
    assert_eq!(image["type"], "image");
    assert!(image["data"].as_str().unwrap().starts_with("iVBORw0KGgo"));
    assert_eq!(replies[9]["error"]["code"], -32602);
    assert_eq!(
        replies[10],
        json!({"jsonrpc": "2.0", "id": 10, "result": {}})
    );
}

/// `mcp --bridge` passes messages to a running window. Without one, the
/// agent is told so in the protocol, and the process keeps serving.
#[test]
fn mcp_bridge_checks_its_flags_and_reports_a_window_that_is_not_there() {
    let dir = workdir("bridge");
    let token = path(&dir, "control.token");
    std::fs::write(&token, format!("{}\n", "ab".repeat(32))).unwrap();
    // Private, as a token file must be.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    // A tool call: `initialize`, `ping` and `tools/list` are answered by
    // the bridge itself when no window is there (2026-11-04).
    let request = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"doc_inspect","arguments":{}}}"#;
    let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let input = format!("{request}\n{notification}\n{request}\n");
    let run_on = |port: &str| {
        cli_in(
            &dir,
            &["mcp", "--bridge", port, "--control-token-file", &token],
            Some(&input),
        )
        .ok()
    };
    let messages = |ran: &Ran| -> Vec<String> {
        let replies = ran.json_lines();
        assert_eq!(replies.len(), 2, "{}", ran.stdout);
        replies
            .iter()
            .map(|reply| {
                assert_eq!(reply["id"], 4);
                reply["error"]["message"].as_str().unwrap().to_string()
            })
            .collect()
    };
    let not_reachable = |message: &str, port: &str| {
        message.contains("not reachable") && message.contains(&format!("--control {port}"))
    };
    // A program that takes the connection and drops it before answering
    // (here one the test owns, so nothing else can be on its port) is no
    // window either: the agent is told the same (2026-11-04, the gate
    // failed on a "connection reset" from a port another process took).
    let dropper = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dropper.local_addr().unwrap().port().to_string();
    std::thread::spawn(move || {
        for stream in dropper.incoming() {
            drop(stream);
        }
    });
    for message in messages(&run_on(&port)) {
        assert!(not_reachable(&message, &port), "{message}");
    }
    // A port nothing listens on: bound once to find a free one, then
    // closed again. Another process (a parallel test) may take it in
    // between: one that drops the connection is read as above, and only a
    // peer that answers shows in the reply as not being the window; then
    // the run is made again on a new port, a few times at most.
    let mut attempt = 0;
    loop {
        attempt += 1;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
            .to_string();
        let messages = messages(&run_on(&port));
        let foreign = messages
            .iter()
            .any(|message| message.contains("is not the 3DCraft window"));
        if foreign && attempt < 5 {
            continue;
        }
        for message in &messages {
            assert!(not_reachable(message, &port), "{message}");
        }
        break;
    }

    let wrong = path(&dir, "wrong.token");
    std::fs::write(&wrong, "not a token").unwrap();
    let dir_text = dir.to_str().unwrap().to_string();
    // Without a token file the window is found through the discovery
    // entries of the state folder (2026-11-02): with none there, and no
    // default token file, each request is told what to do.
    let state = dir.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let state_text = state.to_str().unwrap().to_string();
    let found = cli_in(
        &dir,
        &["mcp", "--bridge", "--state-dir", &state_text],
        Some(&input),
    )
    .ok();
    let replies = found.json_lines();
    assert_eq!(replies.len(), 2, "{}", found.stdout);
    for reply in &replies {
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(message.contains("Allow AI agents"), "{message}");
    }
    let on_port = cli_in(
        &dir,
        &["mcp", "--bridge", &port, "--state-dir", &state_text],
        Some(&input),
    )
    .ok();
    // A port with no entry is no window that allows agents: no token is
    // offered to whatever listens there.
    for reply in &on_port.json_lines() {
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(
            message.contains(&format!("listens on port {port}")),
            "{message}"
        );
    }
    for (args, code, word) in [
        (
            vec!["mcp", "--control-token-file", &token],
            2,
            "go with `--bridge`",
        ),
        (
            vec!["mcp", "--state-dir", &dir_text],
            2,
            "go with `--bridge`",
        ),
        (
            vec![
                "mcp",
                "--bridge",
                &port,
                "--control-token-file",
                &token,
                "--state-dir",
                &dir_text,
            ],
            2,
            "one or the other",
        ),
        (
            vec![
                "mcp",
                "--bridge",
                "8.8.8.8:7878",
                "--control-token-file",
                &token,
            ],
            2,
            "only connects to this machine",
        ),
        (
            vec!["mcp", "--bridge", "0", "--control-token-file", &token],
            2,
            "not a port",
        ),
        (
            vec!["mcp", "--bridge", "seven", "--control-token-file", &token],
            2,
            "not an address",
        ),
        (
            vec![
                "mcp",
                "--bridge",
                &port,
                "--bridge",
                &port,
                "--control-token-file",
                &token,
            ],
            2,
            "given twice",
        ),
        (
            vec![
                "mcp",
                "--bridge",
                &port,
                "--control-token-file",
                &path(&dir, "none.token"),
            ],
            1,
            "cannot read it",
        ),
        (
            vec!["mcp", "--bridge", &port, "--control-token-file", &wrong],
            1,
            "does not hold a control token",
        ),
        (
            vec![
                "mcp",
                "--bridge",
                &port,
                "--control-token-file",
                &token,
                "--automation-write-root",
                dir.to_str().unwrap(),
            ],
            2,
            "the window's",
        ),
        // The bridge is for MCP only.
        (
            vec!["serve", "--bridge", &port, "--control-token-file", &token],
            2,
            "unknown flag",
        ),
    ] {
        let ran = cli_in(&dir, &args, Some("")).failed(code);
        assert!(ran.stderr.contains(word), "{args:?}: {}", ran.stderr);
        assert_eq!(ran.stdout, "", "{args:?}");
    }
    let help = cli(&["mcp", "--help"]).ok();
    assert!(help.stdout.contains("--bridge PORT"), "{}", help.stdout);
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ replay

/// The binary with `PATH` and `CRAFT3D_FFMPEG` as given.
fn cli_with_ffmpeg(args: &[&str], path: &str, ffmpeg: Option<&str>) -> Ran {
    let mut command = Command::new(env!("CARGO_BIN_EXE_craft3d-cli"));
    command
        .args(args)
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
        .env("PATH", path)
        .env_remove("CRAFT3D_FFMPEG");
    if let Some(ffmpeg) = ffmpeg {
        command.env("CRAFT3D_FFMPEG", ffmpeg);
    }
    let output = command.output().expect("the binary runs");
    Ran {
        code: output.status.code().expect("an exit code, not a signal"),
        stdout: String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr is UTF-8"),
    }
}

#[test]
fn replay_writes_frames_or_a_video_and_says_when_ffmpeg_is_missing() {
    let dir = workdir("replay");
    let part = build_part(&dir);
    // A directory of frames: the start, the plate, the pin, and a hold.
    let frames = path(&dir, "frames");
    let ran = cli(&[
        "replay", &part, "--out", &frames, "--size", "64x48", "--hold", "2",
    ])
    .ok();
    let result = &ran.json_lines()[0];
    assert_eq!(result["frames"], 5);
    assert!(dir.join("frames/frame-00004.png").is_file());
    assert!(dir.join("frames/captions.srt").is_file());

    // A video without ffmpeg: exit 1, and the frames are kept.
    let empty = dir.join("no-path");
    std::fs::create_dir_all(&empty).unwrap();
    let video = path(&dir, "part.mp4");
    let ran = cli_with_ffmpeg(
        &["replay", &part, "--out", &video, "--size", "64x48"],
        empty.to_str().unwrap(),
        None,
    )
    .failed(1);
    assert!(
        ran.stderr.contains("ffmpeg was not found"),
        "{}",
        ran.stderr
    );
    assert!(dir.join("part.mp4.frames/frame-00002.png").is_file());
    assert!(!dir.join("part.mp4").exists());

    // Usage errors.
    cli(&["replay", &part]).failed(2);
    cli(&["replay", "--out", &frames]).failed(2);
    cli(&["replay", &part, "--out", &frames, "--fps", "many"]).failed(2);
    cli(&["replay", &part, "--out", &frames, "--params", "[1]"]).failed(2);
    // A bad view.replay param fails the command.
    cli(&["replay", &part, "--out", &frames, "--fps", "0"]).failed(1);

    // With ffmpeg, when this machine has it: a GIF, and the frames are
    // cleared away.
    let ffmpeg = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("ffmpeg"))
            .find(|f| f.is_file())
    });
    let Some(ffmpeg) = ffmpeg else {
        return;
    };
    let gif = path(&dir, "part.gif");
    let ran = cli_with_ffmpeg(
        &[
            "replay", &part, "--out", &gif, "--size", "64x48", "--hold", "3",
        ],
        empty.to_str().unwrap(),
        ffmpeg.to_str(),
    )
    .ok();
    assert_eq!(ran.json_lines()[0]["video"], gif.as_str());
    let bytes = std::fs::read(&gif).unwrap();
    assert!(bytes.starts_with(b"GIF89a"));
    assert!(!dir.join("part.gif.frames").exists());
}

#[test]
fn render_and_replay_say_which_renderer_drew_and_the_cpu_can_be_asked_for() {
    let dir = workdir("renderer");
    let part = build_part(&dir);
    let shot = path(&dir, "shot.png");
    // auto: the GPU where this machine has an adapter, else the CPU.
    let auto = cli(&["render", &part, "--out", &shot, "--size", "64x48"]).ok();
    let said = &auto.json_lines()[0];
    assert!(
        said["renderer"] == "gpu" || said["renderer"] == "cpu",
        "{said}"
    );
    let cpu = cli(&[
        "render",
        &part,
        "--out",
        &shot,
        "--size",
        "64x48",
        "--renderer",
        "cpu",
    ])
    .ok();
    assert_eq!(cpu.json_lines()[0]["renderer"], "cpu");
    let ran = cli(&["render", &part, "--out", &shot, "--renderer", "vulkan"]).failed(1);
    assert!(ran.stderr.contains("vulkan"), "{}", ran.stderr);
    let frames = path(&dir, "frames");
    let ran = cli(&[
        "replay",
        &part,
        "--out",
        &frames,
        "--size",
        "64x48",
        "--renderer",
        "cpu",
    ])
    .ok();
    assert_eq!(ran.json_lines()[0]["renderer"], "cpu");
}

#[test]
fn view_replay_makes_a_video_of_an_animation_or_names_the_png_sequence() {
    let dir = workdir("animation-video");
    let part = build_part(&dir);
    let empty = dir.join("no-path");
    std::fs::create_dir_all(&empty).unwrap();
    let frames = path(&dir, "sweep");
    let video = path(&dir, "sweep.webm");
    let replay = |video: &str| {
        format!(
            r#"{{"dir": "{frames}", "video": "{video}", "width": 64, "height": 48, "frames": 4, "renderer": "cpu",
                "animate": [{{"variable": "width", "from": 30, "to": 50, "unit": "mm"}}]}}"#
        )
    };
    // Without ffmpeg: refused before a frame is drawn, naming the
    // PNG sequence.
    let params = replay(&video);
    let ran = cli_with_ffmpeg(
        &["run", &part, "--cmd", "view.replay", "--params", &params],
        empty.to_str().unwrap(),
        None,
    )
    .failed(1);
    assert!(ran.stderr.contains("PNG sequence"), "{}", ran.stderr);
    assert!(ran.stderr.contains("ffmpeg"), "{}", ran.stderr);
    assert!(!dir.join("sweep").exists());
    // With ffmpeg, when this machine has it: a WebM, the frames kept.
    let ffmpeg = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("ffmpeg"))
            .find(|f| f.is_file())
    });
    let Some(ffmpeg) = ffmpeg else {
        return;
    };
    let ran = cli_with_ffmpeg(
        &["run", &part, "--cmd", "view.replay", "--params", &params],
        empty.to_str().unwrap(),
        ffmpeg.to_str(),
    )
    .ok();
    let result = &ran.json_lines()[0];
    assert_eq!(result["video"], video.as_str(), "{result}");
    assert_eq!(result["frames"], 4);
    let bytes = std::fs::read(&video).unwrap();
    // The EBML header of a Matroska (WebM) file.
    assert!(
        bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        "{:?}",
        &bytes[..8]
    );
    assert!(dir.join("sweep/frame-00003.png").is_file());
}

/// The task set's directory, `docs/tasks`.
fn tasks_dir() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/tasks")
        .to_str()
        .expect("a UTF-8 path")
        .to_string()
}

#[test]
fn score_prints_a_scorecard_for_a_task_and_a_candidate() {
    let dir = workdir("score");
    // The task's own reference solution, by id.
    let card = cli(&["score", "spacer", "--tasks", &tasks_dir()])
        .ok()
        .json_lines();
    assert_eq!(card.len(), 1);
    let card = &card[0];
    assert_eq!(card["format"], "craft3d-scorecard/1");
    assert_eq!(card["task"], "spacer");
    assert_eq!(card["pass"], true, "{card:#}");
    assert_eq!(card["passed"], card["total"]);

    // A candidate without variables: the part is right, the variant fails.
    let candidate = path(&dir, "candidate.json");
    std::fs::write(
        &candidate,
        r#"[["sketch.create", {"name": "S"}],
            ["sketch.circle", {"sketch": "S", "center": [0, 0], "radius": 10}],
            ["sketch.circle", {"sketch": "S", "center": [0, 0], "radius": 4.2}],
            ["feature.extrude", {"sketch": "S", "distance": 5}],
            ["feature.fillet", {"edges": [], "radius": 1}]]"#,
    )
    .unwrap();
    let task = format!("{}/spacer.json", tasks_dir());
    let card = &cli(&["score", &task, "--script", &candidate])
        .ok()
        .json_lines()[0];
    assert_eq!(card["steps"], 5);
    assert_eq!(card["refusals"], 1);
    assert_eq!(card["failedSteps"][0]["step"], 4);
    assert_eq!(card["base"]["pass"], true, "{card:#}");
    assert_eq!(card["variants"][0]["pass"], false);
    assert_eq!(card["variants"][0]["missingVariables"][0], "outer_d");
    assert_eq!(card["pass"], false);

    // What cannot be scored is an error, not a card.
    cli(&["score", "no-such-task", "--tasks", &tasks_dir()]).failed(1);
    cli(&["score", &task, "--script", &path(&dir, "missing.json")]).failed(1);
    std::fs::write(&candidate, "{}").unwrap();
    cli(&["score", &task, "--script", &candidate]).failed(1);
    cli(&["score"]).failed(2);
    cli(&["score", "a", "b"]).failed(2);
}

/// A Jig: a plate `w` x 20 x `h` and a pin of height `8 mm - t`, with
/// `t` published up to `t_max` (the pin fails from 8 mm on).
fn build_jig(dir: &Path, name: &str, t_max: &str) -> String {
    let file = path(dir, name);
    let script = path(dir, &format!("{name}.json"));
    let steps = json!([
        ["variable.set", {"name": "w", "expr": "40 mm"}],
        ["variable.set", {"name": "h", "expr": "5 mm"}],
        ["variable.set", {"name": "t", "expr": "2 mm"}],
        ["feature.block", {"name": "Plate", "size": ["w", "20 mm", "h"]}],
        ["feature.block", {"name": "Pin", "size": ["4 mm", "4 mm", "8 mm - t"], "position": [100, 0, 0]}],
        ["variable.publish", {"name": "w", "min": "10 mm", "max": "80 mm"}],
        ["variable.publish", {"name": "h", "min": "1 mm", "max": "10 mm"}],
        ["variable.publish", {"name": "t", "min": "1 mm", "max": t_max}],
    ]);
    std::fs::write(&script, steps.to_string()).unwrap();
    cli(&["run", "--script", &script, "--out", &file]).ok();
    file
}

/// `check-range` counts a point whose assertion is not proven apart from
/// the passed and the failed ones, lists it, and fails it with --strict:
/// a ring turned from a spline, 5 thick but proven near 0 (a rational
/// surface has no normal cone), with an assertion that it is at least
/// `lim`, from 0 mm (passed) to 4 mm (not proven); 2 mm by default.
#[test]
fn check_range_counts_unproven_points_apart_and_fails_them_when_strict() {
    let dir = workdir("check-range-unproven");
    let file = path(&dir, "ring.3dcraft");
    let script = path(&dir, "ring.json");
    let steps = json!([
        ["variable.set", {"name": "lim", "expr": "2 mm"}],
        ["sketch.create", {"name": "Profile", "plane": "zx"}],
        ["sketch.curve", {"sketch": "Profile", "x": "10 mm * t", "y": "10 mm + 8 mm * t * (1 - t)", "t": [0, 1]}],
        ["sketch.path", {"sketch": "Profile", "start": "e2", "segments": [{"line": ["10 mm", "5 mm"]}, {"line": ["0 mm", "5 mm"]}, {"line": "e1"}]}],
        ["feature.revolve", {"sketch": "Profile", "axis": "x", "name": "Ring"}],
        ["assert.add", {"name": "wall", "measure": "thickness", "of": "Ring", "min": "lim"}],
        ["variable.publish", {"name": "lim", "min": "0 mm", "max": "4 mm"}],
    ]);
    std::fs::write(&script, steps.to_string()).unwrap();
    cli(&["run", "--script", &script, "--out", &file]).ok();
    let ran = cli(&["check-range", &file, "--samples", "0"]).ok();
    println!("{}", ran.stdout);
    assert!(
        ran.stdout.contains(
            "3 points (1 default, all 2 corners, 0 choices, 0 random): 1 passed, 2 with unproven assertions, 0 failed"
        ),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("point 0 (default, "), "{}", ran.stdout);
    assert!(
        ran.stdout.contains("not proven: wall: lim=2 mm"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout
            .contains("NOT PROVEN point 0 (default): lim=2 mm"),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("--strict"), "{}", ran.stdout);
    let ran = cli(&["check-range", &file, "--samples", "0", "--strict"]).failed(1);
    assert!(
        ran.stdout
            .contains("1 passed, 0 with unproven assertions, 2 failed"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("wall: not proven (strict)"),
        "{}",
        ran.stdout
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `check-range` counts an inactive assertion (its `when` is 0 there) as
/// neither passed nor failed nor unproven: the ring of
/// [`check_range_counts_unproven_points_apart_and_fails_them_when_strict`]
/// with its wall assertion switched off from `lim` 3 mm on, so the corner
/// at 4 mm passes, and the default (2 mm) stays unproven.
#[test]
fn check_range_does_not_count_an_inactive_assertion_as_unproven() {
    let dir = workdir("check-range-inactive");
    let file = path(&dir, "ring.3dcraft");
    let script = path(&dir, "ring.json");
    let steps = json!([
        ["variable.set", {"name": "lim", "expr": "2 mm"}],
        ["sketch.create", {"name": "Profile", "plane": "zx"}],
        ["sketch.curve", {"sketch": "Profile", "x": "10 mm * t", "y": "10 mm + 8 mm * t * (1 - t)", "t": [0, 1]}],
        ["sketch.path", {"sketch": "Profile", "start": "e2", "segments": [{"line": ["10 mm", "5 mm"]}, {"line": ["0 mm", "5 mm"]}, {"line": "e1"}]}],
        ["feature.revolve", {"sketch": "Profile", "axis": "x", "name": "Ring"}],
        ["assert.add", {"name": "wall", "measure": "thickness", "of": "Ring", "min": "lim", "when": "lim < 3 mm"}],
        ["assert.add", {"name": "never", "measure": "volume", "of": "Ring", "max": "1 mm^3", "when": "0"}],
        ["variable.publish", {"name": "lim", "min": "0 mm", "max": "4 mm"}],
    ]);
    std::fs::write(&script, steps.to_string()).unwrap();
    cli(&["run", "--script", &script, "--out", &file]).ok();
    let ran = cli(&["check-range", &file, "--samples", "0"]).ok();
    assert!(
        ran.stdout.contains(
            "3 points (1 default, all 2 corners, 0 choices, 0 random): 2 passed, 1 with unproven assertions, 0 failed"
        ),
        "{}",
        ran.stdout
    );
    // Strict fails the unproven default only, never an inactive one.
    let ran = cli(&["check-range", &file, "--samples", "0", "--strict"]).failed(1);
    assert!(
        ran.stdout
            .contains("2 passed, 0 with unproven assertions, 1 failed"),
        "{}",
        ran.stdout
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// On Linux with glibc `check-range` runs the check in a process of its
/// own (two allocator arenas), told by `CRAFT3D_CHECK_PARENT` which
/// process started it: that one is not started again, and stops before its
/// next point when its starter is gone (killed by a timeout), rather than
/// run on unwatched.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn check_range_runs_in_a_process_of_its_own_that_stops_when_orphaned() {
    let dir = workdir("check-range-process");
    let good = build_jig(&dir, "good.3dcraft", "6 mm");
    let args = ["check-range", good.as_str(), "--samples", "2"];
    // Neither variable set: started again, the whole run, the same report.
    let ran = cli_env(
        &args,
        &[("MALLOC_ARENA_MAX", None), ("CRAFT3D_CHECK_PARENT", None)],
    );
    assert_eq!(ran.code, 0, "{}\n{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout.contains("11 points (1 default, all 8 corners"),
        "{}",
        ran.stdout
    );
    // As that process, its starter alive (this test): to the end.
    let me = std::process::id().to_string();
    let ran = cli_env(&args, &[("CRAFT3D_CHECK_PARENT", Some(me.as_str()))]);
    assert_eq!(ran.code, 0, "{}\n{}", ran.stdout, ran.stderr);
    assert!(ran.stdout.contains("point 10 (random, "), "{}", ran.stdout);
    // As that process, its starter gone (it names another): stops after
    // the point it was checking, as at a time limit (exit code 4).
    let ran = cli_env(&args, &[("CRAFT3D_CHECK_PARENT", Some("1"))]);
    assert_eq!(ran.code, 4, "{}\n{}", ran.stdout, ran.stderr);
    assert!(ran.stdout.contains("point 0 (default, "), "{}", ran.stdout);
    assert!(!ran.stdout.contains("point 1 ("), "{}", ran.stdout);
    assert!(ran.stderr.contains("stopped (caller)"), "{}", ran.stderr);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_range_passes_a_sound_jig_and_fails_a_broken_one() {
    let dir = workdir("check-range");
    let good = build_jig(&dir, "good.3dcraft", "6 mm");
    let ran = cli(&["check-range", &good, "--samples", "5", "--seed", "3"]).ok();
    println!("{}", ran.stdout);
    assert!(
        ran.stdout.contains(
            "3 parameters, 14 points (1 default, all 8 corners, 0 choices, 5 random): 14 passed, 0 with unproven assertions, 0 failed"
        ),
        "{}",
        ran.stdout
    );
    // A line per point as it is checked, before the report.
    let first = ran.stdout.find("point 0 (default, ").unwrap();
    let last = ran.stdout.find("point 13 (random, ").unwrap();
    let report = ran.stdout.find("3 parameters, 14 points").unwrap();
    assert!(first < last && last < report, "{}", ran.stdout);
    assert!(ran.stdout.contains("ms): passed"), "{}", ran.stdout);
    assert!(
        ran.stdout.contains("regeneration: median"),
        "{}",
        ran.stdout
    );
    // --json is document.sample's result, the same for the same seed.
    let json = |file: &str| -> Value {
        let ran = cli(&[
            "check-range",
            file,
            "--samples",
            "5",
            "--seed",
            "3",
            "--json",
        ]);
        serde_json::from_str(ran.stdout.trim()).unwrap()
    };
    let result = json(&good);
    assert_eq!(result["ok"], true);
    assert_eq!(result["counts"]["points"], 14);
    // With --json the lines per point go to standard error.
    let ran = cli(&["check-range", &good, "--samples", "1", "--json"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(
        ran.stderr.starts_with("point 0 (default, "),
        "{}",
        ran.stderr
    );
    assert!(ran.stderr.contains("point 9 (random, "), "{}", ran.stderr);
    assert!(!ran.stdout.contains("point 0"), "{}", ran.stdout);
    // Fewer corners than the box has: the report says how many.
    let ran = cli(&["check-range", &good, "--samples", "0", "--max-corners", "4"]).ok();
    assert!(ran.stdout.contains("4 of 8 corners"), "{}", ran.stdout);
    assert!(
        ran.stdout.contains("corners: 4 of the 8 tried"),
        "{}",
        ran.stdout
    );

    let bad = build_jig(&dir, "bad.3dcraft", "10 mm");
    let ran = cli(&[
        "check-range",
        &bad,
        "--samples",
        "5",
        "--seed",
        "3",
        "--deep",
    ])
    .failed(1);
    println!("{}", ran.stdout);
    assert!(ran.stderr.contains("points fail"), "{}", ran.stderr);
    assert!(ran.stdout.contains("FAIL point"), "{}", ran.stdout);
    assert!(ran.stdout.contains("t=10 mm"), "{}", ran.stdout);
    assert!(ran.stdout.contains("Pin: "), "{}", ran.stdout);
    let a = json(&bad);
    let b = json(&bad);
    assert_eq!(a["ok"], false);
    assert_eq!(a["failed"], b["failed"]);
    let values = |r: &Value| -> Vec<Value> {
        r["failures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["values"].clone())
            .collect()
    };
    assert_eq!(values(&a), values(&b));
    // A budget no regeneration meets fails every point.
    cli(&[
        "check-range",
        &good,
        "--samples",
        "1",
        "--budget-ms",
        "0.000001",
    ])
    .failed(1);

    // A time limit that stops the run: 4 when nothing failed so far, 1
    // when a point did.
    let ran = cli(&["check-range", &good, "--time-limit-ms", "0.000001"]).failed(4);
    assert!(ran.stdout.contains("INCOMPLETE"), "{}", ran.stdout);
    cli(&["check-range", &bad, "--time-limit-ms", "100000"]).failed(1);

    // Usage errors (2, the caps included), and files that cannot be
    // checked (3).
    cli(&["check-range"]).failed(2);
    cli(&["check-range", &good, "--samples", "many"]).failed(2);
    cli(&["check-range", &good, "--budget-ms", "-3"]).failed(2);
    cli(&["check-range", &good, "--time-limit-ms", "0"]).failed(2);
    cli(&["check-range", &good, "--deep=yes"]).failed(2);
    cli(&["check-range", &good, &good]).failed(2);
    cli(&["check-range", &good, "--samples", "20000"]).failed(2);
    cli(&["check-range", &good, "--max-corners", "5000"]).failed(2);
    cli(&["check-range", &path(&dir, "missing.3dcraft")]).failed(3);
    let part = build_part(&dir);
    let ran = cli(&["check-range", &part]).failed(3);
    assert!(
        ran.stderr.contains("no public parameters"),
        "{}",
        ran.stderr
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `convert --set S --width W --gap G` is `file.export {set, width, gap}`:
/// three 30 × 20 panels whose top faces fill one row without a width, and
/// rows of two on a sheet 70 mm wide (30 + 10 + 30).
#[test]
fn convert_set_takes_the_sheet_width_and_gap() {
    let dir = workdir("convert-set");
    let file = path(&dir, "panels.3dcraft");
    let script = path(&dir, "panels.json");
    let steps = json!([
        ["feature.block", {"name": "A", "size": [30, 20, 3]}],
        ["feature.block", {"name": "B", "size": [30, 20, 3], "position": [0, 50, 0]}],
        ["feature.block", {"name": "C", "size": [30, 20, 3], "position": [0, 100, 0]}],
        ["set.define", {"name": "Cut sheet", "kind": "face",
                        "where": {"normal": {"along": "z"}}}],
    ]);
    std::fs::write(&script, steps.to_string()).unwrap();
    cli(&[
        "run",
        "--new",
        r#"{"units": "mm"}"#,
        "--script",
        &script,
        "--out",
        &file,
    ])
    .ok();
    let sheet = |name: &str, extra: &[&str]| -> Value {
        let out = path(&dir, name);
        let mut args = vec!["convert", file.as_str(), out.as_str(), "--set", "Cut sheet"];
        args.extend_from_slice(extra);
        cli(&args).ok().json_lines().remove(0)
    };
    let one_row = sheet("row.svg", &["--gap", "10 mm"]);
    println!("{one_row}");
    assert_eq!(one_row["sheet"].as_array().unwrap().len(), 3, "{one_row}");
    assert_eq!(one_row["rows"], 1, "{one_row}");
    // 30 + 10 + 30 + 10 + 30 by 20.
    assert_eq!(one_row["bbox"], json!([0.0, 0.0, 110.0, 20.0]), "{one_row}");
    assert_eq!(one_row["gap"], 10.0, "{one_row}");
    let narrow = sheet("rows.dxf", &["--gap", "10", "--width", "70 mm"]);
    println!("{narrow}");
    assert_eq!(narrow["gap"], 10.0, "{narrow}");
    // Two 30 mm faces and a 10 mm gap fill a 70 mm row.
    // Rows of two and one: 70 by 20 + 10 + 20.
    assert_eq!(narrow["rows"], 2, "{narrow}");
    assert_eq!(narrow["bbox"], json!([0.0, 0.0, 70.0, 50.0]), "{narrow}");
    // A negative gap is the engine's error (1); --width or --gap without
    // --set, or for a .3dcraft, a usage error (2).
    let out = path(&dir, "bad.svg");
    let ran = cli(&["convert", &file, &out, "--set", "Cut sheet", "--gap", "-1"]).failed(1);
    assert!(ran.stderr.contains("`gap`"), "{}", ran.stderr);
    let ran = cli(&["convert", &file, &out, "--sketch", "S1", "--width", "70"]).failed(2);
    assert!(ran.stderr.contains("`--set S`"), "{}", ran.stderr);
    let copy = path(&dir, "copy.3dcraft");
    cli(&["convert", &file, &copy, "--gap", "3"]).failed(2);
    std::fs::remove_dir_all(&dir).unwrap();
}
