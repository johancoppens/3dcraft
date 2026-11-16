//! The window's own handler for device errors (docs/design/rendering.md ›
//! *A device error never ends the window*). wgpu hands an error that no
//! error scope catches to the device's handler, and its default handler
//! panics: on a slow card drawing through OpenGL, egui's painter
//! configures the surface after a resize, wgpu first waits for the GPU to
//! come idle, OpenGL's wait gives up after about two seconds while a studio
//! frame is still being drawn, and the window ended with exit code 101.
//! Here every error is counted and said instead, and [`Recovery`]
//! configures the surface again once the device is idle, with a growing
//! pause between tries, so an error that repeats never becomes a loop.
//! What the shell does with an error (the studio look gives way, a bar
//! says so) is `Shell::device_errors`.

use craft3d_gpu::diag;
use eframe::wgpu;
use std::cell::OnceCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::WindowId;

/// The environment variable that injects a device error, for the test of
/// this handler (`apps/craft3d/tests/device_error.rs`): with `configure`,
/// once the window has drawn the studio look, it raises a validation error
/// on the window's device while it hands a resize to egui's painter, where
/// the painter's `Surface::configure` raised its error on the slow card.
/// Unset, empty or anything else: no fault.
pub const FAULT_VAR: &str = "CRAFT3D_GPU_FAULT";

/// How many errors are said in full on standard error and in the trace;
/// after that every hundredth.
const SAID: u64 = 20;

/// The longest pause between two tries to configure the surface again.
const LONGEST_PAUSE: Duration = Duration::from_secs(60);

/// How long one try waits for the device to come idle.
const IDLE_WAIT: Duration = Duration::from_secs(1);

/// The device errors of the window's device that no error scope caught.
#[derive(Default)]
pub struct DeviceErrors {
    count: Arc<AtomicU64>,
    last: Mutex<String>,
}

impl DeviceErrors {
    /// The counter, shared with the GPU tour (`tour.rs`).
    pub fn counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.count)
    }

    /// How many errors were taken so far.
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// The last error's text, on one line, at most 2000 characters.
    pub fn last(&self) -> String {
        self.last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Takes `error` in place of wgpu's default handler, which panics.
    pub fn take(&self, error: &wgpu::Error) {
        let count = self.count.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        let text: String = error
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(2000)
            .collect();
        if count <= SAID || count.is_multiple_of(100) {
            let line = format!(
                "device error {count}, taken by the window's handler (the program carries on): {text}"
            );
            diag::say(&line);
            diag::note("device", &line);
        }
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = text;
    }
}

/// Installs the window's handler on `device`: every error that no error
/// scope catches is taken by `errors` ([`DeviceErrors::take`]).
pub fn install(device: &wgpu::Device, errors: Arc<DeviceErrors>) {
    device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| errors.take(&error)));
}

/// Raises one validation error on `device`, outside any error scope: a
/// buffer without a usage. What [`FAULT_VAR`] injects.
pub fn raise_fault(device: &wgpu::Device) {
    let _ = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("craft3d injected fault (CRAFT3D_GPU_FAULT)"),
        size: 4,
        usage: wgpu::BufferUsages::empty(),
        mapped_at_creation: false,
    });
}

/// Whether `value` of [`FAULT_VAR`] asks for the fault.
pub fn fault_asked(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim().eq_ignore_ascii_case("configure"))
}

/// The surface's recovery, kept by the event loop's driver: the last size
/// the window was given, whether a configure of the surface failed, and
/// when to try again.
pub struct Recovery {
    errors: Arc<DeviceErrors>,
    /// The window's device, once the window exists.
    device: Rc<OnceCell<wgpu::Device>>,
    size: Option<(WindowId, PhysicalSize<u32>)>,
    /// A configure of the surface failed and is owed.
    owed: bool,
    tries: u32,
    next: Instant,
    /// [`FAULT_VAR`] asked for the fault and it was not raised yet.
    fault: bool,
    /// The fault is raised inside the next resize.
    raise_next: bool,
    /// How many resizes were handed to eframe.
    resizes: u64,
}

impl Recovery {
    pub fn new(errors: Arc<DeviceErrors>, device: Rc<OnceCell<wgpu::Device>>, fault: bool) -> Self {
        Self {
            errors,
            device,
            size: None,
            owed: false,
            tries: 0,
            next: Instant::now(),
            fault,
            raise_next: false,
            resizes: 0,
        }
    }

