//! The JSON-lines protocol, driven through `rpc::handle_line` and
//! `rpc::serve` exactly as `craft3d-cli serve` drives them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_automation::{MAX_BATCH_STEPS, MAX_REQUEST_BYTES, rpc};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

fn session() -> Session {
    Session::new(Workspace::none())
}

/// Sends one request line and parses the one reply line.
fn send_line(session: &mut Session, line: &str) -> Value {
    let reply = rpc::handle_line(session, line).unwrap_or_else(|| panic!("no reply to {line}"));
    assert!(!reply.contains('\n'), "a reply is one line: {reply}");
    serde_json::from_str(&reply).unwrap_or_else(|e| panic!("reply is not JSON ({e}): {reply}"))
}

/// Sends a request and returns its result; the request must succeed.
fn call(session: &mut Session, method: &str, params: Value) -> Value {
    let request = json!({"id": 1, "method": method, "params": params});
    let reply = send_line(session, &request.to_string());
    assert_eq!(reply["ok"], true, "{method} {params}: {reply}");
    assert_eq!(reply["id"], 1);
    assert!(reply.get("error").is_none(), "{reply}");
    reply["result"].clone()
}

/// Sends a request that must fail and returns the error text.
fn call_error(session: &mut Session, method: &str, params: Value) -> String {
    let request = json!({"id": "e", "method": method, "params": params});
    let reply = send_line(session, &request.to_string());
    assert_eq!(reply["ok"], false, "{method} {params} should fail: {reply}");
    assert_eq!(reply["id"], "e");
    assert!(reply.get("result").is_none(), "{reply}");
    reply["error"]
        .as_str()
        .expect("error is a string")
        .to_string()
}

fn execute(session: &mut Session, command: &str, params: Value) -> Value {
    call(
        session,
        "engine.execute",
        json!({"command": command, "params": params}),
    )
}

/// Mass properties are integrated over the faces in floating point, so a
/// volume is compared with a relative tolerance of 1e-9 (the bound the
/// engine's own tests use), not bit for bit.
fn close(actual: &Value, expected: f64) -> bool {
    let actual = actual.as_f64().expect("a number");
    (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0)
}

