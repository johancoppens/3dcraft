//! What the fold rule's tests share (fold_rule.rs): the fitted top
//! pulled through itself or into an overhang, a document built by
//! commands, the review's notches, a STEP file read.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

pub fn tolerance(linear: f64) -> Tolerance {
    Tolerance::new(linear, 1e-9).unwrap()
}

pub fn read_step(path: &Path, t: f64) -> Option<Body> {
    let bytes = std::fs::read(path).ok()?;
    let tol = tolerance(t);
    let options = craft3d_stepread::Options {
        metres_per_unit: 1e-3,
        tolerance: tol,
        loosest: 1000.0 * tol.linear,
    };
    craft3d_stepread::parse(&bytes)
        .and_then(|p| p.build(0, &options))
        .ok()
        .map(|read| read.body)
}

const N: usize = 13;

fn knots(degree: usize) -> Vec<f64> {
    let inner = N - degree - 1;
    let mut k = vec![0.0; degree + 1];
    for i in 1..=inner {
        k.push(i as f64 / (inner + 1) as f64);
    }
    k.extend(vec![1.0; degree + 1]);
    k
}

fn axis(l: f64, s: f64) -> [f64; N] {
    let c = 0.5 * l;
    let a = c - 4.0 * s;
    [
        0.0,
        0.5 * a,
        a,
        c - 3.0 * s,
        c - 2.0 * s,
        c - s,
        c,
        c + s,
        c + 2.0 * s,
        c + 3.0 * s,
        c + 4.0 * s,
        c + 4.0 * s + 0.5 * a,
        l,
    ]
}

const ROWS: [f64; N] = [
    0.0, 0.0, 0.0, 0.3, 0.7, 1.0, 1.0, 1.0, 0.7, 0.3, 0.0, 0.0, 0.0,
];

/// fold_probe's block, its top pulled through itself (`looped`) or into
/// an overhang, of degree `degree` (the review's).
pub fn roof_block(l: f64, s: f64, looped: bool, degree: usize) -> Body {
    let xs = axis(l, s);
    let h = l / 3.0;
    let (mut dx, mut dz) = ([0.0; N], [0.0; N]);
    if looped {
        dx[5] = 2.2 * s;
        dx[7] = -2.2 * s;
        dz[5] = 1.2 * s;
        dz[6] = 2.0 * s;
        dz[7] = 1.2 * s;
    } else {
        dx[5] = 1.6 * s;
        dx[7] = -1.6 * s;
        let rise = [0.0, 0.3, 0.7, 1.1, 1.5, 1.8, 2.0, 2.0, 2.0, 2.0];
        for (k, r) in rise.iter().enumerate() {
            dz[3 + k] = r * s;
        }
    }
    let mut net = Vec::with_capacity(N * N);
    for i in 0..N {
        for j in 0..N {
            let a = ROWS[j];
            net.push(Vec3::new(xs[i] + a * dx[i], xs[j], h + a * dz[i]));
        }
    }
    let surface = NurbsSurface::new(
        degree,
        degree,
        knots(degree),
        knots(degree),
        N,
        N,
        net.clone(),
        vec![1.0; N * N],
    )
    .unwrap();
    let at = |i: usize, j: usize| net[i * N + j];
    let mut body = Body::new(BodyKind::Solid);
    let ground = [(0.0, 0.0), (l, 0.0), (l, l), (0.0, l)];
    let low: Vec<VertexId> = ground
        .iter()
        .map(|&(x, y)| body.add_vertex(Vec3::new(x, y, 0.0)))
        .collect();
    let high: Vec<VertexId> = [(0, 0), (N - 1, 0), (N - 1, N - 1), (0, N - 1)]
        .iter()
        .map(|&(i, j)| body.add_vertex(at(i, j)))
        .collect();
    let spline = |points: Vec<Vec3>| {
        Curve3::Nurbs(NurbsCurve::polynomial(degree, knots(degree), points).unwrap())
    };
    let along_u = |j: usize| spline((0..N).map(|i| at(i, j)).collect());
    let along_v = |i: usize| spline((0..N).map(|j| at(i, j)).collect());
    let roof = [
        (
            body.add_edge(along_u(0), (0.0, 1.0), high[0], high[1]),
            true,
        ),
        (
            body.add_edge(along_v(N - 1), (0.0, 1.0), high[1], high[2]),
            true,
        ),
        (
            body.add_edge(along_u(N - 1), (0.0, 1.0), high[3], high[2]),
            false,
        ),
        (
            body.add_edge(along_v(0), (0.0, 1.0), high[0], high[3]),
            false,
        ),
    ];
    let straight = |body: &mut Body, from: VertexId, to: VertexId| -> EdgeId {
        let (a, b) = (
            body.vertex(from).unwrap().point,
            body.vertex(to).unwrap().point,
        );
        body.add_edge(
            Curve3::Line(Line::through(a, b).unwrap()),
            (0.0, a.distance(b)),
            from,
            to,
        )
    };
    let floor = [
        straight(&mut body, low[0], low[1]),
        straight(&mut body, low[1], low[2]),
        straight(&mut body, low[2], low[3]),
        straight(&mut body, low[3], low[0]),
    ];
    let up = [
        straight(&mut body, low[0], high[0]),
        straight(&mut body, low[1], high[1]),
        straight(&mut body, low[2], high[2]),
        straight(&mut body, low[3], high[3]),
    ];
    let coedge = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
    let mut faces: Vec<FaceId> = Vec::new();
    for k in 0..4 {
        let next = (k + 1) % 4;
        let lp = body.add_loop(vec![
            coedge(floor[k], false),
            coedge(up[next], false),
            coedge(roof[k].0, roof[k].1),
            coedge(up[k], true),
        ]);
        let start = Vec3::new(ground[k].0, ground[k].1, 0.0);
        let direction = Vec3::new(ground[next].0, ground[next].1, 0.0) - start;
        let frame = Frame::new(start, direction.cross(Vec3::Z), direction).unwrap();
        faces.push(body.add_face(Surface::Plane(Plane::new(frame)), false, vec![lp]));
    }
    let floor_loop = body.add_loop(floor.iter().rev().map(|&e| coedge(e, true)).collect());
    let floor_frame = Frame::new(Vec3::ZERO, -Vec3::Z, Vec3::X).unwrap();
    faces.push(body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    ));
    let roof_loop = body.add_loop(roof.iter().map(|&(e, along)| coedge(e, !along)).collect());
    faces.push(body.add_face(Surface::Nurbs(surface), false, vec![roof_loop]));
    body.add_shell(faces);
    body
}

