//! The self-contained way in: find an adapter, make a device, render to
//! textures and read them back.

#[cfg(not(target_arch = "wasm32"))]
use crate::GpuMesh;
use crate::{GpuError, Renderer};
use craft3d_render::{Camera, Frame, RenderParams};
use craft3d_tess::Mesh;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

/// Which adapter a renderer runs on, for reports and test logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterSummary {
    pub name: String,
    /// `Vulkan`, `Metal`, `Dx12`, `Gl` or `BrowserWebGpu`.
    pub backend: String,
    /// `DiscreteGpu`, `IntegratedGpu`, `Cpu` (a software rasteriser), …
    pub device_type: String,
    pub driver: String,
}

impl AdapterSummary {
    fn of(info: &wgpu::AdapterInfo) -> Self {
        Self {
            name: info.name.clone(),
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
            driver: format!("{} {}", info.driver, info.driver_info)
                .trim()
                .to_string(),
        }
    }
}

impl fmt::Display for AdapterSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}, {}, {})",
            self.name, self.backend, self.device_type, self.driver
        )
    }
}

/// A renderer with a device of its own.
pub struct Offscreen {
    renderer: Renderer,
    adapter: AdapterSummary,
    /// The last error wgpu reported outside an error scope (a lost device).
    uncaptured: Arc<Mutex<Option<String>>>,
}

/// The descriptor of the instance: WGPU_BACKEND, WGPU_VALIDATION and the
/// backend options from the environment.
///
/// Debug names (`InstanceFlags::DEBUG`) are off unless `WGPU_DEBUG` asks
/// for them, in debug builds too. With them wgpu names objects through
/// `vkSetDebugUtilsObjectNameEXT` (its own internal ones as well, so
/// discarding our labels is not enough), and the Vulkan loader walks its
/// list of devices for that without a lock: a second thread making or
/// dropping a device at that moment crashes the process (SIGSEGV in
/// `loader_get_icd_and_device`; four in ten runs of the tests under load).
/// No code of ours can catch that, so the call is not made.
fn descriptor() -> wgpu::InstanceDescriptor {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    let asked = std::env::var("WGPU_DEBUG").is_ok_and(|value| value != "0");
    if !asked {
        descriptor.flags.remove(wgpu::InstanceFlags::DEBUG);
    }
    descriptor
}

// One instance per caller, not one for the process: wgpu's OpenGL backend
// ties an instance's EGL context to the thread that uses it, and a shared
// one fails with `BadAccess` (a panic inside wgpu-hal) when two threads
// render at once.
fn instance() -> wgpu::Instance {
    wgpu::Instance::new(descriptor())
}

/// The adapter `WGPU_ADAPTER_NAME` asks for, if it is set.
async fn adapter_by_name(instance: &wgpu::Instance) -> Option<Result<wgpu::Adapter, GpuError>> {
    let wanted = std::env::var("WGPU_ADAPTER_NAME").ok()?.to_lowercase();
    let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
    let names: Vec<String> = adapters.iter().map(|a| a.get_info().name).collect();
    let found = adapters
        .into_iter()
        .find(|a| a.get_info().name.to_lowercase().contains(&wanted));
    Some(found.ok_or_else(|| {
        GpuError::NoAdapter(format!(
            "WGPU_ADAPTER_NAME=`{wanted}` matches none of the adapters found: [{}]",
            names.join(", ")
        ))
    }))
}

async fn find_adapter() -> Result<wgpu::Adapter, GpuError> {
    let instance = instance();
    if let Some(named) = adapter_by_name(&instance).await {
        return named;
    }
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::from_env().unwrap_or_default(),
        ..wgpu::RequestAdapterOptions::default()
    };
    match instance.request_adapter(&options).await {
        Ok(adapter) => Ok(adapter),
        // A software rasteriser will do.
        Err(first) => instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                force_fallback_adapter: true,
                ..options
            })
            .await
            .map_err(|_| GpuError::NoAdapter(first.to_string())),
    }
}

impl Offscreen {
    /// A renderer on the adapter wgpu's environment variables choose
    /// (`WGPU_BACKEND`, `WGPU_ADAPTER_NAME`, `WGPU_POWER_PREF`), or else on
    /// the default one; a software adapter is accepted.
    pub async fn new_async() -> Result<Offscreen, GpuError> {
        let adapter = find_adapter().await?;
        Self::on_adapter_async(&adapter).await
    }

