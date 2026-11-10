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
    // The index and its 22 chapters (2026-11-10), each read below: a chapter
    // lost, or one the index leaves out or names but that is not there,
    // fails here.
    assert!(pages.len() >= 23, "guide pages: {pages:?}");
    let mut chapters: Vec<String> = pages
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .filter(|name| name != "README.md")
        .collect();
    chapters.sort();
    let index = read_text(&docs().join("guide/README.md"));
    let mut listed: Vec<String> = index
        .split("](")
        .skip(1)
        .filter_map(|rest| rest.split(')').next())
        .filter_map(|link| link.split('#').next())
        .filter(|link| link.ends_with(".md") && !link.contains('/'))
        .map(String::from)
        .collect();
    listed.sort();
    listed.dedup();
    assert_eq!(
        listed, chapters,
        "the chapters the index (docs/guide/README.md) links to, and the chapters there are"
    );
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

/// The weight of the measuring chapter (`measure.md` › *Outputs*) runs
/// as written and says what the text says: 30 000 mm³ of aluminium at
/// 2.7 g/cm³ weighs 81 g and passes "at most 100 g"; at steel's 7.85 g/cm³
/// it weighs 235.5 g and fails.
#[test]
fn the_weight_of_the_measuring_chapter_is_as_it_says() {
    let text = read_text(&docs().join("guide/measure.md"));
    for said in ["*Weight 81 g*", "235.5 g", "7.85 g/cm³"] {
        assert!(text.contains(said), "the page says {said}");
    }
    let block = json_blocks(&text)
        .into_iter()
        .find(|b| b.contains("variable.output") && b.contains("density"))
        .expect("the chapter has a block of outputs");
    let steps = steps(&serde_json::from_str(&block).unwrap()).unwrap();
    let mut s = Session::new(craft3d_engine::Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    let mut last = Value::Null;
    for (id, params) in steps {
        last = run(&mut s, &id, params);
    }
    let weight = |listed: &Value| {
        let o = listed["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["name"] == "weight")
            .unwrap()
            .clone();
        assert_eq!(o["status"], "known", "{o}");
        assert_eq!(o["unit"], "g", "{o}");
        o["value"].as_f64().unwrap()
    };
    assert!((weight(&last) - 81.0).abs() < 1e-9, "{last}");
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["ok"], true, "{listed}");
    run(
        &mut s,
        "variable.set",
        json!({"name": "density", "expr": "7.85 g / (1 cm)^3"}),
    );
    let listed = run(&mut s, "output.list", json!({}));
    assert!((weight(&listed) - 235.5).abs() < 1e-9, "{listed}");
    let checked = run(&mut s, "assert.list", json!({}));
    assert_eq!(checked["failed"], 1, "{checked}");
}

/// The hull of the engineering chapter (`engineering-checks.md` ›
/// *Stability as outputs*) runs as written and says what the text says:
/// the box hull's GM is 65 mm, the assertion on it passes, and with KG at
/// 300 mm GM is -55 mm and it fails.
#[test]
fn the_outputs_of_the_engineering_chapter_are_as_it_says() {
    let text = read_text(&docs().join("guide/engineering-checks.md"));
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

/// The *notProven* of the measuring chapter (`measure.md` › *Assertions*)
/// is one: the case lesson's wall assertion, given the bound the chapter
/// writes, answers the chapter's message word for word, and the bound it
/// says passes passes, with the proven bound it quotes.
#[test]
fn the_not_proven_wall_of_the_measuring_chapter_is_as_it_says() {
    let text = read_text(&docs().join("guide/measure.md")).replace('\n', " ");
    let said = text
        .split("A real *notProven*: with the bound written as `\"min\": \"")
        .nth(1)
        .expect("the chapter has its notProven example");
    let bound = said.split('"').next().unwrap();
    let message = said
        .split("the case's thickness answers *")
        .nth(1)
        .and_then(|rest| rest.split("*.").next())
        .expect("the chapter quotes the message");
    let passes = said
        .split("A bound of `")
        .nth(1)
        .and_then(|rest| rest.split('`').next())
        .expect("the chapter names a bound that passes");
    let proven = said
        .split("here to at least ")
        .nth(1)
        .and_then(|rest| rest.split(" mm.").next())
        .expect("the chapter says how far the proof reaches then");
    let script: Value =
        serde_json::from_str(&read_text(&docs().join("examples/case.json"))).unwrap();
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (id, params) in steps(&script).unwrap() {
        run(&mut s, &id, params);
    }
    let walls = |s: &mut Session, min: &str| {
        run(s, "assert.edit", json!({"name": "walls", "min": min}));
        let listed = run(s, "assert.list", json!({}));
        listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "walls")
            .unwrap_or_else(|| panic!("the case has its walls assertion: {listed}"))
            .clone()
    };
    let unproven = walls(&mut s, bound);
    assert_eq!(unproven["status"], "notProven", "{bound}: {unproven}");
    assert_eq!(unproven["message"], message, "{bound}: {unproven}");
    let passed = walls(&mut s, passes);
    assert_eq!(passed["status"], "passed", "{passes}: {passed}");
    let reached = format!("(at least {proven} mm)");
    assert!(
        passed["message"].as_str().unwrap().contains(&reached),
        "{passes}: the chapter says {reached}: {passed}"
    );
}

/// The floating assertions of the engineering chapter (`engineering-checks.md`
/// › *Floating, as assertions and outputs*) run as written and say what the
/// text says: the barge trimmed by tan⁻¹ 0.05 (the wall-sided closed form,
/// xg = tan θ (GML + BML tan² θ / 2) with xg 64 mm) passes both, its GM
/// 66.75 mm (B² / (12 T cos θ) less the height of G above B, 53.4 mm);
/// with KG at 300 mm `upright` fails, and `trim` too (3.16°, past 3°).
#[test]
fn the_floating_assertions_of_the_engineering_chapter_are_as_it_says() {
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
    assert_eq!(listed["assertions"][1]["status"], "failed", "{listed}");
    let trim = listed["assertions"][1]["measured"].as_f64().unwrap();
    assert!(
        (trim.abs().to_degrees() - 3.16).abs() < 0.005,
        "the trim at KG 300 mm, {trim} rad, is not the 3.16° the chapter says: {listed}"
    );
}

/// The third starter task of the agent chapter (`agent.md` › *Try this
/// first*) says what its script makes: 11 holes and 6552.05 mm³ at 120 mm,
/// and 14 holes and 8175.33 mm³ once the strip is 150 mm long, its two
/// assertions passing both times.
#[test]
fn the_starter_strip_is_as_the_agent_chapter_says() {
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
        let said = format!("**{holes} holes and {volume:.2} mm³**");
        assert!(text.contains(&said), "the page says {said}");
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

/// Starter tasks 1, 2 and 4 of the agent chapter (`agent.md` › *Try this
/// first*) give the answers it prints: the block's 8000 mm³, and what the
/// task set's reference solutions build, the L-bracket's 6600 mm³ and the
/// case's 17385.50 mm³; and the case's 1 mm wall breaks an assertion of
/// "at least 1.5 mm", as task 4 tells the reader to see.
#[test]
fn the_starter_tasks_give_the_answers_the_agent_chapter_says() {
    let text = read_text(&docs().join("guide/agent.md"));
    for said in [
        "40 × 20 × 10 mm block",
        "**8000 mm³**",
        "**6600 mm³**",
        "**17385.50 mm³**",
        "1 mm wall",
        "at least 1.5 mm",
    ] {
        assert!(text.contains(said), "the page says {said}");
    }
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": ["40 mm", "20 mm", "10 mm"], "name": "Block"}),
    );
    let block = run(&mut s, "document.measure", json!({"feature": "Block"}))["volume"]
        .as_f64()
        .unwrap();
    assert!((block - 8000.0).abs() < 1e-9, "{block}");
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

/// What a result shown in the guide says: each `"key": value` it writes,
/// the value a number, a string (whose pieces between `...` must all be in
/// the real one, in order), `true`, `false`, `null`, an empty list `[]`, or
/// something else (an object, a list with items, `...`: only the key is
/// held).
#[derive(Debug)]
enum Shown {
    Number(f64),
    Text(Vec<String>),
    Bool(bool),
    Null,
    Empty,
    Other,
}

fn shown_pairs(text: &str) -> Vec<(String, Shown)> {
    let chars: Vec<char> = text.chars().collect();
    let mut pairs = Vec::new();
    let mut i = 0;
    // A string from the quote at `at`: its text and the index after it.
    let string = |at: usize| -> (String, usize) {
        let mut s = String::new();
        let mut j = at + 1;
        while j < chars.len() && chars[j] != '"' {
            if chars[j] == '\\' && j + 1 < chars.len() {
                j += 1;
            }
            s.push(chars[j]);
            j += 1;
        }
        (s, j + 1)
    };
    while i < chars.len() {
        if chars[i] != '"' {
            i += 1;
            continue;
        }
        let (word, after) = string(i);
        let mut j = after;
        while j < chars.len() && chars[j] == ' ' {
            j += 1;
        }
        if j >= chars.len() || chars[j] != ':' {
            i = after;
            continue;
        }
        j += 1;
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        let value = match chars.get(j) {
            Some('"') => {
                let (s, end) = string(j);
                i = end;
                let pieces = s
                    .split("...")
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(String::from)
                    .collect();
                Shown::Text(pieces)
            }
            Some('[') if chars[j + 1..].iter().find(|c| !c.is_whitespace()) == Some(&']') => {
                i = j + 1;
                Shown::Empty
            }
            Some('t' | 'f' | 'n') => {
                let mut end = j;
                while end < chars.len() && chars[end].is_ascii_alphabetic() {
                    end += 1;
                }
                let literal: String = chars[j..end].iter().collect();
                i = end;
                match literal.as_str() {
                    "true" => Shown::Bool(true),
                    "false" => Shown::Bool(false),
                    "null" => Shown::Null,
                    _ => panic!("the guide shows \"{word}\": {literal}, which is not JSON"),
                }
            }
            Some(c) if c.is_ascii_digit() || *c == '-' => {
                let mut end = j;
                while end < chars.len()
                    && (chars[end].is_ascii_digit() || "+-.eE".contains(chars[end]))
                {
                    end += 1;
                }
                let number: String = chars[j..end].iter().collect();
                i = end;
                Shown::Number(number.parse().unwrap_or_else(|e| panic!("{number}: {e}")))
            }
            _ => {
                i = j;
                Shown::Other
            }
        };
        pairs.push((word, value));
    }
    pairs
}

/// Every `(key, value)` of a JSON value, at any depth.
fn real_pairs<'a>(value: &'a Value, out: &mut Vec<(&'a str, &'a Value)>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                out.push((key.as_str(), inner));
                real_pairs(inner, out);
            }
        }
        Value::Array(items) => {
            for inner in items {
                real_pairs(inner, out);
            }
        }
        _ => {}
    }
}

