//! The MCP server, driven through `mcp::handle_line` and `mcp::serve`
//! exactly as `craft3d-cli mcp` drives them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_automation::{MAX_BATCH_STEPS, MAX_REQUEST_BYTES, mcp};
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

fn session() -> Session {
    Session::new(Workspace::none())
}

/// Sends one line and parses the one reply line.
fn send_line(session: &mut Session, line: &str) -> Value {
    let reply = mcp::handle_line(session, line).unwrap_or_else(|| panic!("no reply to {line}"));
    assert!(!reply.contains('\n'), "a reply is one line: {reply}");
    let reply: Value =
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("reply is not JSON ({e}): {reply}"));
    assert_eq!(reply["jsonrpc"], "2.0", "{reply}");
    assert!(
        reply.get("result").is_some() != reply.get("error").is_some(),
        "a response has a result or an error, never both: {reply}"
    );
    reply
}

fn request(session: &mut Session, method: &str, params: Value) -> Value {
    send_line(
        session,
        &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string(),
    )
}

/// A request that must succeed; returns its result.
fn result(session: &mut Session, method: &str, params: Value) -> Value {
    let reply = request(session, method, params.clone());
    assert_eq!(reply["id"], 1);
    assert!(reply.get("error").is_none(), "{method} {params}: {reply}");
    reply["result"].clone()
}

/// A request that must be refused; returns (code, message).
fn rpc_error(session: &mut Session, method: &str, params: Value) -> (i64, String) {
    let reply = request(session, method, params.clone());
    assert_eq!(reply["id"], 1);
    let error = reply
        .get("error")
        .unwrap_or_else(|| panic!("{method} {params} should be refused: {reply}"));
    (
        error["code"].as_i64().expect("code is an integer"),
        error["message"]
            .as_str()
            .expect("message is a string")
            .to_string(),
    )
}

/// Calls a tool; returns the tool result (`content`, `isError`).
fn tool(session: &mut Session, name: &str, arguments: Value) -> Value {
    let result = result(
        session,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    );
    assert!(result["isError"].is_boolean(), "{result}");
    for block in result["content"].as_array().expect("content is an array") {
        assert!(
            matches!(block["type"].as_str(), Some("text" | "image")),
            "{block}"
        );
    }
    result
}

/// Calls a tool that must succeed with one text block of JSON.
fn tool_json(session: &mut Session, name: &str, arguments: Value) -> Value {
    let result = tool(session, name, arguments.clone());
    assert_eq!(result["isError"], false, "{name} {arguments}: {result}");
    let content = result["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    serde_json::from_str(content[0]["text"].as_str().unwrap()).expect("the text is JSON")
}

/// Calls a tool that must fail in execution; returns the error text.
fn tool_error(session: &mut Session, name: &str, arguments: Value) -> String {
    let result = tool(session, name, arguments.clone());
    assert_eq!(
        result["isError"], true,
        "{name} {arguments} should fail: {result}"
    );
    result["content"][0]["text"].as_str().unwrap().to_string()
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    tool_json(session, "command_run", json!({"id": id, "params": params}))
}

fn assert_still_serving(session: &mut Session) {
    let reply = send_line(
        session,
        r#"{"jsonrpc": "2.0", "id": "alive", "method": "ping"}"#,
    );
    assert_eq!(
        reply,
        json!({"jsonrpc": "2.0", "id": "alive", "result": {}})
    );
}

/// See `tests/rpc.rs`: volumes are compared with a relative tolerance of
/// 1e-9 because mass properties are integrated in floating point.
fn close(actual: &Value, expected: f64) -> bool {
    let actual = actual.as_f64().expect("a number");
    (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0)
}

/// Decodes standard base64 with padding: an implementation independent of
/// the engine's encoder, so the image block is checked by another route.
fn base64_decode(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    assert_eq!(text.len() % 4, 0, "padded base64 comes in groups of four");
    let mut out = Vec::new();
    for group in text.as_bytes().chunks(4) {
        let mut bits = 0_u32;
        let mut padding = 0;
        for &symbol in group {
            bits <<= 6;
            if symbol == b'=' {
                padding += 1;
            } else {
                assert_eq!(padding, 0, "padding only at the end");
                bits |= ALPHABET
                    .iter()
                    .position(|&a| a == symbol)
                    .expect("a base64 symbol") as u32;
            }
        }
        let bytes = [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8];
        out.extend_from_slice(&bytes[..3 - padding]);
    }
    out
}

/// Width and height from the IHDR chunk of a PNG.
fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "the PNG signature");
    assert_eq!(&png[12..16], b"IHDR");
    let be = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
    (be(16), be(20))
}

// -------------------------------------------------------------- lifecycle

#[test]
fn initialize_describes_the_server() {
    let mut s = session();
    let result = result(
        &mut s,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"roots": {"listChanged": true}},
            "clientInfo": {"name": "test-client", "version": "1.0"},
        }),
    );
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(
        result["capabilities"],
        json!({"tools": {}, "resources": {}})
    );
    assert_eq!(result["serverInfo"]["name"], "craft3d");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    let instructions = result["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("command_list") && instructions.contains("command_run"),
        "{instructions}"
    );
    assert!(
        instructions.len() < 1000,
        "short enough to sit in every context"
    );
}

