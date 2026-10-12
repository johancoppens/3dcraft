//! From the resolved textures back to a [`Frame`].

use crate::GpuError;
use crate::renderer::Targets;
use craft3d_render::Frame;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};

/// Bytes per pixel of the colour target (`Rgba8Unorm`).
const COLOR_BYTES: u32 = 4;
/// Bytes per pixel of the data target (`Rg32Uint`).
const DATA_BYTES: u32 = 8;

#[derive(Default)]
struct MapState {
    result: Option<Result<(), String>>,
    waker: Option<Waker>,
}

/// One texture on its way to a mapped buffer.
struct Copy {
    buffer: wgpu::Buffer,
    pixel_bytes: u32,
    /// Bytes per row in the buffer: rows are padded to wgpu's alignment.
    stride: u32,
    state: Arc<Mutex<MapState>>,
}

/// Resolves when the buffer's mapping has succeeded or failed.
struct Mapped<'a>(&'a Mutex<MapState>);

impl Future for Mapped<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.result.is_some() {
            Poll::Ready(())
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// A readback that has been submitted.
pub(crate) struct Pending {
    width: u32,
    height: u32,
    color: Copy,
    data: Copy,
}

/// Copies the two resolved textures into buffers and asks for them to be
/// mapped.
pub(crate) fn start(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    targets: &Targets,
) -> Result<Pending, GpuError> {
    let (width, height) = (targets.width, targets.height);
    let too_large = || GpuError::Readback(format!("a {width}x{height} image is too large"));
    let layout = |pixel_bytes: u32| {
        let stride = width
            .checked_mul(pixel_bytes)
            .and_then(|row| row.checked_next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
            .ok_or_else(too_large)?;
        let size = u64::from(stride) * u64::from(height);
        if size > device.limits().max_buffer_size {
            return Err(too_large());
        }
        Ok((stride, size))
    };
    let (color_layout, data_layout) = (layout(COLOR_BYTES)?, layout(DATA_BYTES)?);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("craft3d readback"),
    });
    let mut copy = |texture: &wgpu::Texture, pixel_bytes: u32, (stride, size): (u32, u64)| {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("craft3d readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Copy {
            buffer,
            pixel_bytes,
            stride,
            state: Arc::new(Mutex::new(MapState::default())),
        }
    };
    let color = copy(&targets.color, COLOR_BYTES, color_layout);
    let data = copy(&targets.data, DATA_BYTES, data_layout);
    queue.submit([encoder.finish()]);
    for copy in [&color, &data] {
        let state = Arc::clone(&copy.state);
        copy.buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                state.result = Some(result.map_err(|e| e.to_string()));
                if let Some(waker) = state.waker.take() {
                    waker.wake();
                }
            });
    }
    Ok(Pending {
        width,
        height,
        color,
        data,
    })
}

impl Copy {
    /// The rows of the mapped buffer, without their padding, handed to
    /// `each` one pixel at a time.
    fn pixels(&self, width: u32, height: u32, mut each: impl FnMut(&[u8])) -> Result<(), GpuError> {
        let outcome = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .result
            .clone();
        match outcome {
            Some(Ok(())) => {}
            Some(Err(why)) => return Err(GpuError::Readback(why)),
            None => {
                return Err(GpuError::Readback(
                    "the GPU did not map the buffer".to_string(),
                ));
            }
        }
        {
            // One copy out of the mapping first: mapped GPU memory can be
            // very slow to read a few bytes at a time.
            let view = self
                .buffer
                .get_mapped_range(..)
                .map_err(|e| GpuError::Readback(e.to_string()))?
                .to_vec();
            let row_bytes = (width as usize).saturating_mul(self.pixel_bytes as usize);
            let mut rows = 0_u32;
            for row in view
                .chunks(self.stride.max(1) as usize)
                .take(height as usize)
            {
                let row = row.get(..row_bytes).ok_or_else(|| {
                    GpuError::Readback("the mapped buffer is shorter than the image".to_string())
                })?;
                row.chunks_exact(self.pixel_bytes.max(1) as usize)
                    .for_each(&mut each);
                rows += 1;
            }
            if rows != height {
                return Err(GpuError::Readback(
                    "the mapped buffer is shorter than the image".to_string(),
                ));
            }
        }
        self.buffer.unmap();
        Ok(())
    }
}

impl Pending {
    /// Waits until both buffers are mapped (or have failed to).
    pub(crate) async fn mapped(&self) {
        for copy in [&self.color, &self.data] {
            Mapped(&copy.state).await;
        }
    }

    /// Builds the frame from the mapped buffers.
    pub(crate) fn finish(self) -> Result<Frame, GpuError> {
        let (width, height) = (self.width, self.height);
        // Bounded: the renderer accepts at most MAX_SIDE per side.
        let pixels = (width as usize).saturating_mul(height as usize);
        let mut frame = Frame {
            width,
            height,
            rgb: Vec::with_capacity(pixels.saturating_mul(3)),
            ids: Vec::with_capacity(pixels),
            depth: Vec::with_capacity(pixels),
        };
        self.color.pixels(width, height, |pixel| {
            if let [r, g, b, _] = *pixel {
                frame.rgb.extend([r, g, b]);
            }
        })?;
        self.data.pixels(width, height, |pixel| {
            if let [a, b, c, d, e, f, g, h] = *pixel {
                frame.ids.push(u32::from_le_bytes([a, b, c, d]));
                let dist = f32::from_bits(u32::from_le_bytes([e, f, g, h]));
                frame.depth.push(f64::from(dist));
            }
        })?;
        if frame.rgb.len() != pixels.saturating_mul(3)
            || frame.ids.len() != pixels
            || frame.depth.len() != pixels
        {
            return Err(GpuError::Readback(
                "the mapped buffers do not hold the whole image".to_string(),
            ));
        }
        Ok(frame)
    }
}
