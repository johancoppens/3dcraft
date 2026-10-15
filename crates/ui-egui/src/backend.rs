//! Which renderer makes the pixels of the viewport: the wgpu renderer
//! (`craft3d-gpu`) on a device of its own when a graphics adapter is
//! there, and the CPU reference renderer (`craft3d-render`) otherwise, or
//! when it is asked for (`CRAFT3D_RENDERER=cpu`), or when the GPU fails.
//!
//! Both give the same [`Frame`] (colour, face ids, view distance), so
//! everything after the bodies are shaded (the overlay tested against the
//! depth buffer, the texture, the screenshot) is one code path. The GPU
//! frame is read back to the CPU for that: a viewer needs the depth
//! buffer anyway, and the picture goes on screen as a texture, as the CPU
//! renderer's does. The GPU renderer matches the CPU one within the
//! tolerance derived in docs/design/rendering.md.
//!
//! A [`BodyRenderer`] belongs to one thread: the window has one on the
//! thread that draws (quick previews) and one on the render thread (sharp
//! images), each with its own wgpu instance and device, because wgpu's
//! OpenGL backend ties a device to the thread that uses it.
//!
//! Nothing here panics: no adapter, a lost device, a mesh or target the
//! device cannot take, all end in the CPU renderer, and the reason is kept
//! for `ui.get` and the terminal. In the browser the viewport is rendered
//! on the CPU for now: the GPU renderer's blocking entry points do not
//! exist there.

use crate::state::CameraState;
use crate::theme::Tokens;
use crate::viewport::{Quality, render_params};
use craft3d_engine::Scene;
use craft3d_render::{Frame, Highlight, RenderParams};

/// A renderer of the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The CPU reference renderer: the oracle.
    Cpu,
    /// The wgpu renderer, read back to the CPU.
    Gpu,
}

impl Backend {
    /// `cpu` or `gpu`, as `ui.get` writes it.
    pub fn name(self) -> &'static str {
        match self {
            Backend::Cpu => "cpu",
            Backend::Gpu => "gpu",
        }
    }
}

/// Which renderer a host asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RendererChoice {
    /// The GPU when an adapter is found, else the CPU.
    #[default]
    Auto,
    /// The CPU renderer, whatever the machine has: to compare the two.
    Cpu,
}

impl RendererChoice {
    /// The value of `CRAFT3D_RENDERER` or `--renderer`: `auto` (also
    /// `gpu`, or nothing), or `cpu`.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "" | "auto" | "gpu" => Ok(RendererChoice::Auto),
            "cpu" => Ok(RendererChoice::Cpu),
            other => {
                let shown: String = other.chars().take(40).collect();
                Err(format!(
                    "`{shown}` is not a renderer; it is `auto` (the GPU when there is one), `gpu` or `cpu`"
                ))
            }
        }
    }
}

/// What the viewport renders with, for `ui.get` and the terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererReport {
    pub backend: Backend,
    /// The adapter the GPU renderer runs on.
    pub adapter: Option<String>,
    /// Why it is not the GPU on the window's own device although that was
    /// wanted: the reason the CPU renders, or the reason the GPU renders
    /// on a device of its own and reads back. `None` when the CPU was
    /// asked for or the GPU draws directly.
    pub fallback: Option<String>,
    /// True when the GPU draws on the window's own device, into the
    /// texture that is shown (`crate::direct`); false when the frame is
    /// read back from a device of the viewport's own, or made on the CPU.
    pub direct: bool,
    /// Samples per pixel of the picture: 1 without multisampling.
    pub msaa: u32,
}

impl RendererReport {
    /// One line for the terminal: `gpu (Intel …; direct, 4 samples)`,
    /// `gpu (Intel …; read back)` or `cpu: no adapter …`.
    pub fn describe(&self) -> String {
        let path = if self.direct {
            format!("direct, {} samples", self.msaa)
        } else {
            "read back".to_string()
        };
        let why = self
            .fallback
            .as_ref()
            .map_or_else(String::new, |why| format!(": {why}"));
        match (self.backend, &self.adapter) {
            (Backend::Gpu, Some(adapter)) => format!("gpu ({adapter}; {path}){why}"),
            (Backend::Gpu, None) => format!("gpu ({path}){why}"),
            (Backend::Cpu, _) => format!("cpu{why}"),
        }
    }
}

/// Shades the bodies of a scene with the renderer it was given, and keeps
/// the model on the GPU between frames.
pub struct BodyRenderer {
    #[cfg(not(target_arch = "wasm32"))]
    gpu: Option<gpu::Gpu>,
    report: RendererReport,
    /// The faces the next frames show highlighted.
    highlight: Highlight,
    /// The edges the overlay of the next frames points at.
    marks: crate::viewport::Marks,
    /// How opaque the bodies of the next frames are.
    opacity: f64,
}

