//! Pointing at edges and vertices (roadmap M8.5): a position in a picture
//! near an edge in sight gives that edge, the way `document.edges` reports
//! it: its own name where it has one, the faces it lies between, its place
//! in the list. Near a vertex in sight it gives the vertex, as
//! `document.edges` lists it under `vertices`.
//!
//! Neither side of the comparison is the code under test. The edges, their
//! names and their geometry come from `document.edges` (a line from its
//! ends; a circle from its start point and the axis the test built the
//! cylinder on). The points picked at are points of the chords the scene
//! draws an edge with, after those chords were found to lie on the edge
//! `document.edges` lists at that place (`chords_of`): points of the mesh,
//! so that "in sight" has one meaning. Whether a point is in sight is worked out here
//! by another route than the pick's: a ray from the eye to the point,
//! tested against every triangle of the scene's mesh in space
//! (Möller-Trumbore), where the pick uses the renderer's barycentric
//! weights in the picture. Where a point lands in the picture is computed
//! here from the camera `view.pick` says it used.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, DEFAULT_PICK_RADIUS, Scene, Session, Workspace};
use craft3d_math::Vec3;
use craft3d_render::{Camera, Projection, ViewAxes};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn session() -> Session {
    Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    })
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// The document of `pointing.rs`: a box, a cylinder, a block with a hole
/// drilled by a boolean, a row of four separate copies and a row of three
/// copies that overlap into one piece.
fn workshop() -> Session {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Box"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 8, "height": 25, "position": [70, 10, 0], "name": "Post"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [30, 30, 12], "position": [0, 50, 0], "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 6, "height": 30, "position": [15, 65, -5], "name": "Drill"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Drilled"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [6, 6, 6], "position": [0, -40, 0], "name": "Stud"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Stud", "count": 4, "step": [11, 0, 3], "name": "Studs"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [8, 8, 5], "position": [50, -40, 0], "name": "Tile"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Tile", "count": 3, "step": [6, 3, 2], "name": "Tiles"}),
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    s
}

/// The axes of the cylinders of the workshop, all along z: (x, y) per
/// body. The circles of `document.edges` are about them.
fn axis_of(body: &str) -> Option<(f64, f64)> {
    match body {
        "Post" => Some((70.0, 10.0)),
        "Drilled" => Some((15.0, 65.0)),
        _ => None,
    }
}

fn vec3(value: &Value) -> Vec3 {
    let c: Vec<f64> = value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    Vec3::new(c[0], c[1], c[2])
}

/// The camera `view.pick` says it used.
fn camera_of(result: &Value) -> Camera {
    let camera = &result["camera"];
    Camera {
        eye: vec3(&camera["eye"]),
        target: vec3(&camera["target"]),
        up: vec3(&camera["up"]),
        projection: match camera["projection"].as_str().unwrap() {
            "perspective" => Projection::Perspective {
                fov_y_deg: camera["fovY"].as_f64().unwrap(),
            },
            "orthographic" => Projection::Orthographic {
                half_height: camera["height"].as_f64().unwrap() / 2.0,
            },
            other => panic!("{other}"),
        },
    }
}

fn pick_params(view: &Value, x: f64, y: f64) -> Value {
    let mut params = view.clone();
    params["x"] = x.into();
    params["y"] = y.into();
    params
}

/// The geometry of an edge, from what `document.edges` says about it.
#[derive(Clone, Copy, Debug)]
enum Curve {
    Line(Vec3, Vec3),
    /// A full circle about a vertical axis through `center`, from `start`.
    Circle {
        center: Vec3,
        start: Vec3,
    },
}

impl Curve {
    /// The point at the fraction `u` of the edge (of the turn, for a
    /// circle).
    fn at(&self, u: f64) -> Vec3 {
        match *self {
            Curve::Line(a, b) => a + (b - a) * u,
            Curve::Circle { center, start, .. } => {
                let arm = start - center;
                let angle = u * std::f64::consts::TAU;
                // About z: the arm is horizontal.
                center
                    + Vec3::new(
                        arm.x * angle.cos() - arm.y * angle.sin(),
                        arm.x * angle.sin() + arm.y * angle.cos(),
                        0.0,
                    )
            }
        }
    }

    /// How far `p` is from the edge, in model units.
    fn distance_to(&self, p: Vec3) -> f64 {
        match *self {
            Curve::Line(a, b) => {
                let d = b - a;
                let t = ((p - a).dot(d) / d.dot(d)).clamp(0.0, 1.0);
                (p - (a + d * t)).length()
            }
            Curve::Circle { center, start, .. } => {
                let radius = (start - center).length();
                let rel = p - center;
                let radial = rel.x.hypot(rel.y);
                (radial - radius).hypot(rel.z)
            }
        }
    }

    fn closed(&self) -> bool {
        matches!(self, Curve::Circle { .. })
    }
}

/// One edge as `document.edges` lists it.
#[derive(Clone, Debug)]
struct Listed {
    feature: String,
    body: String,
    index: usize,
    name: Option<String>,
    faces: BTreeSet<String>,
    curve: Curve,
}