/// Holds a result as the guide shows it: every key it writes is one the
/// command answers with, every number is the real one (to 1e-9 of it),
/// every string is in the real one, and every `true`, `false`, `null` and
/// `[]` is the real value, under the same key.
fn as_shown(call: &str, shown: &str, real: &Value) {
    let mut pairs = Vec::new();
    real_pairs(real, &mut pairs);
    for (key, value) in shown_pairs(shown) {
        let same: Vec<&Value> = pairs
            .iter()
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .collect();
        assert!(
            !same.is_empty(),
            "{call}: the guide shows `{key}`, the result has none: {real}"
        );
        let found = match &value {
            Shown::Number(n) => same.iter().any(|v| {
                v.as_f64()
                    .is_some_and(|r| (r - n).abs() <= 1e-9 * n.abs().max(1.0))
            }),
            Shown::Text(pieces) => same.iter().any(|v| {
                v.as_str().is_some_and(|r| {
                    let mut rest = r;
                    pieces.iter().all(|p| match rest.find(p.as_str()) {
                        Some(at) => {
                            rest = &rest[at + p.len()..];
                            true
                        }
                        None => false,
                    })
                })
            }),
            Shown::Bool(b) => same.iter().any(|v| v.as_bool() == Some(*b)),
            Shown::Null => same.iter().any(|v| v.is_null()),
            Shown::Empty => same.iter().any(|v| v.as_array().is_some_and(Vec::is_empty)),
            Shown::Other => true,
        };
        assert!(
            found,
            "{call}: the guide shows `{key}`: {value:?}, the result says {real}"
        );
    }
}

