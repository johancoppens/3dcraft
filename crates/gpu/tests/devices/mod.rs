//! Shared by the tests of the viewport renderer: a device on every adapter
//! of the machine. (Not in `common`, which the shell's tests include too,
//! without wgpu of their own.)

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft3d_gpu::ADAPTER_HINT;

/// A device on every adapter of the machine, with the adapter.
pub fn devices() -> Vec<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let adapters = craft3d_gpu::adapters();
    assert!(
        !adapters.is_empty(),
        "no GPU adapter found, so the viewport renderer cannot be tested: {ADAPTER_HINT}"
    );
    adapters
        .into_iter()
        .map(|adapter| {
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("craft3d test"),
                    required_features: wgpu::Features::empty(),
                    required_limits:
                        wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::default(),
                }))
                .unwrap_or_else(|e| {
                    panic!("cannot use adapter {:?}: {e}", adapter.get_info().name)
                });
            device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
                panic!("wgpu reported an error outside a scope: {error}");
            }));
            (adapter, device, queue)
        })
        .collect()
}
