//! wgpu renderer: the same picture as the CPU reference renderer, on the GPU
//! (docs/architecture.md §7).
//!
//! It takes what [`craft3d_render::render`] takes (a [`Mesh`], a [`Camera`],
//! [`RenderParams`]) and gives back the same [`Frame`]: colour, face ids and
//! view distance per pixel. The CPU renderer is the oracle; `tests/parity.rs`
//! compares the two pixel by pixel and states the tolerances.
//!
//! Three ways in:
//!
//! - [`Offscreen`] owns an adapter and a device, renders to textures and
//!   reads them back: `Offscreen::new()?.render(&mesh, &camera, &params)`.
//! - [`Renderer`] is built on a device and queue someone else owns.
//!   [`Renderer::upload`] puts a mesh on the GPU once, [`Renderer::draw`]
//!   renders it into textures the renderer owns ([`Renderer::color_view`]),
//!   and [`Renderer::draw_into`] also copies the picture into a texture
//!   view the caller gives. Both read the frame back.
//! - [`Onscreen`] is the viewport's renderer on the window's own device:
//!   the picture, multisampled, with the lines over it (edges, sketches,
//!   axes: [`Segment`]) drawn and depth-tested on the GPU, into a texture
//!   the window shows as it is. Nothing is read back for display.
//!
//! # How it mirrors the CPU renderer
//!
//! - **One projection.** The camera's basis and scale come from
//!   [`Camera::axes`], the function the CPU renderer uses; the shading
//!   constants are the CPU renderer's `SHADE_*`.
//! - **Supersampling** is done the same way: the scene is drawn with one
//!   sample per pixel into targets `ssaa` times larger per axis, and a
//!   resolve pass takes the rounded integer mean of the colour samples and
//!   the first sample's face id and distance.
//! - **Depth test.** `Depth32Float`, `Less`, triangles in mesh order: on a
//!   tie the triangle drawn first wins, as on the CPU. Near and far are fitted
//!   to the mesh per frame, so the buffer's precision is spent on the model.
//! - **Shading** is per sample in the fragment shader, two-sided, from the
//!   interpolated normal or the triangle's own where a vertex has none.
//!
//! `f32` exists only here, at the GPU boundary: positions are stored relative
//! to the centre of the mesh, and everything about the camera is computed in
//! `f64` before it is narrowed.
//!
//! # Known differences from the CPU renderer
//!
//! - A triangle that crosses the eye plane is clipped by the GPU, where the
//!   CPU renderer drops the whole triangle. Parity is stated for a camera
//!   outside the model.
//! - A triangle with a non-finite corner, or one that does not fit in `f32`
//!   relative to the mesh centre, is not drawn.
//! - A sample that lies exactly on a triangle's edge follows the GPU's
//!   top-left rule; the CPU renderer gives it to the triangle drawn first.
//!
//! # Errors, never panics
//!
//! No adapter, a refused or lost device, a zero-sized or oversized target, an
//! inconsistent mesh and a degenerate camera are all [`GpuError`] values.
//! wgpu reports validation and out-of-memory errors through error scopes,
//! which every entry point here opens on native targets; in the browser
//! WebGPU reports them to the console without unwinding.

mod mesh;
mod offscreen;
mod onscreen;
mod readback;
mod renderer;

pub use mesh::{GpuMesh, MAX_TRIANGLES};
pub use offscreen::{AdapterSummary, Offscreen};
pub use onscreen::{
    DrawOptions, GpuLines, MAX_SEGMENTS, Onscreen, OverlayParams, Segment, WANTED_SAMPLES,
};
pub use renderer::Renderer;

#[cfg(not(target_arch = "wasm32"))]
pub use offscreen::{adapters, render};

// The types of the shared interface, so a caller needs one import.
pub use craft3d_render::{
    Camera, Frame, Highlight, NO_FACE, Projection, RenderError, RenderParams,
};
pub use craft3d_tess::Mesh;

use std::fmt;

/// How to get a software adapter where there is no GPU; part of every
/// "no adapter" message.
pub const ADAPTER_HINT: &str = "install a Vulkan driver: the software rasteriser lavapipe is \
     enough (Fedora: `dnf install mesa-vulkan-drivers`, Debian/Ubuntu: `apt install \
     mesa-vulkan-drivers`). WGPU_BACKEND and WGPU_ADAPTER_NAME choose among several adapters";

#[derive(Clone, Debug, PartialEq)]
pub enum GpuError {
    /// No adapter was found, or none matches `WGPU_ADAPTER_NAME`.
    NoAdapter(String),
    /// The device could not be created, was lost, ran out of memory or
    /// rejected a request; the text is wgpu's.
    Device(String),
    /// The parameters, camera or mesh are ones the CPU renderer rejects too,
    /// with its error.
    Render(RenderError),
    /// The supersampled target is larger than the device's textures can be.
    TargetTooLarge { side: u64, limit: u32 },
    /// More triangles than [`MAX_TRIANGLES`].
    MeshTooLarge { triangles: usize },
    /// More segments than [`MAX_SEGMENTS`], or more than the device's
    /// buffers hold.
    TooManyLines { segments: usize },
    /// Something this renderer does not do.
    Unsupported(&'static str),
    /// A camera or mesh number does not fit in `f32` at the GPU boundary.
    OutOfRange(&'static str),
    /// The mesh was uploaded by another renderer.
    ForeignMesh,
    /// A frame was asked for before anything was drawn, or after a draw
    /// that failed.
    NothingDrawn,
    /// Reading the targets back failed.
    Readback(String),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::NoAdapter(why) => write!(f, "no GPU adapter: {why}; {ADAPTER_HINT}"),
            GpuError::Device(why) => write!(f, "GPU device error: {why}"),
            GpuError::Render(e) => write!(f, "{e}"),
            GpuError::TargetTooLarge { side, limit } => write!(
                f,
                "a render target {side} pixels wide or high exceeds the device limit of \
                 {limit}; lower the size or ssaa"
            ),
            GpuError::MeshTooLarge { triangles } => write!(
                f,
                "{triangles} triangles exceed the limit of {MAX_TRIANGLES}"
            ),
            GpuError::TooManyLines { segments } => write!(
                f,
                "{segments} line segments exceed the limit of {MAX_SEGMENTS} or the device's buffers"
            ),
            GpuError::Unsupported(what) => write!(f, "{what} is not supported by this renderer"),
            GpuError::OutOfRange(what) => {
                write!(f, "{what} does not fit in single precision for the GPU")
            }
            GpuError::ForeignMesh => write!(
                f,
                "the mesh was uploaded by another renderer; upload it with this one"
            ),
            GpuError::NothingDrawn => write!(f, "nothing has been drawn yet"),
            GpuError::Readback(why) => write!(f, "reading the frame back failed: {why}"),
        }
    }
}

impl std::error::Error for GpuError {}

impl From<RenderError> for GpuError {
    fn from(error: RenderError) -> Self {
        GpuError::Render(error)
    }
}
