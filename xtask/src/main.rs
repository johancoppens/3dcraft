//! Build tasks: `cargo xtask layers | wasm | ci | stats | health | freecad | boolean-corpus | fuzz | actions | package | web-size | example-thumbs`.

mod actions;
mod boolean_corpus;
mod ci;
mod fuzz;
mod health;
mod layers;
mod package;
mod stats;
mod thumbs;
mod web;
mod web_size;

use std::process::{Command, ExitCode};

/// The cargo that runs this task.
fn cargo_path() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = cargo_path();
    eprintln!("$ cargo {}", args.join(" "));
    let status = Command::new(cargo)
        .args(args)
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed ({status})", args.join(" ")))
    }
}

/// Every library crate must build for the web (AGENTS.md rule 11), and
/// so must the web app itself.
fn wasm() -> Result<(), String> {
    let mut args = vec!["check", "--target", "wasm32-unknown-unknown"];
    let packages: Vec<String> = layers::LAYERS
        .iter()
        .filter(|(name, layer)| *layer != layers::APP_LAYER || *name == "web")
        .map(|(name, _)| format!("craft3d-{name}"))
        .collect();
    for p in &packages {
        args.extend(["-p", p.as_str()]);
    }
    cargo(&args)
}

/// Runs the opt-in tests that let FreeCAD read our exports (docs/testing.md,
/// oracle 8): the meshes, the DXF and SVG outlines (of the writers, then
/// of documents built by commands), then the STEP files of bodies built
/// on the kernel, then those of documents built by commands, then STEP
/// import (the showcase read back, and solids FreeCAD makes). Needs
/// `CRAFT3D_FREECAD`. The STEP checks leave their tables in
/// `target/tmp/step-io/report.md` and
/// `target/tmp/step-engine-files/report.md`.
fn freecad() -> Result<(), String> {
    if std::env::var_os("CRAFT3D_FREECAD").is_none() {
        return Err(
            "set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable".to_string(),
        );
    }
    for (package, test) in [
        ("craft3d-io", "freecad"),
        ("craft3d-drawing", "freecad"),
        ("craft3d-engine", "drawing_freecad"),
        ("craft3d-io", "step_freecad"),
        ("craft3d-engine", "step_freecad"),
        // STEP import: the showcase read back, and files FreeCAD makes.
        ("craft3d-engine", "step_import"),
    ] {
        cargo(&[
            "test",
            "--release",
            "-p",
            package,
            "--test",
            test,
            "--",
            "--ignored",
            "--nocapture",
        ])?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "layers" => layers::check(),
        "wasm" => wasm(),
        "ci" => ci::run(),
        "stats" => stats::run(),
        "health" => health::run(),
        "freecad" => freecad(),
        "boolean-corpus" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            boolean_corpus::run(&args)
        }
        "fuzz" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            fuzz::run(&args)
        }
        "actions" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            actions::run(&args)
        }
        "package" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            package::run(&args)
        }
        "web-size" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            web_size::run(&args)
        }
        "example-thumbs" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            thumbs::run(&args)
        }
        other => Err(format!(
            "unknown task `{other}`; tasks: layers, wasm, ci, stats, health, freecad, boolean-corpus, fuzz, actions, package, web-size, example-thumbs"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}
