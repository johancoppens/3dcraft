//! The 3DCraft desktop app: a native window (eframe on wgpu) around the
//! egui shell, and the control channel through which scripts and agents
//! drive the very session the window shows (docs/control-protocol.md).
//!
//! Phase 1 is a viewer: the window draws the active document and does
//! not model. Every change is a command, sent over the control channel;
//! the window itself only opens and saves files.
//!
//! - [`args`]: the command line and its environment variables.
//! - [`token`]: the control token and its file.
//! - [`control`]: the TCP listener; authentication, limits, one thread per
//!   connection, nothing dispatched on them.
//! - [`host`]: the session and the shell, and answering a request line on
//!   the thread that owns them.
//! - [`files`]: New, Open, Save and Save As for the person at the window,
//!   with the system's file dialog.
//! - [`worker`]: the engine thread, where commands run while the window
//!   keeps drawing.
//! - [`app`]: the eframe window.
//!
//! Everything but [`app::run`] works without a window, which is how the
//! tests drive the control channel.

pub mod app;
pub mod args;
pub mod control;
pub mod files;
pub mod host;
pub mod token;
pub mod worker;

#[cfg(test)]
mod tests;