    /// A renderer on a device made from `adapter`.
    pub async fn on_adapter_async(adapter: &wgpu::Adapter) -> Result<Offscreen, GpuError> {
        let summary = AdapterSummary::of(&adapter.get_info());
        let descriptor = |required_limits| wgpu::DeviceDescriptor {
            label: Some("craft3d offscreen"),
            // The opaque scene in two steps (`renderer::FirstWrite`) where
            // the adapter can.
            required_features: adapter.features() & wgpu::Features::DEPTH32FLOAT_STENCIL8,
            required_limits,
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
        };
        let (device, queue) = match adapter
            .request_device(&descriptor(wgpu::Limits::default()))
            .await
        {
            Ok(pair) => pair,
            // An adapter below WebGPU's baseline: take what it has.
            Err(_) => adapter
                .request_device(&descriptor(
                    wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                ))
                .await
                .map_err(|e| GpuError::Device(format!("{summary}: {e}")))?,
        };
        // Without a handler wgpu panics on an error outside a scope.
        let uncaptured = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&uncaptured);
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            *sink.lock().unwrap_or_else(PoisonError::into_inner) = Some(error.to_string());
        }));
        let renderer = Renderer::new(&device, &queue)?;
        Ok(Offscreen {
            renderer,
            adapter: summary,
            uncaptured,
        })
    }

    /// [`Offscreen::new_async`], blocking.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Result<Offscreen, GpuError> {
        pollster::block_on(Self::new_async())
    }

    /// [`Offscreen::on_adapter_async`], blocking.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn on_adapter(adapter: &wgpu::Adapter) -> Result<Offscreen, GpuError> {
        pollster::block_on(Self::on_adapter_async(adapter))
    }

    /// The adapter this renderer runs on.
    pub fn adapter(&self) -> &AdapterSummary {
        &self.adapter
    }

    /// The renderer, for uploading a mesh once and drawing it many times.
    pub fn renderer(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    /// An error wgpu reported outside any of our calls, if there was one.
    fn check_uncaptured(&self) -> Result<(), GpuError> {
        let taken = self
            .uncaptured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        taken.map_or(Ok(()), |why| Err(GpuError::Device(why)))
    }

    /// Renders `mesh` as seen from `camera`: the GPU counterpart of
    /// [`craft3d_render::render`], with the same result type.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render(
        &mut self,
        mesh: &Mesh,
        camera: &Camera,
        params: &RenderParams,
    ) -> Result<Frame, GpuError> {
        let uploaded = self.renderer.upload(mesh)?;
        self.render_uploaded(&uploaded, camera, params)
    }

    /// The GPU counterpart of [`craft3d_render::render_colored`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render_colored(
        &mut self,
        mesh: &Mesh,
        colors: &craft3d_render::FaceColors,
        camera: &Camera,
        params: &RenderParams,
    ) -> Result<Frame, GpuError> {
        let uploaded = self.renderer.upload_colored(mesh, colors)?;
        self.render_uploaded(&uploaded, camera, params)
    }

    /// [`Offscreen::render`] for a mesh that is on the GPU already
    /// (`self.renderer().upload(..)`): what a viewer does, which uploads a
    /// model once and draws it at every camera. A lost device is reported
    /// here as with `render`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render_uploaded(
        &mut self,
        mesh: &GpuMesh,
        camera: &Camera,
        params: &RenderParams,
    ) -> Result<Frame, GpuError> {
        self.renderer.draw(mesh, camera, params)?;
        let frame = self.renderer.read_frame()?;
        self.check_uncaptured()?;
        Ok(frame)
    }

    /// [`Offscreen::render`] for callers that cannot block (the browser).
    pub async fn render_async(
        &mut self,
        mesh: &Mesh,
        camera: &Camera,
        params: &RenderParams,
    ) -> Result<Frame, GpuError> {
        let uploaded = self.renderer.upload(mesh)?;
        self.renderer.draw(&uploaded, camera, params)?;
        let frame = self.renderer.read_frame_async().await?;
        self.check_uncaptured()?;
        Ok(frame)
    }
}

/// Every adapter on this machine that the enabled backends (`WGPU_BACKEND`)
/// offer and `WGPU_ADAPTER_NAME`, if set, matches. Empty when there is none.
#[cfg(not(target_arch = "wasm32"))]
pub fn adapters() -> Vec<wgpu::Adapter> {
    let wanted = std::env::var("WGPU_ADAPTER_NAME")
        .ok()
        .map(|name| name.to_lowercase());
    pollster::block_on(instance().enumerate_adapters(wgpu::Backends::all()))
        .into_iter()
        .filter(|adapter| {
            wanted
                .as_ref()
                .is_none_or(|w| adapter.get_info().name.to_lowercase().contains(w))
        })
        .collect()
}

/// Renders one frame on a device made for the occasion. For more than one
/// frame keep an [`Offscreen`]: making a device costs far more than a draw.
#[cfg(not(target_arch = "wasm32"))]
pub fn render(mesh: &Mesh, camera: &Camera, params: &RenderParams) -> Result<Frame, GpuError> {
    Offscreen::new()?.render(mesh, camera, params)
}
