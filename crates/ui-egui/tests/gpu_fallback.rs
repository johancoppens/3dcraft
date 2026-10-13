//! Without a graphics adapter the viewport is rendered on the CPU, and
//! says why: the one case the parity tests cannot cover on a machine that
//! has one. The adapter is taken away with `WGPU_ADAPTER_NAME`, which the
//! GPU renderer honours and no adapter matches; the environment of a test
//! cannot be changed from inside it (that is `unsafe`), so the check runs
//! in a child process: this very test binary, with the variable set.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::backend::{Backend, BodyRenderer, RendererChoice};
use craft3d_ui_egui::state::CameraState;
use craft3d_ui_egui::viewport::{self, Quality};
use craft3d_ui_egui::{Shell, Tokens};
use serde_json::json;
use std::process::Command;

const CHILD: &str = "CRAFT3D_TEST_NO_ADAPTER_CHILD";
const NO_SUCH_ADAPTER: &str = "no-such-adapter-3dcraft";

#[test]
fn without_an_adapter_the_viewport_falls_back_to_the_cpu_renderer_and_says_why() {
    if std::env::var_os(CHILD).is_some() {
        child();
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("without_an_adapter_the_viewport_falls_back_to_the_cpu_renderer_and_says_why")
        .arg("--exact")
        .arg("--nocapture")
        .env(CHILD, "1")
        .env("WGPU_ADAPTER_NAME", NO_SUCH_ADAPTER)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("{stdout}");
    assert!(
        output.status.success(),
        "the child failed ({}):\n{stdout}\n{stderr}",
        output.status
    );
    assert!(stdout.contains("fallback: cpu:"), "{stdout}");
}

/// Runs with no adapter to be found.
fn child() {
    let mut renderer = BodyRenderer::new(RendererChoice::Auto);
    assert_eq!(renderer.backend(), Backend::Cpu);
    let report = renderer.report().clone();
    let why = report.fallback.as_deref().unwrap();
    assert!(
        why.contains(NO_SUCH_ADAPTER) && why.contains("no GPU adapter"),
        "{why}"
    );
    assert_eq!(report.adapter, None);
    println!("fallback: {}", report.describe());

    // And it renders, on the CPU.
    let mut session = Session::new(Workspace::none());
    session.execute("file.new", &json!({})).unwrap();
    session
        .execute("feature.block", &json!({"size": [20, 10, 5]}))
        .unwrap();
    let scene = session.scene(None).unwrap().unwrap();
    let mut camera = CameraState::default();
    camera.fit(&scene.bbox, 1.5);
    let quality = Quality {
        width: 96,
        height: 64,
        ssaa: 1,
    };
    let (frame, backend) = renderer
        .render(Some(&scene), &camera, quality, &Tokens::default())
        .unwrap();
    assert_eq!(backend, Backend::Cpu);
    assert!(frame.coverage() > 0.1);
    let composed = viewport::compose_with(
        &mut renderer,
        Some(&scene),
        &camera,
        quality,
        &Tokens::default(),
        None,
    )
    .unwrap();
    assert_eq!(composed.backend, Backend::Cpu);
    assert_eq!(composed.rgb, frame.rgb);

    // A shell asked for the GPU shows the CPU's picture and reports the
    // fallback, with and without a render thread.
    for mut shell in [
        Shell::with_renderer(RendererChoice::Auto),
        Shell::threaded_with(RendererChoice::Auto),
    ] {
        let ctx = egui::Context::default();
        let frame = |shell: &mut Shell, session: &mut Session| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
            // What a backend would upload.
            output.textures_delta.clear();
        };
        frame(&mut shell, &mut session);
        frame(&mut shell, &mut session);
        assert!(shell.wait_for_sharp(std::time::Duration::from_secs(120)));
        frame(&mut shell, &mut session);
        let stats = shell.render_stats().unwrap();
        assert_eq!(stats.renderer, Backend::Cpu);
        assert!(stats.sharp);
        let report = shell.renderer_report();
        assert_eq!(report.backend, Backend::Cpu);
        assert!(
            report
                .fallback
                .as_deref()
                .unwrap()
                .contains(NO_SUCH_ADAPTER),
            "{report:?}"
        );
        let got = shell
            .handle_method(&mut session, "ui.get", None)
            .unwrap()
            .unwrap();
        assert_eq!(got["renderer"]["backend"], "cpu");
        assert_eq!(got["render"]["renderer"], "cpu");
        assert!(
            got["renderer"]["fallback"]
                .as_str()
                .unwrap()
                .contains(NO_SUCH_ADAPTER)
        );
        assert!(shell.problem().is_none(), "{:?}", shell.problem());
    }
}
