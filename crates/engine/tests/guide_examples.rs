//! The user guide (`docs/guide/`) and its example models
//! (`docs/examples/`) cannot rot: every example script is run here and must
//! build without a failed feature and meet the assertions it carries (each
//! one a known volume at least), every example has its picture and its
//! entry in the examples' README, and every command the guide names, and
//! every parameter of a step the guide writes out as JSON, is one the
//! registry has.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn docs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

/// The files with this extension in `dir`, sorted by name.
fn files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{dir:?}: {e}"))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == extension))
        .collect();
    found.sort();
    found
}

fn stem(path: &Path) -> String {
    path.file_stem().unwrap().to_str().unwrap().to_string()
}

/// A script as `craft3d-cli run --script` reads it: `[["id", {params}], …]`.
fn steps(script: &Value) -> Option<Vec<(String, Value)>> {
    script
        .as_array()?
        .iter()
        .map(|step| {
            let id = step.get(0)?.as_str()?.to_string();
            let params = step.get(1).cloned().unwrap_or_else(|| json!({}));
            params.is_object().then_some((id, params))
        })
        .collect()
}

#[test]
fn every_example_builds_and_meets_its_assertions() {
    let scripts = files(&docs().join("examples"), "json");
    assert!(scripts.len() >= 6, "examples: {scripts:?}");
    for path in &scripts {
        let name = stem(path);
        let script: Value = serde_json::from_str(&read_text(path))
            .unwrap_or_else(|e| panic!("{name}: not JSON: {e}"));
        let steps = steps(&script).unwrap_or_else(|| panic!("{name}: not a list of steps"));
        let mut s = common::session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        for (id, params) in steps {
            run(&mut s, &id, params);
        }
        let document = run(&mut s, "document.inspect", json!({}));
        for feature in document["features"].as_array().unwrap() {
            assert_eq!(feature["status"], "ok", "{name}: {feature}");
        }
        assert!(
            !document["bodies"].as_array().unwrap().is_empty(),
            "{name}: no body"
        );
        let assertions = run(&mut s, "assert.list", json!({}));
        assert_eq!(assertions["ok"], true, "{name}: {assertions}");
        assert_eq!(
            assertions["passed"], assertions["count"],
            "{name}: {assertions}"
        );
        let volume = assertions["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["measure"]["measure"] == "volume");
        assert!(volume, "{name}: every example asserts its volume");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{name}: {check}");
    }
}

#[test]
fn every_example_has_a_picture_and_an_entry() {
    let dir = docs().join("examples");
    let readme = read_text(&dir.join("README.md"));
    for path in files(&dir, "json") {
        let name = stem(&path);
        let picture = dir.join(format!("{name}.png"));
        let size = std::fs::metadata(&picture)
            .unwrap_or_else(|e| panic!("{picture:?}: {e}"))
            .len();
        assert!(size < 200 * 1024, "{picture:?} is {size} bytes");
        assert!(
            readme.contains(&format!("({name}.json)")),
            "README.md has no entry linking {name}.json"
        );
        assert!(
            readme.contains(&format!("({name}.png)")),
            "README.md does not show {name}.png"
        );
    }
}

/// Words in backticks that look like a command id (`feature.extrude`):
/// a namespace of the registry, a dot, more.
fn named_commands(text: &str, namespaces: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for (i, piece) in text.split('`').enumerate() {
        // Odd pieces are inside backticks; a code block's lines are
        // searched too, word by word.
        let words: Vec<&str> = if i % 2 == 1 {
            piece
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_'))
                .collect()
        } else {
            continue;
        };
        for word in words {
            let word = word.trim_end_matches('.');
            let Some((namespace, rest)) = word.split_once('.') else {
                continue;
            };
            if !rest.is_empty()
                && rest.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && namespaces.contains(&namespace)
                && !word.ends_with(".json")
                && !word.ends_with(".3dcraft")
            {
                found.push(word.to_string());
            }
        }
    }
    found
}

/// The JSON code blocks of a Markdown text.
fn json_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        match (&mut current, line.trim_start()) {
            (None, "```json") => current = Some(String::new()),
            (Some(block), "```") => {
                blocks.push(std::mem::take(block));
                current = None;
            }
            (Some(block), _) => {
                block.push_str(line);
                block.push('\n');
            }
            (None, _) => {}
        }
    }
    blocks
}

