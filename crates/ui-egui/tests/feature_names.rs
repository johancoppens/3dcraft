//! The window's own methods that take face names (`ui.highlight`, the
//! section plane of `ui.set`) take a feature's name for its id, as the
//! commands do (`Plate:face(1)`), and keep ids in the state.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

fn refused(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> String {
    match shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
    {
        Ok(value) => panic!("{method} {params} should fail, returned {value}"),
        Err(e) => e,
    }
}

/// A block with a ball cut out of it (`F3`, "Hollow", of `F1` "Block").
fn part() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Block", "size": [60, 40, 20]}),
    );
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Ball", "position": [30, 20, 10], "radius": 7}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Ball", "name": "Hollow"}),
    );
    s
}

#[test]
fn the_highlight_and_the_section_take_feature_names() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let by_ids = call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": "F3:copy(0)/F1:face(1)", "feature": "F3"}),
    );
    let by_names = call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": "Hollow:copy(0)/Block:face(1)", "feature": "Hollow"}),
    );
    assert_eq!(
        by_names["highlight"]["face"], by_ids["highlight"]["face"],
        "{by_names}"
    );
    assert_eq!(by_names["faces"], by_ids["faces"], "{by_names}");
    let edge = call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"between": ["Hollow:copy(0)/Block:face(1)", "Hollow:copy(0)/Block:face(2)"]}),
    );
    assert!(
        !edge.to_string().contains("Block:face"),
        "the state holds ids: {edge}"
    );
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": true, "plane": "Hollow:copy(0)/Block:face(1)", "offset": -5}}),
    );
    assert_eq!(
        shell.state.section.plane.as_deref(),
        Some("F3:copy(0)/F1:face(1)")
    );
    // A name of no feature: refused, and nothing changes.
    let before = shell.state.clone();
    for (method, params) in [
        ("ui.highlight", json!({"face": "Nobody:face(1)"})),
        ("ui.set", json!({"section": {"plane": "Nobody:face(1)"}})),
    ] {
        let error = refused(&mut shell, &mut session, method, params);
        assert!(error.starts_with(method), "{error}");
        assert!(error.contains("no feature is called `Nobody`"), "{error}");
    }
    assert_eq!(shell.state, before);
}
