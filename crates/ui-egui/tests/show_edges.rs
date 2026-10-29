//! `state.show.edges`: the bodies' edges drawn in the viewport or not, set
//! with `ui.set {"show": {"edges": false}}` and by the Edges toggle over
//! the viewport, honoured by the CPU overlay, the GPU's lines and
//! `ui.screenshot`. With the edges shown the picture is what it was before
//! the toggle existed, bit for bit; hidden, only the pixels near an edge
//! change; a highlighted edge is drawn either way. And the studio look's
//! `look.edges` (`soft`, `sharp`, `off`): `soft` is `sharp` without a lens.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Scene, SceneQuality, Session, Workspace};
use craft3d_render::Highlight;
use craft3d_ui_egui::state::{CameraState, EdgeRef};
use craft3d_ui_egui::viewport::{self, EdgeBlur, LineKind, Marks, Quality};
use craft3d_ui_egui::{Shell, Tokens};
use serde_json::{Value, json};
use std::sync::Arc;

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

fn refused(shell: &mut Shell, session: &mut Session, params: Value) -> String {
    shell
        .handle_method(session, "ui.set", Some(&params))
        .unwrap()
        .unwrap_err()
}

/// A plate with a post on it.
fn part() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Post", "position": [15, 20, 8], "radius": 6, "height": 30}),
    );
    s
}

fn screenshot(shell: &mut Shell, session: &mut Session) -> Value {
    call(
        shell,
        session,
        "ui.screenshot",
        json!({"width": 320, "height": 240, "scale": 1.0}),
    )
}

fn scene(session: &mut Session) -> Arc<Scene> {
    session
        .scene_with(SceneQuality::display())
        .unwrap()
        .unwrap()
}

/// The first edge of the plate, as a highlight names it.
fn plate_edge(session: &mut Session) -> EdgeRef {
    let listed = run(session, "document.edges", json!({"feature": "Plate"}));
    let edge = &listed["edges"][0];
    let faces: Vec<String> = edge["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    match edge.get("name").and_then(Value::as_str) {
        Some(name) => EdgeRef {
            feature: Some("Plate".to_string()),
            edge: Some(name.to_string()),
            between: Vec::new(),
        },
        None => EdgeRef {
            feature: Some("Plate".to_string()),
            edge: None,
            between: faces,
        },
    }
}

#[test]
fn ui_set_hides_and_shows_the_edges_and_shown_is_the_picture_of_before() {
    let mut session = part();
    let mut shell = Shell::new();
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["show"]["edges"], json!(true), "the default");
    assert_eq!(got["state"]["look"]["edges"], json!("soft"), "the default");
    let before = screenshot(&mut shell, &mut session);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"edges": false}}),
    );
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        got["state"]["show"],
        json!({"datums": true, "sketches": true, "edges": false})
    );
    let hidden = screenshot(&mut shell, &mut session);
    let coverage = |shot: &Value| shot["overlayCoverage"].as_f64().unwrap();
    assert!(
        coverage(&before) > coverage(&hidden),
        "{} against {}",
        coverage(&before),
        coverage(&hidden)
    );
    assert_ne!(before["base64"], hidden["base64"]);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"edges": true}}),
    );
    let again = screenshot(&mut shell, &mut session);
    assert_eq!(before["base64"], again["base64"], "bit for bit");
    let error = refused(&mut shell, &mut session, json!({"show": {"edges": "no"}}));
    assert!(error.contains("ui.set"), "{error}");
    assert!(shell.state.show.edges, "a refused patch changes nothing");
}

