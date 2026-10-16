//! Pointing at edges and vertices in the viewport (roadmap M8.5): a click
//! near an edge or a vertex in sight gives it as UI state an agent reads, an agent asks what
//! is at a pixel (`ui.pick`) and has an edge highlighted (`ui.highlight`),
//! and the picture shows the highlight, the same on the GPU's direct path
//! as in the CPU oracle.
//!
//! The oracle for "that edge" is `document.edges`: the entry at the
//! edge's place has that name and those faces. Where an edge is in the
//! picture is worked out here from the chords the scene draws it with and
//! the shell's camera, and whether a point of it is in sight by a ray
//! through the mesh in space (the pick itself uses the renderer's
//! arithmetic in the picture).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Scene, Session, Workspace};
use craft3d_gpu::ADAPTER_HINT;
use craft3d_math::Vec3;
use craft3d_render::Projection;
use craft3d_ui_egui::backend::Backend;
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::state::{CameraState, EdgeRef, PickedKind, VertexRef};
use craft3d_ui_egui::viewport::{self, Composed, LineKind, Marks, OverlayLine, Quality};
use craft3d_ui_egui::{RendererChoice, Shell, Tokens};
use egui::{Event, PointerButton, Pos2, pos2, vec2};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;
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

fn refused(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> String {
    match shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
    {
        Ok(value) => panic!("{method} {params} should fail, returned {value}"),
        Err(e) => {
            assert!(e.starts_with(method), "the error names the method: {e}");
            e
        }
    }
}

/// The part of `pointing.rs`: a plate with a hole drilled through it by a
/// boolean, a post beside it and a row of three studs.
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
        json!({"name": "Drill", "position": [42, 20, -4], "radius": 9, "height": 20}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Drilled"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Post", "position": [12, 20, 8], "radius": 6, "height": 30}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Stud", "size": [6, 6, 10], "position": [0, -20, 0]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Stud", "count": 3, "step": [14, 0, 0], "name": "Studs"}),
    );
    s
}

/// A window of a fixed size, fed one frame at a time.
struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: vec2(width, height),
            time: 0.0,
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    /// Moves to `at`, presses and lets go, and lets the picture settle.
    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            self.frame(
                shell,
                session,
                vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        for _ in 0..3 {
            self.frame(shell, session, Vec::new());
        }
    }
}

/// One edge as `document.edges` lists it, with the chords the scene draws
/// it with.
#[derive(Clone, Debug)]
struct Listed {
    feature: String,
    body: String,
    index: usize,
    name: Option<String>,
    faces: Vec<String>,
    chords: Vec<Vec3>,
}

impl Listed {
    /// The point at the fraction `u` of the length of the chords.
    fn at(&self, u: f64) -> Vec3 {
        let lengths: Vec<f64> = self
            .chords
            .windows(2)
            .map(|w| (w[1] - w[0]).length())
            .collect();
        let mut left = u.clamp(0.0, 1.0) * lengths.iter().sum::<f64>();
        for (pair, length) in self.chords.windows(2).zip(&lengths) {
            if left <= *length && *length > 0.0 {
                return pair[0] + (pair[1] - pair[0]) * (left / length);
            }
            left -= length;
        }
        *self.chords.last().unwrap()
    }

    /// How a highlight refers to it: by its own name, or by its faces.
    fn reference(&self) -> EdgeRef {
        EdgeRef {
            feature: Some(self.feature.clone()),
            edge: self.name.clone(),
            between: match (&self.name, self.faces.as_slice()) {
                (Some(_), _) => Vec::new(),
                (None, [a, b]) => vec![a.clone(), b.clone()],
                (None, [a]) => vec![a.clone(), a.clone()],
                (None, other) => panic!("{other:?}"),
            },
        }
    }
}

