//! The R1 task set (`docs/tasks/`, `docs/testing.md` › *The task set*):
//! every task's reference solution is replayed headless and scored by
//! `craft3d_engine::task_score`, which holds the model to the task's
//! closed-form expectations (volume, area, centroid, box, counts), checks
//! it deeply, and then sets the task's variants (another value of a main
//! variable), regenerates and holds the same closed forms, evaluated with
//! the new values, to the new model.
//!
//! One test per task, so one task can be run alone. A task marked
//! `refused` is one the kernel refuses today with no workaround a user
//! would choose; its test asserts that refusal, and turns red the day the
//! kernel learns it: then take the mark out of the task file and the
//! expectations are held instead.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::task_score::{reference_solution, score_task};
use serde_json::Value;
use std::path::PathBuf;

fn tasks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/tasks")
}

fn load(id: &str) -> Value {
    let path = tasks_dir().join(format!("{id}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

/// What failed in a scorecard, for the message of a failed test.
fn failures(card: &Value) -> String {
    let mut out = Vec::new();
    for s in card["failedSteps"].as_array().unwrap() {
        out.push(format!("step failed: {s}"));
    }
    for f in card["failedFeatures"].as_array().unwrap() {
        out.push(format!("feature failed: {f}"));
    }
    let states = std::iter::once(&card["base"]).chain(card["variants"].as_array().unwrap());
    for (i, state) in states.enumerate() {
        let name = if i == 0 {
            "base".to_string()
        } else {
            format!("variant {}", i - 1)
        };
        if state["valid"] != true {
            out.push(format!("{name}: not valid: {}", state["problems"]));
        }
        for e in state["expectations"].as_array().into_iter().flatten() {
            if e["pass"] != true {
                out.push(format!("{name}: {e}"));
            }
        }
        if let Some(missing) = state.get("missingVariables")
            && missing.as_array().is_some_and(|m| !m.is_empty())
        {
            out.push(format!("{name}: missing variables {missing}"));
        }
    }
    out.join("\n")
}

fn check_task(id: &str) {
    let task = load(id);
    assert_eq!(task["id"], id, "the id is the file's name");
    let card = score_task(&task, reference_solution(&task).unwrap())
        .unwrap_or_else(|e| panic!("{id}: {}", e.message()));
    assert_eq!(card["task"], id);
    match task.get("refused") {
        None => {
            assert_eq!(card["pass"], true, "{id}:\n{}", failures(&card));
            assert_eq!(card["refusals"], 0, "{id}:\n{}", failures(&card));
            assert_eq!(card["passed"], card["total"]);
        }
        Some(refused) => {
            // Still refused, for the reason it was marked with. When this
            // fails because the feature builds, the kernel has learnt the
            // case: remove `refused` from the task file.
            let feature = refused["feature"].as_str().unwrap();
            let error = refused["error"].as_str().unwrap();
            let hit = card["failedFeatures"].as_array().unwrap().iter().any(|f| {
                f["name"] == feature && f["error"].as_str().is_some_and(|e| e.contains(error))
            });
            assert!(
                hit,
                "{id} is marked refused at `{feature}` (`{error}`), but the scorecard says:\n{}\n\
                 If the feature builds now, take `refused` out of docs/tasks/{id}.json.",
                failures(&card)
            );
            assert_eq!(card["pass"], false);
        }
    }
}

/// Every task, with its level. The list is checked against the files in
/// `docs/tasks`, so a task cannot be added without its test.
const TASKS: &[(&str, u64)] = &[
    ("spacer", 1),
    ("plate-holes", 1),
    ("l-bracket", 1),
    ("u-channel", 1),
    ("slotted-plate", 2),
    ("flanged-bushing", 2),
    ("pulley-blank", 2),
    ("bolt-flange", 2),
    ("hole-strip", 2),
    ("cable-clip", 2),
    ("countersunk-panel", 2),
    ("gusset-bracket", 2),
    ("standoff", 2),
    ("stepped-shaft", 3),
    ("rounded-cover", 3),
    ("knurled-knob", 3),
    ("pipe-tee", 3),
    ("phone-stand", 3),
    ("bent-bracket", 3),
    ("shelled-case", 3),
    ("enclosure", 4),
    ("hinge-leaf", 4),
];

#[test]
fn the_set_is_the_files_and_every_task_is_complete() {
    let mut files: Vec<String> = std::fs::read_dir(tasks_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .map(|p| p.file_stem().unwrap().to_str().unwrap().to_string())
        .collect();
    files.sort();
    let mut listed: Vec<String> = TASKS.iter().map(|(id, _)| id.to_string()).collect();
    listed.sort();
    assert_eq!(files, listed, "docs/tasks and TASKS differ");
    assert!(TASKS.len() >= 20);
    for (id, level) in TASKS {
        let task = load(id);
        assert_eq!(task["format"], "craft3d-task/1", "{id}");
        assert_eq!(task["level"], *level, "{id}");
        for key in ["title", "units"] {
            assert!(
                task[key].as_str().is_some_and(|s| !s.is_empty()),
                "{id}: {key}"
            );
        }
        for lang in ["en", "nl"] {
            assert!(
                task["description"][lang]
                    .as_str()
                    .is_some_and(|s| s.len() > 80),
                "{id}: description.{lang}"
            );
        }
        assert!(!task["derivation"].as_array().unwrap().is_empty(), "{id}");
        assert!(!task["variants"].as_array().unwrap().is_empty(), "{id}");
        let expect = task["expect"].as_array().unwrap();
        assert!(
            expect.iter().any(|e| e["measure"] == "volume"),
            "{id}: every task has a known volume"
        );
        // Every variable a variant sets is one the description names, so a
        // candidate knows what to call it.
        for variant in task["variants"].as_array().unwrap() {
            for name in variant["set"].as_object().unwrap().keys() {
                assert!(
                    task["description"]["en"]
                        .as_str()
                        .unwrap()
                        .contains(name.as_str()),
                    "{id}: the description names the variable {name}"
                );
            }
        }
    }
}

macro_rules! task_tests {
    ($($name:ident => $id:literal),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                check_task($id);
            }
        )*
    };
}

task_tests! {
    spacer => "spacer",
    plate_holes => "plate-holes",
    l_bracket => "l-bracket",
    u_channel => "u-channel",
    slotted_plate => "slotted-plate",
    flanged_bushing => "flanged-bushing",
    pulley_blank => "pulley-blank",
    bolt_flange => "bolt-flange",
    hole_strip => "hole-strip",
    cable_clip => "cable-clip",
    countersunk_panel => "countersunk-panel",
    gusset_bracket => "gusset-bracket",
    standoff => "standoff",
    stepped_shaft => "stepped-shaft",
    rounded_cover => "rounded-cover",
    knurled_knob => "knurled-knob",
    pipe_tee => "pipe-tee",
    phone_stand => "phone-stand",
    bent_bracket => "bent-bracket",
    shelled_case => "shelled-case",
    enclosure => "enclosure",
    hinge_leaf => "hinge-leaf",
}
