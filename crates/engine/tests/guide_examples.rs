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

use craft3d_math::Portable;
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
    let reference = read_text(&docs().join("guide/reference.md"));
    let said = reference
        .split("has ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse::<usize>().ok());
    assert_eq!(
        said,
        Some(commands.len()),
        "reference.md must say \"has {} commands\"",
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
    let text = read_text(&docs().join("guide/origami.md"));
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
    let r2 = 2.0 * (-0.5 * 20f64.to_radians().ptan()).patan().to_degrees();
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

/// The animations of the command-line chapter (`command-line.md` ›
/// *Animations and videos*) run as written: the lid's lift and the width
/// sweep write their frames and captions and leave the document as it was.
#[test]
fn the_animations_of_the_command_line_chapter_render_as_they_say() {
    let text = read_text(&docs().join("guide/command-line.md"));
    let section = text
        .split("## Animations and videos")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("the section is there");
    let dir = common::workdir("guide", "animations");
    let mut s = common::session_in(&dir);
    let mut replays = Vec::new();
    for block in json_blocks(section) {
        let value: Value = serde_json::from_str(&block).unwrap();
        let Some(steps) = steps(&value) else {
            continue;
        };
        for (id, params) in steps {
            let before = (id == "view.replay").then(|| run(&mut s, "document.inspect", json!({})));
            let result = run(&mut s, &id, params.clone());
            if let Some(before) = before {
                assert_eq!(
                    run(&mut s, "document.inspect", json!({})),
                    before,
                    "{params}"
                );
                replays.push(result);
            }
        }
    }
    assert_eq!(replays.len(), 2, "{replays:?}");
    assert_eq!(replays[0]["frames"], 12);
    assert_eq!(replays[0]["animate"]["tracks"][0]["timeline"], true);
    assert_eq!(replays[1]["frames"], 18);
    for (name, frames) in [("explode", 12), ("sweep", 18)] {
        for k in 0..frames {
            assert!(
                dir.join(format!("{name}/frame-{k:05}.png")).is_file(),
                "{name} {k}"
            );
        }
        let srt = read_text(&dir.join(format!("{name}/captions.srt")));
        assert!(
            srt.contains(if name == "sweep" {
                "width = 60 mm"
            } else {
                "t = 1"
            }),
            "{srt}"
        );
    }
    // The lid lifts: the first and the last frame differ.
    assert_ne!(
        std::fs::read(dir.join("explode/frame-00000.png")).unwrap(),
        std::fs::read(dir.join("explode/frame-00011.png")).unwrap()
    );
}

/// The outputs of the measuring chapter (`measure.md` › *Outputs*) run
/// as written and say what the text says: the box hull's GM is 65 mm, the
/// assertion on it passes, and with KG at 300 mm GM is -55 mm and it fails.
#[test]
fn the_outputs_of_the_measuring_chapter_are_as_it_says() {
    let text = read_text(&docs().join("guide/measure.md"));
    let block = json_blocks(&text)
        .into_iter()
        .find(|b| b.contains("variable.measure") && b.contains("levelFor"))
        .expect("the chapter has a block of outputs");
    let steps = steps(&serde_json::from_str(&block).unwrap()).unwrap();
    let mut s = Session::new(craft3d_engine::Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    let mut last = Value::Null;
    for (id, params) in steps {
        last = run(&mut s, &id, params);
    }
    let value = |listed: &Value, name: &str| {
        let o = listed["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["name"] == name)
            .unwrap()
            .clone();
        assert_eq!(o["status"], "known", "{o}");
        o["value"].as_f64().unwrap()
    };
    for (name, want) in [
        ("T", 250.0),
        ("KB", 125.0),
        ("I", 2000.0 * 600f64.ppowi(3) / 12.0),
        ("BM", 120.0),
        ("GM", 65.0),
    ] {
        let got = value(&last, name);
        assert!((got - want).abs() <= 1e-9 * want, "{name}: {got} {last}");
    }
    let stable = run(&mut s, "assert.list", json!({}));
    assert_eq!(stable["ok"], true, "{stable}");
    run(
        &mut s,
        "variable.set",
        json!({"name": "KG", "expr": "300 mm"}),
    );
    let listed = run(&mut s, "output.list", json!({}));
    assert!((value(&listed, "GM") + 55.0).abs() < 1e-9, "{listed}");
    let stable = run(&mut s, "assert.list", json!({}));
    assert_eq!(stable["failed"], 1, "{stable}");
}

/// The conditions of the measuring chapter (`measure.md` ›
/// *Assertions that apply only sometimes*) run as written and say what the
/// text says: with every corner open the gap assertion is inactive, `cg`
/// unknown and the `if` that does not take it passed; with the corners
/// closed (corner 2) both are measured and pass.
#[test]
fn the_conditions_of_the_measuring_chapter_are_as_it_says() {
    let text = read_text(&docs().join("guide/measure.md"));
    let block = json_blocks(&text)
        .into_iter()
        .find(|b| b.contains("\"when\"") && b.contains("cornerGap"))
        .expect("the chapter has a block of conditions");
    let steps = steps(&serde_json::from_str(&block).unwrap()).unwrap();
    let mut s = Session::new(craft3d_engine::Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    let mut last = Value::Null;
    for (id, params) in steps {
        last = run(&mut s, &id, params);
    }
    let status = |listed: &Value, name: &str| {
        listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap_or_else(|| panic!("no {name}: {listed}"))["status"]
            .clone()
    };
    assert_eq!(status(&last, "gap"), "inactive", "{last}");
    assert_eq!(status(&last, "gap or open"), "passed", "{last}");
    assert_eq!(last["inactive"], 1, "{last}");
    assert_eq!(last["ok"], true, "{last}");
    let message = last["assertions"][0]["message"].as_str().unwrap();
    assert_eq!(message, "inactive: its condition `corner != 1` is 0");
    let outputs = run(&mut s, "output.list", json!({}));
    assert_eq!(outputs["outputs"][0]["status"], "unknown", "{outputs}");
    run(
        &mut s,
        "variable.set",
        json!({"name": "corner", "expr": "2"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(status(&listed, "gap"), "passed", "{listed}");
    assert_eq!(status(&listed, "gap or open"), "passed", "{listed}");
    assert_eq!(listed["ok"], true, "{listed}");
}

/// The floating assertions of the measuring chapter (`engineering-checks.md`
/// › *Floating, as assertions and outputs*) run as written and say what the
/// text says: the barge trimmed by tan⁻¹ 0.05 (the wall-sided closed form,
/// xg = tan θ (GML + BML tan² θ / 2) with xg 64 mm) passes both, its GM
/// 66.75 mm (B² / (12 T cos θ) less the height of G above B, 53.4 mm);
/// with KG at 300 mm `upright` fails.
#[test]
fn the_floating_assertions_of_the_measuring_chapter_are_as_it_says() {
    let text = read_text(&docs().join("guide/engineering-checks.md"));
    let block = json_blocks(&text)
        .into_iter()
        .find(|b| b.contains("floatingGM"))
        .expect("the chapter has a block of floating assertions");
    let steps = steps(&serde_json::from_str(&block).unwrap()).unwrap();
    let mut s = Session::new(craft3d_engine::Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (id, params) in steps {
        run(&mut s, &id, params);
    }
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["ok"], true, "{listed}");
    let trim = listed["assertions"][1]["measured"].as_f64().unwrap();
    assert!(
        (trim.abs() - 0.05_f64.patan()).abs() < 1e-7,
        "trim {trim}: {listed}"
    );
    let outputs = run(&mut s, "output.list", json!({}));
    let gm = outputs["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == "GMf")
        .unwrap()["value"]
        .as_f64()
        .unwrap();
    assert!((gm - 66.75).abs() < 1e-4, "GM {gm}: {outputs}");
    run(
        &mut s,
        "variable.set",
        json!({"name": "KG", "expr": "300 mm"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["assertions"][0]["status"], "failed", "{listed}");
}

/// The agent chapter's third starter task (`agent.md` › task 3) says what its script
/// makes: 11 holes and 6552.05 mm³ at 120 mm, and 14 holes and 8175.33 mm³
/// once the strip is 150 mm long, its two assertions passing both times.
#[test]
fn the_field_test_strip_is_as_the_page_says() {
    let text = read_text(&docs().join("guide/agent.md"));
    let block = json_blocks(&text)
        .into_iter()
        .find(|b| b.contains("feature.pattern"))
        .expect("the page has the strip's script");
    let steps = steps(&serde_json::from_str(&block).unwrap()).unwrap();
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (id, params) in steps {
        run(&mut s, &id, params);
    }
    let measured = |s: &mut Session, name: &str| {
        let listed = run(s, "assert.list", json!({}));
        assert_eq!(listed["passed"], 2, "{listed}");
        listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap()["measured"]
            .as_f64()
            .unwrap()
    };
    for (length, holes, volume) in [("120 mm", 11.0, 6552.05), ("150 mm", 14.0, 8175.33)] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "length", "expr": length}),
        );
        assert_eq!(measured(&mut s, "holes"), holes, "{length}");
        let got = measured(&mut s, "volume");
        assert!((got - volume).abs() < 0.005, "{length}: {got}");
    }
    let document = run(&mut s, "document.inspect", json!({}));
    for feature in document["features"].as_array().unwrap() {
        assert_eq!(feature["status"], "ok", "{feature}");
    }
}

/// The volume of the body a task's reference solution names, after it ran.
fn task_volume(s: &mut Session, task: &str, body: &str) -> f64 {
    let path = docs().join("tasks").join(format!("{task}.json"));
    let task: Value = serde_json::from_str(&read_text(&path)).unwrap();
    run(s, "file.new", json!({"units": "mm"}));
    for (id, params) in steps(&task["solution"]).unwrap() {
        run(s, &id, params);
    }
    let measured = run(s, "document.measure", json!({"feature": body}));
    measured["volume"].as_f64().unwrap()
}

/// Starter tasks 2 and 4 of the agent chapter (`agent.md`) give the answers the
/// task set's reference solutions build: the L-bracket's 6600 mm³ and the
/// case's 17385.50 mm³; and the case's 1 mm wall breaks an assertion of
/// "at least 1.5 mm", as task 4 tells the tester to see.
#[test]
fn the_field_test_tasks_two_and_four_are_as_the_page_says() {
    let text = read_text(&docs().join("guide/agent.md"));
    for said in [
        "**6600 mm³**",
        "**17385.50 mm³**",
        "1 mm wall",
        "at least 1.5 mm",
    ] {
        assert!(text.contains(said), "the page says {said}");
    }
    let mut s = common::session();
    let bracket = task_volume(&mut s, "l-bracket", "Bracket");
    assert!((bracket - 6600.0).abs() < 1e-6, "{bracket}");
    let case = task_volume(&mut s, "shelled-case", "Case");
    assert!((case - 17385.50).abs() < 0.005, "{case}");
    run(
        &mut s,
        "assert.add",
        json!({"name": "thin", "measure": "thickness", "of": "Case", "min": "1.5 mm"}),
    );
    let thin = |s: &mut Session| {
        run(s, "assert.list", json!({}))["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "thin")
            .unwrap()["status"]
            .clone()
    };
    // At 2 mm the wall is proven (since 2026-10-31: the exact faces bound
    // the lines where rounds meet rounds).
    let at_two = thin(&mut s);
    assert_eq!(at_two, "passed");
    run(
        &mut s,
        "variable.set",
        json!({"name": "wall", "expr": "1 mm"}),
    );
    assert_eq!(thin(&mut s), "failed");
}

/// The sweep along two rails of the features chapter (`features.md` ›
/// *Sweep along two rails*) runs as written: the spout is built, meets
/// its volume (a hollow frustum of a cone in closed form) and passes the
/// deep check.
#[test]
fn the_spout_of_the_features_chapter_meets_its_volume() {
    let text = read_text(&docs().join("guide/features.md"));
    let block = json_blocks(&text)
        .into_iter()
        .find(|b| b.contains("\"rail\": \"Right\""))
        .expect("the chapter has the spout");
    let steps = steps(&serde_json::from_str(&block).unwrap()).unwrap();
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (id, params) in steps {
        run(&mut s, &id, params);
    }
    let assertions = run(&mut s, "assert.list", json!({}));
    assert_eq!(assertions["ok"], true, "{assertions}");
    assert_eq!(assertions["passed"], 1, "{assertions}");
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

/// The rings and the up direction of the features chapter (`features.md`
/// › *Sweep along two rails*) run as written: the torus round two rings
/// meets its volume (Pappus), the wall with a leaning up direction its
/// (section times run times the cosine of the lean), and both pass the
/// deep check.
#[test]
fn the_rings_of_the_features_chapter_are_built() {
    let text = read_text(&docs().join("guide/features.md"));
    for marker in ["\"rail\": \"Outer\"", "\"up\": \"Lean\""] {
        let block = json_blocks(&text)
            .into_iter()
            .find(|b| b.contains(marker))
            .expect("the chapter has the example");
        let steps = steps(&serde_json::from_str(&block).unwrap()).unwrap();
        let mut s = common::session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        for (id, params) in steps {
            let out = run(&mut s, &id, params);
            assert_ne!(out["status"], "failed", "{id}: {out}");
        }
        let assertions = run(&mut s, "assert.list", json!({}));
        assert_eq!(assertions["ok"], true, "{assertions}");
        assert_eq!(assertions["passed"], 1, "{assertions}");
        let inspect = run(&mut s, "document.inspect", json!({}));
        let closed = inspect["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["valid"] == true);
        assert!(closed, "{inspect}");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
}