/// Every edge of every body of the scene, from `document.inspect` and
/// `document.edges`; the chords are the scene's for the edge at that
/// place, checked to run between the ends `document.edges` gives.
fn listed_edges(session: &mut Session, scene: &Scene) -> Vec<Listed> {
    let inspect = run(session, "document.inspect", json!({}));
    let mut out = Vec::new();
    for body in inspect["bodies"].as_array().unwrap() {
        let name = body["name"].as_str().unwrap();
        let feature = body["feature"].as_str().unwrap();
        let listed = run(session, "document.edges", json!({"feature": name}));
        let scene_body = scene.bodies.iter().find(|b| b.feature == feature).unwrap();
        let edges = listed["edges"].as_array().unwrap();
        assert_eq!(edges.len(), scene_body.edges.len());
        for (index, edge) in edges.iter().enumerate() {
            let chords = scene_body.edges[index].clone();
            let end = |key: &str| {
                let c: Vec<f64> = edge[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect();
                Vec3::new(c[0], c[1], c[2])
            };
            let (first, last) = (chords[0], chords[chords.len() - 1]);
            let direct = (first - end("start")).length() + (last - end("end")).length();
            let swapped = (first - end("end")).length() + (last - end("start")).length();
            assert!(direct.min(swapped) < 1e-9, "{name} edge {index}");
            out.push(Listed {
                feature: feature.to_string(),
                body: name.to_string(),
                index,
                name: edge.get("name").map(|n| n.as_str().unwrap().to_string()),
                faces: edge["faces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|f| f.as_str().unwrap().to_string())
                    .collect(),
                chords,
            });
        }
    }
    out
}

/// True when no triangle of the scene's mesh is between the eye and `p`
/// (Möller-Trumbore in space; a triangle met within a millionth of the
/// way of `p` is the surface `p` lies on).
fn in_sight(scene: &Scene, camera: &CameraState, p: Vec3) -> bool {
    let axes = camera.to_render().axes().unwrap();
    let origin = match axes.projection {
        Projection::Perspective { .. } => axes.eye,
        Projection::Orthographic { .. } => p - axes.forward * (p - axes.eye).dot(axes.forward),
    };
    let dir = p - origin;
    let mesh = &scene.mesh;
    !mesh.triangles.iter().any(|tri| {
        let [a, b, c] = tri.map(|k| mesh.positions[k as usize]);
        let (e1, e2) = (b - a, c - a);
        let h = dir.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-300 {
            return false;
        }
        let s = origin - a;
        let u = s.dot(h) / det;
        let q = s.cross(e1);
        let v = dir.dot(q) / det;
        let t = e2.dot(q) / det;
        u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 1e-9 && t < 1.0 - 1e-6
    })
}

/// The edges with a stretch clearly in sight about the fraction `u` of
/// their length (it and the points a little along the edge either way),
/// with where that point lands in a picture of `width` × `height`.
fn edges_in_sight(
    scene: &Scene,
    camera: &CameraState,
    edges: &[Listed],
    [width, height]: [u32; 2],
    u: f64,
) -> Vec<(Listed, Vec3, [f64; 2])> {
    let projector =
        Projector::new(&camera.to_render(), f64::from(width), f64::from(height)).unwrap();
    edges
        .iter()
        .filter_map(|listed| {
            let p = listed.at(u);
            let at = projector.project(p)?;
            let inside = at.x >= 8.0
                && at.y >= 8.0
                && at.x < f64::from(width) - 8.0
                && at.y < f64::from(height) - 8.0;
            let clear = [u - 0.04, u, u + 0.04]
                .into_iter()
                .all(|v| in_sight(scene, camera, listed.at(v)));
            // Further from every vertex than a click reaches: nearer, the
            // vertex is what is pointed at.
            let by_a_vertex = scene.bodies.iter().flat_map(|b| &b.vertices).any(|v| {
                projector
                    .project(*v)
                    .is_some_and(|c| (c.x - at.x).hypot(c.y - at.y) < 6.0)
            });
            (inside && clear && !by_a_vertex).then(|| (listed.clone(), p, [at.x, at.y]))
        })
        .collect()
}

/// Whether a point of the viewport's picture (one pixel per point) is
/// under the controls over the viewport, or near them: a click there is
/// theirs. (Added 2026-10-16, when the view cube grew and the controls
/// went on glass plates: some edges and corners the tests clicked are
/// under them in a 1000 × 700 window.)
fn under_controls(shell: &Shell, at: [f64; 2]) -> bool {
    let rect = shell.viewport_rect().unwrap();
    let p = egui::pos2(rect.left() + at[0] as f32, rect.top() + at[1] as f32);
    shell
        .controls_layout()
        .is_some_and(|l| l.parts().iter().any(|(_, r)| r.expand(8.0).contains(p)))
}

fn evidence(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn write_png(path: &PathBuf, width: u32, height: u32, rgb: &[u8]) {
    let png = craft3d_render::encode_png(width, height, rgb).unwrap();
    std::fs::write(path, png).unwrap();
}

fn pixel(rgb: &[u8], width: u32, p: [f64; 2]) -> [u8; 3] {
    let i = (p[1].floor() as usize * width as usize + p[0].floor() as usize) * 3;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

fn levels(a: [u8; 3], b: [u8; 3]) -> u8 {
    (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap()
}

/// Distance from `p` to the segment `line`, in pixels.
fn distance_to(p: [f64; 2], line: &OverlayLine) -> f64 {
    let (ax, ay) = (line.from[0], line.from[1]);
    let (dx, dy) = (line.to[0] - ax, line.to[1] - ay);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p[0] - ax) * dx + (p[1] - ay) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - (ax + dx * t)).hypot(p[1] - (ay + dy * t))
}

/// The screenshot of the viewport at its own size, as RGB.
fn shot(shell: &mut Shell, session: &mut Session) -> (u32, u32, Vec<u8>) {
    let [width, height] = shell.viewport_size();
    let result = call(
        shell,
        session,
        "ui.screenshot",
        json!({"width": width, "height": height, "scale": 1.0}),
    );
    let png = base64_decode(result["base64"].as_str().unwrap());
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
        .read_info()
        .unwrap();
    let mut rgb = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut rgb).unwrap();
    (width, height, rgb)
}

fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        }
    };
    let bytes: Vec<u8> = text.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut word = 0_u32;
        for (k, &c) in chunk.iter().enumerate() {
            word |= value(c) << (18 - 6 * k);
        }
        for k in 0..chunk.len().saturating_sub(1) {
            out.push((word >> (16 - 8 * k)) as u8);
        }
    }
    out
}

/// How many pixels along the highlight lines of `overlay` that are in
/// sight are the highlight's colour in `rgb`, of how many looked at: the
/// pixel under a point of a line three pixels wide is all ink.
fn inked(
    overlay: &viewport::Overlay,
    rgb: &[u8],
    width: u32,
    height: u32,
    tokens: &Tokens,
) -> (usize, usize) {
    let (mut looked, mut inked) = (0, 0);
    for line in overlay
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Highlight && !l.hidden)
    {
        let length = (line.to[0] - line.from[0]).hypot(line.to[1] - line.from[1]);
        let steps = (length / 2.0).floor() as usize;
        for i in 1..steps {
            let t = i as f64 / steps as f64;
            let p = [
                line.from[0] + (line.to[0] - line.from[0]) * t,
                line.from[1] + (line.to[1] - line.from[1]) * t,
            ];
            if p[0] < 1.0
                || p[1] < 1.0
                || p[0] >= f64::from(width) - 1.0
                || p[1] >= f64::from(height) - 1.0
            {
                continue;
            }
            looked += 1;
            inked += usize::from(levels(pixel(rgb, width, p), tokens.face_highlight) <= 8);
        }
    }
    (looked, inked)
}

