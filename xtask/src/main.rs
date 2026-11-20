//! Build tasks: `cargo xtask layers | wasm | ci | test | stats | health | freecad | boolean-corpus | fuzz | abc | actions | notices | package | web-size | example-thumbs | bundle-examples`.

mod abc;
mod actions;
mod boolean_corpus;
mod bundle;
mod ci;
mod fonts;
mod fuzz;
mod health;
mod layers;
mod mcpb;
mod notices;
mod package;
mod pe;
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
/// so must the web app itself; then clippy must pass for the web as it
/// does natively, over the same packages: the native clippy does not see
/// what is compiled for `wasm32` alone (a loop whose only `continue` was
/// native went unseen until 2026-11-15).
fn wasm() -> Result<(), String> {
    const WASM: &str = "wasm32-unknown-unknown";
    let packages: Vec<String> = layers::LAYERS
        .iter()
        .filter(|(name, layer)| *layer != layers::APP_LAYER || *name == "web")
        .map(|(name, _)| format!("craft3d-{name}"))
        .collect();
    let mut check = vec!["check", "--target", WASM];
    let mut clippy = vec!["clippy", "--target", WASM];
    for p in &packages {
        check.extend(["-p", p.as_str()]);
        clippy.extend(["-p", p.as_str()]);
    }
    clippy.extend(["--", "-D", "warnings"]);
    cargo(&check)?;
    cargo(&clippy).map_err(|e| {
        format!(
            "{e}\nclippy for the web build: rerun it for the crate it names with \
             `cargo clippy -p <crate> --target {WASM} -- -D warnings`, or all of it with `cargo xtask wasm`"
        )
    })
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
        // FreeCAD documents: FreeCAD agrees with the numbers their tests
        // hold the reader to.
        ("craft3d-fcstdread", "freecad"),
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
        "test" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            ci::test(&args)
        }
        "stats" => stats::run(),
        "health" => health::run(),
        "freecad" => freecad(),
        "fonts" => fonts::run(),
        "boolean-corpus" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            boolean_corpus::run(&args)
        }
        "fuzz" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            fuzz::run(&args)
        }
        "abc" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            abc::run(&args)
        }
        "actions" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            actions::run(&args)
        }
        "notices" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            notices::run(&args)
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
        "bundle-examples" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            bundle::run(&args)
        }
        other => Err(format!(
            "unknown task `{other}`; tasks: layers, wasm, ci, test, stats, health, freecad, fonts, boolean-corpus, fuzz, abc, actions, notices, package, web-size, example-thumbs, bundle-examples"
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