impl BodyRenderer {
    /// What `viewport::compose_with` draws over the frames made from now
    /// on to point at something (the highlighted edges), and the section
    /// those frames are cut open with: both renderers draw that; a device
    /// that cannot (`craft3d_gpu::GpuError::Section`) hands such a frame
    /// to the CPU renderer.
    pub fn set_marks(&mut self, marks: crate::viewport::Marks) {
        self.marks = marks;
    }

    pub fn marks(&self) -> &crate::viewport::Marks {
        &self.marks
    }

    /// How opaque the bodies of the frames made from now on are, 0 to 1
    /// (`RenderParams::opacity`). Below 1 the bodies are see-through: both
    /// renderers draw that; a device that cannot
    /// (`craft3d_gpu::GpuError::SeeThrough`) hands such a frame to the CPU
    /// renderer rather than show a picture that is not the oracle's.
    pub fn set_opacity(&mut self, opacity: f64) {
        self.opacity = opacity;
    }

    pub fn opacity(&self) -> f64 {
        if self.opacity.is_finite() {
            self.opacity.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// As on a device that cannot draw see-through bodies: from now on a
    /// see-through frame is the CPU renderer's. For tests of that
    /// fallback; a CPU renderer is not changed by it.
    pub fn without_gpu_see_through(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(gpu) = &mut self.gpu {
            gpu.without_see_through();
        }
    }

    /// As on a device that cannot draw a cut-open picture: from now on a
    /// frame with a section is the CPU renderer's. For tests of that
    /// fallback; a CPU renderer is not changed by it.
    pub fn without_gpu_section(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(gpu) = &mut self.gpu {
            gpu.without_section();
        }
    }

    /// Which faces the frames made from now on show highlighted, and in
    /// which colour: the same faces whichever renderer makes the frame.
    pub fn set_highlight(&mut self, highlight: Highlight) {
        self.highlight = highlight;
    }

    pub fn highlight(&self) -> Highlight {
        self.highlight
    }

    /// The CPU reference renderer.
    pub fn cpu() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            gpu: None,
            report: RendererReport {
                backend: Backend::Cpu,
                adapter: None,
                fallback: None,
                direct: false,
                msaa: 1,
            },
            highlight: Highlight::NONE,
            marks: crate::viewport::Marks::default(),
            opacity: 1.0,
        }
    }

