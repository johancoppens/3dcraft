//! No line makes a handler panic: a deterministic sweep of both protocols
//! with random bytes and with random JSON built from a small grammar that
//! knows the real method, tool, command and parameter names (so a good
//! share of the lines get past parsing and into the engine).
//!
//! Every line goes to both handlers. Each must return nothing or exactly
//! one line of valid JSON in its protocol's reply shape. The handlers catch
//! panics and report them as "internal error", so the test also fails on
//! that text: a caught panic is still a bug.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_automation::{mcp, rpc};
use craft3d_engine::{Session, Workspace};
use serde_json::{Map, Value, json};

/// Lines per run. Each goes to both handlers.
const LINES: usize = 200_000;
/// A fresh session every so many lines: documents fill with features, and
/// regeneration time grows with them.
const LINES_PER_SESSION: usize = 500;

/// xorshift64*: deterministic, so a failure reproduces from its line number.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T: ?Sized>(&mut self, items: &[&'a T]) -> &'a T {
        items[self.below(items.len())]
    }
}

const RPC_METHODS: &[&str] = &["engine.execute", "engine.commands", "batch", "methods"];
const MCP_METHODS: &[&str] = &[
    "initialize",
    "ping",
    "tools/list",
    // Three times: most of the server is behind this method.
    "tools/call",
    "tools/call",
    "tools/call",
    "notifications/initialized",
    "notifications/cancelled",
];
const TOOLS: &[&str] = &[
    "command_list",
    "command_run",
    "command_batch",
    "doc_inspect",
    "doc_render_preview",
];
/// Keys of the envelopes, of tool arguments and of command params.
const KEYS: &[&str] = &[
    "id",
    "method",
    "params",
    "jsonrpc",
    "command",
    "steps",
    "stopOnError",
    "stop_on_error",
    "name",
    "arguments",
    "filter",
    "enabled_only",
    "view",
    "mode",
    "max_side",
    "expr",
    "size",
    "position",
    "radius",
    "height",
    "feature",
    "path",
    "units",
    "tolerance",
    "width",
    "discard",
    "suppressed",
    "to",
    "format",
    "protocolVersion",
    "result",
    "error",
    "",
];
const STRINGS: &[&str] = &[
    "",
    "mm",
    "in",
    "40 mm",
    "width",
    "width / 2",
    "width / 0",
    "2 * (",
    "1e400 mm",
    "-5 mm",
    "0",
    "F1",
    "F999999999999999999999",
    "Block1",
    "iso",
    "top",
    "ids",
    "shaded",
    "stl",
    "3mf",
    "part.3dcraft",
    "../x.stl",
    "/etc/passwd",
    "a\0b",
    "2.0",
    "2025-06-18",
    "\u{1F600}",
    "\"quoted\"",
    "line\nbreak",
    "feature",
    "file",
    "\u{feff}",
    "é",
];

fn string(rng: &mut Rng) -> String {
    match rng.below(10) {
        0 => {
            let length = rng.below(40);
            (0..length)
                .map(|_| char::from_u32(rng.below(0x3000) as u32).unwrap_or('?'))
                .collect()
        }
        1 => rng.pick(KEYS).to_string(),
        2 => Session::commands()[rng.below(Session::commands().len())]
            .id
            .to_string(),
        _ => rng.pick(STRINGS).to_string(),
    }
}

fn number(rng: &mut Rng) -> Value {
    match rng.below(12) {
        0 => json!(0),
        1 => json!(-1),
        2 => json!(1),
        3 => json!(16),
        4 => json!(48),
        5 => json!(u64::MAX),
        6 => json!(i64::MIN),
        7 => json!(1e308),
        8 => json!(-1e-300),
        9 => json!(2.5),
        10 => json!(rng.below(5000)),
        _ => json!(rng.next() as f64 / 1e12),
    }
}

