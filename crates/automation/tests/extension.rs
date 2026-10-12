//! A host's own methods on the JSON-lines protocol (`rpc::handle_line_with`),
//! as the desktop app adds `ui.*`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_automation::rpc::{self, Extension, Method};
use craft3d_automation::{Line, read_line};
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

const EXTRA: &[Method] = &[
    Method {
        name: "host.echo",
        params: r#"{"text": string}"#,
        summary: "Returns its params and whether a document is open.",
    },
    Method {
        name: "host.fail",
        params: "{}",
        summary: "Always fails.",
    },
    Method {
        name: "host.panic",
        params: "{}",
        summary: "Panics.",
    },
];

/// Sends `line` with a host that counts how often it was asked.
fn send(session: &mut Session, calls: &mut Vec<String>, line: &str) -> Value {
    let mut handle = |session: &mut Session, method: &str, params: Option<&Value>| {
        calls.push(method.to_string());
        match method {
            "host.echo" => Some(Ok(json!({
                "params": params,
                "document": session.has_document(),
            }))),
            "host.fail" => Some(Err("host.fail: no".to_string())),
            "host.panic" => panic!("the host fell over"),
            _ => None,
        }
    };
    let mut extension = Extension {
        methods: EXTRA,
        handle: &mut handle,
    };
    let reply = rpc::handle_line_with(session, line, &mut extension).expect("a reply");
    serde_json::from_str(&reply).expect("the reply is JSON")
}

#[test]
fn a_host_method_uses_the_same_envelope() {
    let mut session = Session::new(Workspace::none());
    let mut calls = Vec::new();
    let reply = send(
        &mut session,
        &mut calls,
        r#"{"id": 5, "method": "host.echo", "params": {"text": "hi"}}"#,
    );
    assert_eq!(
        reply,
        json!({"id": 5, "ok": true,
               "result": {"params": {"text": "hi"}, "document": false}})
    );
    let reply = send(
        &mut session,
        &mut calls,
        r#"{"id": "x", "method": "host.fail"}"#,
    );
    assert_eq!(
        reply,
        json!({"id": "x", "ok": false, "error": "host.fail: no"})
    );
    assert_eq!(calls, ["host.echo", "host.fail"]);
}

#[test]
fn the_protocols_own_methods_never_reach_the_host() {
    let mut session = Session::new(Workspace::none());
    let mut calls = Vec::new();
    let reply = send(
        &mut session,
        &mut calls,
        r#"{"id": 1, "method": "engine.execute", "params": {"command": "file.new"}}"#,
    );
    assert_eq!(reply["ok"], true, "{reply}");
    // The host's method runs on the same session: it sees the document.
    let reply = send(
        &mut session,
        &mut calls,
        r#"{"id": 2, "method": "host.echo"}"#,
    );
    assert_eq!(reply["result"]["document"], true);
    let reply = send(
        &mut session,
        &mut calls,
        r#"{"id": 3, "method": "batch", "params": {"steps": []}}"#,
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(calls, ["host.echo"]);
}

#[test]
fn methods_lists_the_hosts_methods_and_an_unknown_one_names_them() {
    let mut session = Session::new(Workspace::none());
    let mut calls = Vec::new();
    let reply = send(
        &mut session,
        &mut calls,
        r#"{"id": 1, "method": "methods"}"#,
    );
    let names: Vec<&str> = reply["result"]["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "engine.execute",
            "engine.commands",
            "batch",
            "methods",
            "host.echo",
            "host.fail",
            "host.panic"
        ]
    );
    assert!(calls.is_empty(), "`methods` is the protocol's own");

    let reply = send(
        &mut session,
        &mut calls,
        r#"{"id": 2, "method": "host.nope"}"#,
    );
    assert_eq!(reply["ok"], false);
    let error = reply["error"].as_str().unwrap();
    assert!(
        error.contains("unknown method `host.nope`") && error.contains("host.echo"),
        "{error}"
    );
    // The host was asked, and said it is not its method.
    assert_eq!(calls, ["host.nope"]);

    // Without a host the list is the protocol's own, as before.
    let plain: Value = serde_json::from_str(
        &rpc::handle_line(&mut session, r#"{"id": 3, "method": "methods"}"#).unwrap(),
    )
    .unwrap();
    assert_eq!(plain["result"]["methods"].as_array().unwrap().len(), 4);
    let plain: Value = serde_json::from_str(
        &rpc::handle_line(&mut session, r#"{"id": 4, "method": "host.echo"}"#).unwrap(),
    )
    .unwrap();
    assert_eq!(plain["ok"], false);
}

#[test]
fn a_panic_in_the_host_is_an_error_reply_and_the_session_keeps_its_document() {
    let mut session = Session::new(Workspace::none());
    let mut calls = Vec::new();
    send(
        &mut session,
        &mut calls,
        r#"{"id": 1, "method": "engine.execute", "params": {"command": "file.new"}}"#,
    );
    let reply = send(
        &mut session,
        &mut calls,
        r#"{"id": 2, "method": "host.panic"}"#,
    );
    assert_eq!(reply["id"], 2);
    assert_eq!(reply["ok"], false);
    let error = reply["error"].as_str().unwrap();
    assert!(
        error.contains("internal error") && error.contains("the host fell over"),
        "{error}"
    );
    let reply = send(
        &mut session,
        &mut calls,
        r#"{"id": 3, "method": "host.echo"}"#,
    );
    assert_eq!(reply["result"]["document"], true);
}

#[test]
fn the_envelope_is_checked_before_the_host_sees_anything() {
    let mut session = Session::new(Workspace::none());
    let mut calls = Vec::new();
    for bad in [
        "not json",
        "[1, 2]",
        r#"{"id": 1}"#,
        r#"{"id": 1, "method": 7}"#,
        r#"{"id": 1, "method": "host.echo", "extra": true}"#,
    ] {
        let reply = send(&mut session, &mut calls, bad);
        assert_eq!(reply["ok"], false, "{bad}");
    }
    assert!(calls.is_empty());
}

#[test]
fn the_reply_builders_and_the_line_reader_are_the_protocols_own() {
    assert_eq!(
        serde_json::from_str::<Value>(&rpc::ok_reply(&json!(4), &json!({"a": 1}))).unwrap(),
        json!({"id": 4, "ok": true, "result": {"a": 1}})
    );
    assert_eq!(
        serde_json::from_str::<Value>(&rpc::error_reply(&Value::Null, "why")).unwrap(),
        json!({"id": null, "ok": false, "error": "why"})
    );
    for reply in [rpc::too_long_reply(), rpc::not_utf8_reply()] {
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!((&reply["id"], &reply["ok"]), (&Value::Null, &json!(false)));
    }
    let mut input = std::io::Cursor::new(b"short\nmuch too long\nlast".to_vec());
    assert_eq!(
        read_line(&mut input, 8).unwrap(),
        Line::Bytes(b"short".to_vec())
    );
    assert_eq!(read_line(&mut input, 8).unwrap(), Line::TooLong);
    assert_eq!(
        read_line(&mut input, 8).unwrap(),
        Line::Bytes(b"last".to_vec())
    );
    assert_eq!(read_line(&mut input, 8).unwrap(), Line::End);
}
