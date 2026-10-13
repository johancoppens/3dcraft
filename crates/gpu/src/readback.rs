//! From the resolved textures back to a [`Frame`].

use crate::GpuError;
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

/// A buffer a texture is copied into, kept between frames.
struct Stage {
    buffer: wgpu::Buffer,
    pixel_bytes: u32,
    /// Bytes per row in the buffer: rows are padded to wgpu's alignment.
    stride: u32,
}

/// The staging buffers of one image size. Making them anew for every
/// frame cost as much as a quarter of a frame (25 to 40 ms for a
/// 1020 × 777 image on an Intel iGPU), so a renderer keeps one set and
/// uses it again while the image size stays. They are free again once
/// [`Pending::finish`] has unmapped them; a second readback started
/// before that is a validation error, not a wrong picture.
pub(crate) struct Staging {
    width: u32,
    height: u32,
    color: Stage,
    /// `None` for a renderer that reads only the picture back.
    data: Option<Stage>,
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
    data: Option<Copy>,
}

/// The staging buffers for an image of `width` × `height`, with a buffer
/// for the data target when `with_data`.
fn staging(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    with_data: bool,
) -> Result<Staging, GpuError> {
    let too_large = || GpuError::Readback(format!("a {width}x{height} image is too large"));
    let stage = |pixel_bytes: u32| {
        let stride = width
            .checked_mul(pixel_bytes)
            .and_then(|row| row.checked_next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
            .ok_or_else(too_large)?;
        let size = u64::from(stride) * u64::from(height);
        if size > device.limits().max_buffer_size {
            return Err(too_large());
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("craft3d readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Stage {
            buffer,
            pixel_bytes,
            stride,
        })
    };
    Ok(Staging {
        width,
        height,
        color: stage(COLOR_BYTES)?,
        data: if with_data {
            Some(stage(DATA_BYTES)?)
        } else {
            None
        },
    })
}

/// Copies the colour texture, and the data texture when there is one,
/// `width` × `height` each, into the staging buffers (made, or made again
/// for a new image size, in `staging`) and asks for them to be mapped.
pub(crate) fn start(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (color_texture, data_texture): (&wgpu::Texture, Option<&wgpu::Texture>),
    (width, height): (u32, u32),
    staging: &mut Option<Staging>,
) -> Result<Pending, GpuError> {
    let with_data = data_texture.is_some();
    if staging
        .as_ref()
        .is_some_and(|s| (s.width, s.height) != (width, height) || s.data.is_some() != with_data)
    {
        // The old buffers go before the new ones are made.
        *staging = None;
    }
    let staging = match staging {
        Some(staging) => staging,
        None => staging.insert(self::staging(device, width, height, with_data)?),
    };
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("craft3d readback"),
    });
    let mut copy = |texture: &wgpu::Texture, stage: &Stage| {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &stage.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stage.stride),
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
            buffer: stage.buffer.clone(),
            pixel_bytes: stage.pixel_bytes,
            stride: stage.stride,
            state: Arc::new(Mutex::new(MapState::default())),
        }
    };
    let color = copy(color_texture, &staging.color);
    let data = match (data_texture, &staging.data) {
        (Some(texture), Some(stage)) => Some(copy(texture, stage)),
        _ => None,
    };
    queue.submit([encoder.finish()]);
    for copy in [Some(&color), data.as_ref()].into_iter().flatten() {
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
    /// `each` one pixel at a time. The buffer is unmapped afterwards,
    /// whatever was found in it: the next frame uses it again.
    fn pixels(&self, width: u32, height: u32, each: impl FnMut(&[u8])) -> Result<(), GpuError> {
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
        let read = self.rows(width, height, each);
        self.buffer.unmap();
        read
    }

    fn rows(&self, width: u32, height: u32, mut each: impl FnMut(&[u8])) -> Result<(), GpuError> {
        // One copy out of the mapping first: mapped GPU memory is often
        // uncached, and very slow to read a few bytes at a time. (Even the
        // copy runs at a few hundred MB/s on an Intel iGPU, which bounds
        // what a readback can do for a viewport.)
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
        Ok(())
    }
}

impl Pending {
    /// Waits until the buffers are mapped (or have failed to).
    pub(crate) async fn mapped(&self) {
        for copy in [Some(&self.color), self.data.as_ref()]
            .into_iter()
            .flatten()
        {
            Mapped(&copy.state).await;
        }
    }

    /// The picture from the mapped colour buffer, 3 bytes per pixel. The
    /// data buffer, if there is one, is unmapped unread.
    pub(crate) fn finish_rgb(self) -> Result<Vec<u8>, GpuError> {
        let (width, height) = (self.width, self.height);
        let pixels = (width as usize).saturating_mul(height as usize);
        let mut rgb = Vec::with_capacity(pixels.saturating_mul(3));
        self.color.pixels(width, height, |pixel| {
            if let [r, g, b, _] = *pixel {
                rgb.extend([r, g, b]);
            }
        })?;
        if let Some(data) = &self.data {
            data.pixels(width, height, |_| {})?;
        }
        if rgb.len() != pixels.saturating_mul(3) {
            return Err(GpuError::Readback(
                "the mapped buffer does not hold the whole image".to_string(),
            ));
        }
        Ok(rgb)
    }

    /// Builds the frame from the mapped buffers.
    pub(crate) fn finish(self) -> Result<Frame, GpuError> {
        let (width, height) = (self.width, self.height);
        let Some(data) = &self.data else {
            return Err(GpuError::NothingDrawn);
        };
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
        data.pixels(width, height, |pixel| {
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
