//! Dependency layering and lint opt-in checks (docs/architecture.md §2).

use std::process::Command;

/// Layer of every workspace crate, by its name without the `craft3d-`
/// prefix. A crate may depend only on crates in lower layers. Register new
/// crates here.
pub const LAYERS: &[(&str, u32)] = &[
    ("math", 0),
    ("geom", 0),
    ("step", 0),
    ("brep", 0),
    ("meshio", 0),
    ("drawing", 0),
    ("zipstore", 0),
    ("buildinfo", 0),
    ("font", 0),
    ("topo", 1),
    ("expr", 1),
    ("doc", 2),
    ("ops", 3),
    ("solve", 3),
    ("assembly", 3),
    ("kernel", 3),
    ("tess", 3),
    ("stepread", 3),
    ("fcstdread", 3),
    ("regen", 4),
    ("render", 4),
    ("gpu", 4),
    ("format", 4),
    ("io", 5),
    ("engine", 6),
    ("ui-egui", UI_LAYER),
    ("automation", UI_LAYER),
    ("cli", APP_LAYER),
    ("fuzz", APP_LAYER),
    ("web", APP_LAYER),
    (APP, APP_LAYER),
    ("testkit", TESTKIT),
];

/// The desktop app's package is the bare machine name, without a suffix;
/// it is registered under this name.
const APP: &str = "app";
const APP_PACKAGE: &str = "craft3d";

/// A workspace package's name in [`LAYERS`]; `None` for one that is not a
/// 3DCraft crate (xtask).
fn short_name(package: &str) -> Option<&str> {
    if package == APP_PACKAGE {
        Some(APP)
    } else {
        package.strip_prefix("craft3d-")
    }
}

const UI_LAYER: u32 = 7;
/// Binaries: may use every library crate, are used by none.
pub const APP_LAYER: u32 = 8;
/// `testkit` may use any library crate, and may itself only be a
/// dev-dependency.
const TESTKIT: u32 = 100;

/// Dependencies allowed inside one layer: (crate, depends on).
const SAME_LAYER: &[(&str, &str)] = &[
    ("geom", "math"),
    // The wgpu renderer takes its camera, parameters, frame type and shading
    // constants from the CPU reference renderer, so the two share one
    // projection and can be compared pixel by pixel (docs/design/rendering.md).
    ("gpu", "render"),
    // FreeCAD documents become bodies by the same sewing, edge proofs and
    // checks as STEP files, and with the same options, report and result
    // (docs/design/step-import.md › FreeCAD documents).
    ("fcstdread", "stepread"),
];

/// Dev-dependencies allowed inside one layer, for tests only: (crate,
/// depends on). The kernel's tests mesh the bodies it makes.
const SAME_LAYER_DEV: &[(&str, &str)] = &[
    ("kernel", "tess"),
    // The FreeCAD import's tests take a body's tight box from its mesh.
    ("fcstdread", "tess"),
];

/// Format crates that must not depend on any workspace crate.
const STANDALONE: &[&str] = &[
    "step",
    "brep",
    "meshio",
    "zipstore",
    "drawing",
    "buildinfo",
    "font",
];

/// The documented exceptions to that: (crate, depends on), both standalone.
const STANDALONE_EXCEPTIONS: &[(&str, &str)] = &[("meshio", "zipstore")];

/// UI crates no library below the UI layer may use.
const UI_CRATES: &[&str] = &["egui", "eframe", "winit", "rfd"];

fn layer_of(short: &str) -> Option<u32> {
    LAYERS
        .iter()
        .find(|(name, _)| *name == short)
        .map(|&(_, layer)| layer)
}

/// The workspace's packages as cargo reports them.
pub fn packages() -> Result<Vec<serde_json::Value>, String> {
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
    metadata
        .get("packages")
        .and_then(|p| p.as_array())
        .cloned()
        .ok_or_else(|| "cargo metadata has no `packages` array".to_string())
}