/// Any JSON value, nested up to `depth`.
fn value(rng: &mut Rng, depth: usize) -> Value {
    match rng.below(if depth == 0 { 6 } else { 9 }) {
        0 => Value::Null,
        1 => json!(rng.chance(50)),
        2 | 3 => number(rng),
        4 | 5 => json!(string(rng)),
        6 => (0..rng.below(4)).map(|_| value(rng, depth - 1)).collect(),
        _ => Value::Object(object(rng, depth - 1)),
    }
}

fn object(rng: &mut Rng, depth: usize) -> Map<String, Value> {
    (0..rng.below(4))
        .map(|_| {
            let key = if rng.chance(85) {
                rng.pick(KEYS).to_string()
            } else {
                string(rng)
            };
            (key, value(rng, depth))
        })
        .collect()
}

/// Params that are often right for the command, so that commands succeed
/// and later lines find documents with features in them.
fn command_params(rng: &mut Rng, command: &str) -> Value {
    if rng.chance(25) {
        return value(rng, 2);
    }
    let expr = |rng: &mut Rng| {
        if rng.chance(70) {
            json!(1 + rng.below(50))
        } else {
            json!(string(rng))
        }
    };
    let triple = |rng: &mut Rng| json!([expr(rng), expr(rng), expr(rng)]);
    let feature = |rng: &mut Rng| {
        if rng.chance(70) {
            json!(format!("F{}", 1 + rng.below(6)))
        } else {
            value(rng, 1)
        }
    };
    let mut params = match command {
        "file.new" | "document.inspect" | "edit.undo" | "edit.redo" => json!({}),
        "file.close" => json!({"discard": rng.chance(80)}),
        "file.open" | "file.save" | "file.export" => json!({"path": string(rng)}),
        "variable.set" => json!({"name": rng.pick(&["width", "w", "t"]), "expr": expr(rng)}),
        "variable.delete" => json!({"name": rng.pick(&["width", "w", "t"])}),
        "variable.rename" => {
            json!({"name": rng.pick(&["width", "w"]), "to": rng.pick(&["w", "t", "width"])})
        }
        "feature.block" => json!({"size": triple(rng), "position": triple(rng)}),
        "feature.cylinder" => json!({"radius": expr(rng), "height": expr(rng)}),
        "feature.sphere" => json!({"radius": expr(rng)}),
        "feature.edit" => json!({"feature": feature(rng), "radius": expr(rng)}),
        "feature.rename" => json!({"feature": feature(rng), "name": string(rng)}),
        "feature.move" => json!({"feature": feature(rng), "position": rng.below(4)}),
        "view.render" => {
            json!({"width": 16 + rng.below(80), "height": 16 + rng.below(80), "view": rng.pick(&["iso", "top", "left", "x"])})
        }
        _ => json!({"feature": feature(rng)}),
    };
    // Now and then an extra key, known or not.
    if rng.chance(15) {
        params[rng.pick(KEYS)] = value(rng, 1);
    }
    params
}

fn command_id(rng: &mut Rng) -> String {
    if rng.chance(92) {
        // Weighted towards the commands that build state.
        let builders = [
            "file.new",
            "feature.block",
            "feature.cylinder",
            "feature.sphere",
            "variable.set",
        ];
        if rng.chance(40) {
            rng.pick(&builders).to_string()
        } else {
            Session::commands()[rng.below(Session::commands().len())]
                .id
                .to_string()
        }
    } else {
        string(rng)
    }
}

fn step(rng: &mut Rng, id_key: &str) -> Value {
    if rng.chance(8) {
        return value(rng, 2);
    }
    let command = command_id(rng);
    let params = command_params(rng, &command);
    json!({id_key: command, "params": params})
}

fn steps(rng: &mut Rng, id_key: &str) -> Value {
    // Rarely more steps than a batch takes.
    let count = if rng.chance(1) {
        257 + rng.below(3)
    } else {
        rng.below(5)
    };
    (0..count).map(|_| step(rng, id_key)).collect()
}

fn id(rng: &mut Rng) -> Value {
    match rng.below(8) {
        0 => value(rng, 1),
        1 => json!(string(rng)),
        _ => json!(rng.below(1000)),
    }
}

