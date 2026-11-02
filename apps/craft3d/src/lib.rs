//! The 3DCraft desktop app: a native window (eframe on wgpu) around the
//! egui shell, and the control channel through which scripts and agents
//! drive the very session the window shows (docs/control-protocol.md).
//!
//! Phase 1 is a viewer: the window draws the active document and does
//! not model. Every change is a command, sent over the control channel;
//! the window itself only opens, saves, switches and closes documents.
//!
//! - [`args`]: the command line and its environment variables.
//! - [`token`]: the control token and its file.
//! - [`agents`]: the switch Allow AI agents: the listener turned on and
//!   off while the window runs, the discovery entry, the remembered choice.
//! - [`control`]: the TCP listener; authentication, limits, one thread per
//!   connection, nothing dispatched on them.
//! - [`host`]: the session and the shell, and answering a request line on
//!   the thread that owns them.
//! - [`files`]: New, Open, Save and Save As for the person at the window,
//!   with the system's file dialog.
//! - [`tabs`]: what the tabs of the open documents ask for, done with
//!   `file.activate`, `file.close` and `file.save`.
//! - [`worker`]: the engine thread, where commands run while the window
//!   keeps drawing.
//! - [`places`]: the recent files on disk, and the pages and folders the
//!   Help menu opens with the platform's opener.
//! - [`refine`]: the display scene built on a thread of its own while the
//!   window shows the quick first picture.
//! - [`app`]: the eframe window.
//!
//! Everything but [`app::run`] works without a window, which is how the
//! tests drive the control channel.

pub mod agents;
pub mod app;
pub mod args;
pub mod consent;
pub mod control;
pub mod files;
pub mod host;
pub mod places;
pub mod recording;
pub mod refine;
pub mod tabs;
pub mod token;
pub mod worker;

#[cfg(test)]
mod tests;