#[test]
fn a_click_on_an_edge_picks_it_and_shows_it_highlighted() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let tokens = shell.tokens;
    let mut window = Window::new(1000.0, 700.0);
    for _ in 0..4 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let rect = shell.viewport_rect().unwrap();
    let size = shell.viewport_size();
    let [width, height] = size;
    let scene: Arc<Scene> = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let camera = shell.state.camera;
    let edges = listed_edges(&mut session, &scene);
    let (_, _, plain) = shot(&mut shell, &mut session);
    let seen = edges_in_sight(&scene, &camera, &edges, size, 0.37);
    let seen: Vec<_> = seen
        .into_iter()
        .filter(|s| !under_controls(&shell, s.2))
        .collect();
    assert!(seen.len() >= 20, "{} edges are in sight", seen.len());
    let mut serial = 0;
    let (mut clicked, mut crossings) = (0, 0);
    let mut bodies = BTreeSet::new();
    for (round, (listed, point, [x, y])) in seen.iter().enumerate() {
        // One pixel per point here.
        let at = pos2(rect.left() + *x as f32, rect.top() + *y as f32);
        window.click(&mut shell, &mut session, at);
        assert_eq!(shell.state.camera, camera, "a click is not a drag");
        serial += 1;
        let picked = shell.state.picked.clone().unwrap();
        let context = format!("{} edge {}: {picked:?}", listed.body, listed.index);
        assert_eq!(picked.serial, serial, "{context}");
        assert_eq!(picked.kind, Some(PickedKind::Edge), "{context}");
        let edge = picked.edge.clone().unwrap();
        if (edge.feature.as_str(), edge.index as usize) != (listed.feature.as_str(), listed.index) {
            // Another edge passes through the same spot, nearer to the
            // eye. Counted; rare.
            crossings += 1;
            continue;
        }
        clicked += 1;
        bodies.insert(listed.feature.clone());
        assert_eq!(edge.body, listed.body, "{context}");
        assert_eq!(edge.name, listed.name, "{context}");
        assert_eq!(edge.faces, listed.faces, "{context}");
        let at_edge = Vec3::new(edge.point[0], edge.point[1], edge.point[2]);
        // The click is a float of the position: a hundredth of a pixel.
        assert!(
            (at_edge - *point).length() < 0.05,
            "{context}: {at_edge:?} for {point:?}"
        );
        // The edge is what is highlighted, and no face.
        assert_eq!(
            shell.state.highlight_edge,
            Some(listed.reference()),
            "{context}"
        );
        assert_eq!(shell.state.highlight, None, "{context}");
        // The window draws it: lines of the highlight kind along the
        // chords of that edge, in the overlay it paints.
        let (overlay, overlay_size) = shell.overlay();
        assert_eq!(overlay_size, size);
        let marks: Vec<&OverlayLine> = overlay
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Highlight)
            .collect();
        assert!(!marks.is_empty(), "{context}");
        assert!(
            marks
                .iter()
                .any(|l| !l.hidden && distance_to([*x, *y], l) < 0.01),
            "{context}: no highlight line through the click"
        );
        let projector =
            Projector::new(&camera.to_render(), f64::from(width), f64::from(height)).unwrap();
        for line in &marks {
            for end in [line.from, line.to] {
                // Every end of a highlight line is on that edge's chords.
                let near = listed
                    .chords
                    .windows(2)
                    .filter_map(|w| Some((projector.project(w[0])?, projector.project(w[1])?)))
                    .map(|(a, b)| {
                        distance_to(
                            end,
                            &OverlayLine {
                                from: [a.x, a.y],
                                to: [b.x, b.y],
                                kind: LineKind::Highlight,
                                hidden: false,
                            },
                        )
                    })
                    .fold(f64::INFINITY, f64::min);
                assert!(near < 1e-6, "{context}: a highlight line off the edge");
            }
        }
        // An agent reads the same through ui.get.
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        assert_eq!(got["state"]["picked"]["kind"], "edge", "{context}");
        assert_eq!(got["state"]["picked"]["edge"]["index"], listed.index);
        assert_eq!(
            got["state"]["highlightEdge"],
            serde_json::to_value(listed.reference()).unwrap()
        );
        assert_eq!(got["state"]["highlight"], json!(null));
        // And the screenshot shows it in the highlight's colour, there
        // and nowhere far from it.
        if round % 4 == 0 {
            let (_, _, marked) = shot(&mut shell, &mut session);
            let oracle_overlay = shell.overlay().0.clone();
            let (looked, inked) = inked(&oracle_overlay, &marked, width, height, &tokens);
            assert!(
                looked >= 3,
                "{context}: {looked} points of the edge looked at"
            );
            assert!(
                inked * 10 >= looked * 9,
                "{context}: {inked} of {looked} points are in the highlight's colour"
            );
            let lines: Vec<OverlayLine> = oracle_overlay
                .lines
                .iter()
                .filter(|l| l.kind == LineKind::Highlight)
                .copied()
                .collect();
            for py in (0..height).step_by(3) {
                for px in (0..width).step_by(3) {
                    let p = [f64::from(px) + 0.5, f64::from(py) + 0.5];
                    if lines.iter().all(|l| distance_to(p, l) > 4.0) {
                        assert_eq!(
                            pixel(&marked, width, p),
                            pixel(&plain, width, p),
                            "{context}: the picture changed at {p:?}, away from the edge"
                        );
                    }
                }
            }
            if round == 4 {
                let path = evidence("pointing-edge-click-cpu.png");
                write_png(&path, width, height, &marked);
                println!("evidence: {}", path.display());
            }
        }
    }
    println!("{clicked} edges clicked, {crossings} crossings");
    assert!(
        clicked >= 20 && crossings * 10 <= clicked,
        "{clicked}, {crossings}"
    );
    assert_eq!(bodies.len(), 3, "the boolean, the post and the pattern");

    // A click on a face away from every edge: the face is highlighted and
    // the edge no more.
    let on_face = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": 0.5, "y": 0.5}),
    );
    assert_eq!(on_face["hit"], false, "the corner is background");
    window.click(
        &mut shell,
        &mut session,
        pos2(rect.left() + 3.5, rect.top() + 3.5),
    );
    assert_eq!(shell.state.highlight_edge, None);
    assert_eq!(shell.state.highlight, None);
    assert_eq!(shell.state.picked.as_ref().unwrap().kind, None);
    let (overlay, _) = shell.overlay();
    assert!(overlay.lines.iter().all(|l| l.kind != LineKind::Highlight));
    assert_eq!(shot(&mut shell, &mut session).2, plain);
}

