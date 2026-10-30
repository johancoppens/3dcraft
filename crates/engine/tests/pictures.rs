//! The renderer a host gives the session (`craft3d_engine::pictures`),
//! with fake renderers: `auto` falls back to the CPU when the host's
//! renderer cannot draw, fails half-way or panics, a studio still is then
//! averaged over the frames the CPU can, and the host's renderer goes
//! back into the session after every command (docs/design/rendering.md ›
//! *Headless GPU*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::pictures::{PictureRenderer, Shading};
use craft3d_engine::{Access, Session, Workspace};
use craft3d_render::{Camera, FaceColors, Frame, RenderParams};
use craft3d_tess::Mesh;
use serde_json::{Value, json};
use std::path::Path;

/// A renderer that says `describe`, then draws `good` pictures (the CPU's
/// plain picture, whatever is asked: cheap) and fails or panics after.
struct Fake {
    describe: Result<String, String>,
    good: usize,
    panics: bool,
}

impl PictureRenderer for Fake {
    fn describe(&mut self) -> Result<String, String> {
        self.describe.clone()
    }

    fn render(
        &mut self,
        mesh: &Mesh,
        colors: &FaceColors,
        camera: &Camera,
        params: &RenderParams,
        _shading: Shading<'_>,
        _key: Option<u64>,
    ) -> Result<Frame, String> {
        if self.good == 0 {
            if self.panics {
                panic!("the fake renderer panics");
            }
            return Err("the device was lost".to_string());
        }
        self.good -= 1;
        craft3d_render::render_colored(mesh, colors, camera, params).map_err(|e| e.to_string())
    }
}

fn fake(describe: Result<&str, &str>, good: usize, panics: bool) -> Box<Fake> {
    Box::new(Fake {
        describe: describe.map(str::to_string).map_err(str::to_string),
        good,
        panics,
    })
}

fn session(dir: &Path, renderer: Box<Fake>) -> Session {
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    });
    s.set_picture_renderer(renderer);
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [20, 20, 5]}),
    );
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Ball", "radius": 4, "position": [30, 10, 4]}),
    );
    s
}

/// The studio look at its cheapest: what is left is the accumulation.
fn bare_studio() -> Value {
    json!({"ao": false, "shadows": false, "reflections": false, "bloom": 0, "contactShadows": false,
           "specularShadow": false, "ground": false, "background": "plain"})
}

/// 1030 × 1020 with 4 samples a pixel: the CPU averages at most 15 frames
/// within its 2^26 samples, fewer than the 16 a replay's studio still
/// takes by default.
const BIG: (u32, u32) = (1030, 1020);

#[test]
fn auto_without_an_adapter_sizes_a_studio_replay_for_the_cpu() {
    let dir = common::workdir("pictures", "no-adapter");
    let mut s = session(&dir, fake(Err("no adapter on this machine"), 0, false));
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "out", "width": BIG.0, "height": BIG.1, "frames": 2, "animate": [],
               "look": bare_studio()}),
    );
    assert_eq!(result["renderer"], "cpu", "{result}");
    assert_eq!(result["rendererFallback"], "no adapter on this machine");
    assert_eq!(result["look"]["accumulate"], 15, "{result}");
    assert!(dir.join("out/frame-00001.png").is_file());
}

#[test]
fn a_failure_in_the_middle_of_a_replay_finishes_it_on_the_cpu() {
    let dir = common::workdir("pictures", "midway");
    // Plain: two pictures on the host, the rest on the CPU.
    let mut s = session(&dir, fake(Ok("fake adapter"), 2, false));
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "plain", "width": 64, "height": 48, "orbit": 90, "hold": 3}),
    );
    assert_eq!(result["renderer"], "mixed", "{result}");
    assert_eq!(result["pictures"]["gpu"], 2);
    assert_eq!(
        result["pictures"]["cpu"],
        result["frames"].as_u64().unwrap() - 2
    );
    assert_eq!(result["rendererFallback"], "the device was lost");
    for k in 0..result["frames"].as_u64().unwrap() {
        assert!(dir.join(format!("plain/frame-{k:05}.png")).is_file(), "{k}");
    }
    // Studio, above the CPU's frames: the frame after the failure is
    // averaged over the 15 the CPU can, rather than the replay failing.
    let mut s = session(&dir, fake(Ok("fake adapter"), 1, false));
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "studio", "width": BIG.0, "height": BIG.1, "frames": 2, "animate": [],
               "camera": {"orbit": 10}, "look": bare_studio()}),
    );
    assert_eq!(result["renderer"], "mixed", "{result}");
    assert_eq!(result["look"]["accumulate"], 16);
    assert_eq!(result["cpuAccumulate"], 15, "{result}");
    assert!(dir.join("studio/frame-00001.png").is_file());
}

#[test]
fn a_renderer_that_panics_is_let_go_of_and_the_cpu_draws() {
    let dir = common::workdir("pictures", "panic");
    let mut s = session(&dir, fake(Ok("fake adapter"), 0, true));
    let drawn = run(&mut s, "view.render", json!({"width": 64, "height": 48}));
    assert_eq!(drawn["renderer"], "cpu");
    assert!(
        drawn["rendererFallback"]
            .as_str()
            .unwrap()
            .contains("panicked"),
        "{drawn}"
    );
    // It is not trusted again: the session has no host renderer now.
    let refused = s
        .execute(
            "view.render",
            &json!({"width": 64, "height": 48, "renderer": "gpu"}),
        )
        .unwrap_err();
    assert!(
        refused.message().contains("no GPU renderer"),
        "{}",
        refused.message()
    );
    // `gpu` on a renderer that panics is an error, not a crash.
    let mut s = session(&dir, fake(Ok("fake adapter"), 0, true));
    let refused = s
        .execute(
            "view.render",
            &json!({"width": 64, "height": 48, "renderer": "gpu"}),
        )
        .unwrap_err();
    assert!(
        refused.message().contains("panicked"),
        "{}",
        refused.message()
    );
}

#[test]
fn the_host_renderer_goes_back_after_every_command_and_ids_stay_on_the_cpu() {
    let dir = common::workdir("pictures", "back");
    let mut s = session(&dir, fake(Ok("fake adapter"), 100, false));
    let drawn = run(&mut s, "view.render", json!({"width": 64, "height": 48}));
    assert_eq!(drawn["renderer"], "gpu");
    assert_eq!(drawn["adapter"], "fake adapter");
    // A command that fails after it took the renderer out gives it back.
    assert!(
        s.execute("view.replay", &json!({"dir": "../out", "renderer": "gpu"}))
            .is_err()
    );
    assert!(
        s.execute("view.replay", &json!({"dir": "out", "view": "sideways"}))
            .is_err()
    );
    let again = run(
        &mut s,
        "view.render",
        json!({"width": 64, "height": 48, "renderer": "gpu"}),
    );
    assert_eq!(again["renderer"], "gpu");
    // A picture of face ids is the CPU's.
    let ids = run(
        &mut s,
        "view.render",
        json!({"width": 64, "height": 48, "mode": "ids"}),
    );
    assert_eq!(ids["renderer"], "cpu");
    // `cpu` keeps to the CPU whatever the host has.
    let cpu = run(
        &mut s,
        "view.render",
        json!({"width": 64, "height": 48, "renderer": "cpu"}),
    );
    assert_eq!(cpu["renderer"], "cpu");
    assert!(cpu.get("rendererFallback").is_none());
}
