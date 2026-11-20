//! The MCP bundle of the release (`packaging/mcpb/manifest.json`,
//! docs/development.md, Packaging › *The MCP bundle*) starts this server
//! headless, `craft3d-cli mcp` with a work folder, and lists its tools for
//! a client to show before it starts it: the list must name exactly the
//! tools the headless server gives.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_automation::{MAX_BATCH_STEPS, mcp};
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

#[test]
fn the_bundle_lists_the_tools_the_headless_server_gives() {
    let mut session = Session::new(Workspace::none());
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}});
    let reply = mcp::handle_line(&mut session, &line.to_string()).unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    let mut served: Vec<String> = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packaging/mcpb/manifest.json"
    );
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut listed: Vec<String> = manifest["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            assert!(tool["description"].as_str().is_some_and(|d| !d.is_empty()));
            tool["name"].as_str().unwrap().to_string()
        })
        .collect();
    served.sort();
    listed.sort();
    assert_eq!(
        listed, served,
        "packaging/mcpb/manifest.json lists other tools than `craft3d-cli mcp` gives"
    );
    // The window's own tool is not the headless server's.
    assert!(!listed.iter().any(|name| name == mcp::WINDOW_TOOL));
    assert_ne!(manifest["tools_generated"], true);
}

/// What the manifest says of a batch is the server's own limit: the
/// number of steps `command_batch` takes, `MAX_BATCH_STEPS`, which its
/// schema also gives (`maxItems`).
#[test]
fn the_bundle_says_the_batch_limit_the_server_keeps() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packaging/mcpb/manifest.json"
    );
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let batch = manifest["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "command_batch")
        .unwrap();
    let said = batch["description"].as_str().unwrap();
    assert!(
        said.contains(&format!("up to {MAX_BATCH_STEPS} commands")),
        "packaging/mcpb/manifest.json says of command_batch `{said}`; the server takes up to \
         {MAX_BATCH_STEPS} steps"
    );
    // The number the manifest names is the one the server enforces and
    // gives in its own schema.
    let mut session = Session::new(Workspace::none());
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}});
    let reply: Value =
        serde_json::from_str(&mcp::handle_line(&mut session, &line.to_string()).unwrap()).unwrap();
    let served = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "command_batch")
        .unwrap();
    assert_eq!(
        served["inputSchema"]["properties"]["steps"]["maxItems"],
        MAX_BATCH_STEPS
    );
}