/// The agent chapter's session (`agent.md` › *A full session*) runs as it
/// is shown: each `command_run` and `command_batch` of its blocks is run
/// in a new session, and every key, number, string, `true`, `false`,
/// `null` and `[]` its results show is in the real result. So a renamed
/// key, such as `assertionSummary`, or a proven bound that moved fails
/// here, not in the reader's hands. The batch's own counts are held
/// against its steps, each of which must succeed (`completed` all of them,
/// `failed` 0, `errors` none, `last` the last step's index and result);
/// the `command_list` entry against the registry as it stands before
/// `file.new` (its params, and why it is disabled); the preview's keys
/// against `view.render`'s, at the tool's default size (the MCP tools
/// themselves are `craft3d-automation`'s, above this crate).
#[test]
fn the_agent_chapters_session_answers_as_shown() {
    let text = read_text(&docs().join("guide/agent.md"));
    let section = text
        .split("## A full session")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("the chapter has its session");
    let dir = common::workdir("guide", "agent-session");
    let mut s = common::session_in(&dir);
    let tools = [
        "command_run",
        "command_batch",
        "command_list",
        "doc_render_preview",
        "doc_inspect",
        "selection_get",
    ];
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in section.lines() {
        match (&mut current, line.trim_start()) {
            (None, "```text") => current = Some(Vec::new()),
            (Some(block), "```") => {
                blocks.push(std::mem::take(block));
                current = None;
            }
            (Some(block), _) => block.push(line),
            (None, _) => {}
        }
    }
    let mut ran = 0;
    for block in blocks {
        // Calls: a tool's name and its argument, up to the line `->`, then
        // the result shown, up to the next call.
        let mut calls: Vec<(String, String, String)> = Vec::new();
        for line in block {
            let trimmed = line.trim_start();
            if let Some(tool) = tools.iter().find(|t| trimmed.starts_with(&format!("{t} "))) {
                calls.push((
                    tool.to_string(),
                    trimmed[tool.len()..].to_string(),
                    String::new(),
                ));
            } else if let Some(last) = calls.last_mut() {
                if trimmed.starts_with("->") || !last.2.is_empty() {
                    last.2.push_str(trimmed.trim_start_matches("->"));
                    last.2.push('\n');
                } else {
                    last.1.push_str(line);
                    last.1.push('\n');
                }
            }
        }
        for (tool, argument, result) in calls {
            match tool.as_str() {
                "command_list" => {
                    let call: Value = serde_json::from_str(&argument)
                        .unwrap_or_else(|e| panic!("{argument}: {e}"));
                    let filter = call["filter"].as_str().unwrap().to_lowercase();
                    // The entries `command_list` gives (craft3d-automation's
                    // registry.rs): those whose id or label holds the
                    // filter, whether they can run now and why not.
                    let commands: Vec<Value> = Session::commands()
                        .iter()
                        .filter(|spec| {
                            spec.id.to_lowercase().contains(&filter)
                                || spec.label.to_lowercase().contains(&filter)
                        })
                        .map(|spec| {
                            let disabled = s.disabled_reason(spec);
                            json!({
                                "id": spec.id,
                                "label": spec.label,
                                "params": spec.params,
                                "summary": spec.summary,
                                "needsDocument": spec.needs_document,
                                "enabled": disabled.is_none(),
                                "disabledReason": disabled,
                            })
                        })
                        .collect();
                    as_shown("command_list", &result, &json!({"commands": commands}));
                    // The params shown are the command's, whole up to the
                    // `...` where the guide shortens them.
                    let shown_params = result
                        .split("\"params\": \"")
                        .nth(1)
                        .and_then(|rest| rest.split(", ...").next())
                        .expect("the entry shows its params")
                        .replace("\\\"", "\"");
                    let pattern = Session::commands()
                        .iter()
                        .find(|spec| spec.id == "feature.pattern")
                        .unwrap()
                        .params;
                    assert!(
                        pattern.starts_with(&shown_params),
                        "the guide shows the params of feature.pattern as {shown_params}; they are {pattern}"
                    );
                    ran += 1;
                }
                "doc_render_preview" => {
                    let call: Value = serde_json::from_str(&argument)
                        .unwrap_or_else(|e| panic!("{argument}: {e}"));
                    let mut params = call.clone();
                    // The tool's default size, 1024 on the longer side, 4:3.
                    params["width"] = json!(1024);
                    params["height"] = json!(768);
                    let real = run(&mut s, "view.render", params);
                    let shown = result
                        .split("and ")
                        .nth(1)
                        .expect("the preview shows what comes with the picture");
                    as_shown("doc_render_preview", shown, &real);
                    ran += 1;
                }
                "command_run" => {
                    let call: Value = serde_json::from_str(&argument)
                        .unwrap_or_else(|e| panic!("{argument}: {e}"));
                    let id = call["id"].as_str().unwrap();
                    let real = run(&mut s, id, call["params"].clone());
                    as_shown(id, &result, &real);
                    ran += 1;
                }
                "command_batch" => {
                    let call: Value = serde_json::from_str(&argument)
                        .unwrap_or_else(|e| panic!("{argument}: {e}"));
                    let steps = call["steps"].as_array().unwrap();
                    let mut last = Value::Null;
                    for step in steps {
                        // Checked: a step that fails fails the test, so the
                        // batch completed every step and failed none.
                        last = run(&mut s, step["id"].as_str().unwrap(), step["params"].clone());
                        ran += 1;
                    }
                    assert_eq!(call["terse"], true, "the session's batch is terse");
                    // What a terse batch answers when every step succeeded
                    // (craft3d-automation's registry.rs): the counts, no
                    // errors, and the last step by its index.
                    let report = json!({
                        "completed": steps.len(),
                        "failed": 0,
                        "errors": [],
                        "last": {"step": steps.len() - 1, "result": last},
                    });
                    as_shown("command_batch", &result, &report);
                    let shown = result
                        .split("\"result\":")
                        .nth(1)
                        .expect("the batch shows its last result");
                    as_shown("command_batch", shown, &last);
                }
                other => panic!("the session calls {other}, which this test does not run"),
            }
        }
    }
    assert!(ran >= 25, "{ran} commands run");
}

