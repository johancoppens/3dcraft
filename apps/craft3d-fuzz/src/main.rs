//! `craft3d-fuzz`: one random design per process (docs/testing.md ›
//! Random multi-step designs). `cargo xtask fuzz` runs it.
//!
//! Exit codes: 0 when the design ran to a verdict (which is in the JSON
//! line on stdout, also when it is a crash or a wrong result), 1 when it
//! could not run, 2 for a usage error.

use craft3d_fuzz::{
    Deep, Design, Generator, ModelOptions, Options, Runner, Setup, run_design, run_model, run_seed,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

const HELP: &str = "\
craft3d-fuzz: random multi-step designs held to the oracles

Usage: craft3d-fuzz seed N [options]     generate and run the design of seed N
       craft3d-fuzz check FILE [options] run the design in FILE (a reproducer)
       craft3d-fuzz step FILE [--booleans] [--deep] [--scratch DIR]
                                         import a real-world STEP model, check,
                                         mesh and measure it, and with
                                         --booleans cut it with three tools
                                         (cargo xtask abc)

Prints one line of JSON: the verdict (ok, crash, bug, wrong), the finding, the
groups run, accepted and refused (with their reasons).

Options:
  --scale S          seed only: the scale (1e-3, 1, 1e3); default from the seed
  --steps-out FILE   seed only: the design so far, written before every group
  --script-out FILE  the accepted commands, ids filled in, as a script for
                     `craft3d-cli run --new '{\"tolerance\": T}' --script FILE`
  --deep every|end   when the deep check runs (default every)
  --no-identities    skip the volume identities of the booleans
  --no-determinism   skip save, open and compare at the end
  --scratch DIR      where the determinism check saves (default the system's
                     temporary directory)

With CRAFT3D_FUZZ_TRACE set, how long each command took goes to stderr.";

struct Args {
    mode: String,
    target: String,
    scale: Option<f64>,
    steps_out: Option<PathBuf>,
    script_out: Option<PathBuf>,
    options: Options,
}

fn parse() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("no subcommand")?;
    if mode == "--help" || mode == "-h" {
        return Err(String::new());
    }
    let target = args.next().ok_or("no seed or file")?;
    let mut out = Args {
        mode,
        target,
        scale: None,
        steps_out: None,
        script_out: None,
        options: Options {
            deep: Deep::EveryGroup,
            identities: true,
            determinism: true,
            scratch: std::env::temp_dir().join("craft3d-fuzz"),
        },
    };
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--scale" => {
                let s: f64 = value()?.parse().map_err(|_| "--scale is not a number")?;
                if !(s.is_finite() && s > 0.0) {
                    return Err("--scale must be positive".into());
                }
                out.scale = Some(s);
            }
            "--steps-out" => out.steps_out = Some(PathBuf::from(value()?)),
            "--script-out" => out.script_out = Some(PathBuf::from(value()?)),
            "--deep" => {
                out.options.deep = match value()?.as_str() {
                    "every" => Deep::EveryGroup,
                    "end" => Deep::AtEnd,
                    other => return Err(format!("--deep every|end, not {other}")),
                }
            }
            "--no-identities" => out.options.identities = false,
            "--no-determinism" => out.options.determinism = false,
            "--scratch" => out.options.scratch = PathBuf::from(value()?),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(out)
}

/// Writes `text` to `path` through a file beside it, so a reader never
/// sees half of it.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("partial");
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// `craft3d-fuzz step FILE [--booleans] [--deep] [--scratch DIR]`.
fn run_step(args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    let file = it.next().ok_or("no STEP file")?;
    let mut options = ModelOptions {
        booleans: false,
        deep: false,
    };
    let mut scratch = std::env::temp_dir().join("craft3d-abc");
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--booleans" => options.booleans = true,
            "--deep" => options.deep = true,
            "--scratch" => scratch = PathBuf::from(it.next().ok_or("--scratch needs a value")?),
            other => return Err(format!("unknown option {other}")),
        }
    }
    let report = run_model(Path::new(file), options, &scratch)?;
    println!("{report}");
    Ok(())
}

fn run(args: Args) -> Result<(), String> {
    let start = Instant::now();
    let runner: Runner = match args.mode.as_str() {
        "seed" => {
            let seed: u64 = args
                .target
                .parse()
                .map_err(|_| "the seed is not a number")?;
            let scale = args.scale.unwrap_or_else(|| Generator::scale_of(seed));
            let steps_out = args.steps_out.clone();
            let mut keep = |design: &Design| {
                if let Some(path) = &steps_out {
                    // Losing the design costs only the reproducer.
                    let text = design.to_json().to_string();
                    if let Err(e) = write_atomic(path, &text) {
                        eprintln!("craft3d-fuzz: {e}");
                    }
                }
            };
            run_seed(Setup { seed, scale }, args.options, &mut keep)?.1
        }
        "check" => {
            let text = std::fs::read_to_string(&args.target)
                .map_err(|e| format!("cannot read {}: {e}", args.target))?;
            let value: Value =
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", args.target))?;
            run_design(&Design::from_json(&value)?, args.options)?
        }
        other => return Err(format!("unknown subcommand {other}")),
    };
    if let Some(path) = &args.script_out {
        let text = serde_json::to_string_pretty(&runner.script).map_err(|e| e.to_string())?;
        write_atomic(path, &text)?;
    }
    let mut report = runner.report.to_json();
    if let Some(map) = report.as_object_mut() {
        map.insert("tolerance".into(), runner.setup.tolerance().into());
        let ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        map.insert("ms".into(), ms.into());
    }
    println!("{report}");
    Ok(())
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("step") {
        return match run_step(raw.get(1..).unwrap_or_default()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("craft3d-fuzz: {message}");
                eprintln!("{HELP}");
                ExitCode::from(2)
            }
        };
    }
    let args = match parse() {
        Ok(args) => args,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("craft3d-fuzz: {message}");
            }
            eprintln!("{HELP}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("craft3d-fuzz: {message}");
            ExitCode::FAILURE
        }
    }
}