    /// Before `event` is handed to eframe: the size of a resize (which
    /// egui's painter configures the surface for), remembered, and the
    /// count of errors so far.
    pub fn before(
        &mut self,
        id: WindowId,
        event: &WindowEvent,
    ) -> (Option<PhysicalSize<u32>>, u64) {
        let size = match event {
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                self.size = Some((id, *size));
                self.resizes = self.resizes.saturating_add(1);
                if self.resizes <= SAID || self.resizes.is_multiple_of(100) {
                    diag::note(
                        "window",
                        &format!(
                            "resize {} to {}x{}: egui's painter configures the surface",
                            self.resizes, size.width, size.height
                        ),
                    );
                }
                Some(*size)
            }
            _ => None,
        };
        let before = self.errors.count();
        // The injected fault, inside the resize as the painter's was.
        if size.is_some()
            && std::mem::take(&mut self.raise_next)
            && let Some(device) = self.device.get()
        {
            raise_fault(device);
        }
        (size, before)
    }

    /// How many errors were taken so far.
    pub fn errors(&self) -> u64 {
        self.errors.count()
    }

    /// After a resize was handed to eframe: an error raised meanwhile means
    /// the surface may keep its old configuration, and is owed a configure.
    pub fn after(&mut self, resized: Option<PhysicalSize<u32>>, before: u64) {
        if resized.is_some() && self.errors.count() > before {
            if !self.owed {
                diag::note(
                    "device",
                    "the surface could not be configured for the window's size; it is configured \
                     again once the device is idle",
                );
            }
            self.owed = true;
        }
    }

    /// The fault of [`FAULT_VAR`], once `studio_presented`: the resize to
    /// hand to the driver's `window_event`, which raises the error inside
    /// it ([`Recovery::before`]), as egui's painter's configure raised it.
    pub fn fault_due(&mut self, studio_presented: bool) -> Option<(WindowId, PhysicalSize<u32>)> {
        if !self.fault || !studio_presented || self.device.get().is_none() {
            return None;
        }
        let size = self.size?;
        self.fault = false;
        self.raise_next = true;
        diag::note(
            "device",
            "CRAFT3D_GPU_FAULT: a device error is raised at a resize",
        );
        Some(size)
    }

    /// The resize to hand to eframe now so the painter configures the
    /// surface again: when one is owed, its pause is over and the device
    /// came idle within [`IDLE_WAIT`]. Each try doubles the pause, up to
    /// [`LONGEST_PAUSE`].
    pub fn due(&mut self, now: Instant) -> Option<(WindowId, PhysicalSize<u32>)> {
        if !self.owed || now < self.next {
            return None;
        }
        let size = self.size?;
        let device = self.device.get()?;
        self.tries = self.tries.saturating_add(1);
        let pause = Duration::from_secs(1u64 << self.tries.min(6)).min(LONGEST_PAUSE);
        self.next = now + pause;
        let idle = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(IDLE_WAIT),
        });
        match idle {
            Ok(_) => Some(size),
            Err(why) => {
                diag::note(
                    "device",
                    &format!(
                        "try {} to configure the surface again: the device is not idle ({why}); \
                         again in {} s",
                        self.tries,
                        pause.as_secs()
                    ),
                );
                None
            }
        }
    }

    /// After the owed resize was handed to eframe: done when no error came
    /// with it, else owed again after the pause.
    pub fn configured(&mut self, before: u64) {
        if self.errors.count() > before {
            diag::note(
                "device",
                &format!("try {} to configure the surface again failed", self.tries),
            );
        } else {
            diag::note(
                "device",
                &format!("the surface was configured again (try {})", self.tries),
            );
            self.owed = false;
            self.tries = 0;
        }
    }

    /// Wakes the event loop for the next try, no later than eframe asked.
    pub fn schedule(&self, event_loop: &ActiveEventLoop) {
        if !self.owed {
            return;
        }
        let flow = match event_loop.control_flow() {
            ControlFlow::Poll => ControlFlow::Poll,
            ControlFlow::Wait => ControlFlow::WaitUntil(self.next),
            ControlFlow::WaitUntil(when) => ControlFlow::WaitUntil(when.min(self.next)),
        };
        event_loop.set_control_flow(flow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fault_is_asked_by_its_word_only() {
        assert!(fault_asked(Some("configure")));
        assert!(fault_asked(Some(" Configure ")));
        assert!(!fault_asked(Some("")));
        assert!(!fault_asked(Some("1")));
        assert!(!fault_asked(None));
    }
}
