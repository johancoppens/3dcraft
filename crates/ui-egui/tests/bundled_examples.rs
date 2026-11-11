//! The start screen's examples, Jigs and assemblies (docs/decisions.md,
//! 2026-11-10): the list built into the program is the manifest's
//! (`docs/examples/published/bundle.json`), and every one opens through the
//! engine as the start screen opens it ([`run_example`]:
//! `Session::open_bytes`, no file of its own), regenerates and is what its
//! repository recorded when it built it (the `stats` of its `meta.json`,
//! copied into the manifest): as many features and assertions, no feature
//! failed, no assertion failed and none fewer proven; for an assembly as
//! many instances and mates, solved with the freedom it had. Only looked
//! at, it is clean and named by its title (`Session::open_bytes_titled`).
//! Save asks where, so the bytes built into the program are never
//! written. How long
//! each takes to open and show is printed (`--nocapture`). An assembly
//! opened from the start screen shows its player, on its motion.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::shell::{EXAMPLES, ExampleSection, run_example};
use egui::{Pos2, vec2};
use serde_json::{Value, json};
use std::time::Instant;

fn manifest() -> Vec<Value> {
    let text = include_str!("../../../docs/examples/published/bundle.json");
    let manifest: Value = serde_json::from_str(text).unwrap();
    manifest["entries"].as_array().unwrap().clone()
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn stat(entry: &Value, key: &str) -> u64 {
    entry["stats"][key]
        .as_u64()
        .unwrap_or_else(|| panic!("{}: no stats.{key}", entry["id"]))
}

/// The list the start screen shows is the manifest's, in its order: the
/// set the developer chose, the first two for a newcomer.
#[test]
fn the_start_screen_lists_what_the_manifest_bundles() {
    let entries = manifest();
    assert_eq!(EXAMPLES.len(), entries.len());
    for (example, entry) in EXAMPLES.iter().zip(&entries) {
        assert_eq!(example.id, entry["id"], "{entry}");
        assert_eq!(Some(example.title), entry["title"].as_str());
        assert_eq!(Some(example.summary), entry["summary"].as_str());
        assert_eq!(example.file(), entry["file"].as_str());
        assert_eq!(Some(example.section.word()), entry["section"].as_str());
        assert_eq!(Some(example.start_here()), entry["start"].as_bool());
        let seconds = entry["stats"]["load_s"].as_f64().unwrap();
        assert!(
            (f64::from(example.seconds()) - seconds).abs() < 1e-3,
            "{}",
            example.id
        );
    }
    let ids: Vec<&str> = EXAMPLES.iter().map(|e| e.id).collect();
    assert_eq!(
        ids,
        [
            "steel-bookend",
            "t-slot-profile",
            "moka-pot",
            "ship-propeller",
            "snap-fit-enclosure",
            "hydraulic-cylinder",
            "spiral-staircase",
            "solitaire-ring",
            "flat-pack-bookcase",
            "mailer-box",
            "laser-box",
            "gridfinity-bin",
            "joint-drawer",
            "sheet-metal-enclosure",
            "planetary-gearbox",
            "sundial",
            "toggle-clamp",
            "jansen-walker",
            "scissor-jack",
            "radial-engine",
        ],
        "the set of 2026-11-10"
    );
    // The examples MIT, the Jigs and assemblies CC BY 4.0.
    for entry in &entries {
        let licence = if entry["section"] == "parts" {
            "MIT"
        } else {
            "CC-BY-4.0"
        };
        assert_eq!(entry["licence"], licence, "{}", entry["id"]);
    }
}

#[test]
fn every_bundled_file_opens_regenerates_and_is_what_its_repository_recorded() {
    let entries = manifest();
    let mut timings = Vec::new();
    for (example, entry) in EXAMPLES.iter().zip(&entries) {
        let id = example.id;
        let mut session = Session::new(Workspace::memory());
        let started = Instant::now();
        let opened = run_example(&mut session, example).unwrap_or_else(|e| panic!("{id}: {e}"));
        // What the window shows first: the bodies regenerated and meshed.
        let scene = session.scene(None).unwrap().unwrap();
        let shown = started.elapsed().as_secs_f64();
        assert!(!scene.bodies.is_empty(), "{id}: nothing to show");
        let status = session.document_status().unwrap();
        assert!(status.path.is_none(), "{id}: no file of its own");
        assert_eq!(
            status.jig_in_use,
            example.section == ExampleSection::Jigs,
            "{id}"
        );
        match example.section {
            ExampleSection::Parts | ExampleSection::Jigs => {
                assert_eq!(
                    opened["opened"]["features"].as_u64(),
                    Some(stat(entry, "features")),
                    "{id}: features"
                );
                let document = run(&mut session, "document.inspect", json!({}));
                for feature in document["features"].as_array().unwrap() {
                    assert_ne!(feature["status"], "failed", "{id}: {feature}");
                }
            }
            ExampleSection::Assemblies => {
                assert_eq!(opened["opened"]["assembly"], true, "{id}");
                let inspected = run(&mut session, "assembly.inspect", json!({}));
                assert_eq!(
                    inspected["instances"].as_array().unwrap().len() as u64,
                    stat(entry, "instances"),
                    "{id}: instances"
                );
                assert_eq!(
                    inspected["mates"].as_array().unwrap().len() as u64,
                    stat(entry, "mates"),
                    "{id}: mates"
                );
                let solve = &inspected["solve"];
                assert_eq!(solve["ok"], true, "{id}: {solve}");
                assert_eq!(solve["dof"].as_u64(), Some(stat(entry, "dof")), "{id}");
            }
        }
        // Every assertion it carries holds (its closed forms), none
        // fewer proven than when its repository built it.
        let listed = run(&mut session, "assert.list", json!({}));
        let all = started.elapsed().as_secs_f64();
        assert_eq!(
            listed["count"].as_u64(),
            Some(stat(entry, "assertions")),
            "{id}: {listed}"
        );
        assert_eq!(listed["failed"], 0, "{id}: {listed}");
        assert_eq!(listed["errors"], 0, "{id}: {listed}");
        assert_eq!(listed["pending"], 0, "{id}: {listed}");
        let not_proven = listed["notProven"].as_u64().unwrap_or(0);
        assert!(
            not_proven <= stat(entry, "warnings"),
            "{id}: {not_proven} not proven, {} when it was built: {listed}",
            stat(entry, "warnings")
        );
        // Opened, regenerated and checked, and still no unsaved work: no
        // mark, no question on closing it, named by its title, no file
        // for the recent files (docs/decisions.md, 2026-11-10).
        let status = session.document_status().unwrap();
        assert!(
            !status.dirty && !status.close_asks,
            "{id}: only looked at: {status:?}"
        );
        assert_eq!(status.title.as_deref(), Some(example.title), "{id}");
        assert!(status.file.is_none(), "{id}");
        assert_eq!(
            status.assembly,
            example.section == ExampleSection::Assemblies,
            "{id}"
        );
        // Save asks where: the bytes built into the program are not a
        // file it could write.
        let refused = session.execute("file.save", &json!({}));
        assert!(refused.is_err(), "{id}: saved without a place");
        assert!(session.file_names().is_empty(), "{id}");
        timings.push((id, shown, all));
    }
    for (id, shown, all) in &timings {
        println!("bundled {id}: shown after {shown:.2} s, assertions after {all:.2} s");
    }
}

/// A frame of the window at `time`, 1280 × 800; the texts it drew.
fn frame(ctx: &egui::Context, shell: &mut Shell, session: &mut Session, time: f64) -> Vec<String> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
        time: Some(time),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
    output.textures_delta.clear();
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

/// The start screen promises that an assembly "opens with its player:
/// press play and it moves". Each of the four, opened as a click on its
/// card opens it (chosen, taken by the host, built, then
/// `Shell::example_opened`), opens on the Assembly tab, where the player
/// is, on its motion, although each has public parameters, on which a
/// document opens otherwise (before 2026-11-11 it did here too, the player
/// one click away). The tab heads its checks "Checks", as a part's
/// Features tab does.
#[test]
fn every_bundled_assembly_opens_on_its_player_from_the_start_screen() {
    let assemblies: Vec<_> = EXAMPLES
        .iter()
        .filter(|e| e.section == ExampleSection::Assemblies)
        .collect();
    assert_eq!(assemblies.len(), 4);
    for example in assemblies {
        let id = example.id;
        let mut session = Session::new(Workspace::memory());
        let mut shell = Shell::new();
        let ctx = egui::Context::default();
        let mut time = 0.0;
        frame(&ctx, &mut shell, &mut session, time);
        shell.show_section(ExampleSection::Assemblies);
        shell.choose_example(example);
        let taken = shell.take_example().unwrap();
        let opened = run_example(&mut session, taken);
        assert!(opened.is_ok(), "{id}: {opened:?}");
        shell.example_opened(&opened);
        // The instances and mates folded, so the checks' header is in
        // sight (a header out of sight is not drawn).
        shell.state.side_panel.set_open("instances-folded", true);
        shell.state.side_panel.set_open("mates-folded", true);
        let mut drawn = Vec::new();
        for _ in 0..3 {
            time += 1.0 / 60.0;
            drawn = frame(&ctx, &mut shell, &mut session, time);
        }
        let got = shell
            .handle_method(&mut session, "ui.get", Some(&json!({})))
            .unwrap()
            .unwrap();
        assert_eq!(got["state"]["panels"]["sideTab"], "assembly", "{id}");
        assert_eq!(got["state"]["timeline"]["motion"], true, "{id}");
        assert_eq!(got["timeline"]["playsMotion"], true, "{id}");
        assert_eq!(got["parameters"]["jig"], true, "{id}: public parameters");
        assert_eq!(got["parameters"]["tab"], "features", "{id}");
        assert!(
            shell.player_hits().iter().any(|(name, _)| *name == "play"),
            "{id}: the player is drawn"
        );
        assert!(drawn.iter().any(|t| t == "Checks"), "{id}: {drawn:?}");
        assert!(!drawn.iter().any(|t| t == "Assertions"), "{id}: {drawn:?}");
    }
}