/// True when the manifest opts in to the workspace lints (the never-crash
/// rules of AGENTS.md §3).
fn inherits_workspace_lints(manifest: &str) -> bool {
    let mut in_lints = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_lints = line == "[lints]";
        } else if in_lints && line.replace(' ', "") == "workspace=true" {
            return true;
        }
    }
    false
}

/// A format migration step marked as a stand-in for another branch's
/// step (`PLACEHOLDER` in `crates/format/src/migrate.rs`) must be replaced
/// before the branches meet on `main`: a released build with a stand-in
/// would read that version's files wrongly. One error per line marked.
fn placeholder_steps(migrate: &str) -> Vec<String> {
    migrate
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("PLACEHOLDER"))
        .map(|(i, _)| {
            format!(
                "crates/format/src/migrate.rs:{}: a PLACEHOLDER migration step; replace it by the \
                 real step of that format version",
                i + 1
            )
        })
        .collect()
}

pub fn check() -> Result<(), String> {
    let mut errors = Vec::new();
    let mut checked = 0;
    for package in packages()? {
        let name = package
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default();
        let manifest_path = package
            .get("manifest_path")
            .and_then(|p| p.as_str())
            .unwrap_or_default();
        match std::fs::read_to_string(manifest_path) {
            Ok(manifest) if inherits_workspace_lints(&manifest) => {}
            Ok(_) => errors.push(format!(
                "{name} does not opt in to the workspace lints (`[lints] workspace = true`)"
            )),
            Err(e) => errors.push(format!("cannot read {manifest_path}: {e}")),
        }
        if name == "craft3d-format" {
            let migrate = std::path::Path::new(manifest_path)
                .with_file_name("src")
                .join("migrate.rs");
            match std::fs::read_to_string(&migrate) {
                Ok(text) => errors.extend(placeholder_steps(&text)),
                Err(e) => errors.push(format!("cannot read {}: {e}", migrate.display())),
            }
        }
        let Some(short) = short_name(name) else {
            continue; // xtask
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
            if STANDALONE_EXCEPTIONS.contains(&(short, dep_short)) {
                continue;
            }
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
                Some(dep_layer)
                    if dep_layer == layer
                        && is_dev
                        && SAME_LAYER_DEV.contains(&(short, dep_short)) => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_placeholder_migration_step_is_refused() {
        let text = "fn v1_to_v2() {}\n/// PLACEHOLDER for another branch\nfn v2_to_v3() {}\n";
        let errors = placeholder_steps(text);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("migrate.rs:2:"), "{errors:?}");
        assert!(placeholder_steps("fn v1_to_v2() {}\n").is_empty());
    }

    #[test]
    fn lint_opt_in_is_detected() {
        assert!(inherits_workspace_lints(
            "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n"
        ));
        assert!(inherits_workspace_lints("[lints]\nworkspace=true"));
        assert!(!inherits_workspace_lints("[package]\nname = \"x\"\n"));
        // `workspace = true` under another table does not count.
        assert!(!inherits_workspace_lints(
            "[package]\nversion.workspace = true\n[dependencies]\nworkspace = true\n"
        ));
        assert!(!inherits_workspace_lints("[lints]\nworkspace = false\n"));
    }

    #[test]
    fn the_layer_table_is_consistent() {
        for (i, (name, _)) in LAYERS.iter().enumerate() {
            assert!(
                LAYERS[..i].iter().all(|(other, _)| other != name),
                "{name} registered twice"
            );
        }
        for (a, b) in SAME_LAYER {
            assert_eq!(
                layer_of(a),
                layer_of(b),
                "{a} -> {b} is not a same-layer pair"
            );
        }
        for name in STANDALONE {
            assert_eq!(layer_of(name), Some(0));
        }
    }

    #[test]
    fn the_desktop_app_is_a_registered_binary() {
        assert_eq!(short_name("craft3d"), Some("app"));
        assert_eq!(layer_of("app"), Some(APP_LAYER));
        assert_eq!(short_name("craft3d-cli"), Some("cli"));
        assert_eq!(short_name("craft3d-ui-egui"), Some("ui-egui"));
        assert_eq!(short_name("xtask"), None);
    }
}