/// The server still answers: what every error test ends with.
fn assert_still_serving(session: &mut Session) {
    let reply = send_line(session, r#"{"id": "alive", "method": "methods"}"#);
    assert_eq!(reply["id"], "alive");
    assert_eq!(reply["ok"], true);
}

// ------------------------------------------------------------ happy paths

#[test]
fn engine_execute_runs_commands_and_returns_their_results() {
    let mut s = session();
    let new = execute(&mut s, "file.new", json!({"units": "mm"}));
    assert_eq!(new, json!({"document": 0, "units": "mm"}));
    let block = execute(&mut s, "feature.block", json!({"size": [40, 20, 10]}));
    assert_eq!(block["status"], "ok");
    let measure = execute(&mut s, "document.measure", json!({}));
    // Closed form: 40 x 20 x 10.
    assert!(close(&measure["volume"], 8000.0), "{measure}");
    assert_eq!(measure["volumeUnit"], "mm^3");
}

#[test]
fn command_params_may_be_omitted_or_null() {
    let mut s = session();
    let reply = send_line(
        &mut s,
        r#"{"id": 1, "method": "engine.execute", "params": {"command": "file.new"}}"#,
    );
    assert_eq!(reply["result"]["units"], "mm");
    let reply = send_line(
        &mut s,
        r#"{"id": 2, "method": "engine.execute", "params": {"command": "document.inspect", "params": null}}"#,
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["features"], json!([]));
}

#[test]
fn the_reply_echoes_any_id_and_null_when_there_is_none() {
    let mut s = session();
    for id in [
        json!(7),
        json!("seven"),
        json!([1, {"a": null}]),
        json!({"k": 1.5}),
        json!(true),
        json!(null),
    ] {
        let reply = send_line(&mut s, &json!({"id": id, "method": "methods"}).to_string());
        assert_eq!(reply["id"], id);
        assert_eq!(reply["ok"], true);
    }
    let reply = send_line(&mut s, r#"{"method": "methods"}"#);
    assert_eq!(reply["id"], Value::Null);
    assert_eq!(reply["ok"], true);
    // Errors echo the id too.
    let reply = send_line(&mut s, r#"{"id": [9], "method": "nope"}"#);
    assert_eq!(reply["id"], json!([9]));
    assert_eq!(reply["ok"], false);
}

#[test]
fn engine_commands_lists_the_registry_with_enablement() {
    let mut s = session();
    let commands = call(&mut s, "engine.commands", json!({}))["commands"].clone();
    let commands = commands.as_array().unwrap();
    assert_eq!(commands.len(), Session::commands().len());
    for (entry, spec) in commands.iter().zip(Session::commands()) {
        assert_eq!(entry["id"], spec.id);
        assert_eq!(entry["label"], spec.label);
        assert_eq!(entry["params"], spec.params);
        assert_eq!(entry["summary"], spec.summary);
        assert_eq!(entry["needsDocument"], spec.needs_document);
        // Nothing is open: exactly the commands that need a document are off.
        assert_eq!(entry["enabled"], !spec.needs_document, "{}", spec.id);
        if spec.needs_document {
            assert!(
                entry["disabledReason"]
                    .as_str()
                    .unwrap()
                    .contains("file.new")
            );
        } else {
            assert_eq!(entry["disabledReason"], Value::Null);
        }
        assert_eq!(
            entry.as_object().unwrap().len(),
            7,
            "no undocumented fields"
        );
    }

    execute(&mut s, "file.new", json!({}));
    let commands = call(&mut s, "engine.commands", Value::Null)["commands"].clone();
    for entry in commands.as_array().unwrap() {
        assert_eq!(entry["enabled"], true, "{entry}");
        assert_eq!(entry["disabledReason"], Value::Null);
    }
}

#[test]
fn engine_commands_filters_on_id_or_label_ignoring_case() {
    let mut s = session();
    let ids = |s: &mut Session, filter: &str| -> Vec<String> {
        call(s, "engine.commands", json!({"filter": filter}))["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .collect()
    };
    // By id.
    let expected: Vec<String> = Session::commands()
        .iter()
        .filter(|c| c.id.contains("variable."))
        .map(|c| c.id.to_string())
        .collect();
    assert_eq!(expected.len(), 3, "the fixture of this test");
    assert_eq!(ids(&mut s, "variable."), expected);
    assert_eq!(ids(&mut s, "VARIABLE."), expected);
    // By label only: "Roll Back" is the label of history.rollback, and the
    // id does not contain "roll back".
    assert_eq!(ids(&mut s, "roll b"), vec!["history.rollback"]);
    assert_eq!(ids(&mut s, "no such thing"), Vec::<String>::new());
    assert_eq!(ids(&mut s, "").len(), Session::commands().len());
}

#[test]
fn batch_runs_steps_in_order() {
    let mut s = session();
    let report = call(
        &mut s,
        "batch",
        json!({"steps": [
            {"command": "file.new"},
            {"command": "variable.set", "params": {"name": "w", "expr": "30 mm"}},
            {"command": "feature.block", "params": {"size": ["w", "w", "w"]}},
            {"command": "document.measure", "params": {}},
        ]}),
    );
    assert_eq!(report["completed"], 4);
    assert_eq!(report["failed"], 0);
    let results = report["results"].as_array().unwrap();
    assert_eq!(results.len(), 4);
    assert!(
        results
            .iter()
            .all(|r| r["ok"] == true && r.get("error").is_none())
    );
    // Closed form: 30 x 30 x 30.
    assert!(close(&results[3]["result"]["volume"], 27000.0), "{report}");
}

#[test]
fn batch_stops_at_the_first_error_by_default_and_is_not_a_transaction() {
    let mut s = session();
    let report = call(
        &mut s,
        "batch",
        json!({"steps": [
            {"command": "file.new"},
            {"command": "feature.block", "params": {"size": [1, 2, 3]}},
            {"command": "feature.cylinder", "params": {"radius": 1}},
            {"command": "feature.sphere", "params": {"radius": 1}},
        ]}),
    );
    assert_eq!(report["completed"], 2);
    assert_eq!(report["failed"], 1);
    let results = report["results"].as_array().unwrap();
    assert_eq!(results.len(), 3, "the step after the failure did not run");
    assert_eq!(results[2]["ok"], false);
    assert!(results[2].get("result").is_none());
    let error = results[2]["error"].as_str().unwrap();
    assert!(
        error.contains("feature.cylinder") && error.contains("height"),
        "{error}"
    );

    // The steps before the failure stay applied.
    let inspect = execute(&mut s, "document.inspect", json!({}));
    assert_eq!(inspect["features"].as_array().unwrap().len(), 1);
}

#[test]
fn batch_continues_past_errors_when_asked() {
    let mut s = session();
    let report = call(
        &mut s,
        "batch",
        json!({"stopOnError": false, "steps": [
            {"command": "file.new"},
            {"command": "edit.undo"},
            {"command": "no.such.command"},
            {"command": "feature.sphere", "params": {"radius": 1}},
        ]}),
    );
    assert_eq!(report["completed"], 2);
    assert_eq!(report["failed"], 2);
    let oks: Vec<bool> = report["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["ok"].as_bool().unwrap())
        .collect();
    assert_eq!(oks, [true, false, false, true]);

    let empty = call(&mut s, "batch", json!({"steps": []}));
    assert_eq!(empty, json!({"completed": 0, "failed": 0, "results": []}));
}

#[test]
fn methods_lists_every_method_and_each_one_exists() {
    let mut s = session();
    let result = call(&mut s, "methods", json!({}));
    let names: Vec<&str> = result["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["engine.execute", "engine.commands", "batch", "methods"]
    );
    assert_eq!(
        names,
        rpc::METHODS.iter().map(|m| m.name).collect::<Vec<_>>()
    );
    // A listed method is never "unknown".
    for name in names {
        let reply = send_line(&mut s, &json!({"id": 1, "method": name}).to_string());
        let unknown = reply["error"]
            .as_str()
            .is_some_and(|e| e.contains("unknown method"));
        assert!(!unknown, "{name}: {reply}");
    }
    assert_eq!(result["limits"]["batchSteps"], MAX_BATCH_STEPS);
    assert_eq!(result["limits"]["requestBytes"], MAX_REQUEST_BYTES);
}

#[test]
fn blank_lines_get_no_reply() {
    let mut s = session();
    for line in ["", " ", "\t", "   \r"] {
        assert_eq!(rpc::handle_line(&mut s, line), None, "{line:?}");
    }
}

// ------------------------------------------------------------ error paths

#[test]
fn malformed_requests_get_an_error_reply_with_a_null_id() {
    let mut s = session();
    for (line, expected) in [
        ("not json", "not valid JSON"),
        ("{\"id\": 1, \"method\": ", "not valid JSON"),
        ("{'id': 1}", "not valid JSON"),
        ("[1, 2]", "must be a JSON object"),
        ("\"engine.execute\"", "must be a JSON object"),
        ("42", "must be a JSON object"),
        ("null", "must be a JSON object"),
        ("true", "must be a JSON object"),
    ] {
        let reply = send_line(&mut s, line);
        assert_eq!(reply["id"], Value::Null, "{line}");
        assert_eq!(reply["ok"], false, "{line}");
        let error = reply["error"].as_str().unwrap();
        assert!(error.contains(expected), "{line}: {error}");
        assert_still_serving(&mut s);
    }
}

#[test]
fn requests_of_the_wrong_shape_get_an_error_reply_with_their_id() {
    let mut s = session();
    for (line, expected) in [
        (r#"{"id": 5}"#, "no `method`"),
        (r#"{"id": 5, "method": 5}"#, "`method` must be a string"),
        (r#"{"id": 5, "method": null}"#, "`method` must be a string"),
        (
            r#"{"id": 5, "method": ["methods"]}"#,
            "`method` must be a string",
        ),
        (
            r#"{"id": 5, "method": "engine.run"}"#,
            "unknown method `engine.run`",
        ),
        (r#"{"id": 5, "method": ""}"#, "unknown method"),
        (
            r#"{"id": 5, "method": "methods", "extra": 1}"#,
            "unknown request field `extra`",
        ),
        (
            r#"{"id": 5, "method": "engine.execute", "command": "file.new"}"#,
            "unknown request field `command`",
        ),
    ] {
        let reply = send_line(&mut s, line);
        assert_eq!(reply["id"], 5, "{line}");
        assert_eq!(reply["ok"], false, "{line}");
        let error = reply["error"].as_str().unwrap();
        assert!(error.contains(expected), "{line}: {error}");
        assert_still_serving(&mut s);
    }
    // The unknown-method error tells an agent what does exist.
    let error = call_error(&mut s, "engine.run", json!({}));
    for method in rpc::METHODS {
        assert!(error.contains(method.name), "{error}");
    }
}

#[test]
fn params_of_the_wrong_shape_are_refused_and_name_the_key() {
    let mut s = session();
    for (method, params, expected) in [
        (
            "engine.execute",
            json!([1]),
            "must be a JSON object, got an array",
        ),
        (
            "engine.execute",
            json!("file.new"),
            "must be a JSON object, got a string",
        ),
        (
            "engine.execute",
            json!(3),
            "must be a JSON object, got a number",
        ),
        ("engine.execute", json!({}), "missing `command`"),
        (
            "engine.execute",
            json!({"command": 1}),
            "`command` must be a string",
        ),
        (
            "engine.execute",
            json!({"command": ["file.new"]}),
            "`command` must be a string",
        ),
        (
            "engine.execute",
            json!({"command": "file.new", "params": [1]}),
            "`params` must be a JSON object, got an array",
        ),
        (
            "engine.execute",
            json!({"command": "file.new", "params": "mm"}),
            "`params` must be a JSON object, got a string",
        ),
        (
            "engine.execute",
            json!({"command": "file.new", "wait": false}),
            "unknown parameter `wait`",
        ),
        (
            "engine.execute",
            json!({"id": "file.new"}),
            "unknown parameter `id`",
        ),
        (
            "engine.commands",
            json!({"filter": 1}),
            "`filter` must be a string",
        ),
        (
            "engine.commands",
            json!({"enabled_only": true}),
            "unknown parameter `enabled_only`",
        ),
        ("engine.commands", json!([]), "must be a JSON object"),
        ("batch", json!({}), "missing `steps`"),
        ("batch", json!({"steps": {}}), "`steps` must be an array"),
        (
            "batch",
            json!({"steps": "file.new"}),
            "`steps` must be an array",
        ),
        (
            "batch",
            json!({"steps": [], "stopOnError": "no"}),
            "`stopOnError` must be true or false",
        ),
        (
            "batch",
            json!({"steps": [], "stop_on_error": true}),
            "unknown parameter `stop_on_error`",
        ),
        (
            "batch",
            json!({"steps": ["file.new"]}),
            "steps[0] must be an object",
        ),
        (
            "batch",
            json!({"steps": [{"command": "file.new"}, {}]}),
            "steps[1]: missing `command`",
        ),
        (
            "batch",
            json!({"steps": [{"command": "file.new"}, {"command": 2}]}),
            "steps[1]: `command` must be a string",
        ),
        (
            "batch",
            json!({"steps": [{"command": "file.new", "params": 1}]}),
            "steps[0]: `params` must be a JSON object",
        ),
        (
            "batch",
            json!({"steps": [{"command": "file.new", "id": 1}]}),
            "steps[0]: unknown parameter `id`",
        ),
        (
            "batch",
            json!({"steps": [{"method": "methods"}]}),
            "steps[0]: unknown parameter `method`",
        ),
        ("methods", json!({"x": 1}), "unknown parameter `x`"),
        ("methods", json!([]), "must be a JSON object"),
    ] {
        let error = call_error(&mut s, method, params.clone());
        assert!(error.contains(expected), "{method} {params}: {error}");
        assert!(
            error.starts_with(method),
            "the error names the method: {error}"
        );
        assert_still_serving(&mut s);
    }
    // None of the refused requests ran anything: `file.new` appeared in
    // several, and no document is open.
    assert!(!s.has_document());
}

#[test]
fn a_batch_is_checked_before_any_step_runs() {
    let mut s = session();
    let error = call_error(
        &mut s,
        "batch",
        json!({"steps": [{"command": "file.new"}, {"command": "file.new"}, {"commnad": "file.new"}]}),
    );
    assert!(
        error.contains("steps[2]") && error.contains("commnad"),
        "{error}"
    );
    assert!(!s.has_document(), "steps 0 and 1 must not have run");
}

#[test]
fn command_failures_are_error_replies_written_for_agents() {
    let mut s = session();
    let error = call_error(&mut s, "engine.execute", json!({"command": "feature.blok"}));
    assert_eq!(
        error,
        "unknown command `feature.blok`; did you mean `feature.block`?"
    );
    let error = call_error(
        &mut s,
        "engine.execute",
        json!({"command": "document.inspect"}),
    );
    assert!(error.contains("no document is open"), "{error}");
    execute(&mut s, "file.new", json!({}));
    let error = call_error(
        &mut s,
        "engine.execute",
        json!({"command": "feature.block", "params": {"size": [1, 2]}}),
    );
    assert!(
        error.contains("feature.block") && error.contains("size"),
        "{error}"
    );
    assert_still_serving(&mut s);
    // The session survived the errors with its document.
    assert!(s.has_document());
}

// ----------------------------------------------------------------- limits

#[test]
fn a_request_line_over_one_mebibyte_is_refused() {
    assert_eq!(MAX_REQUEST_BYTES, 1 << 20);
    let mut s = session();
    let request = |total: usize| {
        let head = r#"{"id": 3, "method": "engine.commands", "params": {"filter": ""#;
        let tail = r#""}}"#;
        format!(
            "{head}{}{tail}",
            "x".repeat(total - head.len() - tail.len())
        )
    };
    // Exactly at the limit: served (nothing matches the filter).
    let at_limit = request(MAX_REQUEST_BYTES);
    assert_eq!(at_limit.len(), MAX_REQUEST_BYTES);
    let reply = send_line(&mut s, &at_limit);
    assert_eq!(
        reply,
        json!({"id": 3, "ok": true, "result": {"commands": []}})
    );

    // One byte more: refused, and not parsed for its id.
    let reply = send_line(&mut s, &request(MAX_REQUEST_BYTES + 1));
    assert_eq!(reply["id"], Value::Null);
    assert_eq!(reply["ok"], false);
    let error = reply["error"].as_str().unwrap();
    assert!(
        error.contains("longer than the limit of 1048576 bytes"),
        "{error}"
    );
    assert!(error.len() < 300, "the error does not repeat the request");
    assert_still_serving(&mut s);
}

#[test]
fn a_batch_takes_at_most_256_steps() {
    assert_eq!(MAX_BATCH_STEPS, 256);
    let mut s = session();
    let steps = |n: usize| -> Vec<Value> {
        let mut steps = vec![json!({"command": "file.new"})];
        steps.extend((1..n).map(
            |i| json!({"command": "variable.set", "params": {"name": format!("v{i}"), "expr": i}}),
        ));
        steps
    };
    let error = call_error(&mut s, "batch", json!({"steps": steps(257)}));
    assert!(
        error.contains("257 steps") && error.contains("at most 256"),
        "{error}"
    );
    assert!(!s.has_document(), "an over-long batch runs nothing");
    assert_still_serving(&mut s);

    let report = call(&mut s, "batch", json!({"steps": steps(256)}));
    assert_eq!(report["completed"], 256);
    assert_eq!(report["results"].as_array().unwrap().len(), 256);
}

#[test]
fn error_messages_do_not_echo_huge_input() {
    let mut s = session();
    let long = "k".repeat(100_000);
    for line in [
        json!({"id": 1, "method": long}).to_string(),
        json!({"id": 1, "method": "methods", long.clone(): 1}).to_string(),
        json!({"id": 1, "method": "methods", "params": {long.clone(): 1}}).to_string(),
    ] {
        let reply = send_line(&mut s, &line);
        assert_eq!(reply["ok"], false);
        let error = reply["error"].as_str().unwrap();
        assert!(error.len() < 1000, "{} bytes", error.len());
    }
}

// ------------------------------------------------------------ file access

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("rpc-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn files_are_reachable_only_through_the_sessions_roots() {
    // No roots: no file is written.
    let mut s = session();
    execute(&mut s, "file.new", json!({}));
    execute(&mut s, "feature.block", json!({"size": [1, 2, 3]}));
    let error = call_error(
        &mut s,
        "engine.execute",
        json!({"command": "file.save", "params": {"path": "part.3dcraft"}}),
    );
    assert!(error.contains("no write access"), "{error}");

    // A write root: saving works beneath it and nowhere else.
    let dir = workdir("roots");
    let mut s = Session::new(Workspace {
        read: Access::None,
        write: Access::Root(dir.clone()),
    });
    execute(&mut s, "file.new", json!({}));
    execute(&mut s, "feature.block", json!({"size": [1, 2, 3]}));
    execute(&mut s, "file.save", json!({"path": "part.3dcraft"}));
    let bytes = std::fs::read(dir.join("part.3dcraft")).unwrap();
    assert!(
        bytes.starts_with(b"PK"),
        "a .3dcraft file is a zip container"
    );
    let error = call_error(
        &mut s,
        "engine.execute",
        json!({"command": "file.save", "params": {"path": "../escape.3dcraft"}}),
    );
    assert!(error.contains("write root"), "{error}");
    assert!(!dir.parent().unwrap().join("escape.3dcraft").exists());
    // Reading has its own root, and none was given.
    let error = call_error(
        &mut s,
        "engine.execute",
        json!({"command": "file.open", "params": {"path": "part.3dcraft"}}),
    );
    assert!(error.contains("no read access"), "{error}");
}

// ------------------------------------------------------------- serve loop

fn serve(session: &mut Session, input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    rpc::serve(session, input, &mut output).unwrap();
    let text = String::from_utf8(output).expect("replies are UTF-8");
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "every reply ends with a newline"
    );
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
        .collect()
}

#[test]
fn serve_answers_every_line_and_survives_every_kind_of_bad_line() {
    let mut input = Vec::new();
    input.extend_from_slice(
        b"{\"id\": 1, \"method\": \"engine.execute\", \"params\": {\"command\": \"file.new\"}}\n",
    );
    input.extend_from_slice(b"\n   \n");
    input.extend_from_slice(b"garbage\r\n");
    // Not UTF-8.
    input.extend_from_slice(b"{\"id\": 2, \"method\": \"\xff\xfe\"}\n");
    // Longer than the limit, and not JSON either.
    input.extend_from_slice(&vec![b'['; MAX_REQUEST_BYTES + 10]);
    input.extend_from_slice(b"\n");
    input.extend_from_slice(b"{\"id\": 3, \"method\": \"nope\"}\r\n");
    input.extend_from_slice(
        b"{\"id\": 4, \"method\": \"engine.execute\", \"params\": {\"command\": \"feature.sphere\", \"params\": {\"radius\": 2}}}\n",
    );
    // The last line has no newline.
    input.extend_from_slice(b"{\"id\": 5, \"method\": \"engine.execute\", \"params\": {\"command\": \"document.inspect\"}}");

    let mut s = session();
    let replies = serve(&mut s, &input);
    assert_eq!(
        replies.len(),
        7,
        "one reply per non-blank line: {replies:#?}"
    );
    assert_eq!(
        replies[0],
        json!({"id": 1, "ok": true, "result": {"document": 0, "units": "mm"}})
    );
    assert_eq!(replies[1]["id"], Value::Null);
    assert!(
        replies[1]["error"]
            .as_str()
            .unwrap()
            .contains("not valid JSON")
    );
    assert_eq!(replies[2]["id"], Value::Null);
    assert!(
        replies[2]["error"]
            .as_str()
            .unwrap()
            .contains("not valid UTF-8")
    );
    assert_eq!(replies[3]["id"], Value::Null);
    assert!(
        replies[3]["error"]
            .as_str()
            .unwrap()
            .contains("longer than the limit")
    );
    assert_eq!(replies[4]["id"], 3);
    assert_eq!(replies[4]["ok"], false);
    assert_eq!(replies[5]["result"]["status"], "ok");
    // The document built before and after the bad lines is intact.
    assert_eq!(replies[6]["id"], 5);
    assert_eq!(replies[6]["result"]["bodies"].as_array().unwrap().len(), 1);
}

#[test]
fn serve_returns_when_the_input_ends_and_reports_a_failed_write() {
    let mut s = session();
    assert!(serve(&mut s, b"").is_empty());

    /// A pipe whose reader has gone away.
    struct Closed;
    impl std::io::Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let error = rpc::serve(&mut s, &b"{\"method\": \"methods\"}\n"[..], Closed).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
}

#[test]
fn serve_flushes_each_reply_before_reading_the_next_request() {
    use std::cell::RefCell;
    use std::rc::Rc;

    // Both ends log to one journal, so the test sees the interleaving.
    #[derive(Clone, Default)]
    struct Journal(Rc<RefCell<Vec<String>>>);
    struct In(Journal, std::io::Cursor<Vec<u8>>);
    impl std::io::Read for In {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            // One byte at a time: a pipe gives no more than is there.
            let n = self.1.read(&mut buffer[..1])?;
            if n == 1 && buffer[0] == b'\n' {
                self.0.0.borrow_mut().push("request".into());
            }
            Ok(n)
        }
    }
    struct Out(Journal);
    impl std::io::Write for Out {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.0.borrow_mut().push("flush".into());
            Ok(())
        }
    }
    let journal = Journal::default();
    let input = b"{\"id\":1,\"method\":\"methods\"}\n{\"id\":2,\"method\":\"methods\"}\n".to_vec();
    let reader =
        std::io::BufReader::with_capacity(1, In(journal.clone(), std::io::Cursor::new(input)));
    rpc::serve(&mut session(), reader, Out(journal.clone())).unwrap();
    assert_eq!(
        *journal.0.borrow(),
        ["request", "flush", "request", "flush"]
    );
}