/// The assemblies chapter's small assembly (`assemblies.md` › *A small
/// assembly, step by step*) builds as it says: the bar is saved, placed
/// twice (the second 80 mm long), mated with nothing left free, and its
/// arm turns a full turn without touching the base's volume at any of the
/// samples `motion.check` takes.
#[test]
fn the_small_assembly_of_the_assemblies_chapter_moves_as_it_says() {
    let text = read_text(&docs().join("guide/assemblies.md"));
    let section = text
        .split("## A small assembly, step by step")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("the chapter builds a small assembly");
    let dir = common::workdir("guide", "assembly");
    let mut s = common::session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let mut results = std::collections::BTreeMap::new();
    // The part is held to "incremental = full regeneration" as every part
    // here is; an assembly has no history of features to regenerate, so
    // from `assembly.new` on the commands run plainly, as the assembly
    // tests run them.
    let mut assembly = false;
    for block in json_blocks(section) {
        let value: Value = serde_json::from_str(&block).unwrap();
        for (id, params) in steps(&value).unwrap() {
            assembly |= id == "assembly.new";
            let result = if assembly {
                common::plain::run(&mut s, &id, params.clone())
            } else {
                run(&mut s, &id, params.clone())
            };
            assert_ne!(result["status"], "failed", "{id} {params}: {result}");
            results.insert(id, result);
        }
    }
    let solved = &results["assembly.solve"];
    assert_eq!(solved["ok"], true, "{solved}");
    assert_eq!(solved["dof"], 0, "{solved}");
    let checked = &results["motion.check"];
    assert_eq!(checked["ok"], true, "{checked}");
    assert_eq!(checked["samples"], 17, "{checked}");
    assert_eq!(checked["assertions"][0]["status"], "passed", "{checked}");
    let arm = common::plain::run(&mut s, "assembly.inspect", json!({}))["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "Arm")
        .unwrap()
        .clone();
    assert_eq!(arm["params"]["len"], "80 mm", "{arm}");
    assert!(dir.join("two-bars.3dcraftasm").is_file());
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
