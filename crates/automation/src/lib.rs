//! Automation: two line-based servers over one engine [`Session`], for
//! scripts and AI agents (docs/control-protocol.md).
//!
//! - [`rpc`]: the JSON-lines protocol of `craft3d-cli serve`. One request
//!   per line, one reply per line.
//! - [`mcp`]: the Model Context Protocol over stdio (JSON-RPC 2.0, one
//!   message per line), as `craft3d-cli mcp` serves it.
//!
//! Each is a pure function from one input line to at most one reply line
//! (`handle_line`), plus a loop over a reader and a writer (`serve`), so
//! tests drive the protocols without spawning a process.
//!
//! Neither server adds behaviour of its own: everything they do is a command
//! in the engine's registry (AGENTS.md rule 1), and file access is whatever
//! the session's `Workspace` allows. Input is hostile: no line, however
//! malformed, makes a handler panic or stops a server.
//!
//! Deviations from docs/control-protocol.md, for now: there are no
//! background jobs (`wait`, `jobs.*`), no `session.*`/`doc.*` methods and
//! tools (the `file.*` commands do the same through `engine.execute` and
//! `command_run`); the one MCP resource is the selection.
//!
//! [`bridge`] is the MCP server without a session: it passes every message
//! to a running desktop app, which answers it on the document in its
//! window.
//!
//! [`Session`]: craft3d_engine::Session

#[cfg(not(target_arch = "wasm32"))]
pub mod bridge;
mod fields;
mod lines;
pub mod mcp;
mod registry;
pub mod rpc;

/// Reading one request line with a size cap, for a host that serves the
/// protocol on a transport of its own (the desktop app's TCP listener).
pub use lines::{Line, read_line};

/// Longest request line a server reads, in bytes. A longer line is skipped
/// up to its newline and answered with an error.
pub const MAX_REQUEST_BYTES: usize = 1 << 20;

/// Longest reply line a server writes, in bytes. A larger reply is replaced
/// by an error reply that says so.
pub const MAX_REPLY_BYTES: usize = 8 << 20;

/// Most steps in one batch.
pub const MAX_BATCH_STEPS: usize = 256;
