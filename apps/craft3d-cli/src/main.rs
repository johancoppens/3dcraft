//! Headless command-line interface for 3DCraft (docs/development.md,
//! "Headless CLI").
//!
//! Every subcommand is a thin wrapper around the engine's command registry:
//! `run`, `info`, `render`, `convert` and `replay` execute commands for a trusted
//! local user; `serve` and `mcp` hand stdin and stdout to the servers in
//! `craft3d-automation`, with file access only beneath the roots given on
//! the command line.
//!
//! Exit codes: 0 success, 1 failure, 2 usage error. Errors go to stderr as
//! one line; there is no panic path.

mod args;
mod local;
mod replay;
mod score;
mod server;

use args::{Arg, Args};
use std::fmt;
use std::io::Write;
use std::process::ExitCode;

/// Why the program stops with a non-zero exit code.
#[derive(Debug)]
pub(crate) enum CliError {
    /// The command line was wrong. Exit code 2.
    Usage(String),
    /// The command line was right and the work failed. Exit code 1.
    Failed(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Usage(message) | CliError::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CliError {}

/// What a subcommand's argument parser decided.
pub(crate) enum Parsed<T> {
    /// Run with these options.
    Run(T),
    /// `--help` was among the arguments.
    Help,
}

const HELP: &str = "\
craft3d-cli: headless command-line interface for 3DCraft

Usage: craft3d-cli <subcommand> [options]
       craft3d-cli <subcommand> --help
       craft3d-cli --version | --help

Subcommands:
  commands   List the command registry: id, params, summary
  run        Open or create a document, run commands, optionally save it
  info       Print a document's state as JSON
  render     Render a document to a PNG with the CPU renderer
  convert    Convert a .3dcraft file to STEP, STL, 3MF, OBJ, DXF, SVG or .3dcraft
  replay     Make a video (MP4, GIF) or frames of how a document is built
  score      Run a script for a task of the task set and print its scorecard
  serve      JSON-lines server on stdin/stdout, for scripts
  mcp        Model Context Protocol server on stdin/stdout, for agents

Exit codes: 0 success, 1 failure, 2 usage error.";

const HELP_COMMANDS: &str = "\
Usage: craft3d-cli commands [--filter TEXT]

Prints the command registry, one command per block: its id, its params and
a summary. These are the ids `run --cmd`, `serve` and `mcp` accept.

Options:
  --filter TEXT   Only commands whose id or label contains TEXT (any case)";

const HELP_RUN: &str = "\
Usage: craft3d-cli run [FILE] [--new JSON] [--script FILE]
                       [--cmd ID [--params JSON]]... [--out FILE]
                       [--keep-going]

Opens FILE (a .3dcraft file), or creates a document when no FILE is given,
then runs commands in order and prints one line of JSON per result. Stops at
the first command that fails, with exit code 1 and nothing saved, unless
--keep-going is given.

Options:
  --new JSON      Params of file.new for the new document, e.g.
                  '{\"units\": \"mm\"}'. Default {}. Not with FILE
  --script FILE   A JSON array of steps, run before any --cmd. A step is
                  [\"id\", {params}] or {\"command\": \"id\", \"params\": {...}}
  --cmd ID        A command to run; may be repeated
  --params JSON   The params of the --cmd before it, as a JSON object
  --out FILE      Save the document as a .3dcraft file afterwards
  --keep-going    Go on after a command that fails: its line is
                  {\"failed\": id, \"step\": n, \"error\": text}, the next
                  command runs, --out still saves what was built (a failed
                  command changes nothing), and the exit code is 1

Example:
  craft3d-cli run --new '{\"units\": \"mm\"}' \\
      --cmd feature.block --params '{\"size\": [\"40 mm\", \"20 mm\", \"10 mm\"]}' \\
      --cmd document.measure --out box.3dcraft";

const HELP_INFO: &str = "\
Usage: craft3d-cli info FILE

Prints the state of a .3dcraft file as JSON (the command document.inspect):
units, variables with values, features with status, bodies with counts,
volume, area and centroid.";

const HELP_RENDER: &str = "\
Usage: craft3d-cli render FILE --out PNG [--size WxH] [--view V] [--mode M]

Renders the bodies of a .3dcraft file to a PNG with the CPU renderer (no
window, no GPU) and prints what it drew as one line of JSON. The bodies are
meshed for the picture (the command view.render without a tolerance):
between a quarter and half a pixel where it shows the model, never finer
than the floor of 2e-4 of the diagonal of the bodies' box, within a total
budget of a million triangles; the JSON says the tolerance used and
whether it was coarsened.

Options:
  --out PNG     The image file to write (required)
  --size WxH    Image size in pixels, each 16 to 4096. Default 800x600
  --view V      iso, front, back, left, right, top or bottom. Default iso
  --mode M      shaded, or ids (one flat colour per face). Default shaded";

const HELP_CONVERT: &str = "\
Usage: craft3d-cli convert IN OUT [--tolerance T]
                           [--sketch S | --face F [--feature B]]

Converts the .3dcraft file IN. The extension of OUT picks the format:
.step or .stp write the bodies' exact geometry, with the document's units,
tolerance and names; .stl, .3mf or .obj tessellate the bodies and write a
mesh; .dxf or .svg write the flat outline of a sketch or of a flat face,
at true size, for laser cutting; .3dcraft writes the document again in the
current format version.

Options:
  --tolerance T   Chordal tolerance of the mesh, in document units.
                  Default: chosen from the size of the model. For .svg:
                  how far the Béziers written for a spline of a degree
                  above 3 may lie from it. Default: the document's
  --sketch S      For .dxf and .svg: the sketch to draw, by id or name
  --face F        For .dxf and .svg: the flat face to draw, by its name
                  from `info` (faceNames), e.g. 'F2:endCap(e1,e2,e3,e4)'
  --feature B     With --face: the body that has the face, when several do";

const HELP_REPLAY: &str = "\
Usage: craft3d-cli replay FILE --out OUT [--size WxH] [--view V] [--orbit DEG]
                          [--fps N] [--frames-per-step N] [--hold N] [--every]
                          [--from F] [--to F] [--params JSON] [--keep-frames]

Renders how the .3dcraft file FILE is built (the command view.replay): the
history is rolled forward one feature at a time from nothing, and every step
that changes a body becomes a frame, seen from one camera framed on the
finished model. Prints the result as one line of JSON.

The extension of OUT picks what is made: .mp4 or .gif is a video made by
ffmpeg (found on PATH, or named by CRAFT3D_FFMPEG; without it the command
fails and keeps the frames in OUT.frames); anything else is a directory that
gets the frames, frame-00000.png and on, and captions.srt. An MP4 carries the
captions (each step's feature) as a subtitle track.

Options:
  --out OUT              The video or the directory to write (required)
  --size WxH             Frame size in pixels, each 16 to 4096. Default 800x600
  --view V               iso, front, back, left, right, top or bottom. Default iso
  --orbit DEG            Turn the camera this many degrees about the vertical
                         over the whole replay. Default 0
  --fps N                Frames per second of the video. Default 30
  --frames-per-step N    Frames each step is shown. Default 1
  --hold N               Extra frames of the finished model at the end. Default 0
  --every                Also give frames to steps that change no body
                         (sketches, datums)
  --from F, --to F       The range of the history, by feature id or name.
                         Default: all of it, up to the rollback marker
  --params JSON          More view.replay params, such as
                         '{\"projection\": \"orthographic\", \"zoom\": 1.2}'
  --keep-frames          Keep OUT.frames after the video is made

Example:
  craft3d-cli replay part.3dcraft --out part.mp4 --size 1280x720 \\
      --frames-per-step 6 --hold 60 --orbit 90";

const HELP_SCORE: &str = "\
Usage: craft3d-cli score TASK [--script FILE] [--tasks DIR]

Runs a candidate script for a task of the task set headless, in a session
that may neither read nor write files, and prints its scorecard as one line
of JSON: every expectation of the task with the value expected and the
value measured, whether the model is valid (document.check deep), the
number of steps and the steps that failed, and the same again for every
variant of the task (its variables set to other values). The format is in
docs/testing.md, The task set. Exit code 0 when a scorecard is printed,
whether the candidate passed or not.

TASK is a task file (FILE.json) or a task id, the file ID.json in the
tasks directory.

Options:
  --script FILE   The candidate: a JSON array of steps, as run --script
                  takes. Default: the task's own reference solution
  --tasks DIR     Where task ids are looked up. Default: docs/tasks

Example:
  craft3d-cli score plate-holes --script my-plate.json";

const HELP_SERVE: &str = "\
Usage: craft3d-cli serve [--automation-read-root DIR]
                         [--automation-write-root DIR]
                         [--recovery-dir DIR|none] [--autosave-interval SECONDS]

Serves the JSON-lines protocol on stdin and stdout until stdin closes: one
request per line, {\"id\": ..., \"method\": ..., \"params\": {...}}, and one
reply per line. Send {\"method\": \"methods\"} for the list of methods.

The session reads files only beneath the read root and writes only beneath
the write root. Each is optional and independent: without one, that
direction is denied.

Options:
  --automation-read-root DIR    An existing directory commands may read from
  --automation-write-root DIR   An existing directory commands may write to
  --recovery-dir DIR|none       Autosave documents with unsaved changes into
                                DIR, and offer what a killed server left
                                there (recovery.list, recovery.open,
                                recovery.discard). Default: none
  --autosave-interval SECONDS   How long after a change a document is
                                autosaved. Default: 30";

const HELP_MCP: &str = "\
Usage: craft3d-cli mcp [--automation-read-root DIR]
                       [--automation-write-root DIR]
                       [--recovery-dir DIR|none] [--autosave-interval SECONDS]
       craft3d-cli mcp --bridge PORT --control-token-file FILE

Serves the Model Context Protocol on stdin and stdout (JSON-RPC 2.0, one
message per line), for agent clients. Tools:
{tools}

The session reads files only beneath the read root and writes only beneath
the write root. Each is optional and independent: without one, that
direction is denied.

With --bridge there is no session here: every message is passed to a
running 3DCraft window (started with `craft3d --control PORT
--control-token-file FILE`) and answered on the document that window shows,
so an agent models where you can watch. The files a command may touch are
then the ones that window was given.

Options:
  --automation-read-root DIR    An existing directory commands may read from
  --automation-write-root DIR   An existing directory commands may write to
  --recovery-dir DIR|none       Autosave documents with unsaved changes into
                                DIR, and offer what a killed server left
                                there (recovery.list, recovery.open,
                                recovery.discard). Default: none
  --autosave-interval SECONDS   How long after a change a document is
                                autosaved. Default: 30
  --bridge PORT                 Port (or 127.0.0.1:PORT) of a running window
  --control-token-file FILE     The token file that window was started with";

const SUBCOMMANDS: &[(&str, &str)] = &[
    ("commands", HELP_COMMANDS),
    ("run", HELP_RUN),
    ("info", HELP_INFO),
    ("render", HELP_RENDER),
    ("convert", HELP_CONVERT),
    ("replay", HELP_REPLAY),
    ("score", HELP_SCORE),
    ("serve", HELP_SERVE),
    ("mcp", HELP_MCP),
];

fn subcommand_help(name: &str) -> Option<String> {
    SUBCOMMANDS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(known, help)| match *known {
            "mcp" => help.replace("{tools}", &mcp_tools()),
            _ => (*help).to_string(),
        })
}

/// The tools the MCP server offers, from the server's own list, as the
/// help of `mcp` says them: wrapped, indented by two.
fn mcp_tools() -> String {
    let mut names = craft3d_automation::mcp::tool_names();
    names.push(format!(
        "and, with --bridge, {} (the window's view: ui.get, ui.pick, ui.select, ui.screenshot, ...)",
        craft3d_automation::mcp::WINDOW_TOOL
    ));
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::from(" ");
    let count = names.len();
    for (index, name) in names.into_iter().enumerate() {
        let item = if index + 1 < count {
            format!("{name},")
        } else {
            format!("{name}.")
        };
        for word in item.split(' ') {
            if line.len() + 1 + word.len() > 74 && !line.trim().is_empty() {
                lines.push(line);
                line = String::from(" ");
            }
            line.push(' ');
            line.push_str(word);
        }
    }
    lines.push(line);
    lines.join("\n")
}

fn unknown_subcommand(name: &str) -> CliError {
    let names: Vec<&str> = SUBCOMMANDS.iter().map(|(name, _)| *name).collect();
    CliError::Usage(format!(
        "unknown subcommand `{name}`; the subcommands are: {}",
        names.join(", ")
    ))
}

/// Writes to stdout. A closed pipe (`craft3d-cli commands | head`) is a
/// failure to report, not a panic, which is what `println!` would make it.
pub(crate) fn print(text: &str) -> Result<(), CliError> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| out.write_all(b"\n"))
        .and_then(|()| out.flush())
        .map_err(|e| CliError::Failed(format!("cannot write to standard output: {e}")))
}

fn run(mut args: Args) -> Result<(), CliError> {
    let Some(first) = args.next() else {
        return print(HELP);
    };
    let name = match first {
        Arg::Flag { name, inline } => {
            return match name.as_str() {
                "--help" | "-h" => {
                    args::no_value(&name, inline.as_ref())?;
                    print(HELP)
                }
                "--version" | "-V" => {
                    args::no_value(&name, inline.as_ref())?;
                    print(concat!("craft3d-cli ", env!("CARGO_PKG_VERSION")))
                }
                _ => Err(CliError::Usage(format!(
                    "unknown flag `{name}`; a subcommand comes first"
                ))),
            };
        }
        Arg::Positional(name) => name,
    };
    if name == "help" {
        // `craft3d-cli help [subcommand]`.
        return match args.next() {
            None => print(HELP),
            Some(Arg::Positional(topic)) => {
                print(&subcommand_help(&topic).ok_or_else(|| unknown_subcommand(&topic))?)
            }
            Some(Arg::Flag { name, .. }) => Err(CliError::Usage(format!("unknown flag `{name}`"))),
        };
    }
    let help = subcommand_help(&name).ok_or_else(|| unknown_subcommand(&name))?;
    let ran = match name.as_str() {
        "commands" => local::commands(args),
        "run" => local::run(args),
        "info" => local::info(args),
        "render" => local::render(args),
        "convert" => local::convert(args),
        "replay" => replay::replay(args),
        "score" => score::score(args),
        "serve" => server::serve(args, server::Protocol::JsonLines),
        "mcp" => server::serve(args, server::Protocol::Mcp),
        _ => return Err(unknown_subcommand(&name)),
    };
    match ran {
        Ok(Parsed::Run(())) => Ok(()),
        Ok(Parsed::Help) => print(&help),
        // Say where the usage is, once, for whichever subcommand it was.
        Err(CliError::Usage(message)) => Err(CliError::Usage(format!(
            "{name}: {message}\nTry `craft3d-cli {name} --help`."
        ))),
        Err(other) => Err(other),
    }
}

fn main() -> ExitCode {
    let outcome = Args::from_os(std::env::args_os().skip(1)).and_then(run);
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // If stderr is gone too there is nobody left to tell.
            let _ = writeln!(std::io::stderr(), "craft3d-cli: {error}");
            match error {
                CliError::Usage(_) => ExitCode::from(2),
                CliError::Failed(_) => ExitCode::FAILURE,
            }
        }
    }
}