#[test]
fn ui_pick_and_ui_highlight_speak_of_edges() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let tokens = shell.tokens;
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    let size = shell.viewport_size();
    let [width, height] = size;
    let scene: Arc<Scene> = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let camera = shell.state.camera;
    let edges = listed_edges(&mut session, &scene);
    let (_, _, plain) = shot(&mut shell, &mut session);
    let seen = edges_in_sight(&scene, &camera, &edges, size, 0.61);
    assert!(seen.len() >= 20, "{}", seen.len());

    // ui.pick: the edge, as view.pick gives it; with no radius, the face.
    let mut same = 0;
    for (listed, _, [x, y]) in &seen {
        let picked = call(&mut shell, &mut session, "ui.pick", json!({"x": x, "y": y}));
        let context = format!("{} edge {}: {picked}", listed.body, listed.index);
        assert_eq!(picked["kind"], "edge", "{context}");
        assert_eq!(picked["hit"], true, "{context}");
        let faces_only = call(
            &mut shell,
            &mut session,
            "ui.pick",
            json!({"x": x, "y": y, "radius": 0}),
        );
        assert!(faces_only.get("edge").is_none(), "{context}");
        assert_eq!(faces_only.get("face"), picked.get("face"), "{context}");
        // The same place in a picture twice as large, with the radius
        // that goes with it.
        let doubled = call(
            &mut shell,
            &mut session,
            "ui.pick",
            json!({"x": x * 2.0, "y": y * 2.0, "width": width * 2, "height": height * 2}),
        );
        assert_eq!(
            doubled["edge"]["index"], picked["edge"]["index"],
            "{context}"
        );
        assert_eq!(doubled["edge"]["feature"], picked["edge"]["feature"]);
        if picked["edge"]["feature"] == listed.feature.as_str()
            && picked["edge"]["index"] == listed.index
        {
            same += 1;
            assert_eq!(picked["edge"]["body"], listed.body.as_str(), "{context}");
            assert_eq!(
                picked["edge"]["name"].as_str().map(str::to_string),
                listed.name,
                "{context}"
            );
            assert_eq!(picked["edge"]["faces"], json!(listed.faces), "{context}");
            assert!(picked["edge"]["distance"].as_f64().unwrap() < 1e-6);
        }
    }
    assert!(same * 10 >= seen.len() * 9, "{same} of {}", seen.len());
    // Asking is not clicking.
    assert_eq!(shell.state.picked, None);
    assert_eq!(shell.state.highlight_edge, None);

    // With select it is a click on the edge.
    // (An edge of the body a boolean made: named by the faces it lies
    // between, since 2026-10-14.)
    let (listed, _, [x, y]) = seen
        .iter()
        .find(|(l, ..)| l.body == "Drilled")
        .expect("an edge of the drilled plate is in sight");
    assert!(
        listed
            .name
            .as_deref()
            .is_some_and(|n| n.contains("between["))
    );
    let clicked = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x, "y": y, "select": true}),
    );
    assert_eq!(clicked["picked"]["serial"], 1);
    assert_eq!(clicked["picked"]["kind"], "edge");
    assert_eq!(clicked["picked"]["edge"]["index"], listed.index);
    assert_eq!(
        clicked["highlightEdge"],
        serde_json::to_value(listed.reference()).unwrap()
    );
    assert_eq!(clicked["highlight"], json!(null));
    assert_eq!(shell.state.highlight_edge, Some(listed.reference()));

    // ui.highlight by name and by the faces an edge lies between, for
    // every edge: the screenshot has the highlight's colour along what is
    // in sight of it and is the plain picture away from it.
    let mut shown = 0;
    for (round, listed) in edges.iter().enumerate() {
        let reference = listed.reference();
        let mut params = json!({"feature": listed.feature});
        match &reference.edge {
            Some(name) => params["edge"] = json!(name),
            None => params["between"] = json!(reference.between),
        }
        let result = call(&mut shell, &mut session, "ui.highlight", params.clone());
        let context = format!("{params}: {result}");
        assert_eq!(
            result["highlightEdge"],
            serde_json::to_value(&reference).unwrap(),
            "{context}"
        );
        assert_eq!(result["name"], listed.body.as_str(), "{context}");
        let count = result["edges"].as_u64().unwrap();
        // A name is one edge; two faces can have several between them.
        assert!(if listed.name.is_some() {
            count == 1
        } else {
            count >= 1
        });
        assert_eq!(shell.state.highlight_edge, Some(reference.clone()));
        assert_eq!(shell.state.highlight, None);
        if round % 3 != 0 {
            continue;
        }
        let (_, _, marked) = shot(&mut shell, &mut session);
        // What the screenshot drew: the overlay of the same state, made
        // here the way ui.screenshot makes it.
        let frame = viewport::render_bodies(
            Some(&scene),
            &camera,
            Quality {
                width,
                height,
                ssaa: 1,
            },
            &tokens,
        )
        .unwrap();
        let overlay = viewport::overlay_marked(
            Some(&scene),
            &camera,
            &frame,
            None,
            viewport::depth_slack(Backend::Cpu, Some(&scene), &camera),
            &viewport::edge_marks(Some(&scene), &reference),
        );
        let (looked, inked) = inked(&overlay, &marked, width, height, &tokens);
        if looked >= 3 {
            shown += 1;
            assert!(
                inked * 10 >= looked * 9,
                "{context}: {inked} of {looked} points are in the highlight's colour"
            );
        }
        // A stretch behind a body is drawn too, dimmed: the picture is
        // not the plain one there, nor the full colour.
        let hidden: Vec<&OverlayLine> = overlay
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Highlight && l.hidden)
            .collect();
        for line in hidden
            .iter()
            .filter(|l| (l.to[0] - l.from[0]).hypot(l.to[1] - l.from[1]) > 12.0)
        {
            let mid = [
                0.5 * (line.from[0] + line.to[0]),
                0.5 * (line.from[1] + line.to[1]),
            ];
            let crowded = overlay
                .lines
                .iter()
                .any(|other| other.kind != LineKind::Highlight && distance_to(mid, other) < 4.0);
            if crowded || mid[0] < 2.0 || mid[1] < 2.0 {
                continue;
            }
            let (now, before) = (pixel(&marked, width, mid), pixel(&plain, width, mid));
            assert_ne!(now, before, "{context}: nothing drawn at {mid:?}");
            assert!(
                levels(now, tokens.face_highlight) > 30,
                "{context}: a hidden stretch in full colour at {mid:?}"
            );
        }
        if round == 6 {
            let path = evidence("pointing-edge-highlight-cpu.png");
            write_png(&path, width, height, &marked);
            println!("evidence: {}", path.display());
        }
    }
    assert!(shown >= 8, "{shown} highlighted edges were in view");

    // ui.set sets it too, without the checks; a name nobody has is
    // dropped.
    // An edge of another body than the drilled plate (a refusal below
    // asks that plate for it).
    let named = edges
        .iter()
        .find(|l| l.name.is_some() && l.body != "Drilled")
        .unwrap();
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"highlightEdge": {"edge": named.name}}),
    );
    assert_eq!(
        shell.state.highlight_edge,
        Some(EdgeRef {
            feature: None,
            edge: named.name.clone(),
            between: Vec::new()
        })
    );
    assert_ne!(shot(&mut shell, &mut session).2, plain);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"highlightEdge": {"edge": "F99:edge(0)"}}),
    );
    assert_eq!(shell.state.highlight_edge, None);
    // A face and an edge at once, through ui.set.
    let face = scene.bodies[0].face_names[0].clone();
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"highlight": {"face": face}, "highlightEdge": {"edge": named.name}}),
    );
    assert!(shell.state.highlight.is_some() && shell.state.highlight_edge.is_some());
    // ui.highlight points at one thing: a face takes the edge away.
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": face}),
    );
    assert_eq!(shell.state.highlight_edge, None);
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"edge": named.name}),
    );
    assert_eq!(shell.state.highlight, None);

    // What is wrong is said and leaves the highlight as it was.
    let state = shell.state.clone();
    let name = named.name.clone().unwrap();
    let (a, b) = (named.faces[0].clone(), named.faces[1].clone());
    for (params, part) in [
        (json!({"edge": 3}), "`edge` must be an edge name"),
        (json!({"edge": null}), "`edge` must be an edge name"),
        (json!({"edge": "rim"}), "is not a name"),
        (json!({"edge": face}), "is a face"),
        (
            json!({"edge": "F99:edge(0)"}),
            "no body has an edge F99:edge(0)",
        ),
        (
            json!({"edge": name, "feature": "Nobody"}),
            "no body of a feature `Nobody`",
        ),
        (
            json!({"edge": name, "feature": "Drilled"}),
            "`Drilled` has no edge",
        ),
        (
            json!({"edge": name, "face": face}),
            "one of `face`, `edge`, `between`",
        ),
        (json!({"edge": name, "between": [a, b]}), "not both"),
        (json!({"between": [a]}), "takes two face names"),
        (json!({"between": a}), "takes two face names"),
        (json!({"between": [a, 7]}), "takes two face names"),
        (json!({"between": [a, name]}), "is not a face"),
        (
            json!({"between": [a, "F99:face(0)"]}),
            "no body has an edge between",
        ),
        (
            json!({"edge": name, "color": "red"}),
            "unknown parameter `color`",
        ),
        (json!({"edge": "F1:edge(".repeat(20_000)}), "is not a name"),
    ] {
        let error = refused(&mut shell, &mut session, "ui.highlight", params.clone());
        assert!(error.contains(part), "{:.80}: {error}", params.to_string());
        assert!(error.len() < 700, "{}", error.len());
        assert_eq!(shell.state, state);
    }
    let error = refused(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": 1, "y": 1, "radius": -1}),
    );
    assert!(error.contains("`radius`"), "{error}");
    let error = refused(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": 1, "y": 1, "radius": "wide"}),
    );
    assert!(error.contains("`radius`"), "{error}");

    // The highlight follows the edge through an edit that keeps it, and
    // goes when the edge goes; so does what was clicked.
    let post_rim = edges
        .iter()
        .find(|l| l.body == "Post" && l.name.is_some())
        .unwrap();
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"edge": post_rim.name}),
    );
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Post", "height": 45}),
    );
    shell.sync(&mut session);
    assert_eq!(shell.state.highlight_edge, Some(post_rim.reference()));
    run(&mut session, "feature.delete", json!({"feature": "Post"}));
    shell.sync(&mut session);
    assert_eq!(shell.state.highlight_edge, None);
    run(&mut session, "edit.undo", json!({}));
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"edge": post_rim.name}),
    );
    // face: null removes every highlight, as before.
    let cleared = call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": null}),
    );
    assert_eq!(cleared, json!({"highlight": null, "faces": 0}));
    assert_eq!(shell.state.highlight_edge, None);

    // No document: an error that says so.
    run(&mut session, "file.close", json!({"discard": true}));
    let error = refused(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"edge": name}),
    );
    assert!(error.contains("no document is open"), "{error}");
}

