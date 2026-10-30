//! The wgpu renderer for the CLI's sessions, without a window: what
//! `view.render`, `view.replay`, `craft3d-cli render`, `replay` and `serve`
//! draw on when the machine has an adapter (`renderer: "auto"` or `"gpu"`;
//! docs/design/rendering.md › *Headless GPU*). The device is made the first
//! time a picture is asked for, so a session that never renders pays
//! nothing, and kept for the session's later pictures.
//!
//! The CPU renderer stays the oracle: `renderer: "cpu"` draws with it, and
//! the parity test below holds this renderer's pictures of whole commands
//! to it with the GPU crate's own rules.

use craft3d_engine::pictures::{PictureRenderer, Shading};
use craft3d_gpu::{
    Camera, FaceColors, Frame, GpuError, GpuMesh, Mesh, Offscreen, RenderParams, Studio,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// An offscreen wgpu renderer, made when first used.
#[derive(Default)]
pub(crate) struct GpuPictures {
    /// The device, or why there is none (kept: it is not asked again).
    device: Option<Result<Offscreen, String>>,
    /// The mesh uploaded last and the key it was given.
    uploaded: Option<(u64, GpuMesh)>,
    /// The device was made again after it was lost (once per session).
    rebuilt: bool,
}

/// The device, made the first time.
fn device(slot: &mut Option<Result<Offscreen, String>>) -> Result<&mut Offscreen, String> {
    if slot.is_none() {
        let made = catch_unwind(Offscreen::new)
            .unwrap_or_else(|_| {
                Err(craft3d_gpu::GpuError::Device(
                    "making the device panicked".to_string(),
                ))
            })
            .map_err(|e| e.to_string());
        *slot = Some(made);
    }
    match slot {
        Some(Ok(offscreen)) => Ok(offscreen),
        Some(Err(why)) => Err(why.clone()),
        None => Err("no device".to_string()),
    }
}

/// One picture on `offscreen`, the mesh uploaded once per key.
#[allow(clippy::too_many_arguments)]
fn draw_once(
    offscreen: &mut Offscreen,
    uploaded: &mut Option<(u64, GpuMesh)>,
    mesh: &Mesh,
    colors: &FaceColors,
    camera: &Camera,
    params: &RenderParams,
    shading: Shading<'_>,
    key: Option<u64>,
) -> Result<Frame, GpuError> {
    if let Shading::Analysed(values) = shading {
        return offscreen.render_analysed(mesh, colors, values, camera, params);
    }
    let reuse = matches!((&*uploaded, key), (Some((had, _)), Some(want)) if *had == want);
    if !reuse {
        *uploaded = None;
        let gpu_mesh = offscreen.renderer().upload_colored(mesh, colors)?;
        *uploaded = Some((key.unwrap_or(0), gpu_mesh));
    }
    let Some((_, gpu_mesh)) = uploaded.as_ref() else {
        return Err(GpuError::NothingDrawn);
    };
    let frame = match shading {
        Shading::Studio(look, env) => offscreen.render_uploaded_studio(
            gpu_mesh,
            camera,
            params,
            &Studio {
                env: env.clone(),
                look: *look,
            },
        ),
        _ => offscreen.render_uploaded(gpu_mesh, camera, params),
    };
    if key.is_none() {
        *uploaded = None;
    }
    frame
}

impl PictureRenderer for GpuPictures {
    fn describe(&mut self) -> Result<String, String> {
        device(&mut self.device).map(|offscreen| offscreen.adapter().to_string())
    }

    fn render(
        &mut self,
        mesh: &Mesh,
        colors: &FaceColors,
        camera: &Camera,
        params: &RenderParams,
        shading: Shading<'_>,
        key: Option<u64>,
    ) -> Result<Frame, String> {
        loop {
            let Self {
                device: slot,
                uploaded,
                rebuilt,
            } = self;
            let offscreen = device(slot)?;
            // wgpu reports its errors as values here; should it panic all
            // the same, the device is let go of and the caller falls back
            // to the CPU, rather than the program ending.
            let drawn = catch_unwind(AssertUnwindSafe(|| {
                draw_once(
                    offscreen, uploaded, mesh, colors, camera, params, shading, key,
                )
            }));
            match drawn {
                Ok(Ok(frame)) => return Ok(frame),
                // A device that was lost (a driver reset, a long-lived
                // `serve` session) is made again, once per session, and the
                // picture drawn on the new one.
                Ok(Err(GpuError::Device(_))) if !*rebuilt => {
                    *rebuilt = true;
                    *uploaded = None;
                    *slot = None;
                }
                Ok(Err(e)) => return Err(e.to_string()),
                Err(_) => {
                    *uploaded = None;
                    *slot = Some(Err(
                        "the GPU renderer panicked; this session draws on the CPU from now on"
                            .to_string(),
                    ));
                    return Err("the GPU renderer panicked".to_string());
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests;