/// Hidden, the picture changes only near the edges: every pixel that
/// differs lies within the widest line's reach of an edge's line.
#[test]
fn hiding_the_edges_changes_the_picture_only_where_they_were() {
    let mut session = part();
    let scene = scene(&mut session);
    let tokens = Tokens::default();
    let (w, h) = (400_u32, 300_u32);
    let mut camera = CameraState::default();
    camera.fit(&scene.bbox, f64::from(w) / f64::from(h));
    let quality = Quality {
        width: w,
        height: h,
        ssaa: 1,
    };
    let picture = |marks: &Marks| {
        viewport::picture_marked(
            Some(&scene),
            &camera,
            quality,
            &tokens,
            None,
            Highlight::NONE,
            marks,
            1.0,
            1.0,
        )
        .unwrap()
        .rgb
    };
    let shown = picture(&Marks::default());
    let hidden = picture(&Marks {
        hide_edges: true,
        ..Marks::default()
    });
    let lines: Vec<_> =
        viewport::overlay_untested(Some(&scene), &camera, w, h, None, &Marks::default())
            .lines
            .into_iter()
            .filter(|l| l.edge.is_some())
            .collect();
    assert!(!lines.is_empty());
    let reach = f64::from(tokens.edge_silhouette_width.max(tokens.edge_width)) * 0.5 + 1.5;
    let near_an_edge = |x: f64, y: f64| {
        lines.iter().any(|l| {
            let (dx, dy) = (l.to[0] - l.from[0], l.to[1] - l.from[1]);
            let l2 = dx * dx + dy * dy;
            let t = if l2 > 0.0 {
                (((x - l.from[0]) * dx + (y - l.from[1]) * dy) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (px, py) = (l.from[0] + dx * t, l.from[1] + dy * t);
            (x - px).hypot(y - py) <= reach
        })
    };
    let mut changed = 0;
    for (k, (a, b)) in shown.chunks(3).zip(hidden.chunks(3)).enumerate() {
        if a != b {
            changed += 1;
            let (x, y) = ((k % w as usize) as f64 + 0.5, (k / w as usize) as f64 + 0.5);
            assert!(
                near_an_edge(x, y),
                "pixel ({x}, {y}) changed far from an edge"
            );
        }
    }
    assert!(changed > 200, "the edges were drawn: {changed}");
}

/// A highlighted edge is feedback, not a line of the model: drawn with
/// the edges hidden, on both paths and in `ui.screenshot`.
#[test]
fn a_highlighted_edge_still_shows_with_the_edges_hidden() {
    let mut session = part();
    let scene = scene(&mut session);
    let tokens = Tokens::default();
    let edge = plate_edge(&mut session);
    let hidden = Marks {
        hide_edges: true,
        ..viewport::edge_marks(Some(&scene), &edge)
    };
    assert!(!hidden.lines.is_empty(), "the edge is found");
    // The GPU's lines: no edge of a body, the highlight still there.
    let segments =
        craft3d_ui_egui::direct::segments_marked(Some(&scene), None, &tokens, 1.0, &hidden, false);
    assert!(segments.iter().all(|s| s.edge.is_none()));
    let highlight = LineKind::Highlight.color(&tokens);
    assert!(segments.iter().any(|s| s.color == highlight));
    let all = craft3d_ui_egui::direct::segments_marked(
        Some(&scene),
        None,
        &tokens,
        1.0,
        &Marks::default(),
        false,
    );
    assert!(all.iter().any(|s| s.edge.is_some()));
    // The CPU overlay.
    let mut camera = CameraState::default();
    camera.fit(&scene.bbox, 4.0 / 3.0);
    let overlay = viewport::overlay_untested(Some(&scene), &camera, 400, 300, None, &hidden);
    assert!(overlay.lines.iter().all(|l| l.kind != LineKind::Edge));
    assert!(overlay.lines.iter().any(|l| l.kind == LineKind::Highlight));
    // ui.screenshot.
    let mut shell = Shell::new();
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"edges": false}}),
    );
    let bare = screenshot(&mut shell, &mut session);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"highlightEdge": serde_json::to_value(&edge).unwrap()}),
    );
    let marked = screenshot(&mut shell, &mut session);
    let coverage = |shot: &Value| shot["overlayCoverage"].as_f64().unwrap();
    assert!(
        coverage(&marked) > coverage(&bare),
        "{} against {}",
        coverage(&marked),
        coverage(&bare)
    );
}

/// The two drawing paths leave the bodies' edges and the section's
/// outline out when the marks say so, and draw what they drew before
/// otherwise.
#[test]
fn both_drawing_paths_honour_the_toggle() {
    let mut session = part();
    let scene = scene(&mut session);
    let tokens = Tokens::default();
    let plain = craft3d_ui_egui::direct::segments(Some(&scene), None, &tokens, 1.0);
    let with = craft3d_ui_egui::direct::segments_marked(
        Some(&scene),
        None,
        &tokens,
        1.0,
        &Marks::default(),
        false,
    );
    assert_eq!(plain, with);
    let hidden = Marks {
        hide_edges: true,
        ..Marks::default()
    };
    let without =
        craft3d_ui_egui::direct::segments_marked(Some(&scene), None, &tokens, 1.0, &hidden, false);
    assert!(without.iter().all(|s| s.edge.is_none()));
    // The world axes are still drawn.
    assert!(without.iter().any(|s| s.axis));
    let mut camera = CameraState::default();
    camera.fit(&scene.bbox, 4.0 / 3.0);
    let edges = |marks: &Marks| {
        viewport::overlay_untested(Some(&scene), &camera, 400, 300, None, marks)
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Edge)
            .count()
    };
    assert!(edges(&Marks::default()) > 0);
    assert_eq!(edges(&hidden), 0);
    // A section's outline is drawn as edges: hidden with them.
    let section = craft3d_render::Section {
        origin: scene.bbox.center().unwrap(),
        normal: craft3d_math::Vec3::X,
        cap: true,
        color: [200, 60, 60],
    };
    let cut = viewport::SectionCut::new(&scene, section).unwrap();
    let cut_open = Marks {
        cut: Some(cut),
        ..Marks::default()
    };
    assert!(!cut_open.outline().is_empty());
    assert!(
        Marks {
            hide_edges: true,
            ..cut_open
        }
        .outline()
        .is_empty()
    );
}