/// The window's own path: an edge highlighted in a viewport drawn
/// directly by the GPU is the line the oracle draws, in the same colour,
/// dimmed where the oracle dims it, and the rest of the picture is what
/// it was.
#[test]
fn a_highlighted_edge_is_the_same_on_the_gpus_direct_path_as_in_the_oracle() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let tokens = shell.tokens;
    shell.attach_gpu(&state).unwrap();
    assert!(shell.draws_directly());
    let mut window = Window::new(1000.0, 700.0);
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    let size = shell.viewport_size();
    let [w, h] = size;
    let rect = shell.viewport_rect().unwrap();
    let scene: Arc<Scene> = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let camera = shell.state.camera;
    let edges = listed_edges(&mut session, &scene);
    let (_, _, direct_plain) = shell.direct_picture().unwrap().unwrap();
    let quality = Quality {
        width: w,
        height: h,
        ssaa: 1,
    };
    let cpu_frame = viewport::render_bodies(Some(&scene), &camera, quality, &tokens).unwrap();
    let slack = viewport::depth_slack(Backend::Gpu, Some(&scene), &camera);
    let everything = viewport::overlay(Some(&scene), &camera, w, h, None);
    let seen = edges_in_sight(&scene, &camera, &edges, size, 0.43);
    let seen: Vec<_> = seen
        .into_iter()
        .filter(|s| !under_controls(&shell, s.2))
        .collect();
    assert!(seen.len() >= 20, "{}", seen.len());
    let (mut samples, mut agree, mut inked_n, mut dimmed) = (0, 0, 0, 0);
    let mut renders = shell.render_count();
    let mut log = String::new();
    for (round, (listed, _, [x, y])) in seen.iter().enumerate().step_by(2) {
        // A click, as the developer makes one.
        let at = pos2(rect.left() + *x as f32, rect.top() + *y as f32);
        window.click(&mut shell, &mut session, at);
        let Some(reference) = shell.state.highlight_edge.clone() else {
            panic!(
                "{} edge {}: the click was not on an edge",
                listed.body, listed.index
            );
        };
        assert!(
            shell.render_count() > renders,
            "the click redrew the viewport"
        );
        renders = shell.render_count();
        assert_eq!(shell.render_stats().unwrap().renderer, Backend::Gpu);
        let (_, _, direct) = shell.direct_picture().unwrap().unwrap();
        // The oracle: the CPU's picture with the same marks, the depth
        // decisions taken with the slack the GPU frame gets.
        let marks = viewport::edge_marks(Some(&scene), &reference);
        assert!(!marks.is_empty());
        let overlay =
            viewport::overlay_marked(Some(&scene), &camera, &cpu_frame, None, slack, &marks);
        let oracle = viewport::rasterise(
            Composed {
                width: w,
                height: h,
                rgb: cpu_frame.rgb.clone(),
                overlay: overlay.clone(),
                coverage: cpu_frame.coverage(),
                triangles: scene.triangles(),
                backend: Backend::Cpu,
            },
            &tokens,
            1.0,
        )
        .rgb;
        let highlight: Vec<OverlayLine> = overlay
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Highlight)
            .copied()
            .collect();
        // Along the highlight: the pixel under a point of a line three
        // pixels wide is all ink in both pictures, so the two agree
        // there as the lines of the viewport do (8 levels, 99 %:
        // gpu_viewport.rs), away from the ends of a stretch and from
        // other lines.
        let (mut here, mut here_agree) = (0, 0);
        for line in &highlight {
            let length = (line.to[0] - line.from[0]).hypot(line.to[1] - line.from[1]);
            let steps = (length / 2.0).floor() as usize;
            for i in 1..steps {
                let t = i as f64 / steps as f64;
                let p = [
                    line.from[0] + (line.to[0] - line.from[0]) * t,
                    line.from[1] + (line.to[1] - line.from[1]) * t,
                ];
                let near_end = highlight.iter().any(|s| {
                    s.hidden != line.hidden
                        && [s.from, s.to]
                            .iter()
                            .any(|e| (p[0] - e[0]).hypot(p[1] - e[1]) < 3.0)
                });
                // Other lines within reach, the edge's own line apart
                // (it is under the highlight all along).
                let crowded = everything.lines.iter().any(|other| {
                    let d = distance_to(p, other);
                    d < 4.0 && d > 0.05
                });
                if near_end
                    || crowded
                    || p[0] < 2.0
                    || p[1] < 2.0
                    || p[0] >= f64::from(w) - 2.0
                    || p[1] >= f64::from(h) - 2.0
                {
                    continue;
                }
                here += 1;
                let (c, g) = (pixel(&oracle, w, p), pixel(&direct, w, p));
                here_agree += usize::from(levels(c, g) <= 8);
                if line.hidden {
                    dimmed += 1;
                } else {
                    inked_n += 1;
                    assert!(
                        levels(c, tokens.face_highlight) <= 1,
                        "the oracle's line is the token's colour at {p:?}: {c:?}"
                    );
                }
            }
        }
        samples += here;
        agree += here_agree;
        log.push_str(&format!(
            "{} edge {}: {here_agree} of {here} samples agree\n",
            listed.body, listed.index
        ));
        // Away from the highlight the picture on the device is what it
        // was before the click.
        let (wu, hu) = (w as usize, h as usize);
        let mut changed = 0;
        for i in (0..wu * hu).step_by(7) {
            let p = [(i % wu) as f64 + 0.5, (i / wu) as f64 + 0.5];
            if highlight.iter().all(|l| distance_to(p, l) > 4.0) {
                changed +=
                    usize::from(levels(pixel(&direct, w, p), pixel(&direct_plain, w, p)) > 1);
            }
        }
        assert_eq!(
            changed, 0,
            "{}: the picture changed away from the edge",
            listed.body
        );
        if round == 4 {
            for (suffix, rgb) in [("direct", &direct), ("oracle", &oracle)] {
                let path = evidence(&format!("pointing-edge-{suffix}.png"));
                write_png(&path, w, h, rgb);
                println!("evidence: {}", path.display());
            }
        }
    }
    println!(
        "{samples} samples along highlighted edges ({inked_n} inked, {dimmed} dimmed), {agree} agree"
    );
    std::fs::write(
        evidence("pointing-edge-parity.txt"),
        format!("{log}{samples} samples ({inked_n} inked, {dimmed} dimmed), {agree} agree\n"),
    )
    .unwrap();
    assert!(inked_n > 150, "the test is hollow: {inked_n} inked samples");
    assert!(
        agree * 100 >= samples * 99,
        "the GPU's highlight differs from the CPU's at {} of {samples} samples",
        samples - agree
    );
    // Without a highlight the device shows the plain picture again.
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": null}),
    );
    window.frame(&mut shell, &mut session, Vec::new());
    let (_, _, cleared) = shell.direct_picture().unwrap().unwrap();
    assert_eq!(cleared, direct_plain);
}