    /// The renderer asked for, on this thread. With [`RendererChoice::Auto`]
    /// the GPU is tried (an instance, an adapter, a device); when that
    /// fails, for whatever reason, the result is the CPU renderer with the
    /// reason in its report.
    pub fn new(choice: RendererChoice) -> Self {
        match choice {
            RendererChoice::Cpu => Self::cpu(),
            RendererChoice::Auto => Self::gpu_or_cpu(),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn gpu_or_cpu() -> Self {
        let mut renderer = Self::cpu();
        renderer.report.fallback =
            Some("the GPU viewport is not built for the browser yet".to_string());
        renderer
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn gpu_or_cpu() -> Self {
        match gpu::Gpu::open() {
            Ok(gpu) => Self {
                report: RendererReport {
                    backend: Backend::Gpu,
                    adapter: Some(gpu.adapter().to_string()),
                    fallback: None,
                    direct: false,
                    msaa: 1,
                },
                gpu: Some(gpu),
                highlight: Highlight::NONE,
                marks: crate::viewport::Marks::default(),
                opacity: 1.0,
            },
            Err(why) => {
                let mut renderer = Self::cpu();
                renderer.report.fallback = Some(why);
                renderer
            }
        }
    }

    pub fn report(&self) -> &RendererReport {
        &self.report
    }

    /// The renderer the next frame is made with.
    pub fn backend(&self) -> Backend {
        self.report.backend
    }

    /// The bodies of `scene` as seen with `camera`, in the theme's colours:
    /// the frame, and which renderer made it (the CPU when the GPU could
    /// not take this frame). No scene, or no bodies, is the background.
    pub fn render(
        &mut self,
        scene: Option<&Scene>,
        camera: &CameraState,
        quality: Quality,
        tokens: &Tokens,
    ) -> Result<(Frame, Backend), String> {
        let params = RenderParams {
            highlight: self.highlight,
            opacity: self.opacity(),
            // The section travels with the marks: the lines over the
            // picture are cut by the same plane.
            section: self.marks.section(),
            ..render_params(quality, tokens)
        };
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(gpu) = &mut self.gpu {
            match gpu.render(scene, camera, &params) {
                Ok(frame) => return Ok((frame, Backend::Gpu)),
                Err(gpu::Failure::Frame(why)) => return Err(why),
                // This frame on the CPU; the next one on the GPU again.
                Err(gpu::Failure::ThisFrame) => {}
                Err(gpu::Failure::ForGood(why)) => {
                    self.gpu = None;
                    self.report = RendererReport {
                        backend: Backend::Cpu,
                        adapter: None,
                        fallback: Some(why),
                        direct: false,
                        msaa: 1,
                    };
                }
            }
        }
        let empty = craft3d_tess::Mesh::default();
        let mesh = scene.map_or(&empty, |scene| &scene.mesh);
        let colors = craft3d_render::FaceColors::new();
        let colors = scene.map_or(&colors, |scene| &scene.face_colors);
        let frame = craft3d_render::render_colored(mesh, colors, &camera.to_render(), &params)
            .map_err(|e| e.to_string())?;
        Ok((frame, Backend::Cpu))
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod gpu {
    use super::*;
    use craft3d_gpu::{GpuError, GpuMesh, Offscreen};

    /// Why the GPU did not make a frame.
    pub(super) enum Failure {
        /// The CPU renderer would refuse it too (the size, the camera, the
        /// mesh): the error for the caller.
        Frame(String),
        /// Too large for this device (the target, the mesh, a number
        /// beyond `f32`): the CPU can still make it.
        ThisFrame,
        /// No device any more: the CPU from now on.
        ForGood(String),
    }

    /// Which mesh of which revision: the revision, and the bits of the
    /// tolerance and of the angle it was sampled with.
    type MeshKey = (u64, u64, Option<u64>);

    pub(super) struct Gpu {
        offscreen: Offscreen,
        /// The scene's mesh on the GPU, with the revision it is of and how
        /// it was sampled: the quick first picture and the display scene
        /// that replaces it are two meshes of one revision.
        uploaded: Option<(MeshKey, GpuMesh)>,
    }

    fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
        payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no details".to_string())
    }

    impl Gpu {
        /// A device on this thread, or why there is none. A panic inside
        /// the graphics stack while looking for one (a driver that falls
        /// over) is a reason like any other, not the end of the window.
        pub(super) fn open() -> Result<Self, String> {
            let opened = std::panic::catch_unwind(Offscreen::new).unwrap_or_else(|payload| {
                Err(GpuError::Device(format!(
                    "the graphics driver failed while a device was made: {}",
                    panic_text(payload.as_ref())
                )))
            });
            let offscreen = opened.map_err(|e| e.to_string())?;
            Ok(Self {
                offscreen,
                uploaded: None,
            })
        }

        pub(super) fn adapter(&self) -> &craft3d_gpu::AdapterSummary {
            self.offscreen.adapter()
        }

        pub(super) fn without_see_through(&mut self) {
            self.offscreen.renderer().without_see_through();
        }

        pub(super) fn without_section(&mut self) {
            self.offscreen.renderer().without_section();
        }

        pub(super) fn render(
            &mut self,
            scene: Option<&Scene>,
            camera: &CameraState,
            params: &RenderParams,
        ) -> Result<Frame, Failure> {
            let camera = camera.to_render();
            let empty = craft3d_tess::Mesh::default();
            let no_colors = craft3d_render::FaceColors::new();
            let colors = scene.map_or(&no_colors, |s| &s.face_colors);
            let (revision, mesh) = scene.map_or(((u64::MAX, 0, None), &empty), |s| {
                (
                    (
                        s.revision,
                        s.tolerance.to_bits(),
                        s.max_angle.map(f64::to_bits),
                    ),
                    &*s.mesh,
                )
            });
            // A render that panics inside wgpu (a driver bug) must not end
            // the thread; it ends the GPU's use.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if self.uploaded.as_ref().is_none_or(|(r, _)| *r != revision) {
                    // The old copy goes first: twice the model would not
                    // fit a small device.
                    self.uploaded = None;
                    let mesh = self.offscreen.renderer().upload_colored(mesh, colors)?;
                    self.uploaded = Some((revision, mesh));
                }
                let (_, uploaded) = self.uploaded.as_ref().ok_or(GpuError::NothingDrawn)?;
                self.offscreen.render_uploaded(uploaded, &camera, params)
            }));
            match outcome {
                Ok(Ok(frame)) => Ok(frame),
                Ok(Err(error)) => Err(classify(error)),
                Err(payload) => Err(Failure::ForGood(format!(
                    "the graphics driver failed while rendering: {}",
                    panic_text(payload.as_ref())
                ))),
            }
        }
    }