/// `look.edges` is checked and kept; in the plain look it changes nothing.
#[test]
fn the_looks_edges_are_checked_and_the_plain_look_ignores_them() {
    let mut session = part();
    let mut shell = Shell::new();
    let before = screenshot(&mut shell, &mut session);
    let error = refused(
        &mut shell,
        &mut session,
        json!({"look": {"edges": "blurry"}}),
    );
    assert!(error.contains("look.edges"), "{error}");
    let error = refused(&mut shell, &mut session, json!({"look": {"edges": 3}}));
    assert!(error.contains("ui.set"), "{error}");
    assert_eq!(shell.state.look.edges, "soft", "a refusal changes nothing");
    for edges in ["off", "sharp", "soft"] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"look": {"edges": edges}}),
        );
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        assert_eq!(got["state"]["look"]["edges"], json!(edges));
        let shot = screenshot(&mut shell, &mut session);
        assert_eq!(
            before["base64"], shot["base64"],
            "plain look, edges {edges}"
        );
    }
}

/// In the studio look: `off` leaves the edges out, `soft` is `sharp`
/// bit for bit without a lens, and with a lens the edges fade.
#[test]
fn the_studio_look_draws_its_edges_as_it_says() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut shot = |shell: &mut Shell, look: Value| {
        call(shell, &mut session, "ui.set", json!({"look": look}));
        screenshot(shell, &mut session)
    };
    let look = |edges: &str, dof: Value| json!({"mode": "studio", "edges": edges, "dof": dof});
    let sharp = shot(&mut shell, look("sharp", Value::Null));
    let soft = shot(&mut shell, look("soft", Value::Null));
    assert_eq!(sharp["base64"], soft["base64"], "no lens: bit for bit");
    let off = shot(&mut shell, look("off", Value::Null));
    let coverage = |shot: &Value| shot["overlayCoverage"].as_f64().unwrap();
    assert!(coverage(&off) < coverage(&sharp));
    let lens = json!({"fstop": 0.5, "focus": [15, 20, 38]});
    let sharp = shot(&mut shell, look("sharp", lens.clone()));
    let soft = shot(&mut shell, look("soft", lens.clone()));
    let off = shot(&mut shell, look("off", lens));
    assert_ne!(sharp["base64"], soft["base64"], "the lens fades the edges");
    assert!(
        coverage(&off) < coverage(&soft) && coverage(&soft) <= coverage(&sharp),
        "{} {} {}",
        coverage(&off),
        coverage(&soft),
        coverage(&sharp)
    );
}

/// The fade: 1 in focus, falling with the blur, the cap behind the eye,
/// and along a line evenly in the reciprocal of the distance.
#[test]
fn an_edge_keeps_less_of_its_ink_the_more_the_lens_blurs_it() {
    let blur = EdgeBlur {
        scale: 4.0,
        focus: 100.0,
        cap: 20.0,
    };
    assert_eq!(blur.radius(100.0), 0.0);
    assert_eq!(blur.fade(100.0, 2.0), 1.0);
    // 4 · |200 − 100| / 200 = 2 pixels: 2 / (2 + π).
    assert!((blur.radius(200.0) - 2.0).abs() < 1e-12);
    let expected = 2.0 / (2.0 + std::f64::consts::PI);
    assert!((blur.fade(200.0, 2.0) - expected).abs() < 1e-12);
    assert!(blur.fade(400.0, 2.0) < blur.fade(200.0, 2.0));
    assert!(
        blur.fade(200.0, 4.0) > blur.fade(200.0, 2.0),
        "wider keeps more"
    );
    assert_eq!(blur.radius(-1.0), 20.0);
    assert_eq!(blur.radius(1e-9), 20.0, "capped");
    assert_eq!(blur.radius(f64::NAN), 0.0);
    assert!(blur.fade(150.0, f64::NAN) >= 0.0);
    // Halfway along a line from 100 to 300 on the screen is where
    // 1/z is halfway: z = 150.
    let halfway = blur.fade_along([100.0, 300.0], 0.5, 2.0);
    assert!((halfway - blur.fade(150.0, 2.0)).abs() < 1e-12);
    assert_eq!(blur.fade_along([100.0, 300.0], -3.0, 2.0), 1.0, "clamped");
}