fn rpc_request(rng: &mut Rng) -> Value {
    let method = if rng.chance(92) {
        rng.pick(RPC_METHODS).to_string()
    } else {
        string(rng)
    };
    let params = if rng.chance(12) {
        value(rng, 2)
    } else {
        match method.as_str() {
            "engine.execute" => {
                let command = command_id(rng);
                json!({"command": command, "params": command_params(rng, &command)})
            }
            "engine.commands" => json!({"filter": string(rng)}),
            "batch" => json!({"steps": steps(rng, "command"), "stopOnError": rng.chance(50)}),
            _ => json!({}),
        }
    };
    let mut request = json!({"id": id(rng), "method": method, "params": params});
    perturb(rng, &mut request);
    request
}

fn mcp_request(rng: &mut Rng) -> Value {
    let method = if rng.chance(92) {
        rng.pick(MCP_METHODS).to_string()
    } else {
        string(rng)
    };
    let params = if rng.chance(12) {
        value(rng, 2)
    } else {
        match method.as_str() {
            "initialize" => json!({"protocolVersion": string(rng), "capabilities": value(rng, 1)}),
            "tools/call" => {
                let tool = if rng.chance(92) {
                    rng.pick(TOOLS).to_string()
                } else {
                    string(rng)
                };
                let arguments = if rng.chance(12) {
                    value(rng, 2)
                } else {
                    match tool.as_str() {
                        "command_list" => {
                            json!({"filter": string(rng), "enabled_only": rng.chance(50)})
                        }
                        "command_run" => {
                            let command = command_id(rng);
                            json!({"id": command, "params": command_params(rng, &command)})
                        }
                        "command_batch" => {
                            json!({"steps": steps(rng, "id"), "stop_on_error": rng.chance(50)})
                        }
                        // Small pictures: this is a test of robustness, not of the renderer.
                        "doc_render_preview" => {
                            json!({"max_side": 16 + rng.below(100), "view": rng.pick(&["iso", "bottom", "?"])})
                        }
                        _ => json!({}),
                    }
                };
                json!({"name": tool, "arguments": arguments})
            }
            _ => json!({}),
        }
    };
    let mut request = json!({"jsonrpc": "2.0", "method": method, "params": params});
    // A notification has no id. Most `notifications/*` are sent as one, and
    // now and then a request is too.
    let notification = rng.chance(if method.starts_with("notifications/") {
        85
    } else {
        4
    });
    if !notification {
        request["id"] = id(rng);
    }
    perturb(rng, &mut request);
    request
}

/// Sometimes drops, replaces or adds a key of the envelope.
fn perturb(rng: &mut Rng, request: &mut Value) {
    if !rng.chance(15) {
        return;
    }
    let map = request.as_object_mut().unwrap();
    let key = rng.pick(KEYS);
    match rng.below(3) {
        0 => {
            map.remove(key);
        }
        _ => {
            map.insert(key.to_string(), value(rng, 2));
        }
    }
}

/// A valid line with one thing wrong with its text.
fn damage(rng: &mut Rng, line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let at = rng.below(chars.len());
    match rng.below(4) {
        0 => chars[..at].iter().collect(),
        1 => chars[at..].iter().collect(),
        2 => {
            let mut damaged = chars.clone();
            damaged[at] = *rng.pick(&[&'{', &'}', &'[', &'"', &'\\', &',', &':', &'\0', &' ']);
            damaged.into_iter().collect()
        }
        _ => format!("{line}{line}"),
    }
}

