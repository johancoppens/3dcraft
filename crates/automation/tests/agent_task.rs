//! A scripted agent task, end to end over MCP (docs/testing.md, test kinds:
//! `automation`). Everything goes through `mcp::handle_line` as lines of
//! JSON-RPC, the way an agent client sends them: no call into the engine,
//! and nothing the tool descriptions do not tell an agent.
//!
//! The task: build a plate with a pin from variables, check the volume,
//! change a variable, check the volume followed, export, look at a picture.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_automation::mcp;
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

/// The client side of the conversation.
struct Agent {
    session: Session,
    next_id: u64,
}

impl Agent {
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        let reply = mcp::handle_line(&mut self.session, &line).expect("a request gets a reply");
        let reply: Value = serde_json::from_str(&reply).expect("the reply is JSON");
        assert_eq!(reply["jsonrpc"], "2.0");
        assert_eq!(reply["id"], id, "the reply answers this request");
        assert!(reply.get("error").is_none(), "{method} {params}: {reply}");
        reply["result"].clone()
    }

    fn notify(&mut self, method: &str) {
        let line = json!({"jsonrpc": "2.0", "method": method}).to_string();
        assert_eq!(mcp::handle_line(&mut self.session, &line), None);
    }

    /// Calls a tool and returns its raw result.
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }

    /// Calls a tool that must succeed and returns the JSON in its text block.
    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.call(name, arguments.clone());
        assert_eq!(result["isError"], false, "{name} {arguments}: {result}");
        serde_json::from_str(result["content"][0]["text"].as_str().expect("a text block"))
            .expect("the text is JSON")
    }

    fn run(&mut self, id: &str, params: Value) -> Value {
        self.tool("command_run", json!({"id": id, "params": params}))
    }

    fn body<'a>(inspect: &'a Value, name: &str) -> &'a Value {
        inspect["bodies"]
            .as_array()
            .expect("bodies")
            .iter()
            .find(|body| body["name"] == name)
            .unwrap_or_else(|| panic!("no body named {name} in {inspect}"))
    }
}

/// Volumes come from integrating over the faces in floating point: they are
/// compared with a relative tolerance of 1e-9, as in the engine's tests.
fn assert_volume(body: &Value, expected: f64) {
    let actual = body["volume"].as_f64().expect("a volume");
    println!("{}: volume {actual}, expected {expected}", body["name"]);
    assert!(
        (actual - expected).abs() <= 1e-9 * expected,
        "{}: volume {actual}, expected {expected}",
        body["name"]
    );
}