#[test]
fn initialize_echoes_a_supported_version_and_offers_the_latest_otherwise() {
    let mut s = session();
    let latest = mcp::PROTOCOL_VERSIONS[0];
    for version in mcp::PROTOCOL_VERSIONS {
        let result = result(&mut s, "initialize", json!({"protocolVersion": version}));
        assert_eq!(result["protocolVersion"], *version);
    }
    for unknown in ["1999-01-01", "2099-12-31", "", "latest"] {
        let result = result(&mut s, "initialize", json!({"protocolVersion": unknown}));
        assert_eq!(result["protocolVersion"], latest, "{unknown}");
    }
    assert_eq!(
        result(&mut s, "initialize", json!({}))["protocolVersion"],
        latest
    );
    // No params at all.
    let reply = send_line(
        &mut s,
        r#"{"jsonrpc": "2.0", "id": 1, "method": "initialize"}"#,
    );
    assert_eq!(reply["result"]["protocolVersion"], latest);

    let (code, message) = rpc_error(&mut s, "initialize", json!({"protocolVersion": 20250618}));
    assert_eq!(code, -32602);
    assert!(message.contains("protocolVersion"), "{message}");
}

#[test]
fn notifications_and_responses_get_no_reply() {
    let mut s = session();
    for line in [
        r#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#,
        r#"{"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}"#,
        r#"{"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 3, "reason": "x"}}"#,
        r#"{"jsonrpc": "2.0", "method": "notifications/roots/list_changed"}"#,
        // Unknown, and with params no method would accept: still no reply.
        r#"{"jsonrpc": "2.0", "method": "notifications/made_up", "params": [1, 2]}"#,
        r#"{"jsonrpc": "2.0", "method": "no/such/method"}"#,
        // Responses to requests this server never sent.
        r#"{"jsonrpc": "2.0", "id": 9, "result": {}}"#,
        r#"{"jsonrpc": "2.0", "id": "x", "error": {"code": -32601, "message": "no"}}"#,
        "",
        "   ",
    ] {
        assert_eq!(mcp::handle_line(&mut s, line), None, "{line}");
    }
    // A request sent as a notification is not run: nobody would see the
    // result or the error.
    let line = r#"{"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "command_run", "arguments": {"id": "file.new"}}}"#;
    assert_eq!(mcp::handle_line(&mut s, line), None);
    assert!(!s.has_document());
    assert_still_serving(&mut s);
}

#[test]
fn ping_returns_an_empty_result_for_any_id_type() {
    let mut s = session();
    for id in [json!(0), json!(-4), json!("abc"), json!(""), json!(1.5)] {
        let reply = send_line(
            &mut s,
            &json!({"jsonrpc": "2.0", "id": id, "method": "ping"}).to_string(),
        );
        assert_eq!(reply, json!({"jsonrpc": "2.0", "id": id, "result": {}}));
    }
    assert_eq!(result(&mut s, "ping", json!({})), json!({}));
}

// ------------------------------------------------------------------ tools

