//! The intent an agent gives with a command (2026-11-02): optional, over
//! the JSON-lines protocol and over MCP, cleaned and capped, and kept in
//! the document's agent log beside the command.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_automation::{mcp, rpc};
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

fn agent() -> Session {
    let mut session = Session::new(Workspace::none());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    session.set_actor(Some("Claude Code"));
    session
}

fn reply(line: Option<String>) -> Value {
    serde_json::from_str(&line.unwrap()).unwrap()
}

fn entries(session: &Session) -> Vec<Value> {
    session.agent_log_json(50)["entries"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn engine_execute_and_batch_take_an_intent() {
    let mut session = agent();
    let got = reply(rpc::handle_line(
        &mut session,
        &json!({"id": 1, "method": "engine.execute", "params": {
            "command": "feature.block", "params": {"name": "Base", "size": [40, 30, 8]},
            "intent": "a base plate\nfor the bracket"}})
        .to_string(),
    ));
    assert_eq!(got["ok"], true, "{got}");
    // Without one, as before.
    let got = reply(rpc::handle_line(
        &mut session,
        &json!({"id": 2, "method": "engine.execute", "params": {"command": "variable.set",
            "params": {"name": "w", "expr": "5 mm"}}})
        .to_string(),
    ));
    assert_eq!(got["ok"], true, "{got}");
    let got = reply(rpc::handle_line(
        &mut session,
        &json!({"id": 3, "method": "batch", "params": {"intent": "two holes", "steps": [
            {"command": "variable.set", "params": {"name": "d", "expr": "4 mm"}},
            {"command": "variable.set", "params": {"name": "e", "expr": "6 mm"}, "intent": "edge distance"}]}})
        .to_string(),
    ));
    assert_eq!(got["ok"], true, "{got}");
    let log = entries(&session);
    let intents: Vec<Value> = log.iter().map(|e| e["intent"].clone()).collect();
    assert_eq!(
        intents,
        [
            json!("edge distance"),
            json!("two holes"),
            Value::Null,
            json!("a base plate for the bracket")
        ]
    );
    assert_eq!(log[3]["created"], json!(["F1"]));
    // Not a string: an error naming the parameter, nothing run.
    let got = reply(rpc::handle_line(
        &mut session,
        &json!({"id": 4, "method": "engine.execute", "params": {"command": "file.new", "intent": 5}})
            .to_string(),
    ));
    assert_eq!(got["ok"], false);
    assert!(got["error"].as_str().unwrap().contains("intent"), "{got}");
    assert_eq!(entries(&session).len(), 4);
}

#[test]
fn mcp_tools_take_an_intent_capped_and_cleaned() {
    let mut session = agent();
    let call = |name: &str, arguments: Value| {
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
               "params": {"name": name, "arguments": arguments}})
        .to_string()
    };
    let long = format!("round the base\u{0} edges {}", "x".repeat(600));
    let got = reply(mcp::handle_line(
        &mut session,
        &call(
            "command_run",
            json!({"id": "feature.block", "params": {"size": [10, 10, 10]}, "intent": long}),
        ),
    ));
    assert_eq!(got["result"]["isError"], false, "{got}");
    let got = reply(mcp::handle_line(
        &mut session,
        &call(
            "command_batch",
            json!({"intent": "the posts", "steps": [
                {"id": "variable.set", "params": {"name": "h", "expr": "20 mm"}}]}),
        ),
    ));
    assert_eq!(got["result"]["isError"], false, "{got}");
    let log = entries(&session);
    assert_eq!(log[0]["intent"], "the posts");
    let intent = log[1]["intent"].as_str().unwrap();
    assert!(intent.starts_with("round the base edges x"), "{intent}");
    assert_eq!(
        intent.chars().count(),
        craft3d_engine::agent_log::MAX_INTENT_CHARS
    );
    assert!(!intent.chars().any(char::is_control));
    // The tools say they take it.
    let tools = reply(mcp::handle_line(
        &mut session,
        &json!({"jsonrpc": "2.0", "id": 8, "method": "tools/list"}).to_string(),
    ));
    let run = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "command_run")
        .unwrap()
        .clone();
    assert!(
        run["inputSchema"]["properties"]["intent"]["description"]
            .as_str()
            .unwrap()
            .contains("agent log")
    );
}