/// Every edge of every body, from `document.inspect` and `document.edges`.
fn listed_edges(s: &mut Session) -> Vec<Listed> {
    let inspect = run(s, "document.inspect", json!({}));
    let mut out = Vec::new();
    for body in inspect["bodies"].as_array().unwrap() {
        let name = body["name"].as_str().unwrap();
        let listed = run(s, "document.edges", json!({"feature": name}));
        assert_eq!(listed["feature"], body["feature"]);
        let edges = listed["edges"].as_array().unwrap();
        assert_eq!(edges.len() as u64, listed["count"].as_u64().unwrap());
        assert_eq!(edges.len() as u64, body["edges"].as_u64().unwrap());
        for (index, edge) in edges.iter().enumerate() {
            let (start, end) = (vec3(&edge["start"]), vec3(&edge["end"]));
            let curve = match edge["curve"].as_str().unwrap() {
                "line" => Curve::Line(start, end),
                "circle" => {
                    assert_eq!(edge["closed"], true, "{edge}");
                    let (x, y) = axis_of(name).unwrap();
                    Curve::Circle {
                        center: Vec3::new(x, y, start.z),
                        start,
                    }
                }
                other => panic!("{name}: an edge on a {other}"),
            };
            out.push(Listed {
                feature: body["feature"].as_str().unwrap().to_string(),
                body: name.to_string(),
                index,
                name: edge.get("name").map(|n| n.as_str().unwrap().to_string()),
                faces: edge["faces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|f| f.as_str().unwrap().to_string())
                    .collect(),
                curve,
            });
        }
    }
    out
}

/// One vertex as `document.edges` lists it under `vertices`.
#[derive(Clone, Debug)]
struct Corner {
    feature: String,
    body: String,
    index: usize,
    name: Option<String>,
    faces: BTreeSet<String>,
    point: Vec3,
}

/// Every vertex of every body, from `document.inspect` and
/// `document.edges`.
fn listed_vertices(s: &mut Session) -> Vec<Corner> {
    let inspect = run(s, "document.inspect", json!({}));
    let mut out = Vec::new();
    for body in inspect["bodies"].as_array().unwrap() {
        let name = body["name"].as_str().unwrap();
        let listed = run(s, "document.edges", json!({"feature": name}));
        let vertices = listed["vertices"].as_array().unwrap();
        // Every vertex of a solid is the end of an edge.
        assert_eq!(vertices.len() as u64, body["vertices"].as_u64().unwrap());
        for (k, vertex) in vertices.iter().enumerate() {
            assert_eq!(vertex["index"], k, "{name}");
            let point = vec3(&vertex["point"]);
            // It is where the edges that end there end.
            let ends = listed["edges"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|e| [vec3(&e["start"]), vec3(&e["end"])])
                .filter(|end| (*end - point).length() < 1e-12)
                .count();
            assert!(ends >= 2, "{name} vertex {k}: {ends} ends of edges");
            out.push(Corner {
                feature: body["feature"].as_str().unwrap().to_string(),
                body: name.to_string(),
                index: k,
                name: vertex.get("name").map(|n| n.as_str().unwrap().to_string()),
                faces: vertex["faces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|f| f.as_str().unwrap().to_string())
                    .collect(),
                point,
            });
        }
    }
    out
}

/// How far (`x`, `y`) is from the nearest vertex in sight in the picture,
/// in pixels: within the pick radius of one, a position points at the
/// vertex and not at an edge.
fn nearest_vertex(picture: &Picture, corners: &[Corner], x: f64, y: f64) -> f64 {
    corners
        .iter()
        .filter_map(|c| {
            let (cx, cy, _) = picture.project(c.point);
            let d = (cx - x).hypot(cy - y);
            // The ray is cast only for the ones that matter.
            (d <= 70.0 && picture.in_sight(c.point)).then_some(d)
        })
        .fold(f64::INFINITY, f64::min)
}

/// What a pick says about its vertex, checked against `document.edges`:
/// the entry at `index` of the body's vertices has that name, those faces
/// and that point; it lands `distance` pixels from the position, within
/// the radius, and is in sight. Returns the entry.
fn check_vertex<'a>(
    picture: &Picture,
    corners: &'a [Corner],
    picked: &Value,
    context: &str,
) -> &'a Corner {
    let vertex = &picked["vertex"];
    assert_eq!(picked["kind"], "vertex", "{context}");
    assert_eq!(picked["hit"], true, "{context}");
    assert!(picked.get("edge").is_none(), "{context}");
    let index = vertex["index"].as_u64().unwrap() as usize;
    let listed = corners
        .iter()
        .find(|c| c.feature == vertex["feature"].as_str().unwrap() && c.index == index)
        .unwrap_or_else(|| panic!("{context}: document.edges has no such vertex"));
    assert_eq!(vertex["body"], listed.body.as_str(), "{context}");
    assert_eq!(
        vertex["name"].as_str().map(str::to_string),
        listed.name,
        "{context}"
    );
    let faces: BTreeSet<String> = vertex["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert_eq!(faces, listed.faces, "{context}");
    let point = vec3(&vertex["point"]);
    assert_eq!(point, listed.point, "{context}");
    let (x, y, depth) = picture.project(point);
    let distance = vertex["distance"].as_f64().unwrap();
    let (px, py) = (picked["x"].as_f64().unwrap(), picked["y"].as_f64().unwrap());
    assert!(
        ((x - px).hypot(y - py) - distance).abs() < 1e-6,
        "{context}: lands at ({x}, {y})"
    );
    assert!(
        distance <= picked["radius"].as_f64().unwrap(),
        "{context}: outside the radius"
    );
    assert!(
        (vertex["depth"].as_f64().unwrap() - depth).abs() <= 1e-9 * depth.abs().max(1.0),
        "{context}"
    );
    assert!(picture.in_sight(point), "{context}: picked through a face");
    listed
}

/// The chords the scene draws an edge with.
struct Chords {
    points: Vec<Vec3>,
    /// The length along the chords up to each point.
    along: Vec<f64>,
    closed: bool,
}

impl Chords {
    fn length(&self) -> f64 {
        *self.along.last().unwrap()
    }

    /// The point at the fraction `u` of the length; a closed edge goes
    /// round.
    fn at(&self, u: f64) -> Vec3 {
        let u = if self.closed {
            u.rem_euclid(1.0)
        } else {
            u.clamp(0.0, 1.0)
        };
        let target = u * self.length();
        let k = self
            .along
            .partition_point(|&l| l <= target)
            .clamp(1, self.points.len() - 1);
        let (l0, l1) = (self.along[k - 1], self.along[k]);
        let t = if l1 > l0 {
            (target - l0) / (l1 - l0)
        } else {
            0.0
        };
        self.points[k - 1] + (self.points[k] - self.points[k - 1]) * t
    }
}

/// The chords of every edge of `edges` in `scene`, each found to be the
/// edge `document.edges` lists at that place: every point of the chords
/// is on the listed curve (a line: between its ends, and the chords run
/// from one end to the other; a circle: on it, all the way round).
fn chords_of(scene: &Scene, edges: &[Listed]) -> Vec<Chords> {
    edges
        .iter()
        .map(|listed| {
            let body = scene
                .bodies
                .iter()
                .find(|b| b.feature == listed.feature)
                .unwrap();
            assert_eq!(body.name, listed.body);
            let points = body.edges[listed.index].clone();
            assert!(points.len() >= 2, "{} edge {}", listed.body, listed.index);
            for p in &points {
                let off = listed.curve.distance_to(*p);
                assert!(off < 1e-9, "{} edge {}: {off}", listed.body, listed.index);
            }
            let (first, last) = (points[0], points[points.len() - 1]);
            match listed.curve {
                Curve::Line(a, b) => {
                    let ends = (first - a).length() + (last - b).length();
                    let swapped = (first - b).length() + (last - a).length();
                    assert!(ends.min(swapped) < 1e-9, "{}", listed.index);
                }
                Curve::Circle { center, start } => {
                    assert!((first - last).length() < 1e-9 && points.len() >= 13);
                    // All the way round: the chords add up to nearly the
                    // circumference.
                    let radius = (start - center).length();
                    let total: f64 = points.windows(2).map(|w| (w[1] - w[0]).length()).sum();
                    let round = std::f64::consts::TAU * radius;
                    assert!(total > 0.98 * round && total <= round, "{total} of {round}");
                }
            }
            let mut along = vec![0.0];
            for pair in points.windows(2) {
                along.push(along[along.len() - 1] + (pair[1] - pair[0]).length());
            }
            Chords {
                points,
                along,
                closed: listed.curve.closed(),
            }
        })
        .collect()
}

/// A picture: where points land in it and what is in sight, worked out
/// here.
struct Picture {
    axes: ViewAxes,
    width: f64,
    height: f64,
    scene: Arc<Scene>,
}

impl Picture {
    fn new(camera: &Camera, width: u32, height: u32, scene: Arc<Scene>) -> Self {
        Self {
            axes: camera.axes().unwrap(),
            width: f64::from(width),
            height: f64::from(height),
            scene,
        }
    }

    /// (x, y) in pixels and the distance along the view (`ViewAxes` says
    /// how).
    fn project(&self, p: Vec3) -> (f64, f64, f64) {
        let rel = p - self.axes.eye;
        let dist = rel.dot(self.axes.forward);
        let scale = match self.axes.projection {
            Projection::Perspective { .. } => 1.0 / (dist * self.axes.half),
            Projection::Orthographic { .. } => 1.0 / self.axes.half,
        };
        let aspect = self.width / self.height;
        (
            (rel.dot(self.axes.right) * scale / aspect * 0.5 + 0.5) * self.width,
            (0.5 - rel.dot(self.axes.up) * scale * 0.5) * self.height,
            dist,
        )
    }

    fn inside(&self, x: f64, y: f64, margin: f64) -> bool {
        x >= margin && y >= margin && x < self.width - margin && y < self.height - margin
    }

    /// True when no triangle of the mesh is between the eye and `p`: a ray
    /// from the eye (from the eye plane, for an orthographic camera) to
    /// `p`, intersected with every triangle in space. A triangle that
    /// meets the ray within a millionth of its length of `p` is the
    /// surface `p` lies on, not something in front of it.
    fn in_sight(&self, p: Vec3) -> bool {
        let origin = match self.axes.projection {
            Projection::Perspective { .. } => self.axes.eye,
            Projection::Orthographic { .. } => {
                p - self.axes.forward * (p - self.axes.eye).dot(self.axes.forward)
            }
        };
        let dir = p - origin;
        let mesh = &self.scene.mesh;
        !mesh.triangles.iter().any(|tri| {
            let [a, b, c] = tri.map(|k| mesh.positions[k as usize]);
            // Möller-Trumbore.
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
}

/// What a pick says about its edge, checked against `document.edges` and
/// the geometry: the entry at `index` of the body's list has that name and
/// those faces, the point is on that edge, lands `distance` pixels from
/// the position, within the radius, and is in sight. Returns the entry.
fn check_edge<'a>(
    picture: &Picture,
    edges: &'a [Listed],
    picked: &Value,
    tolerance: f64,
    context: &str,
) -> &'a Listed {
    let edge = &picked["edge"];
    assert_eq!(picked["kind"], "edge", "{context}");
    assert_eq!(picked["hit"], true, "{context}");
    let index = edge["index"].as_u64().unwrap() as usize;
    let listed = edges
        .iter()
        .find(|l| l.feature == edge["feature"].as_str().unwrap() && l.index == index)
        .unwrap_or_else(|| panic!("{context}: document.edges has no such edge"));
    assert_eq!(edge["body"], listed.body.as_str(), "{context}");
    assert_eq!(
        edge["name"].as_str().map(str::to_string),
        listed.name,
        "{context}"
    );
    let faces: BTreeSet<String> = edge["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert_eq!(faces, listed.faces, "{context}");
    let point = vec3(&edge["point"]);
    let off = listed.curve.distance_to(point);
    // On a line exactly; on a circle within the chords it is drawn with.
    let allowed = if listed.curve.closed() {
        tolerance * 1.01
    } else {
        1e-9
    };
    assert!(off <= allowed, "{context}: {off} from the edge");
    let (x, y, depth) = picture.project(point);
    let distance = edge["distance"].as_f64().unwrap();
    let (px, py) = (picked["x"].as_f64().unwrap(), picked["y"].as_f64().unwrap());
    assert!(
        ((x - px).hypot(y - py) - distance).abs() < 1e-6,
        "{context}: lands at ({x}, {y})"
    );
    assert!(
        distance <= picked["radius"].as_f64().unwrap(),
        "{context}: outside the radius"
    );
    assert!(
        (edge["depth"].as_f64().unwrap() - depth).abs() <= 1e-9 * depth.abs().max(1.0),
        "{context}"
    );
    assert!(picture.in_sight(point), "{context}: picked through a face");
    listed
}

/// The views of the acceptance test: cameras in general position, so that
/// two edges do not land on one line of the picture.
fn views() -> Vec<(&'static str, Value)> {
    vec![
        ("iso", json!({"view": "iso", "width": 320, "height": 240})),
        (
            "iso-ortho",
            json!({"view": "iso", "projection": "orthographic", "width": 300, "height": 300}),
        ),
        (
            "drilled-close",
            json!({"view": "iso", "box": [[0, 50, 0], [30, 80, 12]], "width": 320, "height": 240}),
        ),
        (
            "tiles-zoom",
            json!({"view": "iso", "box": [[50, -40, 0], [70, -26, 9]], "zoom": 0.8,
                   "projection": "orthographic", "width": 320, "height": 240}),
        ),
        (
            "fine",
            json!({"view": "iso", "tolerance": 0.002, "zoom": 1.5, "width": 320, "height": 240}),
        ),
        (
            "below-behind",
            json!({"camera": {"eye": [-120, 190, -90], "target": [35, 20, 5], "up": [0, 0, 1],
                              "fovY": 35}, "width": 320, "height": 240}),
        ),
        (
            "own-ortho",
            json!({"camera": {"eye": [200, -150, 120], "target": [35, 20, 5],
                              "projection": "orthographic", "height": 150},
                   "width": 320, "height": 240}),
        ),
        (
            "nearly-front",
            json!({"camera": {"eye": [48, -400, 31], "target": [35, 20, 8], "up": [0, 0, 1],
                              "projection": "orthographic", "height": 140},
                   "width": 400, "height": 240}),
        ),
        (
            "nearly-top",
            json!({"camera": {"eye": [52, 3, 500], "target": [35, 20, 0], "up": [0, 1, 0],
                              "fovY": 22}, "width": 320, "height": 320}),
        ),
        (
            "post-from-below",
            json!({"camera": {"eye": [130, -60, -40], "target": [70, 10, 12], "up": [0, 0, 1],
                              "fovY": 30}, "width": 320, "height": 240}),
        ),
    ]
}

/// The picture of a view and the scene it shows.
fn picture_of(s: &mut Session, view: &Value) -> Picture {
    let (width, height) = (
        view["width"].as_u64().unwrap() as u32,
        view["height"].as_u64().unwrap() as u32,
    );
    let first = run(s, "view.pick", pick_params(view, 0.5, 0.5));
    let camera = camera_of(&first);
    let tolerance = view.get("tolerance").and_then(Value::as_f64);
    let scene = s.scene(tolerance).unwrap().unwrap();
    Picture::new(&camera, width, height, scene)
}

#[test]
fn a_pick_on_an_edge_in_sight_is_the_edge_document_edges_lists() {
    let mut s = workshop();
    let edges = listed_edges(&mut s);
    assert_eq!(edges.len(), 12 + 2 + 14 + 48 + 48);
    let corners = listed_vertices(&mut s);
    let mut by_a_vertex = 0;
    let (mut checked, mut hidden, mut unclear, mut crossings) = (0, 0, 0, 0);
    let mut seen: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut faces_behind = 0;
    for (label, view) in &views() {
        // Pictures twice as large as the other tests', so that most of an
        // edge is further from its ends than the pick radius.
        let mut view = view.clone();
        for side in ["width", "height"] {
            view[side] = (view[side].as_u64().unwrap() * 2).into();
        }
        let view = &view;
        let picture = picture_of(&mut s, view);
        let tolerance = picture.scene.tolerance;
        let chords = chords_of(&picture.scene, &edges);
        for (listed, chords) in edges.iter().zip(&chords) {
            let fractions: &[f64] = if listed.curve.closed() {
                &[0.03, 0.16, 0.29, 0.41, 0.54, 0.66, 0.79, 0.91]
            } else {
                &[0.2, 0.5, 0.8]
            };
            for &u in fractions {
                let p = chords.at(u);
                let (x, y, _) = picture.project(p);
                if !picture.inside(x, y, 1.0) {
                    continue;
                }
                // Within the radius of a vertex the vertex is what is
                // pointed at (the tests of the vertices): this test is
                // about the stretches of the edges between them.
                if nearest_vertex(&picture, &corners, x, y) <= DEFAULT_PICK_RADIUS + 0.01 {
                    by_a_vertex += 1;
                    continue;
                }
                // Two pixels along the edge either way: a point is taken
                // as in sight, or as hidden, only when its neighbours on
                // the edge agree, so that the border of what covers an
                // edge is not what decides.
                let ahead = picture.project(chords.at(u + 1e-4));
                let pixels_per_u = (ahead.0 - x).hypot(ahead.1 - y) / 1e-4;
                let step = 2.0 / pixels_per_u;
                let sight = [u - step, u, u + step].map(|v| picture.in_sight(chords.at(v)));
                let picked = run(&mut s, "view.pick", pick_params(view, x, y));
                let context = format!(
                    "{label}: {} edge {} at {u} ({x}, {y}): {picked}",
                    listed.body, listed.index
                );
                // Whatever edge the pick gives is one of the list, with
                // its names, at a point in sight.
                let found = (picked["kind"] == "edge")
                    .then(|| check_edge(&picture, &edges, &picked, tolerance, &context));
                if sight == [true; 3] {
                    let found = found.unwrap_or_else(|| panic!("{context}: no edge"));
                    let distance = picked["edge"]["distance"].as_f64().unwrap();
                    if found.feature == listed.feature && found.index == listed.index {
                        // The position is on the edge, and the point is
                        // the one that was projected.
                        assert!(distance <= 1e-6, "{context}");
                        let point = vec3(&picked["edge"]["point"]);
                        assert!((point - p).length() <= 1e-6, "{context}");
                        checked += 1;
                        seen.entry(listed.body.clone())
                            .or_default()
                            .insert(listed.index);
                    } else {
                        // Another edge passes through the same spot of the
                        // picture, at least as near to the position and in
                        // sight (check_edge): the rule's answer. Counted,
                        // and rare in these views.
                        assert!(distance <= 1e-6, "{context}: expected this edge");
                        crossings += 1;
                    }
                } else if sight == [false; 3] {
                    // Behind a face: this edge is not picked here. (It
                    // may be picked at another point of it, in sight and
                    // near, which check_edge has verified.)
                    if let Some(found) = found
                        && found.feature == listed.feature
                        && found.index == listed.index
                    {
                        let point = vec3(&picked["edge"]["point"]);
                        assert!(
                            (point - p).length() > 1e-6,
                            "{context}: picked through a face"
                        );
                    }
                    if picked["kind"] == "face" {
                        faces_behind += 1;
                    }
                    hidden += 1;
                } else {
                    unclear += 1;
                }
            }
        }
    }
    println!(
        "{checked} picks on edges in sight, {hidden} on hidden ones ({faces_behind} gave the face in front), {unclear} at a border, {crossings} crossings, {by_a_vertex} left out by a vertex"
    );
    assert!(checked > 900, "{checked}");
    assert!(
        hidden > 600 && faces_behind > 150,
        "{hidden}, {faces_behind}"
    );
    assert!(crossings * 50 < checked, "{crossings} of {checked}");
    // Every edge of the box, the post and the drilled plate was picked
    // from some side, and most of the patterns'.
    for (body, count) in [("Box", 12), ("Post", 2), ("Drilled", 14)] {
        assert_eq!(seen[body].len(), count, "{body}: {:?}", seen[body]);
    }
    assert!(seen["Studs"].len() >= 40, "{:?}", seen["Studs"]);
    assert!(seen["Tiles"].len() >= 36, "{:?}", seen["Tiles"]);
}

/// The rule, on a grid of positions: the nearest point in sight of any
/// edge within the radius is what a position points at, and a position
/// with no such point points at the face under it or at nothing.
#[test]
fn the_nearest_edge_in_sight_within_the_radius_wins() {
    let mut s = workshop();
    let edges = listed_edges(&mut s);
    let corners = listed_vertices(&mut s);
    let radius = DEFAULT_PICK_RADIUS;
    let (mut with_edge, mut without, mut positions) = (0, 0, 0);
    let mut with_vertex = 0;
    for (label, view) in views().iter().filter(|(label, _)| {
        ["iso", "tiles-zoom", "below-behind", "post-from-below"].contains(label)
    }) {
        let picture = picture_of(&mut s, view);
        let tolerance = picture.scene.tolerance;
        // Every edge as points a quarter of a pixel apart, with where
        // they land; whether one is in sight is worked out when asked.
        struct Sample {
            edge: usize,
            at: (f64, f64),
            point: Vec3,
        }
        let mut samples: Vec<Sample> = Vec::new();
        // Per edge, the range of its samples.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let chords = chords_of(&picture.scene, &edges);
        for (k, chords) in chords.iter().enumerate() {
            let first = samples.len();
            let pixels: f64 = chords
                .points
                .windows(2)
                .map(|w| {
                    let (a, b) = (picture.project(w[0]), picture.project(w[1]));
                    (b.0 - a.0).hypot(b.1 - a.1)
                })
                .sum();
            let count = ((pixels * 4.0).ceil() as usize).clamp(4, 20_000);
            for i in 0..=count {
                let point = chords.at(i as f64 / count as f64);
                let (x, y, _) = picture.project(point);
                samples.push(Sample {
                    edge: k,
                    at: (x, y),
                    point,
                });
            }
            ranges.push((first, samples.len()));
        }
        let mut sight: Vec<Option<bool>> = vec![None; samples.len()];
        let mut in_sight = |i: usize, samples: &[Sample]| -> bool {
            *sight[i].get_or_insert_with(|| picture.in_sight(samples[i].point))
        };
        let (width, height) = (picture.width as u32, picture.height as u32);
        for py in (3..height).step_by(11) {
            for px in (2..width).step_by(11) {
                let (x, y) = (f64::from(px) + 0.5, f64::from(py) + 0.5);
                positions += 1;
                let picked = run(&mut s, "view.pick", pick_params(view, x, y));
                let context = format!("{label}: ({x}, {y}): {picked}");
                // The nearest sample that is clearly in sight (it and the
                // samples a pixel along its edge either way), and the
                // nearest that is in sight at all.
                let mut clearly: Option<(f64, usize)> = None;
                let mut at_all: Option<f64> = None;
                for i in 0..samples.len() {
                    let d = (samples[i].at.0 - x).hypot(samples[i].at.1 - y);
                    if d > radius + 1.0 || !in_sight(i, &samples) {
                        continue;
                    }
                    if at_all.is_none_or(|best| d < best) {
                        at_all = Some(d);
                    }
                    let (first, end) = ranges[samples[i].edge];
                    let clear = [i.saturating_sub(4).max(first), (i + 4).min(end - 1)]
                        .into_iter()
                        .all(|j| in_sight(j, &samples))
                        && [i.saturating_sub(2).max(first), (i + 2).min(end - 1)]
                            .into_iter()
                            .all(|j| in_sight(j, &samples));
                    if clear && clearly.is_none_or(|(best, _)| d < best) {
                        clearly = Some((d, i));
                    }
                }
                if picked["kind"] == "vertex" {
                    // A vertex in sight within the radius: it wins over
                    // any edge. That it is the nearest one is the test of
                    // the vertices.
                    check_vertex(&picture, &corners, &picked, &context);
                    with_vertex += 1;
                } else if nearest_vertex(&picture, &corners, x, y) <= radius - 0.01 {
                    panic!("{context}: a vertex in sight is within the radius");
                } else if picked["kind"] == "edge" {
                    with_edge += 1;
                    check_edge(&picture, &edges, &picked, tolerance, &context);
                    let distance = picked["edge"]["distance"].as_f64().unwrap();
                    // No edge in sight is nearer: half a pixel for the
                    // steps both sides take along an edge and the chords
                    // of a circle.
                    if let Some((nearest, _)) = clearly {
                        assert!(distance <= nearest + 0.5, "{context}: {nearest} is nearer");
                    }
                    // And something in sight is about that near.
                    let nearest = at_all.unwrap_or_else(|| panic!("{context}: nothing is there"));
                    assert!(nearest <= distance + 0.5, "{context}: {nearest}");
                } else {
                    without += 1;
                    // Then no edge is clearly in sight within the radius.
                    if let Some((nearest, i)) = clearly {
                        assert!(
                            nearest > radius - 0.5,
                            "{context}: {} edge {} is {nearest} away",
                            edges[samples[i].edge].body,
                            edges[samples[i].edge].index
                        );
                    }
                    // What is left is the face under the position.
                    assert_eq!(picked["hit"] == true, picked["kind"] == "face", "{context}");
                    assert!(picked.get("edge").is_none(), "{context}");
                }
                // With no radius the same position is the face alone, the
                // same face.
                let faces_only = run(&mut s, "view.pick", {
                    let mut params = pick_params(view, x, y);
                    params["radius"] = 0.into();
                    params
                });
                assert!(faces_only.get("edge").is_none(), "{context}");
                assert!(faces_only.get("vertex").is_none(), "{context}");
                assert_eq!(faces_only.get("face"), picked.get("face"), "{context}");
                assert_eq!(faces_only.get("point"), picked.get("point"), "{context}");
                assert_eq!(
                    faces_only["hit"] == true,
                    picked.get("face").is_some(),
                    "{context}"
                );
            }
        }
    }
    println!(
        "{positions} positions: {with_edge} point at an edge, {with_vertex} at a vertex, {without} at neither"
    );
    assert!(with_edge > 150 && without > 800, "{with_edge}, {without}");
    assert!(with_vertex > 10, "{with_vertex}");
}

/// A box alone, where what is in sight is known in closed form: a box is
/// convex, so an edge is seen exactly when one of its two faces looks at
/// the eye.
#[test]
fn the_edges_at_the_back_of_a_box_are_not_picked_through_it() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Box"}),
    );
    let edges = listed_edges(&mut s);
    assert_eq!(edges.len(), 12);
    let corners = listed_vertices(&mut s);
    assert_eq!(corners.len(), 8);
    let (mut by_a_corner, mut corners_seen) = (0, 0);
    // The faces of a block by number (`NameRole::Face`): bottom, top,
    // front (-y), back (+y), left (-x), right (+x), with a point of each.
    let outward = |face: &str| -> (Vec3, Vec3) {
        let number: usize = face
            .trim_start_matches("F1:face(")
            .trim_end_matches(')')
            .parse()
            .unwrap();
        [
            (Vec3::new(0.0, 0.0, -1.0), Vec3::new(20.0, 10.0, 0.0)),
            (Vec3::new(0.0, 0.0, 1.0), Vec3::new(20.0, 10.0, 10.0)),
            (Vec3::new(0.0, -1.0, 0.0), Vec3::new(20.0, 0.0, 5.0)),
            (Vec3::new(0.0, 1.0, 0.0), Vec3::new(20.0, 20.0, 5.0)),
            (Vec3::new(-1.0, 0.0, 0.0), Vec3::new(0.0, 10.0, 5.0)),
            (Vec3::new(1.0, 0.0, 0.0), Vec3::new(40.0, 10.0, 5.0)),
        ][number]
    };
    let (mut front, mut back, mut faces) = (0, 0, 0);
    for (eye, projection) in [
        ([150.0, -110.0, 90.0], "perspective"),
        ([-90.0, -160.0, 70.0], "orthographic"),
        ([120.0, 170.0, -60.0], "perspective"),
        ([-140.0, 95.0, -85.0], "orthographic"),
        ([55.0, -18.0, 210.0], "perspective"),
    ] {
        let mut camera =
            json!({"eye": eye, "target": [20, 10, 5], "up": [0, 0, 1], "projection": projection});
        if projection == "orthographic" {
            camera["height"] = 70.into();
        }
        let view = json!({"camera": camera, "width": 400, "height": 300});
        let picture = picture_of(&mut s, &view);
        let eye = Vec3::new(eye[0], eye[1], eye[2]);
        let looks_at_eye = |face: &str| {
            let (normal, on) = outward(face);
            match picture.axes.projection {
                Projection::Perspective { .. } => normal.dot(eye - on) > 0.0,
                Projection::Orthographic { .. } => normal.dot(picture.axes.forward) < 0.0,
            }
        };
        let seen: Vec<bool> = edges
            .iter()
            .map(|e| e.faces.iter().any(|f| looks_at_eye(f)))
            .collect();
        // Three faces of a box are seen from a corner: nine of its edges.
        assert_eq!(seen.iter().filter(|&&v| v).count(), 9, "{eye:?}");
        // And seven of its corners: all but the one where the three faces
        // at the back meet. A pick on a corner in sight is that corner; a
        // pick where the hidden one lands is not.
        for corner in &corners {
            let visible = corner.faces.iter().any(|f| looks_at_eye(f));
            let (x, y, _) = picture.project(corner.point);
            let picked = run(&mut s, "view.pick", pick_params(&view, x, y));
            let context = format!("{eye:?}: vertex {}: {picked}", corner.index);
            if visible {
                let found = check_vertex(&picture, &corners, &picked, &context);
                assert_eq!(found.index, corner.index, "{context}");
                assert_eq!(
                    picked["vertex"]["name"],
                    corner.name.clone().unwrap().as_str()
                );
                assert!(picked["vertex"]["distance"].as_f64().unwrap() < 1e-9);
                assert_eq!(corner.faces.len(), 3);
                corners_seen += 1;
            } else {
                assert_ne!(picked["vertex"]["index"], corner.index, "{context}");
                assert_eq!(picked["hit"], true, "{context}");
            }
        }
        for (listed, &visible) in edges.iter().zip(&seen) {
            for u in [0.15, 0.35, 0.5, 0.65, 0.85] {
                let (x, y, _) = picture.project(listed.curve.at(u));
                let picked = run(&mut s, "view.pick", pick_params(&view, x, y));
                let context = format!("{eye:?}: edge {} at {u}: {picked}", listed.index);
                if picked["kind"] == "vertex" {
                    // Near the end of an edge its corner is what is
                    // pointed at: one of the seven in sight.
                    let found = check_vertex(&picture, &corners, &picked, &context);
                    assert!(found.faces.iter().any(|f| looks_at_eye(f)), "{context}");
                    by_a_corner += 1;
                } else if visible {
                    let found = check_edge(&picture, &edges, &picked, 0.0, &context);
                    assert_eq!(found.index, listed.index, "{context}");
                    assert_eq!(
                        picked["edge"]["name"],
                        listed.name.clone().unwrap().as_str()
                    );
                    assert!(picked["edge"]["distance"].as_f64().unwrap() < 1e-6);
                    front += 1;
                } else {
                    // Behind the box: the face in front of it, or an edge
                    // of the front that passes within the radius there.
                    back += 1;
                    assert_eq!(picked["hit"], true, "{context}");
                    if picked["kind"] == "edge" {
                        let found = check_edge(&picture, &edges, &picked, 0.0, &context);
                        assert!(seen[found.index], "{context}: an edge at the back");
                    } else {
                        assert_eq!(picked["kind"], "face", "{context}");
                        assert!(looks_at_eye(picked["face"].as_str().unwrap()), "{context}");
                        faces += 1;
                    }
                }
            }
        }
    }
    assert_eq!(front + back + by_a_corner, 5 * 12 * 5);
    assert!(front >= 150 && back >= 40, "{front}, {back}, {by_a_corner}");
    assert!(
        faces >= 20,
        "{faces} of {back} picks at the back gave a face"
    );
    assert_eq!(corners_seen, 5 * 7, "seven corners of a box are in sight");
}

/// The radius, measured: positions at a known number of pixels from a
/// silhouette edge, on the side where there is no body.
#[test]
fn an_edge_is_picked_from_as_far_as_the_radius_and_no_further() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Box"}),
    );
    // Straight down, 4 pixels per unit: the box is 160 by 80 pixels about
    // the middle of the picture, its top edges on x = 20 and 180 and
    // y = 60 and 140.
    let view = json!({"camera": {"eye": [20, 10, 100], "target": [20, 10, 0], "up": [0, 1, 0],
                                  "projection": "orthographic", "height": 50},
                      "width": 200, "height": 200});
    let pick = |s: &mut Session, x: f64, y: f64, radius: Option<f64>| {
        let mut params = pick_params(&view, x, y);
        if let Some(radius) = radius {
            params["radius"] = radius.into();
        }
        run(s, "view.pick", params)
    };
    // To the left of the left side, at the height of the middle.
    for (off, radius, expected) in [
        (1.5, None, true),
        (3.9, None, true),
        (4.1, None, false),
        (10.0, None, false),
        (10.0, Some(10.5), true),
        (10.0, Some(9.5), false),
        (1.5, Some(1.0), false),
        (0.2, Some(0.0), false),
        (63.0, Some(64.0), false), // off the picture's side: x = -43
        // (Within 64 pixels there is a corner, and a vertex wins: below.)
        (19.0, Some(30.0), true),
    ] {
        let x = 20.0 - off;
        let out = pick(&mut s, x, 100.0, radius);
        let context = format!("{off} pixels off, radius {radius:?}: {out}");
        if x < 0.0 {
            assert_eq!(
                (&out["hit"], &out["outside"]),
                (&json!(false), &json!(true)),
                "{context}"
            );
            continue;
        }
        assert_eq!(out["radius"], radius.unwrap_or(4.0), "{context}");
        if expected {
            assert_eq!(out["kind"], "edge", "{context}");
            assert!(out.get("face").is_none(), "no face is under it: {context}");
            let distance = out["edge"]["distance"].as_f64().unwrap();
            assert!((distance - off).abs() < 1e-9, "{context}");
            // The left side of the top: x = 0, z = 10, at the position's
            // height (y = 10 in the model).
            let point = vec3(&out["edge"]["point"]);
            assert!(
                (point - Vec3::new(0.0, 10.0, 10.0)).length() < 1e-9,
                "{context}"
            );
            let faces: BTreeSet<&str> = out["edge"]["faces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap())
                .collect();
            assert_eq!(faces, BTreeSet::from(["F1:face(1)", "F1:face(4)"]));
            assert_eq!(out["edge"]["feature"], "F1");
            assert_eq!(out["edge"]["body"], "Box");
        } else {
            assert_eq!(out["hit"], false, "{context}");
            assert_eq!(out["outside"], false, "{context}");
            assert!(out.get("edge").is_none() && out.get("kind").is_none());
        }
    }
    // Inside the top face, the same distances: the face is under it either
    // way, and the edge is what is pointed at within the radius.
    let near = pick(&mut s, 22.5, 100.0, None);
    assert_eq!(
        (&near["kind"], &near["face"]),
        (&json!("edge"), &json!("F1:face(1)"))
    );
    let far = pick(&mut s, 30.0, 100.0, None);
    assert_eq!(
        (&far["kind"], &far["face"]),
        (&json!("face"), &json!("F1:face(1)"))
    );
    assert!(far.get("edge").is_none());
    // Near a corner, but further from it than the radius (4.6 pixels),
    // the nearer of the two edges wins: 3.5 pixels from the left side and
    // 3 from the bottom one (y = 140 in the picture).
    let corner = pick(&mut s, 23.5, 137.0, None);
    let faces: BTreeSet<&str> = corner["edge"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert_eq!(
        faces,
        BTreeSet::from(["F1:face(1)", "F1:face(2)"]),
        "{corner}"
    );
    assert!((corner["edge"]["distance"].as_f64().unwrap() - 3.0).abs() < 1e-9);

    // A radius that is not one is refused, and says what it takes.
    for bad in [json!(-1), json!(64.5), json!(1e300), json!("4"), json!([4])] {
        let mut params = pick_params(&view, 10.0, 10.0);
        params["radius"] = bad.clone();
        let error = s.execute("view.pick", &params).unwrap_err();
        assert!(
            error.message().starts_with("view.pick: ") && error.message().contains("radius"),
            "{bad}: {error}"
        );
    }
}

#[test]
fn edges_are_looked_up_in_the_scene_by_name_or_by_their_faces() {
    let mut s = workshop();
    let edges = listed_edges(&mut s);
    let scene = s.scene(None).unwrap().unwrap();
    // By its own name, every edge that has one. A copy's name has its
    // feature in it, a primitive's too: one body each.
    let mut named = 0;
    for listed in edges.iter().filter(|l| l.name.is_some()) {
        let name = listed.name.as_deref().unwrap();
        let found = scene.edges_named(None, Some(name), &[]).unwrap();
        assert_eq!(
            (found.feature.as_str(), found.body.as_str()),
            (listed.feature.as_str(), listed.body.as_str())
        );
        assert_eq!(found.edges, vec![listed.index], "{name}");
        assert_eq!(found.edge.as_deref(), Some(name));
        named += 1;
    }
    // Every edge has a name of its own (since 2026-10-14 also those of a
    // body a boolean made, by the faces they lie between).
    assert!(
        named > 12 + 2 + 48,
        "more than the box, the post and the studs"
    );
    assert_eq!(named, edges.len());
    // By the faces it lies between: what document.edges answers for the
    // same two names.
    let mut between = 0;
    for listed in &edges {
        let faces: Vec<String> = listed.faces.iter().cloned().collect();
        let pair = match faces.as_slice() {
            [a, b] => vec![a.clone(), b.clone()],
            [a] => vec![a.clone(), a.clone()],
            other => panic!("{other:?}"),
        };
        let found = scene
            .edges_named(Some(&listed.feature), None, &pair)
            .unwrap();
        let asked = run(
            &mut s,
            "document.edges",
            json!({"feature": listed.body, "between": pair}),
        );
        assert_eq!(found.edges.len() as u64, asked["count"].as_u64().unwrap());
        assert!(found.edges.contains(&listed.index), "{pair:?}");
        // The same edges: document.edges lists them in order, without
        // their index, so compare ends.
        for (entry, &index) in asked["edges"].as_array().unwrap().iter().zip(&found.edges) {
            let other = edges
                .iter()
                .find(|l| l.feature == listed.feature && l.index == index)
                .unwrap();
            let Curve::Line(a, b) = other.curve else {
                continue;
            };
            assert_eq!((vec3(&entry["start"]), vec3(&entry["end"])), (a, b));
        }
        between += 1;
    }
    assert_eq!(between, edges.len());
    // The hole of the drilled plate: its rims are between the wall of the
    // drill and the top and the bottom of the plate, one circle each.
    let rim = scene
        .edges_named(
            None,
            None,
            &[
                "F5:copy(0)/F3:face(1)".to_string(),
                "F5:copy(1)/F4:face(0)".to_string(),
            ],
        )
        .unwrap();
    assert_eq!((rim.body.as_str(), rim.edges.len()), ("Drilled", 1));

    // What is wrong is said.
    for (feature, edge, between, part) in [
        (None, None, vec![], "`edge` (its own name) or by `between`"),
        (
            None,
            Some("F1:edge(0)"),
            vec!["F1:face(0)", "F1:face(1)"],
            "not both",
        ),
        (None, Some("F1:face(0)"), vec![], "is a face"),
        (None, Some("nonsense"), vec![], ""),
        (
            None,
            Some("F1:edge(99)"),
            vec![],
            "no body has an edge F1:edge(99)",
        ),
        (
            Some("Post"),
            Some("F1:edge(0)"),
            vec![],
            "`Post` has no edge",
        ),
        (
            Some("Nobody"),
            Some("F1:edge(0)"),
            vec![],
            "no body of a feature `Nobody`",
        ),
        (None, None, vec!["F1:face(0)"], "takes two face names"),
        (
            None,
            None,
            vec!["F1:face(0)", "F1:face(1)", "F1:face(2)"],
            "takes two",
        ),
        (
            None,
            None,
            vec!["F1:face(0)", "F1:edge(1)"],
            "is not a face",
        ),
        // Opposite faces of the box share no edge.
        (
            None,
            None,
            vec!["F1:face(0)", "F1:face(1)"],
            "no body has an edge between",
        ),
    ] {
        let between: Vec<String> = between.into_iter().map(str::to_string).collect();
        let error = scene.edges_named(feature, edge, &between).unwrap_err();
        assert!(error.contains(part), "{edge:?} {between:?}: {error}");
    }
}

/// The acceptance test for vertices: every vertex of every body, in every
/// view; where it is in sight, a pick where it lands is that vertex, with
/// the name, the faces and the point `document.edges` lists.
#[test]
fn a_pick_on_a_vertex_in_sight_is_the_vertex_document_edges_lists() {
    let mut s = workshop();
    let corners = listed_vertices(&mut s);
    assert_eq!(corners.len(), 8 + 2 + 10 + 32 + 32);
    let (mut checked, mut hidden, mut crossings) = (0, 0, 0);
    let mut seen: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for (label, view) in &views() {
        let picture = picture_of(&mut s, view);
        for corner in &corners {
            let (x, y, _) = picture.project(corner.point);
            if !picture.inside(x, y, 1.0) {
                continue;
            }
            let picked = run(&mut s, "view.pick", pick_params(view, x, y));
            let context = format!(
                "{label}: {} vertex {} ({x}, {y}): {picked}",
                corner.body, corner.index
            );
            let found = (picked["kind"] == "vertex")
                .then(|| check_vertex(&picture, &corners, &picked, &context));
            if picture.in_sight(corner.point) {
                let found = found.unwrap_or_else(|| panic!("{context}: no vertex"));
                let distance = picked["vertex"]["distance"].as_f64().unwrap();
                assert!(distance < 1e-6, "{context}");
                if found.feature == corner.feature && found.index == corner.index {
                    checked += 1;
                    seen.entry(corner.body.clone())
                        .or_default()
                        .insert(corner.index);
                } else {
                    // Another vertex lands on the same spot, nearer to
                    // the eye and in sight (check_vertex). Counted.
                    assert!(
                        picked["vertex"]["depth"].as_f64().unwrap()
                            <= picture.project(corner.point).2,
                        "{context}"
                    );
                    crossings += 1;
                }
            } else {
                // Behind a face: not this vertex.
                if let Some(found) = found {
                    assert!(
                        (found.feature.as_str(), found.index)
                            != (corner.feature.as_str(), corner.index),
                        "{context}: picked through a face"
                    );
                }
                hidden += 1;
            }
        }
    }
    println!(
        "{checked} picks on vertices in sight, {hidden} on hidden ones, {crossings} crossings"
    );
    assert!(checked > 450 && hidden > 150, "{checked}, {hidden}");
    assert!(crossings * 50 < checked, "{crossings}");
    for (body, count) in [("Box", 8), ("Post", 2), ("Drilled", 10), ("Studs", 32)] {
        assert_eq!(seen[body].len(), count, "{body}: {:?}", seen[body]);
    }
    assert!(seen["Tiles"].len() >= 26, "{:?}", seen["Tiles"]);
}

/// The nearest vertex in sight within the radius wins, over any edge:
/// measured on a box seen straight down, 4 pixels per unit, its top on
/// x = 20 to 180 and y = 60 to 140 of the picture.
#[test]
fn a_vertex_wins_over_an_edge_within_the_radius_and_the_nearest_vertex_wins() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Box"}),
    );
    let view = json!({"camera": {"eye": [20, 10, 100], "target": [20, 10, 0], "up": [0, 1, 0],
                                  "projection": "orthographic", "height": 50},
                      "width": 200, "height": 200});
    let pick = |s: &mut Session, x: f64, y: f64, radius: Option<f64>| {
        let mut params = pick_params(&view, x, y);
        if let Some(radius) = radius {
            params["radius"] = radius.into();
        }
        run(s, "view.pick", params)
    };
    // The top-left corner of the top in the picture is (20, 60): the
    // model's (0, 20, 10), where the top, the back and the left meet.
    let corner_faces = BTreeSet::from(["F1:face(1)", "F1:face(3)", "F1:face(4)"]);
    for (dx, dy, radius, expected) in [
        (0.0, 0.0, None, "vertex"),
        // Along the top's left side, which is 0 pixels away all the time.
        (0.0, 3.5, None, "vertex"),
        (0.0, 4.5, None, "edge"),
        // Off the body, diagonally: 2.83 and 4.24 pixels from the corner.
        (-2.0, -2.0, None, "vertex"),
        (-3.0, -3.0, None, "none"),
        (-3.0, -3.0, Some(4.5), "vertex"),
        // Inside the top face.
        (2.0, 3.0, None, "vertex"),
        (3.0, 3.0, None, "edge"),
        (9.0, 9.0, None, "face"),
        (9.0, 9.0, Some(13.0), "vertex"),
        (2.0, 3.0, Some(0.0), "face"),
    ] {
        let out = pick(&mut s, 20.0 + dx, 60.0 + dy, radius);
        let context = format!("({dx}, {dy}), radius {radius:?}: {out}");
        if expected == "none" {
            assert_eq!(out["hit"], false, "{context}");
            continue;
        }
        assert_eq!(out["kind"], expected, "{context}");
        assert_eq!(
            out.get("vertex").is_some(),
            expected == "vertex",
            "{context}"
        );
        assert_eq!(out.get("edge").is_some(), expected == "edge", "{context}");
        if expected == "vertex" {
            let vertex = &out["vertex"];
            let d = f64::hypot(dx, dy);
            assert!(
                (vertex["distance"].as_f64().unwrap() - d).abs() < 1e-9,
                "{context}"
            );
            assert!(
                (vec3(&vertex["point"]) - Vec3::new(0.0, 20.0, 10.0)).length() < 1e-12,
                "{context}"
            );
            let faces: BTreeSet<&str> = vertex["faces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap())
                .collect();
            assert_eq!(faces, corner_faces, "{context}");
            assert_eq!(vertex["feature"], "F1");
            assert_eq!(vertex["body"], "Box");
            assert!(vertex["name"].as_str().unwrap().starts_with("F1:vertex("));
        }
        // The face under the position is said either way.
        assert_eq!(
            out.get("face").is_some(),
            dx >= 0.0 && dy >= 0.0,
            "{context}"
        );
    }
    // Between two corners 80 pixels apart, with a radius that reaches
    // both: the nearer one. The left side runs from (20, 60) to (20, 140).
    let upper = pick(&mut s, 20.0, 95.0, Some(50.0));
    let lower = pick(&mut s, 20.0, 105.0, Some(50.0));
    assert!((vec3(&upper["vertex"]["point"]) - Vec3::new(0.0, 20.0, 10.0)).length() < 1e-12);
    assert!((vec3(&lower["vertex"]["point"]) - Vec3::new(0.0, 0.0, 10.0)).length() < 1e-12);
    // The corners at the bottom of the box land on the same spots and are
    // behind the top: at a spot, the one in sight.
    assert!((upper["vertex"]["depth"].as_f64().unwrap() - 90.0).abs() < 1e-9);
}