/// One vertex as `document.edges` lists it under `vertices`.
#[derive(Clone, Debug)]
struct Corner {
    feature: String,
    body: String,
    index: usize,
    name: Option<String>,
    faces: Vec<String>,
    point: Vec3,
}

impl Corner {
    /// How a highlight refers to it: by its own name, or by its faces.
    fn reference(&self) -> VertexRef {
        VertexRef {
            feature: Some(self.feature.clone()),
            vertex: self.name.clone(),
            corner: if self.name.is_some() {
                Vec::new()
            } else {
                self.faces.clone()
            },
        }
    }
}

fn listed_vertices(session: &mut Session) -> Vec<Corner> {
    let inspect = run(session, "document.inspect", json!({}));
    let mut out = Vec::new();
    for body in inspect["bodies"].as_array().unwrap() {
        let name = body["name"].as_str().unwrap();
        let listed = run(session, "document.edges", json!({"feature": name}));
        for vertex in listed["vertices"].as_array().unwrap() {
            let c: Vec<f64> = vertex["point"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            out.push(Corner {
                feature: body["feature"].as_str().unwrap().to_string(),
                body: name.to_string(),
                index: vertex["index"].as_u64().unwrap() as usize,
                name: vertex.get("name").map(|n| n.as_str().unwrap().to_string()),
                faces: vertex["faces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|f| f.as_str().unwrap().to_string())
                    .collect(),
                point: Vec3::new(c[0], c[1], c[2]),
            });
        }
    }
    out
}

/// The vertices in sight, with where they land, that have no other vertex
/// within 10 pixels in the picture (so a click on one is a click on it).
fn vertices_in_sight(
    scene: &Scene,
    camera: &CameraState,
    corners: &[Corner],
    [width, height]: [u32; 2],
) -> Vec<(Corner, [f64; 2])> {
    let projector =
        Projector::new(&camera.to_render(), f64::from(width), f64::from(height)).unwrap();
    let landed: Vec<[f64; 2]> = corners
        .iter()
        .map(|c| projector.project(c.point).map_or([-1e9; 2], |p| [p.x, p.y]))
        .collect();
    corners
        .iter()
        .zip(&landed)
        .enumerate()
        .filter(|(k, (corner, at))| {
            at[0] >= 8.0
                && at[1] >= 8.0
                && at[0] < f64::from(width) - 8.0
                && at[1] < f64::from(height) - 8.0
                && in_sight(scene, camera, corner.point)
                && landed
                    .iter()
                    .enumerate()
                    .all(|(j, other)| j == *k || (other[0] - at[0]).hypot(other[1] - at[1]) > 10.0)
        })
        .map(|(_, (corner, at))| (corner.clone(), *at))
        .collect()
}

#[test]
fn a_click_on_a_vertex_picks_it_and_ui_highlight_shows_one() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let tokens = shell.tokens;
    let mut window = Window::new(1000.0, 700.0);
    for _ in 0..4 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let rect = shell.viewport_rect().unwrap();
    let size = shell.viewport_size();
    let [width, height] = size;
    let scene: Arc<Scene> = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let camera = shell.state.camera;
    let corners = listed_vertices(&mut session);
    assert_eq!(corners.len(), 10 + 2 + 24);
    let (_, _, plain) = shot(&mut shell, &mut session);
    let seen = vertices_in_sight(&scene, &camera, &corners, size);
    let seen: Vec<_> = seen
        .into_iter()
        .filter(|s| !under_controls(&shell, s.1))
        .collect();
    assert!(seen.len() >= 15, "{} vertices are in sight", seen.len());
    let mut bodies = BTreeSet::new();
    for (round, (corner, [x, y])) in seen.iter().enumerate() {
        // A click a pixel and a half off the vertex is still on it.
        let at = pos2(rect.left() + *x as f32 + 1.0, rect.top() + *y as f32 - 1.0);
        window.click(&mut shell, &mut session, at);
        let picked = shell.state.picked.clone().unwrap();
        let context = format!("{} vertex {}: {picked:?}", corner.body, corner.index);
        assert_eq!(picked.serial, round as u64 + 1, "{context}");
        assert_eq!(picked.kind, Some(PickedKind::Vertex), "{context}");
        assert_eq!(picked.edge, None, "{context}");
        let vertex = picked.vertex.clone().unwrap();
        assert_eq!(
            (vertex.feature.as_str(), vertex.index as usize),
            (corner.feature.as_str(), corner.index),
            "{context}"
        );
        assert_eq!(vertex.body, corner.body, "{context}");
        assert_eq!(vertex.name, corner.name, "{context}");
        assert_eq!(vertex.faces, corner.faces, "{context}");
        assert_eq!(
            vertex.point,
            [corner.point.x, corner.point.y, corner.point.z],
            "{context}"
        );
        bodies.insert(corner.feature.clone());
        // The vertex is what is highlighted, alone.
        assert_eq!(
            shell.state.highlight_vertex,
            Some(corner.reference()),
            "{context}"
        );
        assert_eq!(shell.state.highlight_edge, None, "{context}");
        assert_eq!(shell.state.highlight, None, "{context}");
        // The window draws a dot of the highlight kind on it.
        let overlay = shell.overlay().0.clone();
        let dots: Vec<_> = overlay
            .points
            .iter()
            .filter(|p| p.kind == LineKind::Highlight)
            .collect();
        assert!(!dots.is_empty(), "{context}");
        assert!(
            dots.iter()
                .any(|d| !d.hidden && (d.at[0] - x).hypot(d.at[1] - y) < 1e-6),
            "{context}: no dot on the vertex"
        );
        assert!(overlay.lines.iter().all(|l| l.kind != LineKind::Highlight));
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        assert_eq!(got["state"]["picked"]["kind"], "vertex", "{context}");
        assert_eq!(got["state"]["picked"]["vertex"]["index"], corner.index);
        assert_eq!(
            got["state"]["highlightVertex"],
            serde_json::to_value(corner.reference()).unwrap()
        );
        // The screenshot has the dot, in the highlight's colour, and is
        // the plain picture away from it.
        if round % 3 == 0 {
            let (_, _, marked) = shot(&mut shell, &mut session);
            assert!(
                levels(pixel(&marked, width, [*x, *y]), tokens.face_highlight) <= 8,
                "{context}: {:?} at the vertex",
                pixel(&marked, width, [*x, *y])
            );
            let mut changed = 0;
            for py in 0..height {
                for px in 0..width {
                    let p = [f64::from(px) + 0.5, f64::from(py) + 0.5];
                    let differs = pixel(&marked, width, p) != pixel(&plain, width, p);
                    let near = dots
                        .iter()
                        .any(|d| (d.at[0] - p[0]).hypot(d.at[1] - p[1]) <= 5.5);
                    assert!(!differs || near, "{context}: the picture changed at {p:?}");
                    changed += usize::from(differs);
                }
            }
            // A dot of radius 4: about 50 pixels of it.
            assert!((30..=95).contains(&changed), "{context}: {changed} pixels");
            if round == 3 {
                let path = evidence("pointing-vertex-click-cpu.png");
                write_png(&path, width, height, &marked);
                println!("evidence: {}", path.display());
            }
        }
    }
    assert_eq!(bodies.len(), 3, "the boolean, the post and the pattern");

    // ui.pick says the same, and with no radius the face.
    let (corner, [x, y]) = &seen[0];
    let picked = call(&mut shell, &mut session, "ui.pick", json!({"x": x, "y": y}));
    assert_eq!(picked["kind"], "vertex", "{picked}");
    assert_eq!(picked["vertex"]["index"], corner.index);
    assert_eq!(picked["vertex"]["faces"], json!(corner.faces));
    assert!(picked.get("edge").is_none());
    let faces_only = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x, "y": y, "radius": 0}),
    );
    assert!(faces_only.get("vertex").is_none());

    // ui.highlight by name and by the faces that meet in a vertex.
    for corner in &corners {
        let reference = corner.reference();
        let mut params = json!({"feature": corner.feature});
        match &reference.vertex {
            Some(name) => params["vertex"] = json!(name),
            None => params["corner"] = json!(reference.corner),
        }
        let result = call(&mut shell, &mut session, "ui.highlight", params.clone());
        assert_eq!(
            result["highlightVertex"],
            serde_json::to_value(&reference).unwrap(),
            "{params}"
        );
        assert_eq!(result["name"], corner.body.as_str());
        assert!(result["vertices"].as_u64().unwrap() >= 1);
        assert_eq!(shell.state.highlight_vertex, Some(reference));
        assert_eq!(shell.state.highlight_edge, None);
        assert_eq!(shell.state.highlight, None);
    }
    // A vertex behind the model is drawn too, dimmed.
    let hidden = corners
        .iter()
        .find(|c| {
            !in_sight(&scene, &camera, c.point)
                && seen.iter().all(|(_, at)| {
                    let p =
                        Projector::new(&camera.to_render(), f64::from(width), f64::from(height))
                            .unwrap()
                            .project(c.point)
                            .unwrap();
                    (p.x - at[0]).hypot(p.y - at[1]) > 12.0
                })
        })
        .expect("a vertex at the back");
    let mut params = json!({"feature": hidden.feature});
    match &hidden.name {
        Some(name) => params["vertex"] = json!(name),
        None => params["corner"] = json!(hidden.faces),
    }
    call(&mut shell, &mut session, "ui.highlight", params);
    let (_, _, marked) = shot(&mut shell, &mut session);
    let at = Projector::new(&camera.to_render(), f64::from(width), f64::from(height))
        .unwrap()
        .project(hidden.point)
        .unwrap();
    let (now, before) = (
        pixel(&marked, width, [at.x, at.y]),
        pixel(&plain, width, [at.x, at.y]),
    );
    assert_ne!(now, before, "a hidden vertex is drawn");
    assert!(
        levels(now, tokens.face_highlight) > 30,
        "and dimmed: {now:?}"
    );

    // ui.set, and what is wrong.
    // A vertex of another body than the drilled plate (the refusals below
    // ask that plate for it).
    let named = corners
        .iter()
        .find(|c| c.name.is_some() && c.body != "Drilled")
        .unwrap();
    let name = named.name.clone().unwrap();
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"highlightVertex": {"vertex": name}}),
    );
    assert_eq!(
        shell.state.highlight_vertex,
        Some(VertexRef {
            feature: None,
            vertex: Some(name.clone()),
            corner: Vec::new()
        })
    );
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"highlightVertex": {"vertex": "F99:vertex(0)"}}),
    );
    assert_eq!(shell.state.highlight_vertex, None);
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"vertex": name}),
    );
    let state = shell.state.clone();
    let face = named.faces[0].clone();
    for (params, part) in [
        (json!({"vertex": 3}), "`vertex` must be a vertex name"),
        (json!({"vertex": "corner"}), "is not a name"),
        (json!({"vertex": face}), "is a face"),
        (
            json!({"vertex": "F99:vertex(0)"}),
            "no body has a vertex F99:vertex(0)",
        ),
        (
            json!({"vertex": name, "feature": "Drilled"}),
            "`Drilled` has no vertex",
        ),
        (json!({"vertex": name, "face": face}), "one of `face`"),
        (
            json!({"vertex": name, "edge": "F1:edge(0)"}),
            "one of `face`",
        ),
        (json!({"vertex": name, "corner": [face]}), "not both"),
        (json!({"corner": []}), "one to 16 face names"),
        (json!({"corner": face}), "one to 16 face names"),
        (json!({"corner": [face, 7]}), "one to 16 face names"),
        (json!({"corner": [name]}), "is not a face"),
        (
            json!({"corner": vec![face.clone(); 17]}),
            "one to 16 face names",
        ),
        (
            json!({"corner": ["F99:face(0)"]}),
            "no body has a vertex where",
        ),
    ] {
        let error = refused(&mut shell, &mut session, "ui.highlight", params.clone());
        assert!(error.contains(part), "{:.80}: {error}", params.to_string());
        assert_eq!(shell.state, state);
    }
    // It goes with its body, and face: null clears it.
    let of_post = corners.iter().find(|c| c.body == "Post").unwrap();
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"vertex": of_post.name}),
    );
    run(&mut session, "feature.delete", json!({"feature": "Post"}));
    shell.sync(&mut session);
    assert_eq!(shell.state.highlight_vertex, None);
    run(&mut session, "edit.undo", json!({}));
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"vertex": of_post.name}),
    );
    let cleared = call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": null}),
    );
    assert_eq!(cleared, json!({"highlight": null, "faces": 0}));
    assert_eq!(shell.state.highlight_vertex, None);
}

