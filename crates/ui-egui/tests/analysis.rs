//! Surface analysis in the window (docs/design/surfacing.md, stage 1):
//! `ui.set {"analysis": …}` switches it, `ui.get` reports it,
//! `ui.screenshot` (the CPU oracle) draws it with its combs; bad analyses
//! are refused and change nothing; null turns it off.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{Shell, Tokens};
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

fn decode(shot: &Value) -> Vec<u8> {
    let text = shot["base64"].as_str().unwrap();
    let value = |c: u8| match c {
        b'A'..=b'Z' => u32::from(c - b'A'),
        b'a'..=b'z' => u32::from(c - b'a') + 26,
        b'0'..=b'9' => u32::from(c - b'0') + 52,
        b'+' => 62,
        b'/' => 63,
        other => panic!("not base64: {other}"),
    };
    let mut bytes = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let data: Vec<u8> = chunk.iter().copied().filter(|&c| c != b'=').collect();
        let mut n = 0u32;
        for (k, &c) in data.iter().enumerate() {
            n |= value(c) << (18 - 6 * k);
        }
        for k in 0..data.len().saturating_sub(1) {
            bytes.push((n >> (16 - 8 * k)) as u8);
        }
    }
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    pixels
}

fn count(rgb: &[u8], color: [u8; 3]) -> usize {
    rgb.chunks(3).filter(|p| *p == color).count()
}

fn torus(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    run(
        session,
        "feature.torus",
        json!({"name": "Ring", "major": "20 mm", "minor": "6 mm"}),
    );
}

#[test]
fn the_window_draws_an_analysis_and_its_combs() {
    let mut session = Session::new(Workspace::none());
    torus(&mut session);
    let mut shell = Shell::new();
    let tokens = Tokens::default();
    let size = json!({"width": 240, "height": 160});
    let plain = decode(&call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        size.clone(),
    ));
    assert_eq!(count(&plain, tokens.analysis_dark), 0);

    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"analysis": {"mode": "zebra", "stripes": 24}}),
    );
    let state = call(&mut shell, &mut session, "ui.get", json!({}));
    let analysis = &state["state"]["analysis"];
    assert_eq!(analysis["mode"], "zebra", "{state}");
    assert_eq!(analysis["stripes"], 24.0, "{state}");
    let zebra = decode(&call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        size.clone(),
    ));
    let (dark, light) = (
        count(&zebra, tokens.analysis_dark),
        count(&zebra, tokens.analysis_light),
    );
    assert!(dark > 500 && light > 500, "{dark} dark, {light} light");

    // Combs on every edge: none on a torus (it has none); on an iso-curve.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"analysis": {"combs": {"iso": [{"face": "F1:face(0)", "u": 0.1}, {"face": "F1:face(0)", "v": 0.25}]}}}),
    );
    let combed = decode(&call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        size.clone(),
    ));
    // The lines are smoothed: pixels near the comb's colour.
    let near = |p: &[u8]| p.iter().zip(tokens.comb).all(|(&a, b)| a.abs_diff(b) <= 40);
    let before = zebra.chunks(3).filter(|p| near(p)).count();
    let after = combed.chunks(3).filter(|p| near(p)).count();
    assert!(
        after > before + 20,
        "the comb is drawn: {before} -> {after}"
    );

    // The window's curvature map is drawn on the mesh refined where the
    // curvature changes fast (`analysis::refined_map`, as view.render):
    // more triangles than the scene's.
    let triangles = |shell: &mut Shell, session: &mut Session, mode: &str| {
        call(
            shell,
            session,
            "ui.set",
            json!({"analysis": {"mode": mode, "combs": null}}),
        );
        call(shell, session, "ui.screenshot", size.clone())["triangles"]
            .as_u64()
            .unwrap()
    };
    let scene_triangles = triangles(&mut shell, &mut session, "zebra");
    let map_triangles = triangles(&mut shell, &mut session, "curvature");
    assert!(
        map_triangles > scene_triangles,
        "the map's mesh is refined: {scene_triangles} -> {map_triangles}"
    );

    // The curvature map and draft are drawn too.
    for mode in ["curvature", "draft", "highlight"] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"analysis": {"mode": mode, "combs": null}}),
        );
        let shot = decode(&call(
            &mut shell,
            &mut session,
            "ui.screenshot",
            size.clone(),
        ));
        assert_ne!(shot, plain, "{mode}");
    }

    // A bad analysis is refused and changes nothing.
    let before = call(&mut shell, &mut session, "ui.get", json!({}));
    for bad in [
        json!({"mode": "rainbow"}),
        json!({"stripes": -1}),
        json!({"pull": [0, 0, 0]}),
    ] {
        let refused = shell
            .handle_method(&mut session, "ui.set", Some(&json!({"analysis": bad})))
            .unwrap();
        assert!(refused.is_err(), "{bad}");
    }
    let after = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(before["state"]["analysis"], after["state"]["analysis"]);

    // Null turns it off: the plain picture again.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"analysis": null}),
    );
    let off = decode(&call(&mut shell, &mut session, "ui.screenshot", size));
    assert_eq!(off, plain);
}