#[test]
fn tools_list_describes_six_tools_with_closed_schemas() {
    let mut s = session();
    let tools = result(&mut s, "tools/list", json!({}))["tools"].clone();
    let tools = tools.as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "command_list",
            "command_run",
            "command_batch",
            "doc_inspect",
            "selection_get",
            "doc_render_preview"
        ]
    );
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "tool names use underscores: {name}"
        );
        assert!(tool["description"].as_str().unwrap().len() > 40, "{name}");
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object", "{name}");
        assert_eq!(schema["additionalProperties"], false, "{name}");
        assert!(schema["properties"].is_object(), "{name}");
        // Every required key is a declared property.
        for key in schema["required"].as_array().into_iter().flatten() {
            assert!(
                schema["properties"].get(key.as_str().unwrap()).is_some(),
                "{name}: {key}"
            );
        }
    }
    let by_name = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap();
    assert_eq!(
        by_name("command_run")["inputSchema"]["required"],
        json!(["id"])
    );
    assert_eq!(
        by_name("command_batch")["inputSchema"]["required"],
        json!(["steps"])
    );
    assert_eq!(
        by_name("command_batch")["inputSchema"]["properties"]["steps"]["maxItems"],
        256
    );
    let side = &by_name("doc_render_preview")["inputSchema"]["properties"]["max_side"];
    assert_eq!(side["type"], "integer");
    assert_eq!(side["maximum"], 2048);
    // A cursor is accepted: the list is a single page.
    assert_eq!(
        result(&mut s, "tools/list", json!({"cursor": "x"}))["tools"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    // The schemas agree with what the server enforces: every declared
    // property is accepted (or refused for its value, never as unknown).
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        for key in tool["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .keys()
        {
            let reply = request(
                &mut s,
                "tools/call",
                json!({"name": name, "arguments": {key: null}}),
            );
            let message = reply["error"]["message"].as_str().unwrap_or("");
            assert!(
                !message.contains("unknown argument"),
                "{name}.{key}: {reply}"
            );
        }
    }
}

#[test]
fn command_list_returns_the_registry_and_filters() {
    let mut s = session();
    let all = tool_json(&mut s, "command_list", json!({}));
    let all = all["commands"].as_array().unwrap();
    assert_eq!(all.len(), Session::commands().len());
    for (entry, spec) in all.iter().zip(Session::commands()) {
        assert_eq!(entry["id"], spec.id);
        assert_eq!(entry["params"], spec.params);
        assert_eq!(entry["summary"], spec.summary);
        assert_eq!(entry["needsDocument"], spec.needs_document);
        assert_eq!(entry["enabled"], !spec.needs_document);
    }

    let ids = |s: &mut Session, arguments: Value| -> Vec<String> {
        tool_json(s, "command_list", arguments)["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(&mut s, json!({"filter": "UNDO"})), ["edit.undo"]);
    // Nothing is open: only the commands that need no document can run.
    let without_document: Vec<&str> = Session::commands()
        .iter()
        .filter(|c| !c.needs_document)
        .map(|c| c.id)
        .collect();
    assert_eq!(
        without_document,
        [
            "file.new",
            "file.open",
            "file.list",
            "recovery.list",
            "recovery.open",
            "recovery.discard"
        ],
        "the fixture of this test"
    );
    assert_eq!(ids(&mut s, json!({"enabled_only": true})), without_document);
    assert_eq!(
        ids(&mut s, json!({"enabled_only": true, "filter": "open"})),
        ["file.open", "file.list", "recovery.open"]
    );
    assert_eq!(
        ids(&mut s, json!({"enabled_only": true, "filter": "feature"})),
        Vec::<String>::new()
    );
    assert_eq!(
        ids(&mut s, json!({"enabled_only": false})).len(),
        Session::commands().len()
    );

    run(&mut s, "file.new", json!({}));
    assert_eq!(
        ids(&mut s, json!({"enabled_only": true})).len(),
        Session::commands().len()
    );
    // `arguments` may be left out altogether.
    let result = result(&mut s, "tools/call", json!({"name": "command_list"}));
    assert_eq!(result["isError"], false);
}

#[test]
fn command_run_returns_the_result_as_json_text() {
    let mut s = session();
    assert_eq!(
        run(&mut s, "file.new", json!({"units": "cm"})),
        json!({"document": 0, "units": "cm"})
    );
    // `params` may be omitted.
    let inspect = tool_json(&mut s, "command_run", json!({"id": "document.inspect"}));
    assert_eq!(inspect["units"]["length"], "cm");
    let sphere = run(&mut s, "feature.sphere", json!({"radius": 3}));
    assert_eq!(sphere["status"], "ok");
    let measure = run(&mut s, "document.measure", json!({}));
    // Closed form: 4/3 pi r^3.
    assert!(
        close(&measure["volume"], 4.0 / 3.0 * std::f64::consts::PI * 27.0),
        "{measure}"
    );
}

#[test]
fn a_failing_command_is_a_result_with_is_error_not_a_protocol_error() {
    let mut s = session();
    let error = tool_error(
        &mut s,
        "command_run",
        json!({"id": "feature.blok", "params": {}}),
    );
    assert_eq!(
        error,
        "unknown command `feature.blok`; did you mean `feature.block`?"
    );
    let error = tool_error(
        &mut s,
        "command_run",
        json!({"id": "feature.block", "params": {"size": [1, 2, 3]}}),
    );
    assert!(error.contains("no document is open"), "{error}");
    run(&mut s, "file.new", json!({}));
    let error = tool_error(
        &mut s,
        "command_run",
        json!({"id": "feature.block", "params": {"sise": [1, 2, 3]}}),
    );
    assert!(
        error.contains("feature.block") && error.contains("sise"),
        "{error}"
    );
    let error = tool_error(&mut s, "command_run", json!({"id": "edit.undo"}));
    assert!(error.contains("nothing to undo"), "{error}");
    // The session and its document are still there.
    assert_eq!(
        run(&mut s, "feature.block", json!({"size": [1, 2, 3]}))["status"],
        "ok"
    );
}

#[test]
fn command_batch_runs_steps_and_flags_a_failed_batch() {
    let mut s = session();
    let report = tool_json(
        &mut s,
        "command_batch",
        json!({"steps": [
            {"id": "file.new"},
            {"id": "feature.block", "params": {"size": [2, 3, 4]}},
            {"id": "document.measure"},
        ]}),
    );
    assert_eq!(report["completed"], 3);
    assert_eq!(report["failed"], 0);
    assert!(
        close(&report["results"][2]["result"]["volume"], 24.0),
        "{report}"
    );

    // Stops at the first failure by default; the report is still the content.
    let steps = json!([
        {"id": "feature.sphere", "params": {"radius": 1}},
        {"id": "feature.sphere", "params": {}},
        {"id": "feature.sphere", "params": {"radius": 2}},
    ]);
    let failed = tool(&mut s, "command_batch", json!({"steps": steps}));
    assert_eq!(failed["isError"], true);
    let report: Value =
        serde_json::from_str(failed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["completed"], 1);
    assert_eq!(report["failed"], 1);
    assert_eq!(report["results"].as_array().unwrap().len(), 2);
    assert!(
        report["results"][1]["error"]
            .as_str()
            .unwrap()
            .contains("radius")
    );

    let failed = tool(
        &mut s,
        "command_batch",
        json!({"steps": steps, "stop_on_error": false}),
    );
    assert_eq!(failed["isError"], true);
    let report: Value =
        serde_json::from_str(failed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["completed"], 2);
    assert_eq!(report["failed"], 1);
    assert_eq!(report["results"].as_array().unwrap().len(), 3);

    // Not atomic: 1 block + 1 sphere + 2 spheres stayed applied.
    let inspect = tool_json(&mut s, "doc_inspect", json!({}));
    assert_eq!(inspect["features"].as_array().unwrap().len(), 4);
}

#[test]
fn doc_inspect_returns_the_document_or_null() {
    let mut s = session();
    assert_eq!(
        tool_json(&mut s, "doc_inspect", json!({})),
        json!({"document": null})
    );
    run(&mut s, "file.new", json!({}));
    run(&mut s, "variable.set", json!({"name": "r", "expr": "5 mm"}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "r", "height": "2 * r", "name": "Pin"}),
    );
    let inspect = tool_json(&mut s, "doc_inspect", json!({}));
    // The same JSON as the command.
    assert_eq!(inspect, run(&mut s, "document.inspect", json!({})));
    assert_eq!(inspect["document"], 0);
    assert_eq!(inspect["variables"][0]["name"], "r");
    assert_eq!(inspect["bodies"][0]["name"], "Pin");
    // Closed form: pi r^2 h.
    assert!(
        close(
            &inspect["bodies"][0]["volume"],
            std::f64::consts::PI * 25.0 * 10.0
        ),
        "{inspect}"
    );
    // Back to nothing open.
    run(&mut s, "file.close", json!({"discard": true}));
    assert_eq!(
        tool_json(&mut s, "doc_inspect", json!({})),
        json!({"document": null})
    );
}

#[test]
fn doc_render_preview_returns_a_png_image_block_and_the_rest_as_text() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(&mut s, "feature.block", json!({"size": [40, 20, 10]}));

    let preview = |s: &mut Session, arguments: Value| -> (Vec<u8>, Value) {
        let result = tool(s, "doc_render_preview", arguments);
        assert_eq!(result["isError"], false, "{result}");
        let content = result["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        let image = &content[0];
        assert_eq!(image["type"], "image");
        assert_eq!(image["mimeType"], "image/png");
        assert_eq!(image.as_object().unwrap().len(), 3);
        let data = image["data"].as_str().unwrap();
        assert!(
            data.starts_with("iVBORw0KGgo"),
            "base64 of the PNG signature"
        );
        assert_eq!(content[1]["type"], "text");
        let rest: Value = serde_json::from_str(content[1]["text"].as_str().unwrap()).unwrap();
        // The image is in the image block only.
        assert!(
            rest.get("base64").is_none() && rest.get("mime").is_none(),
            "{rest}"
        );
        (base64_decode(data), rest)
    };

    // Defaults: 1024 on the longest side, 4:3.
    let (png, rest) = preview(&mut s, json!({}));
    assert_eq!(png_size(&png), (1024, 768));
    assert_eq!(
        (rest["width"].as_u64(), rest["height"].as_u64()),
        (Some(1024), Some(768))
    );
    assert_eq!(rest["view"], "iso");
    // A block is 12 triangles; the body list names it.
    assert_eq!(rest["triangles"], 12);
    assert_eq!(rest["bodies"].as_array().unwrap().len(), 1);
    let coverage = rest["coverage"].as_f64().unwrap();
    assert!(
        coverage > 0.05 && coverage < 1.0,
        "the block is in the picture: {coverage}"
    );

    // `view`, `mode` and `max_side` reach the renderer.
    let (png, rest) = preview(
        &mut s,
        json!({"view": "top", "mode": "ids", "max_side": 64}),
    );
    assert_eq!(png_size(&png), (64, 48));
    assert_eq!(rest["view"], "top");
    let (shaded, _) = preview(&mut s, json!({"view": "top", "max_side": 64}));
    assert_ne!(png, shaded, "ids mode gives another picture than shaded");
    // The limits of `max_side`.
    assert_eq!(
        png_size(&preview(&mut s, json!({"max_side": 2048})).0),
        (2048, 1536)
    );
    assert_eq!(
        png_size(&preview(&mut s, json!({"max_side": 16})).0),
        (16, 16)
    );
}

#[test]
fn doc_render_preview_failures_are_tool_errors() {
    let mut s = session();
    let error = tool_error(&mut s, "doc_render_preview", json!({}));
    assert!(error.contains("no document is open"), "{error}");
    run(&mut s, "file.new", json!({}));
    let error = tool_error(&mut s, "doc_render_preview", json!({}));
    assert!(error.contains("no bodies"), "{error}");
    run(&mut s, "feature.sphere", json!({"radius": 1}));
    // Which views exist is the command's knowledge; its error lists them.
    let error = tool_error(
        &mut s,
        "doc_render_preview",
        json!({"view": "sideways", "max_side": 32}),
    );
    assert!(
        error.contains("sideways") && error.contains("iso"),
        "{error}"
    );
    let error = tool_error(
        &mut s,
        "doc_render_preview",
        json!({"mode": "xray", "max_side": 32}),
    );
    assert!(
        error.contains("xray") && error.contains("shaded"),
        "{error}"
    );
}

// ------------------------------------------------------------ error paths

#[test]
fn unknown_tools_and_bad_arguments_are_invalid_params_naming_the_key() {
    let mut s = session();
    for (params, expected) in [
        (
            json!({"name": "doc_export", "arguments": {}}),
            "unknown tool `doc_export`",
        ),
        (json!({"name": "", "arguments": {}}), "unknown tool"),
        (
            json!({"name": "command.run", "arguments": {}}),
            "unknown tool `command.run`",
        ),
        (json!({"arguments": {}}), "missing `name`"),
        (json!({}), "missing `name`"),
        (json!({"name": 7}), "`name` must be a string"),
        (
            json!({"name": "command_list", "arguments": []}),
            "command_list: arguments must be a JSON object, got an array",
        ),
        (
            json!({"name": "command_list", "arguments": "x"}),
            "command_list: arguments must be a JSON object, got a string",
        ),
        (
            json!({"name": "command_list", "arguments": {"filter": 1}}),
            "`filter` must be a string",
        ),
        (
            json!({"name": "command_list", "arguments": {"enabled_only": "yes"}}),
            "`enabled_only` must be true or false",
        ),
        (
            json!({"name": "command_list", "arguments": {"enabledOnly": true}}),
            "unknown argument `enabledOnly`",
        ),
        (
            json!({"name": "command_run", "arguments": {}}),
            "command_run: missing `id`",
        ),
        (json!({"name": "command_run"}), "command_run: missing `id`"),
        (
            json!({"name": "command_run", "arguments": {"id": 3}}),
            "`id` must be a string",
        ),
        (
            json!({"name": "command_run", "arguments": {"id": "file.new", "params": [1]}}),
            "`params` must be a JSON object, got an array",
        ),
        (
            json!({"name": "command_run", "arguments": {"id": "file.new", "params": "{}"}}),
            "`params` must be a JSON object, got a string",
        ),
        (
            json!({"name": "command_run", "arguments": {"id": "file.new", "wait": false}}),
            "unknown argument `wait`",
        ),
        (
            json!({"name": "command_run", "arguments": {"command": "file.new"}}),
            "unknown argument `command`",
        ),
        (
            json!({"name": "command_batch", "arguments": {}}),
            "command_batch: missing `steps`",
        ),
        (
            json!({"name": "command_batch", "arguments": {"steps": {"id": "file.new"}}}),
            "`steps` must be an array",
        ),
        (
            json!({"name": "command_batch", "arguments": {"steps": [], "stop_on_error": 1}}),
            "`stop_on_error` must be true or false",
        ),
        (
            json!({"name": "command_batch", "arguments": {"steps": [], "stopOnError": true}}),
            "unknown argument `stopOnError`",
        ),
        (
            json!({"name": "command_batch", "arguments": {"steps": [{"id": "file.new"}, "file.new"]}}),
            "steps[1] must be an object",
        ),
        (
            json!({"name": "command_batch", "arguments": {"steps": [{"id": "file.new"}, {"params": {}}]}}),
            "steps[1]: missing `id`",
        ),
        (
            json!({"name": "command_batch", "arguments": {"steps": [{"id": "file.new", "params": 5}]}}),
            "steps[0]: `params` must be a JSON object",
        ),
        (
            json!({"name": "command_batch", "arguments": {"steps": [{"command": "file.new"}]}}),
            "steps[0]: unknown argument `command`",
        ),
        (
            json!({"name": "doc_inspect", "arguments": {"detail": "topology"}}),
            "doc_inspect: unknown argument `detail`; it takes no arguments",
        ),
        (
            json!({"name": "doc_render_preview", "arguments": {"size": 100}}),
            "unknown argument `size`",
        ),
        (
            json!({"name": "doc_render_preview", "arguments": {"view": 1}}),
            "`view` must be a string",
        ),
        (
            json!({"name": "doc_render_preview", "arguments": {"mode": true}}),
            "`mode` must be a string",
        ),
        (
            json!({"name": "doc_render_preview", "arguments": {"max_side": 2049}}),
            "`max_side` must be a whole number from 16 to 2048, got 2049",
        ),
        (
            json!({"name": "doc_render_preview", "arguments": {"max_side": 15}}),
            "`max_side` must be a whole number from 16 to 2048",
        ),
        (
            json!({"name": "doc_render_preview", "arguments": {"max_side": -1}}),
            "`max_side` must be a whole number",
        ),
        (
            json!({"name": "doc_render_preview", "arguments": {"max_side": 512.5}}),
            "`max_side` must be a whole number",
        ),
        (
            json!({"name": "doc_render_preview", "arguments": {"max_side": "512"}}),
            "`max_side` must be a whole number",
        ),
    ] {
        let (code, message) = rpc_error(&mut s, "tools/call", params.clone());
        assert_eq!(code, -32602, "{params}: {message}");
        assert!(message.contains(expected), "{params}: {message}");
        assert_still_serving(&mut s);
    }
    // Arguments are checked before execution: `file.new` appeared in
    // several refused calls, and no document is open.
    assert!(!s.has_document());
}

#[test]
fn the_preview_takes_the_options_of_the_render_command() {
    // Field test 2026-10-12, finding 6: only perspective views of the
    // whole model could be asked for over MCP.
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    run(&mut s, "feature.block", json!({"size": [40, 20, 5]}));
    let preview = |s: &mut Session, arguments: Value| {
        let result = tool(s, "doc_render_preview", arguments.clone());
        assert_eq!(result["isError"], false, "{arguments}: {result}");
        let text = result["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|block| block["type"] == "text")
            .unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string();
        serde_json::from_str::<Value>(&text).unwrap()
    };
    let whole = preview(&mut s, json!({"view": "top", "max_side": 256}));
    assert_eq!(whole["projection"], "perspective");
    let detail = preview(
        &mut s,
        json!({"view": "top", "max_side": 256, "projection": "orthographic",
               "zoom": 2, "box": [[0, 0, 0], [10, 10, 5]], "tolerance": 0.01}),
    );
    assert_eq!(detail["projection"], "orthographic");
    assert_eq!(detail["zoom"], 2.0);
    assert_ne!(detail["coverage"], whole["coverage"]);
    // What the command refuses comes back as the tool's failure, with the
    // command's own message.
    for arguments in [
        json!({"projection": "fisheye"}),
        json!({"zoom": -1}),
        json!({"box": "all"}),
    ] {
        let failed = tool(&mut s, "doc_render_preview", arguments.clone());
        assert_eq!(failed["isError"], true, "{arguments}: {failed}");
    }
}

#[test]
fn a_terse_batch_reports_the_failures_and_the_last_result() {
    // Field test 2026-10-12, finding 8: every step's result is a lot to
    // send back for a batch of hundreds of sketch entities.
    let mut s = session();
    let steps = json!([
        {"id": "file.new"},
        {"id": "feature.block", "params": {"size": [2, 3, 4]}},
        {"id": "feature.sphere", "params": {}},
        {"id": "document.measure"},
        {"id": "feature.nope"},
    ]);
    let terse = tool(
        &mut s,
        "command_batch",
        json!({"steps": steps, "terse": true, "stop_on_error": false}),
    );
    assert_eq!(
        terse["isError"], true,
        "a failed step still flags the batch"
    );
    let report: Value =
        serde_json::from_str(terse["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        (&report["completed"], &report["failed"]),
        (&json!(3), &json!(2))
    );
    assert!(report.get("results").is_none(), "{report}");
    let errors = report["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0]["step"], 2);
    assert!(
        errors[0]["error"].as_str().unwrap().contains("radius"),
        "{report}"
    );
    assert_eq!(errors[1]["step"], 4);
    assert_eq!(report["last"]["step"], 3);
    assert!(close(&report["last"]["result"]["volume"], 24.0), "{report}");

    // Stopping at the first failure: the last result is from before it.
    let mut s = session();
    let stopped = tool(
        &mut s,
        "command_batch",
        json!({"steps": steps, "terse": true}),
    );
    assert_eq!(stopped["isError"], true);
    let report: Value =
        serde_json::from_str(stopped["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        (&report["completed"], &report["failed"]),
        (&json!(2), &json!(1))
    );
    assert_eq!(report["errors"].as_array().unwrap().len(), 1);
    assert_eq!(report["errors"][0]["step"], 2);
    assert_eq!(report["last"]["step"], 1);
    // Nothing succeeded: there is no last result. The same steps without
    // terse give the full list, as before.
    let nothing = tool(
        &mut s,
        "command_batch",
        json!({"steps": [{"id": "feature.nope"}], "terse": true}),
    );
    let report: Value =
        serde_json::from_str(nothing["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["last"], Value::Null);
    let full = tool_json(
        &mut s,
        "command_batch",
        json!({"steps": [{"id": "document.measure"}]}),
    );
    assert!(
        full.get("errors").is_none() && full["results"].is_array(),
        "{full}"
    );
    // The flag is a boolean.
    let (code, message) = rpc_error(
        &mut s,
        "tools/call",
        json!({"name": "command_batch", "arguments": {"steps": [], "terse": "yes"}}),
    );
    assert_eq!(code, -32602);
    assert!(
        message.contains("`terse` must be true or false"),
        "{message}"
    );
}

#[test]
fn a_batch_with_a_malformed_step_runs_nothing() {
    let mut s = session();
    let (code, message) = rpc_error(
        &mut s,
        "tools/call",
        json!({"name": "command_batch", "arguments": {"steps": [{"id": "file.new"}, {"id": "file.new"}, {"idd": "x"}]}}),
    );
    assert_eq!(code, -32602);
    assert!(
        message.contains("steps[2]") && message.contains("idd"),
        "{message}"
    );
    assert!(!s.has_document());
}

#[test]
fn unknown_methods_are_method_not_found() {
    let mut s = session();
    for method in [
        "resources/subscribe",
        "prompts/list",
        "tools/run",
        "engine.execute",
        "",
        "notifications/initialized",
    ] {
        let (code, message) = rpc_error(&mut s, method, json!({}));
        assert_eq!(code, -32601, "{method}: {message}");
        assert!(message.contains("unknown method"), "{message}");
        assert_still_serving(&mut s);
    }
}

#[test]
fn params_that_are_not_an_object_are_invalid_params() {
    let mut s = session();
    for method in ["initialize", "ping", "tools/list", "tools/call"] {
        for params in [json!([1, 2]), json!("x"), json!(4), json!(true)] {
            let (code, message) = rpc_error(&mut s, method, params.clone());
            assert_eq!(code, -32602, "{method} {params}: {message}");
            assert!(message.contains("`params` must be an object"), "{message}");
        }
    }
    assert_still_serving(&mut s);
}

#[test]
fn invalid_json_is_a_parse_error_with_a_null_id() {
    let mut s = session();
    for line in [
        "not json",
        "{",
        r#"{"jsonrpc": "2.0", "id": 1, "method": "ping""#,
        r#"{"jsonrpc": "2.0", "id": 1, "method": "ping"} trailing"#,
        "{'jsonrpc': '2.0'}",
        "\u{feff}{}",
    ] {
        let reply = send_line(&mut s, line);
        assert_eq!(reply["id"], Value::Null, "{line}");
        assert_eq!(reply["error"]["code"], -32700, "{line}: {reply}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("not valid JSON")
        );
        assert_still_serving(&mut s);
    }
}

#[test]
fn messages_that_are_not_requests_are_invalid_requests() {
    let mut s = session();
    for (line, id, expected) in [
        ("[]", json!(null), "batches are not supported"),
        (
            r#"[{"jsonrpc": "2.0", "id": 1, "method": "ping"}]"#,
            json!(null),
            "batches are not supported",
        ),
        ("3", json!(null), "must be a JSON-RPC 2.0 object"),
        ("\"ping\"", json!(null), "must be a JSON-RPC 2.0 object"),
        ("null", json!(null), "must be a JSON-RPC 2.0 object"),
        ("{}", json!(null), "`jsonrpc` must be \"2.0\""),
        (
            r#"{"id": 4, "method": "ping"}"#,
            json!(4),
            "`jsonrpc` must be \"2.0\"",
        ),
        (
            r#"{"jsonrpc": "1.0", "id": 4, "method": "ping"}"#,
            json!(4),
            "`jsonrpc` must be \"2.0\"",
        ),
        (
            r#"{"jsonrpc": 2.0, "id": 4, "method": "ping"}"#,
            json!(4),
            "`jsonrpc` must be \"2.0\"",
        ),
        (r#"{"jsonrpc": "2.0", "method": "ping"}{"#, json!(null), ""),
        (r#"{"jsonrpc": "2.0", "id": 4}"#, json!(4), "no `method`"),
        (r#"{"jsonrpc": "2.0"}"#, json!(null), "no `method`"),
        (
            r#"{"jsonrpc": "2.0", "id": 4, "method": 1}"#,
            json!(4),
            "`method` must be a string",
        ),
        (
            r#"{"jsonrpc": "2.0", "method": null}"#,
            json!(null),
            "`method` must be a string",
        ),
        (
            r#"{"jsonrpc": "2.0", "id": null, "method": "ping"}"#,
            json!(null),
            "`id` must be a string or a number",
        ),
        (
            r#"{"jsonrpc": "2.0", "id": true, "method": "ping"}"#,
            json!(null),
            "`id` must be a string or a number",
        ),
        (
            r#"{"jsonrpc": "2.0", "id": [1], "method": "ping"}"#,
            json!(null),
            "`id` must be a string or a number",
        ),
        (
            r#"{"jsonrpc": "2.0", "id": {"a": 1}, "method": "ping"}"#,
            json!(null),
            "`id` must be a string or a number",
        ),
    ] {
        let reply = send_line(&mut s, line);
        // The one line above that is not JSON at all is a parse error.
        let code = if expected.is_empty() { -32700 } else { -32600 };
        assert_eq!(reply["error"]["code"], code, "{line}: {reply}");
        assert_eq!(reply["id"], id, "{line}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains(expected),
            "{line}: {reply}"
        );
        assert_still_serving(&mut s);
    }
}

// ----------------------------------------------------------------- limits

#[test]
fn a_message_over_one_mebibyte_is_refused() {
    let mut s = session();
    let message = |total: usize| {
        let head = r#"{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "command_list", "arguments": {"filter": ""#;
        let tail = r#""}}}"#;
        format!(
            "{head}{}{tail}",
            "x".repeat(total - head.len() - tail.len())
        )
    };
    let reply = send_line(&mut s, &message(MAX_REQUEST_BYTES));
    assert_eq!(reply["id"], 3);
    assert_eq!(
        reply["result"]["isError"], false,
        "a line at the limit is served"
    );

    let reply = send_line(&mut s, &message(MAX_REQUEST_BYTES + 1));
    assert_eq!(reply["id"], Value::Null);
    assert_eq!(reply["error"]["code"], -32600);
    let text = reply["error"]["message"].as_str().unwrap();
    assert!(
        text.contains("longer than the limit of 1048576 bytes"),
        "{text}"
    );
    assert_still_serving(&mut s);
}

#[test]
fn a_batch_takes_at_most_256_steps() {
    let mut s = session();
    let steps = |n: usize| -> Vec<Value> {
        let mut steps = vec![json!({"id": "file.new"})];
        steps.extend((1..n).map(
            |i| json!({"id": "variable.set", "params": {"name": format!("v{i}"), "expr": i}}),
        ));
        steps
    };
    let (code, message) = rpc_error(
        &mut s,
        "tools/call",
        json!({"name": "command_batch", "arguments": {"steps": steps(MAX_BATCH_STEPS + 1)}}),
    );
    assert_eq!(code, -32602);
    assert!(
        message.contains("257 steps") && message.contains("at most 256"),
        "{message}"
    );
    assert!(!s.has_document(), "an over-long batch runs nothing");

    let report = tool_json(
        &mut s,
        "command_batch",
        json!({"steps": steps(MAX_BATCH_STEPS)}),
    );
    assert_eq!(report["completed"], 256);
}

// ------------------------------------------------------------- serve loop

#[test]
fn serve_answers_requests_skips_notifications_and_survives_bad_lines() {
    let mut input = Vec::new();
    for line in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "",
        "garbage",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"command_run","arguments":{"id":"file.new"}}}"#,
    ] {
        input.extend_from_slice(line.as_bytes());
        input.extend_from_slice(b"\r\n");
    }
    // Not UTF-8, then longer than the limit.
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":\"\xc3\x28\",\"method\":\"ping\"}\n");
    input.extend_from_slice(&vec![b'9'; MAX_REQUEST_BYTES + 1]);
    input.extend_from_slice(b"\n");
    // The last line has no newline.
    input.extend_from_slice(
        br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"doc_inspect"}}"#,
    );

    let mut s = session();
    let mut output = Vec::new();
    mcp::serve(&mut s, &input[..], &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.ends_with('\n'));
    let replies: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 6, "{replies:#?}");
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(replies[1]["error"]["code"], -32700);
    assert_eq!(replies[1]["id"], Value::Null);
    assert_eq!(replies[2]["id"], 2);
    assert_eq!(replies[2]["result"]["isError"], false);
    assert_eq!(
        replies[3]["error"]["code"], -32700,
        "bytes that are not UTF-8 are not JSON"
    );
    assert_eq!(replies[3]["id"], Value::Null);
    assert_eq!(replies[4]["error"]["code"], -32600);
    assert!(
        replies[4]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("longer than the limit")
    );
    // The document made before the bad lines is still open.
    assert_eq!(replies[5]["id"], 3);
    let inspect: Value =
        serde_json::from_str(replies[5]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(inspect["document"], 0);
}

// -------------------------------------------------------------- selection

/// An agent asks what is selected at any time: the tool selection_get
/// and the resource craft3d://selection, the same answer as select.get.
/// Selecting by description over MCP: "select the face with normal -y".
#[test]
fn the_selection_is_a_tool_and_a_resource() {
    let mut s = session();
    assert_eq!(
        tool_json(&mut s, "selection_get", json!({})),
        json!({"document": null})
    );
    let listed = result(&mut s, "resources/list", json!({}));
    assert_eq!(listed["resources"][0]["uri"], "craft3d://selection");
    assert_eq!(
        result(&mut s, "resources/templates/list", json!({}))["resourceTemplates"],
        json!([])
    );
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Cube", "size": [10, 20, 30]}),
    );
    let back = run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"normal": {"along": "-y"}}, "then": "replace"}),
    );
    assert_eq!(back["count"], 1);
    let got = tool_json(&mut s, "selection_get", json!({}));
    assert_eq!(got["summary"], "1 face");
    assert_eq!(got["items"][0]["name"], back["items"][0]["name"]);
    let read = result(
        &mut s,
        "resources/read",
        json!({"uri": "craft3d://selection"}),
    );
    assert_eq!(read["contents"][0]["mimeType"], "application/json");
    let text = read["contents"][0]["text"].as_str().unwrap();
    assert_eq!(serde_json::from_str::<Value>(text).unwrap(), got);
    for params in [
        json!({}),
        json!({"uri": 3}),
        json!({"uri": "craft3d://nothing"}),
    ] {
        let (code, message) = rpc_error(&mut s, "resources/read", params);
        assert_eq!(code, -32602, "{message}");
        assert!(message.starts_with("resources/read"), "{message}");
    }
    let (code, _) = rpc_error(
        &mut s,
        "tools/call",
        json!({"name": "selection_get", "arguments": {"what": 1}}),
    );
    assert_eq!(code, -32602);
    assert_still_serving(&mut s);
}
