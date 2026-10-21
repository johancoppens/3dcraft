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

const SUBCOMMANDS: [&str; 8] = [
    "commands", "run", "info", "render", "convert", "replay", "serve", "mcp",
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
        (vec!["convert", &part], "IN OUT"),
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
    );
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
    assert_eq!(ids, ["variable.set", "variable.delete", "variable.rename"]);
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
    // Without a sketch or a face there is nothing to draw, and the other
    // formats do not take one.
    let ran = cli(&["convert", &part, &path(&dir, "none.dxf")]).failed(1);
    assert!(
        ran.stderr.contains("`sketch`") && ran.stderr.contains("`face`"),
        "{}",
        ran.stderr
    );
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
    let request = r#"{"jsonrpc":"2.0","id":4,"method":"tools/list"}"#;
    let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let input = format!("{request}\n{notification}\n{request}\n");
    // A port nothing listens on: bound once to find a free one, then
    // closed again. Another process (a parallel test) may take it in
    // between; then the bridge rightly reports a peer that is not the
    // window, so the run is made again on a new port, a few times at most.
    let mut attempt = 0;
    let (port, ran) = loop {
        attempt += 1;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
            .to_string();
        let ran = cli_in(
            &dir,
            &["mcp", "--bridge", &port, "--control-token-file", &token],
            Some(&input),
        )
        .ok();
        let taken = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok();
        if !taken || attempt == 5 {
            break (port, ran);
        }
    };
    let replies = ran.json_lines();
    assert_eq!(replies.len(), 2, "{}", ran.stdout);
    for reply in &replies {
        assert_eq!(reply["id"], 4);
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("not reachable") && message.contains(&format!("--control {port}")),
            "{message}"
        );
    }

    let wrong = path(&dir, "wrong.token");
    std::fs::write(&wrong, "not a token").unwrap();
    for (args, code, word) in [
        (vec!["mcp", "--bridge", &port], 2, "go together"),
        (
            vec!["mcp", "--control-token-file", &token],
            2,
            "go together",
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