/// A new document at `linear`, the commands run, its bodies (the
/// review's).
pub fn script(tag: &str, linear: f64, steps: &[(&str, Value)]) -> Vec<Arc<Body>> {
    let dir = super::common::workdir("fold-rule", tag);
    let mut s = super::common::session_in(&dir);
    if let Err(e) = s.execute("file.new", &json!({"units": "mm", "tolerance": linear})) {
        println!("FOLDRULE {tag} | file.new refused: {e}");
        return Vec::new();
    }
    for (id, params) in steps {
        match s.execute(id, params) {
            Ok(value) => {
                if value.get("status").and_then(Value::as_str) == Some("failed") {
                    println!(
                        "FOLDRULE {tag} | {id} failed: {}",
                        value.get("error").map(Value::to_string).unwrap_or_default()
                    );
                }
            }
            Err(e) => println!("FOLDRULE {tag} | {id} refused: {e}"),
        }
    }
    let Some(Ok(doc)) = s.document_bytes().map(|b| craft3d_format::from_bytes(&b)) else {
        println!("FOLDRULE {tag} | no document");
        return Vec::new();
    };
    let regen = craft3d_regen::regenerate(&doc);
    regen.bodies().map(|(_, b)| b.clone()).collect()
}

/// The review's notch: a block whose top is one cubic spline, its root a
/// smooth inside turn.
pub fn notch_bodies(tag: &str, linear: f64, control: &[[f64; 2]], k: f64) -> Vec<Arc<Body>> {
    let control: Vec<[f64; 2]> = control.iter().map(|p| [p[0] * k, p[1] * k]).collect();
    let line = |a: [f64; 2], b: [f64; 2]| {
        (
            "sketch.line",
            json!({"sketch": "P", "from": [a[0] * k, a[1] * k], "to": [b[0] * k, b[1] * k]}),
        )
    };
    script(
        tag,
        linear,
        &[
            ("sketch.create", json!({"name": "P"})),
            (
                "sketch.spline",
                json!({"sketch": "P", "control": control, "degree": 3}),
            ),
            line([0.0, 30.0], [0.0, 0.0]),
            line([0.0, 0.0], [40.0, 0.0]),
            line([40.0, 0.0], [40.0, 30.0]),
            (
                "feature.extrude",
                json!({"sketch": "P", "distance": 10.0 * k, "name": "B"}),
            ),
        ],
    )
}

pub fn right_angle(e: f64) -> Vec<[f64; 2]> {
    vec![
        [40.0, 30.0],
        [32.0, 30.0],
        [20.0 + e, 20.0 + e],
        [20.0, 20.0],
        [20.0 - e, 20.0 + e],
        [8.0, 30.0],
        [0.0, 30.0],
    ]
}

pub fn narrow(e: f64) -> Vec<[f64; 2]> {
    vec![
        [40.0, 30.0],
        [30.0, 30.0],
        [22.68, 30.0],
        [20.0 + 0.259 * e, 20.0 + 0.966 * e],
        [20.0, 20.0],
        [20.0 - 0.259 * e, 20.0 + 0.966 * e],
        [17.32, 30.0],
        [10.0, 30.0],
        [0.0, 30.0],
    ]
}
