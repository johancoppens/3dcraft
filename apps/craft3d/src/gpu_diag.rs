//! The window's part of the graphics diagnostics (`craft3d_gpu::diag`) and
//! of its choice of adapter: the first lines of the trace
//! (`CRAFT3D_GPU_TRACE`: the version, the environment, the adapter, what
//! the device is asked for, the surface), and the adapter chosen by the
//! order and the graphics start journal (`graphics_start`).

use crate::graphics_start;
use craft3d_gpu::diag;
use eframe::egui_wgpu::{self, WgpuConfiguration, WgpuSetup};
use eframe::wgpu;
use std::sync::Arc;

/// The variables the trace repeats at its start, when they are set.
const SAID_VARIABLES: [&str; 7] = [
    diag::SAMPLES_VAR,
    diag::BACKEND_ORDER_VAR,
    diag::PEEL_VARIANT_VAR,
    "WGPU_BACKEND",
    "WGPU_POWER_PREF",
    "WGPU_ADAPTER_NAME",
    "WGPU_VALIDATION",
];

/// Prepares the window's wgpu configuration, before eframe makes the
/// window: the adapter is chosen by `start` ([`graphics_start::Start::selector`]:
/// the order, and what the journal says of earlier starts); with
/// `CRAFT3D_BACKEND_ORDER` (and without `WGPU_BACKEND`) the instance is
/// made with the listed backends only; with the trace, what the device is
/// asked for is noted before it is made.
pub fn prepare(
    config: &mut WgpuConfiguration,
    env: &dyn Fn(&str) -> Option<String>,
    start: &graphics_start::Start,
) {
    if diag::enabled() {
        diag::note(
            "version",
            &format!(
                "{} on {} {}",
                craft3d_engine::version_line("craft3d"),
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        );
        let set: Vec<String> = SAID_VARIABLES
            .iter()
            .filter_map(|name| env(name).map(|value| format!("{name}=`{value}`")))
            .collect();
        diag::note(
            "environment",
            &if set.is_empty() {
                "none of the graphics variables is set".to_string()
            } else {
                set.join(" ")
            },
        );
    }
    let WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup else {
        return;
    };
    // Which adapter: by the rule (the person's order, or the window's own,
    // which the journal may skip in) and the graphics start journal.
    let order = env(diag::BACKEND_ORDER_VAR);
    let wgpu_backend = env("WGPU_BACKEND");
    let rule = graphics_start::rule(
        order.as_deref(),
        wgpu_backend.as_deref(),
        cfg!(windows),
        dxc_beside(),
    );
    // The person's own order (`CRAFT3D_BACKEND_ORDER`): only the listed
    // backends are started, as `WGPU_BACKEND` does.
    let given = !rule.skip && rule.order.is_some();
    if given && let Some(order) = &rule.order {
        setup.instance_descriptor.backends = diag::backends_of(order);
    }
    // The window chooses the adapter itself on Windows (the order and the
    // journal), and wherever the person gave an order; elsewhere wgpu's own
    // choice, as it always was (`graphics_start::self_healing_enabled`).
    if graphics_start::self_healing_enabled() || given {
        diag::note("adapter", &format!("the order: {}", rule.why));
        setup.native_adapter_selector = Some(start.selector(rule, setup.power_preference));
    }
    if diag::enabled() {
        let asked = Arc::clone(&setup.device_descriptor);
        setup.device_descriptor = Arc::new(move |adapter: &wgpu::Adapter| {
            let descriptor = asked(adapter);
            diag::note(
                "adapter",
                &format!(
                    "{}; downlevel {:?}",
                    diag::describe_adapter(&adapter.get_info()),
                    adapter.get_downlevel_capabilities().flags
                ),
            );
            diag::note(
                "device",
                &format!(
                    "asked: features {:?}; limits {:?}; memory hints {:?}; next: the device, \
                     then egui's renderer (egui-wgpu makes its pipelines, not traced)",
                    descriptor.required_features,
                    descriptor.required_limits,
                    descriptor.memory_hints
                ),
            );
            descriptor
        });
    }
}

/// Whether `dxcompiler.dll` lies beside the program (Windows): wgpu then
/// compiles DirectX 12's shaders with DXC, not FXC, which takes minutes
/// over the studio look's.
fn dxc_beside() -> bool {
    cfg!(windows)
        && std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|folder| folder.join("dxcompiler.dll")))
            .is_some_and(|dll| dll.is_file())
}

/// Notes the surface once eframe has made the window and egui's renderer
/// (`surface`: what the window's configuration asks of it), before the
/// viewport's renderer is made.
pub fn render_state_ready(
    state: Option<&egui_wgpu::RenderState>,
    present_mode: wgpu::PresentMode,
    frame_latency: Option<u32>,
) {
    if !diag::enabled() {
        return;
    }
    match state {
        Some(state) => diag::note(
            "surface",
            &format!(
                "egui's renderer is made; surface format {:?}, present mode asked {present_mode:?}, \
                 frame latency asked {frame_latency:?}; on {}; next: the viewport's renderer",
                state.target_format,
                diag::describe_adapter(&state.adapter.get_info())
            ),
        ),
        None => diag::note("surface", "the window has no wgpu render state"),
    }
}
