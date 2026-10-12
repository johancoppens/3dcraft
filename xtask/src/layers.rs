//! Dependency layering check (docs/architecture.md §3).

use std::process::Command;

/// Layer of every workspace library crate, by directory name. A crate may
/// depend only on crates in lower layers. Register new crates here.
pub const LAYERS: &[(&str, u32)] = &[
    ("math", 0),
    ("geom", 0),
    ("tess", 3),
    ("render", 4),
    ("testkit", TESTKIT),
];

/// `testkit` may use any library crate, and may itself only be a
/// dev-dependency.
const TESTKIT: u32 = u32::MAX;

/// Dependencies allowed inside one layer: (crate, depends on).
const SAME_LAYER: &[(&str, &str)] = &[("geom", "math")];

/// Format crates that must not depend on any workspace crate.
const STANDALONE: &[&str] = &["step", "meshio"];

/// UI crates no library below the UI layer may use.
const UI_CRATES: &[&str] = &["egui", "eframe", "winit", "rfd"];
const UI_LAYER: u32 = 7;

fn layer_of(short: &str) -> Option<u32> {
    LAYERS
        .iter()
        .find(|(name, _)| *name == short)
        .map(|&(_, layer)| layer)
}

pub fn check() -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("cannot parse cargo metadata: {e}"))?;
    let packages = metadata
        .get("packages")
        .and_then(|p| p.as_array())
        .ok_or("cargo metadata has no `packages` array")?;

    let mut errors = Vec::new();
    let mut checked = 0;
    for package in packages {
        let name = package
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default();
        let Some(short) = name.strip_prefix("craft3d-") else {
            continue; // xtask and, later, the apps.
        };
        let Some(layer) = layer_of(short) else {
            errors.push(format!("{name} is not registered in xtask/src/layers.rs"));
            continue;
        };
        checked += 1;
        let deps = package.get("dependencies").and_then(|d| d.as_array());
        for dep in deps.into_iter().flatten() {
            let dep_name = dep.get("name").and_then(|n| n.as_str()).unwrap_or_default();
            let is_dev = dep.get("kind").and_then(|k| k.as_str()) == Some("dev");
            if layer < UI_LAYER && UI_CRATES.contains(&dep_name) {
                errors.push(format!(
                    "{name} (L{layer}) must not depend on the UI crate {dep_name}"
                ));
            }
            let Some(dep_short) = dep_name.strip_prefix("craft3d-") else {
                continue;
            };
            if STANDALONE.contains(&short) {
                errors.push(format!(
                    "{name} is standalone and must not depend on {dep_name}"
                ));
                continue;
            }
            if dep_short == "testkit" {
                if !is_dev {
                    errors.push(format!(
                        "{name} may use {dep_name} only as a dev-dependency"
                    ));
                }
                continue;
            }
            match layer_of(dep_short) {
                None => errors.push(format!("{name} depends on unregistered crate {dep_name}")),
                Some(dep_layer) if dep_layer < layer => {}
                Some(dep_layer) if dep_layer == layer && SAME_LAYER.contains(&(short, dep_short)) => {}
                Some(dep_layer) => errors.push(format!(
                    "{name} (L{layer}) must not depend on {dep_name} (L{dep_layer}): only lower layers"
                )),
            }
        }
    }
    if errors.is_empty() {
        println!("layers: {checked} crates ok");
        Ok(())
    } else {
        Err(format!("layering violations:\n  {}", errors.join("\n  ")))
    }
}
