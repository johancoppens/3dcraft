//! A program that took the window's port (another user's, or one that
//! came after a crashed window) learns nothing from the bridge: the bridge
//! asks it to prove it holds the token first and sends nothing else when
//! it cannot (security review, 2026-11-02).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_automation::bridge::{self, BridgeConfig};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener};

#[test]
fn an_impostor_on_the_port_never_sees_the_token() {
    let token = "5ca1ab1e".repeat(8);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    // The impostor answers every line as a window would, without the token:
    // a nonce and a made-up proof; and keeps everything it was sent.
    let impostor = std::thread::spawn(move || {
        let mut heard = String::new();
        if let Ok((stream, _)) = listener.accept() {
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            for _ in 0..4 {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                heard.push_str(&line);
                let id = serde_json::from_str::<Value>(&line)
                    .ok()
                    .and_then(|v| v.get("id").cloned())
                    .unwrap_or(Value::Null);
                let reply = json!({"id": id, "ok": true, "result": {
                    "nonce": "ab".repeat(32), "proof": "00".repeat(32),
                    "authenticated": true, "reply": null}});
                let _ = writeln!(writer, "{reply}");
            }
        }
        heard
    });
    let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let mut output = Vec::new();
    bridge::serve(
        &BridgeConfig::new(address, token.clone()),
        format!("{message}\n").as_bytes(),
        &mut output,
    )
    .unwrap();
    let heard = impostor.join().unwrap();
    assert!(!heard.contains(&token), "the token went out: {heard}");
    assert!(
        !heard.contains("\"auth\""),
        "nothing after the challenge: {heard}"
    );
    assert!(heard.contains("\"challenge\""), "{heard}");
    let reply: Value = serde_json::from_slice(&output).unwrap();
    let error = reply["error"]["message"].as_str().unwrap();
    assert!(error.contains("nothing was sent"), "{error}");
}