fn line(rng: &mut Rng) -> String {
    match rng.below(10) {
        0 | 1 => {
            let length = rng.below(120);
            let bytes: Vec<u8> = (0..length).map(|_| rng.next() as u8).collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
        2 => value(rng, 3).to_string(),
        3..=5 => rpc_request(rng).to_string(),
        6..=8 => mcp_request(rng).to_string(),
        _ => {
            let valid = if rng.chance(50) {
                rpc_request(rng)
            } else {
                mcp_request(rng)
            };
            damage(rng, &valid.to_string())
        }
    }
}

fn check_reply(protocol: &str, number: usize, line: &str, reply: &str) -> Value {
    let context = || format!("{protocol}, line {number}: {line:?}\nreply: {reply}");
    assert!(
        !reply.contains('\n') && !reply.contains('\r'),
        "one line: {}",
        context()
    );
    let value: Value = serde_json::from_str(reply).unwrap_or_else(|e| panic!("{e}: {}", context()));
    assert!(value.is_object(), "{}", context());
    assert!(
        !reply.contains("internal error"),
        "a caught panic: {}",
        context()
    );
    value
}

#[test]
fn no_line_panics_either_handler_and_every_reply_is_one_line_of_json() {
    let mut rng = Rng(0x3D_C4AF7);
    let mut rpc_session = Session::new(Workspace::none());
    let mut mcp_session = Session::new(Workspace::none());
    // What the sweep reached, so a grammar that only produces rejects shows.
    let (
        mut rpc_ok,
        mut rpc_failed,
        mut mcp_ok,
        mut mcp_failed,
        mut mcp_silent,
        mut tool_ok,
        mut tool_failed,
    ) = (
        0_usize, 0_usize, 0_usize, 0_usize, 0_usize, 0_usize, 0_usize,
    );

    for number in 0..LINES {
        if number % LINES_PER_SESSION == 0 {
            rpc_session = Session::new(Workspace::none());
            mcp_session = Session::new(Workspace::none());
        }
        let line = line(&mut rng);

        match rpc::handle_line(&mut rpc_session, &line) {
            None => assert!(
                line.trim().is_empty(),
                "rpc is silent only for a blank line: {line:?}"
            ),
            Some(reply) => {
                let reply = check_reply("rpc", number, &line, &reply);
                assert!(reply.get("id").is_some(), "{reply}");
                match reply["ok"].as_bool() {
                    Some(true) => {
                        assert!(
                            reply.get("result").is_some() && reply.get("error").is_none(),
                            "{reply}"
                        );
                        rpc_ok += 1;
                    }
                    Some(false) => {
                        assert!(
                            reply["error"].is_string() && reply.get("result").is_none(),
                            "{reply}"
                        );
                        rpc_failed += 1;
                    }
                    None => panic!("rpc, line {number}: `ok` is not a boolean: {reply}"),
                }
            }
        }

        match mcp::handle_line(&mut mcp_session, &line) {
            None => mcp_silent += 1,
            Some(reply) => {
                let reply = check_reply("mcp", number, &line, &reply);
                assert_eq!(reply["jsonrpc"], "2.0", "{reply}");
                assert!(reply.get("id").is_some(), "{reply}");
                if let Some(error) = reply.get("error") {
                    assert!(reply.get("result").is_none(), "{reply}");
                    let code = error["code"].as_i64().expect("an integer code");
                    assert!(
                        [-32700, -32600, -32601, -32602].contains(&code),
                        "mcp, line {number}: unexpected code in {reply} for {line:?}"
                    );
                    assert!(error["message"].is_string(), "{reply}");
                    mcp_failed += 1;
                } else {
                    let result = reply.get("result").expect("a result or an error");
                    mcp_ok += 1;
                    match result.get("isError").and_then(Value::as_bool) {
                        Some(true) => tool_failed += 1,
                        Some(false) => tool_ok += 1,
                        None => {}
                    }
                }
            }
        }
    }

    println!(
        "{LINES} lines. rpc: {rpc_ok} ok, {rpc_failed} errors. mcp: {mcp_ok} results \
         ({tool_ok} tool successes, {tool_failed} tool errors), {mcp_failed} errors, {mcp_silent} without reply."
    );
    // The sweep is only worth its name if it gets into every layer.
    assert!(rpc_ok > LINES / 20 && rpc_failed > LINES / 20);
    assert!(mcp_ok > LINES / 20 && mcp_failed > LINES / 20 && mcp_silent > LINES / 100);
    assert!(tool_ok > LINES / 100 && tool_failed > LINES / 100);
}