#[test]
fn the_guide_names_only_commands_and_params_that_exist() {
    let commands = Session::commands();
    let ids: Vec<&str> = commands.iter().map(|c| c.id).collect();
    let mut namespaces: Vec<&str> = ids
        .iter()
        .filter_map(|id| id.split_once('.').map(|(n, _)| n))
        .collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    let pages = files(&docs().join("guide"), "md");
    assert!(pages.len() >= 8, "guide pages: {pages:?}");
    let mut wrong = Vec::new();
    let mut steps_checked = 0;
    for page in pages
        .iter()
        .chain([docs().join("examples/README.md")].iter())
    {
        let text = read_text(page);
        let name = page.file_name().unwrap().to_string_lossy().to_string();
        for word in named_commands(&text, &namespaces) {
            if !ids.contains(&word.as_str()) {
                wrong.push(format!("{name}: `{word}` is not a command"));
            }
        }
        for block in json_blocks(&text) {
            let value: Value = serde_json::from_str(&block)
                .unwrap_or_else(|e| panic!("{name}: a json block that is not JSON: {e}\n{block}"));
            let Some(steps) = steps(&value) else {
                continue;
            };
            for (id, params) in steps {
                steps_checked += 1;
                let Some(spec) = commands.iter().find(|c| c.id == id) else {
                    wrong.push(format!("{name}: step `{id}` is not a command"));
                    continue;
                };
                for key in params.as_object().unwrap().keys() {
                    if !spec.params.contains(&format!("\"{key}\"")) {
                        wrong.push(format!("{name}: `{id}` has no param `{key}`"));
                    }
                }
            }
        }
    }
    assert!(
        steps_checked > 10,
        "only {steps_checked} steps found in the guide"
    );
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    // The reference says how many commands there are: the registry's.
    let reference = read_text(&docs().join("guide/15-reference.md"));
    let said = reference
        .split("has ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse::<usize>().ok());
    assert_eq!(
        said,
        Some(commands.len()),
        "15-reference.md must say \"has {} commands\"",
        commands.len()
    );
}

/// The origami chapter's scripts run as written, one after the other, and
/// fold as the text says: no feature fails, the solved angles of the
/// four-crease vertex are the closed form's (about 20.6 degrees), and the
/// waterbomb passes Kawasaki and Maekawa with the note that this proves
/// no flat fold.
#[test]
fn the_origami_chapter_folds_as_it_says() {
    let text = read_text(&docs().join("guide/16-origami.md"));
    let dir = common::workdir("guide", "origami");
    let mut s = common::session_in(&dir);
    let mut ran = 0;
    let mut last: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    let mut inspects: Vec<Value> = Vec::new();
    for block in json_blocks(&text) {
        let value: Value = serde_json::from_str(&block).unwrap();
        let Some(steps) = steps(&value) else {
            continue;
        };
        for (id, params) in steps {
            let result = run(&mut s, &id, params.clone());
            assert_ne!(result["status"], "failed", "{id} {params}: {result}");
            if id == "fold.inspect" {
                inspects.push(result.clone());
            }
            last.insert(id, result);
            ran += 1;
        }
        let check = run(&mut s, "document.check", json!({}));
        assert_eq!(check["ok"], true, "{check}");
    }
    assert!(ran >= 20, "{ran} steps");
    // The last fold.inspect is the waterbomb's: the other diagonal solved
    // to the linkage's angle.
    let waterbomb = &last["fold.inspect"];
    for piece in waterbomb["creases"][2]["pieces"].as_array().unwrap() {
        assert!(
            (piece["angle"].as_f64().unwrap() - 49.40777113928209).abs() < 1e-7,
            "{waterbomb}"
        );
    }
    let inspect = &inspects[0];
    let angle = |line: &str| {
        inspect["creases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["line"] == line)
            .unwrap()["pieces"][0]["angle"]
            .as_f64()
            .unwrap()
    };
    let r2 = 2.0 * (-0.5 * 20f64.to_radians().tan()).atan().to_degrees();
    assert!(
        (angle("e14") - r2).abs() < 1e-7 && (r2 + 20.6).abs() < 0.05,
        "{inspect}"
    );
    assert!((angle("e17") - 40.0).abs() < 1e-7, "{inspect}");
    assert!((angle("e20") + r2).abs() < 1e-7, "{inspect}");
    let check = &last["fold.check"];
    assert_eq!(check["kawasakiFails"], 0, "{check}");
    assert_eq!(check["maekawaFails"], 0, "{check}");
    assert!(check["note"].as_str().unwrap().contains("not sufficient"));
    assert!(dir.join("waterbomb.svg").exists());
}
