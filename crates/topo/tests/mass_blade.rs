//! Scratch: measure dumped faces.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::Vec3;
use craft3d_topo::{Body, BodyKind, Coedge, mass_properties};
use serde_json::Value;

fn vec3(v: &Value) -> Vec3 {
    Vec3::new(
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    )
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn points(v: &Value) -> Vec<Vec3> {
    v.as_array().unwrap().iter().map(vec3).collect()
}

fn body(json: &str) -> Body {
    let v: Value = serde_json::from_str(json).unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    for p in v["vertices"].as_array().unwrap() {
        body.add_vertex(vec3(p));
    }
    for e in v["edges"].as_array().unwrap() {
        let c = &e["curve"];
        let curve = if let Some(l) = c.get("line") {
            Curve3::Line(Line::new(vec3(&l["o"]), vec3(&l["d"])).unwrap())
        } else if let Some(n) = c.get("nurbs") {
            Curve3::Nurbs(
                NurbsCurve::new(
                    n["degree"].as_u64().unwrap() as usize,
                    floats(&n["knots"]),
                    points(&n["ctrl"]),
                    floats(&n["weights"]),
                )
                .unwrap(),
            )
        } else {
            panic!("{c}")
        };
        let range = (e["range"][0].as_f64().unwrap(), e["range"][1].as_f64().unwrap());
        let start = craft3d_topo::VertexId(e["start"].as_u64().unwrap() as u32);
        let end = craft3d_topo::VertexId(e["end"].as_u64().unwrap() as u32);
        body.add_edge(curve, range, start, end);
    }
    let mut loops = Vec::new();
    for l in v["loops"].as_array().unwrap() {
        let coedges = l
            .as_array()
            .unwrap()
            .iter()
            .map(|c| Coedge {
                edge: craft3d_topo::EdgeId(c[0].as_u64().unwrap() as u32),
                reversed: c[1].as_bool().unwrap(),
            })
            .collect();
        loops.push(body.add_loop(coedges));
    }
    let f = &v["faces"][0];
    let n = &f["surface"]["nurbs"];
    let surface = Surface::Nurbs(
        NurbsSurface::new(
            n["degrees"][0].as_u64().unwrap() as usize,
            n["degrees"][1].as_u64().unwrap() as usize,
            floats(&n["knots_u"]),
            floats(&n["knots_v"]),
            n["counts"][0].as_u64().unwrap() as usize,
            n["counts"][1].as_u64().unwrap() as usize,
            points(&n["ctrl"]),
            floats(&n["weights"]),
        )
        .unwrap(),
    );
    let face = body.add_face(surface, f["reversed"].as_bool().unwrap(), loops);
    body.add_shell(vec![face]);
    body
}

#[test]
fn scratch() {
    let path = std::env::var("FACE").unwrap();
    let body = body(&std::fs::read_to_string(path).unwrap());
    println!("{:?}", mass_properties(&body));
}

#[test]
fn scratch_edges() {
    let path = std::env::var("FACE").unwrap();
    let body = body(&std::fs::read_to_string(path).unwrap());
    let face = &body.faces[0];
    let s = &face.surface;
    if let Surface::Nurbs(n) = s {
        println!("domain {:?} degrees {:?} counts {:?}", n.domain(), n.degrees(), n.counts());
        println!("knots_u {:?}", n.knots_u());
        println!("knots_v {:?}", n.knots_v());
    }
    let lp = &body.loops[0];
    for (i, c) in lp.coedges.iter().enumerate() {
        let e = &body.edges[c.edge.0 as usize];
        let kn = match &e.curve {
            Curve3::Nurbs(n) => format!("deg {} knots {:?}", n.degree(), n.knots()),
            other => other.kind().to_string(),
        };
        println!("edge {i} rev {} range {:?} {kn}", c.reversed, e.range);
        let k = 40;
        let near: f64 = std::env::var("NEAR").ok().and_then(|x| x.parse().ok()).unwrap_or(1.0);
        for j in 0..=k {
            let x = if std::env::var_os("START").is_some() {
                near * j as f64 / k as f64
            } else {
                1.0 - near + near * j as f64 / k as f64
            };
            let t = e.range.0 + (e.range.1 - e.range.0) * x;
            let p = e.curve.eval(t);
            let (u, v) = s.project(p).unwrap();
            let q = s.eval(u, v);
            println!("   t {t:.6} uv ({u:.12}, {v:.12}) off {:.3e}", (p - q).length());
        }
    }
}
