//! The 3DCraft desktop app: a native window (eframe on wgpu) around the
//! egui shell, and the control channel through which scripts and agents
//! drive the very session the window shows (docs/control-protocol.md).
//!
//! Phase 1 is a viewer: the window draws the active document and edits
//! nothing. Every change is a command, sent over the control channel.
//!
//! - [`args`]: the command line and its environment variables.
//! - [`token`]: the control token and its file.
//! - [`control`]: the TCP listener; authentication, limits, one thread per
//!   connection, nothing dispatched on them.
//! - [`host`]: the session and the shell, and answering a request line on
//!   the thread that owns them.
//! - [`app`]: the eframe window.
//!
//! Everything but [`app::run`] works without a window, which is how the
//! tests drive the control channel.

pub mod app;
pub mod args;
pub mod control;
pub mod host;
pub mod token;

#[cfg(test)]
mod tests;