#[test]
fn an_agent_builds_measures_edits_exports_and_previews_a_part() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("agent-task");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("out")).unwrap();
    // The agent may write beneath `out` and read nothing.
    let write_root = root.join("out");
    let mut agent = Agent {
        session: Session::new(Workspace {
            read: Access::None,
            write: Access::Root(write_root.clone()),
        }),
        next_id: 0,
    };

    // 1. Handshake.
    let hello = agent.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "agent-task-test", "version": "1"},
        }),
    );
    assert_eq!(hello["protocolVersion"], "2025-06-18");
    assert_eq!(hello["serverInfo"]["name"], "craft3d");
    assert!(
        hello["instructions"]
            .as_str()
            .unwrap()
            .contains("command_list")
    );
    agent.notify("notifications/initialized");

    // 2. Discover the tools, then the commands.
    let tools = agent.request("tools/list", json!({}));
    let tool_names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for needed in [
        "command_list",
        "command_run",
        "doc_inspect",
        "doc_render_preview",
    ] {
        assert!(tool_names.contains(&needed), "{needed} in {tool_names:?}");
    }
    let features = agent.tool("command_list", json!({"filter": "feature."}));
    let features = features["commands"].as_array().unwrap();
    assert!(!features.is_empty());
    assert!(
        features
            .iter()
            .all(|c| c["id"].as_str().unwrap().starts_with("feature."))
    );
    let block = features
        .iter()
        .find(|c| c["id"] == "feature.block")
        .expect("feature.block");
    // The params notation tells the agent what to send.
    assert!(block["params"].as_str().unwrap().contains("\"size\""));
    // Nothing is open yet, and the list says what to do about it.
    assert_eq!(block["enabled"], false);
    assert!(
        block["disabledReason"]
            .as_str()
            .unwrap()
            .contains("file.new")
    );
    assert_eq!(
        agent.tool("doc_inspect", json!({})),
        json!({"document": null})
    );

    // 3. Model: variables, a block driven by them, a cylinder.
    agent.run("file.new", json!({"units": "mm"}));
    let width = agent.run("variable.set", json!({"name": "width", "expr": "40 mm"}));
    assert_eq!(width["value"], 40.0);
    agent.run("variable.set", json!({"name": "thickness", "expr": "5 mm"}));
    let plate = agent.run(
        "feature.block",
        json!({"name": "Plate", "size": ["width", "width / 2", "thickness"]}),
    );
    assert_eq!(plate["status"], "ok");
    let pin = agent.run(
        "feature.cylinder",
        json!({"name": "Pin", "radius": "4 mm", "height": "20 mm", "position": ["width / 2", "width / 4", "thickness"]}),
    );
    assert_eq!(pin["status"], "ok");

    // 4. Verify without a picture: 40 x 20 x 5.
    let inspect = agent.tool("doc_inspect", json!({}));
    assert_eq!(inspect["ok"], true, "every feature regenerated: {inspect}");
    assert_eq!(inspect["features"].as_array().unwrap().len(), 2);
    assert_eq!(inspect["bodies"].as_array().unwrap().len(), 2);
    let plate = Agent::body(&inspect, "Plate");
    assert_volume(plate, 4000.0);
    assert_eq!(
        (
            plate["faces"].as_u64(),
            plate["edges"].as_u64(),
            plate["vertices"].as_u64()
        ),
        (Some(6), Some(12), Some(8))
    );
    // The pin: pi r^2 h.
    assert_volume(
        Agent::body(&inspect, "Pin"),
        std::f64::consts::PI * 16.0 * 20.0,
    );

    // 5. Change the driving variable; the plate follows: 60 x 30 x 5.
    let width = agent.run("variable.set", json!({"name": "width", "expr": "60 mm"}));
    assert_eq!(width["value"], 60.0);
    let inspect = agent.tool("doc_inspect", json!({}));
    assert_eq!(inspect["ok"], true, "{inspect}");
    assert_volume(Agent::body(&inspect, "Plate"), 9000.0);
    // The pin's size does not depend on `width`; its position does.
    let pin = Agent::body(&inspect, "Pin");
    assert_volume(pin, std::f64::consts::PI * 16.0 * 20.0);
    let centroid: Vec<f64> = pin["centroid"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_f64().unwrap())
        .collect();
    for (actual, expected) in centroid.iter().zip([30.0, 15.0, 15.0]) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "pin centroid {centroid:?}"
        );
    }

    // 6. Export to 3MF beneath the write root.
    let export = agent.run("file.export", json!({"path": "part.3mf"}));
    assert_eq!(export["bodies"], 2);
    let file = write_root.join("part.3mf");
    let bytes = std::fs::read(&file).expect("the exported file exists");
    assert!(bytes.starts_with(b"PK"), "a 3MF file is a zip container");
    assert_eq!(export["bytes"].as_u64(), Some(bytes.len() as u64));

    // 7. Look at it.
    let preview = agent.call(
        "doc_render_preview",
        json!({"view": "iso", "max_side": 320}),
    );
    assert_eq!(preview["isError"], false, "{preview}");
    let image = &preview["content"][0];
    assert_eq!(image["type"], "image");
    assert_eq!(image["mimeType"], "image/png");
    assert!(
        image["data"].as_str().unwrap().starts_with("iVBORw0KGgo"),
        "base64 of the PNG signature"
    );
    let rest: Value =
        serde_json::from_str(preview["content"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(rest["bodies"].as_array().unwrap().len(), 2);
    assert!(rest["coverage"].as_f64().unwrap() > 0.0);
    assert!(
        rest["triangles"].as_u64().unwrap() > 12,
        "a block and a cylinder"
    );

    // 8. Try to leave the root: refused as a tool error the agent can read,
    //    and nothing is written.
    let outside = root.join("escaped.3mf");
    for path in [
        "../escaped.3mf".to_string(),
        outside.to_string_lossy().into_owned(),
    ] {
        let refused = agent.call(
            "command_run",
            json!({"id": "file.export", "params": {"path": path}}),
        );
        assert_eq!(refused["isError"], true, "{path}: {refused}");
        let message = refused["content"][0]["text"].as_str().unwrap();
        assert!(message.contains("write root"), "{message}");
        assert!(!outside.exists(), "{path} must not be written");
    }
    // Reading was never granted.
    let refused = agent.call(
        "command_run",
        json!({"id": "file.open", "params": {"path": "part.3dcraft"}}),
    );
    assert_eq!(refused["isError"], true);
    assert!(
        refused["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no read access")
    );

    // The session is intact after the refusals.
    let inspect = agent.tool("doc_inspect", json!({}));
    assert_volume(Agent::body(&inspect, "Plate"), 9000.0);
}