    fn classify(error: GpuError) -> Failure {
        let text = error.to_string();
        match error {
            GpuError::Render(_) => Failure::Frame(text),
            GpuError::TargetTooLarge { .. }
            | GpuError::MeshTooLarge { .. }
            | GpuError::TooManyLines { .. }
            | GpuError::OutOfRange(_)
            | GpuError::SeeThrough(_)
            | GpuError::Section(_) => Failure::ThisFrame,
            GpuError::NoAdapter(_)
            | GpuError::Device(_)
            | GpuError::ForeignMesh
            | GpuError::NothingDrawn
            | GpuError::Readback(_)
            | GpuError::Unsupported(_) => Failure::ForGood(text),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn what_the_cpu_refuses_too_is_an_error_and_the_rest_falls_back() {
            let frame = classify(GpuError::Render(craft3d_render::RenderError::InvalidSsaa(
                9,
            )));
            assert!(matches!(frame, Failure::Frame(text) if text.contains("ssaa")));
            for this_frame in [
                GpuError::TargetTooLarge {
                    side: 16384,
                    limit: 8192,
                },
                GpuError::MeshTooLarge { triangles: 1 << 30 },
                GpuError::OutOfRange("the camera"),
                GpuError::SeeThrough("no such pipeline".into()),
            ] {
                assert!(matches!(classify(this_frame), Failure::ThisFrame));
            }
            for for_good in [
                GpuError::NoAdapter("none".into()),
                GpuError::Device("lost".into()),
                GpuError::Readback("unmapped".into()),
                GpuError::ForeignMesh,
                GpuError::NothingDrawn,
            ] {
                assert!(matches!(classify(for_good), Failure::ForGood(_)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_is_read_from_its_variable() {
        for auto in ["", "auto", "GPU", " gpu "] {
            assert_eq!(
                RendererChoice::parse(auto),
                Ok(RendererChoice::Auto),
                "{auto:?}"
            );
        }
        assert_eq!(RendererChoice::parse("cpu"), Ok(RendererChoice::Cpu));
        assert_eq!(RendererChoice::parse("Cpu"), Ok(RendererChoice::Cpu));
        let error = RendererChoice::parse("software").unwrap_err();
        assert!(
            error.contains("`software`") && error.contains("cpu"),
            "{error}"
        );
        // A hostile value is shown cut, not whole.
        let long = "x".repeat(500);
        assert!(RendererChoice::parse(&long).unwrap_err().len() < 200);
        assert_eq!(RendererChoice::default(), RendererChoice::Auto);
    }

    #[test]
    fn the_cpu_renderer_is_what_it_says_and_renders() {
        let mut renderer = BodyRenderer::new(RendererChoice::Cpu);
        assert_eq!(renderer.backend(), Backend::Cpu);
        assert_eq!(
            renderer.report(),
            &RendererReport {
                backend: Backend::Cpu,
                adapter: None,
                fallback: None,
                direct: false,
                msaa: 1,
            }
        );
        assert_eq!(renderer.report().describe(), "cpu");
        let quality = Quality {
            width: 32,
            height: 24,
            ssaa: 1,
        };
        let (frame, backend) = renderer
            .render(None, &CameraState::default(), quality, &Tokens::default())
            .unwrap();
        assert_eq!(backend, Backend::Cpu);
        assert_eq!((frame.width, frame.height), (32, 24));
        assert_eq!(frame.coverage(), 0.0);
    }

    #[test]
    fn a_report_describes_itself_in_one_line() {
        let gpu = RendererReport {
            backend: Backend::Gpu,
            adapter: Some("Intel (Vulkan)".into()),
            fallback: None,
            direct: true,
            msaa: 4,
        };
        assert_eq!(gpu.describe(), "gpu (Intel (Vulkan); direct, 4 samples)");
        let read_back = RendererReport {
            direct: false,
            msaa: 1,
            fallback: Some("the viewport texture could not be made".into()),
            ..gpu.clone()
        };
        assert_eq!(
            read_back.describe(),
            "gpu (Intel (Vulkan); read back): the viewport texture could not be made"
        );
        let fallen = RendererReport {
            backend: Backend::Cpu,
            adapter: None,
            fallback: Some("no GPU adapter: none".into()),
            direct: false,
            msaa: 1,
        };
        assert_eq!(fallen.describe(), "cpu: no GPU adapter: none");
        assert_eq!(Backend::Gpu.name(), "gpu");
        assert_eq!(Backend::Cpu.name(), "cpu");
    }
}