/// A highlighted vertex on the GPU's direct path: the dot the oracle
/// draws, in the same place and colour, and nothing else changed.
#[test]
fn a_highlighted_vertex_is_the_same_on_the_gpus_direct_path_as_in_the_oracle() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let tokens = shell.tokens;
    shell.attach_gpu(&state).unwrap();
    let mut window = Window::new(1000.0, 700.0);
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    let size = shell.viewport_size();
    let [w, h] = size;
    let rect = shell.viewport_rect().unwrap();
    let scene: Arc<Scene> = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let camera = shell.state.camera;
    let corners = listed_vertices(&mut session);
    let (_, _, direct_plain) = shell.direct_picture().unwrap().unwrap();
    let cpu_frame = viewport::render_bodies(
        Some(&scene),
        &camera,
        Quality {
            width: w,
            height: h,
            ssaa: 1,
        },
        &tokens,
    )
    .unwrap();
    let slack = viewport::depth_slack(Backend::Gpu, Some(&scene), &camera);
    let seen = vertices_in_sight(&scene, &camera, &corners, size);
    let seen: Vec<_> = seen
        .into_iter()
        .filter(|s| !under_controls(&shell, s.1))
        .collect();
    assert!(seen.len() >= 15, "{}", seen.len());
    let (mut samples, mut agree) = (0, 0);
    for (round, (corner, [x, y])) in seen.iter().enumerate() {
        let at = pos2(rect.left() + *x as f32, rect.top() + *y as f32);
        window.click(&mut shell, &mut session, at);
        assert_eq!(shell.state.highlight_vertex, Some(corner.reference()));
        let (_, _, direct) = shell.direct_picture().unwrap().unwrap();
        let marks: Marks = shell.marks(Some(&scene));
        assert_eq!(marks.points, vec![corner.point]);
        let overlay =
            viewport::overlay_marked(Some(&scene), &camera, &cpu_frame, None, slack, &marks);
        let oracle = viewport::rasterise(
            Composed {
                width: w,
                height: h,
                rgb: cpu_frame.rgb.clone(),
                overlay,
                coverage: cpu_frame.coverage(),
                triangles: scene.triangles(),
                backend: Backend::Cpu,
            },
            &tokens,
            1.0,
        )
        .rgb;
        // Within two pixels of the centre of a dot of radius 4 every
        // pixel is all ink in both pictures: 8 levels, as for the lines.
        for (dx, dy) in [(0.0, 0.0), (2.0, 0.0), (-2.0, 0.0), (0.0, 2.0), (0.0, -2.0)] {
            let p = [x + dx, y + dy];
            let (c, g) = (pixel(&oracle, w, p), pixel(&direct, w, p));
            assert!(levels(c, tokens.face_highlight) <= 1, "the oracle: {c:?}");
            samples += 1;
            agree += usize::from(levels(c, g) <= 8);
        }
        // Further than the dot reaches, the device shows what it showed.
        let (wu, hu) = (w as usize, h as usize);
        for i in (0..wu * hu).step_by(5) {
            let p = [(i % wu) as f64 + 0.5, (i / wu) as f64 + 0.5];
            if (p[0] - x).hypot(p[1] - y) > 6.5 {
                assert!(
                    levels(pixel(&direct, w, p), pixel(&direct_plain, w, p)) <= 1,
                    "{} vertex {}: the picture changed at {p:?}",
                    corner.body,
                    corner.index
                );
            }
        }
        if round == 2 {
            for (suffix, rgb) in [("direct", &direct), ("oracle", &oracle)] {
                let path = evidence(&format!("pointing-vertex-{suffix}.png"));
                write_png(&path, w, h, rgb);
                println!("evidence: {}", path.display());
            }
        }
    }
    println!("{samples} samples on highlighted vertices, {agree} agree");
    assert!(agree * 100 >= samples * 99, "{agree} of {samples}");
}