#[test]
fn vertices_are_looked_up_in_the_scene_by_name_or_by_their_faces() {
    let mut s = workshop();
    let corners = listed_vertices(&mut s);
    let scene = s.scene(None).unwrap().unwrap();
    let mut named = 0;
    for corner in &corners {
        // The scene has the vertex where document.edges has it.
        let body = scene
            .bodies
            .iter()
            .find(|b| b.feature == corner.feature)
            .unwrap();
        assert_eq!(body.vertices[corner.index], corner.point);
        if let Some(name) = &corner.name {
            let found = scene.vertices_named(None, Some(name), &[]).unwrap();
            assert_eq!(
                (found.feature.as_str(), found.vertices.as_slice()),
                (corner.feature.as_str(), [corner.index].as_slice())
            );
            named += 1;
        }
        // By all the faces that meet in it: itself, and whatever other
        // vertex the same faces meet in (the two ends of an edge between
        // two pieces of one face, after a boolean).
        let faces: Vec<String> = corner.faces.iter().cloned().collect();
        let found = scene
            .vertices_named(Some(&corner.feature), None, &faces)
            .unwrap();
        assert!(found.vertices.contains(&corner.index), "{faces:?}");
        for &other in &found.vertices {
            let other = corners
                .iter()
                .find(|c| c.feature == corner.feature && c.index == other)
                .unwrap();
            assert!(corner.faces.is_subset(&other.faces));
        }
        assert_eq!(found.body, corner.body);
    }
    // Every vertex has a name of its own (since 2026-10-14 also those of
    // a body a boolean made, by the faces that meet in it).
    assert!(named > 8 + 2 + 32);
    assert_eq!(named, corners.len());
    // One face of the box: its four corners.
    let four = scene
        .vertices_named(None, None, &["F1:face(1)".to_string()])
        .unwrap();
    assert_eq!(four.vertices.len(), 4);
    for (feature, vertex, corner, part) in [
        (None, None, vec![], "`vertex` (its own name) or by `corner`"),
        (None, Some("F1:vertex(0)"), vec!["F1:face(0)"], "not both"),
        (None, Some("F1:face(0)"), vec![], "is a face"),
        (None, Some("nonsense"), vec![], ""),
        (
            None,
            Some("F1:vertex(99)"),
            vec![],
            "no body has a vertex F1:vertex(99)",
        ),
        (
            Some("Post"),
            Some("F1:vertex(0)"),
            vec![],
            "`Post` has no vertex",
        ),
        (
            Some("Nobody"),
            Some("F1:vertex(0)"),
            vec![],
            "no body of a feature `Nobody`",
        ),
        (None, None, vec!["F1:vertex(1)"], "is not a face"),
        (
            None,
            None,
            vec!["F1:face(0)", "F1:face(1)"],
            "no body has a vertex where",
        ),
        (None, None, vec!["F1:face(0)"; 17], "at most 16"),
    ] {
        let corner: Vec<String> = corner.into_iter().map(str::to_string).collect();
        let error = scene.vertices_named(feature, vertex, &corner).unwrap_err();
        assert!(error.contains(part), "{vertex:?} {corner:?}: {error}");
    }
}
