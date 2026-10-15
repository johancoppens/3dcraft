//! Faces whose fitted edges run off the domain of their spline surface:
//! the side of a lofted blade whose leading edge and tip are rounded
//! (three bodies of `crates/engine/tests/blend_findings/meet-le-tip-blade.json`,
//! face 1 of each, as a sheet; docs/design/topology.md, finding of
//! 2026-10-15 *A fitted edge that runs off its surface*).
//!
//! The fillets' contact edges are fitted within the tolerance of the
//! blade's side and end on its border `v = 1`, tangent to it, up to
//! 8e-7 off the surface: over their last half percent the foot of their
//! points is held at the border, and the parameters along the edge have a
//! kink there. With a panel across it the boundary integral never settled
//! (to 1e-12 and 3e-11 of a piece's size from one doubling to the next,
//! up to the seventh), and each face was refused as one that does not
//! settle. With the panels ending at the kink they settle at the second
//! sum.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Transform, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, VertexId, mass_properties};
use serde_json::Value;

/// The area of each face without the fix, every piece of its boundary
/// summed at the last doubling the measure allows (the seventh: 128 times
/// the panels of the first sums) whether or not it settled, by a build of
/// `topo` with the panel end at the run-off and the convergence test
/// switched off (2026-10-15). The run-off pieces converge slowly but do
/// converge; these are where they go.
const SLOW: [f64; 3] = [
    22.343_815_774_683_396,
    32.100_614_427_099_89,
    22.347_896_070_048_57,
];

const FACES: [&str; 3] = [
    include_str!("fixtures/mass_run_off/side-1.json"),
    include_str!("fixtures/mass_run_off/side-2.json"),
    include_str!("fixtures/mass_run_off/side-3.json"),
];

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

fn index(v: &Value) -> usize {
    usize::try_from(v.as_u64().unwrap()).unwrap()
}

fn id(v: &Value) -> u32 {
    u32::try_from(v.as_u64().unwrap()).unwrap()
}

/// The sheet of one face: a NURBS surface bounded by lines and NURBS
/// curves, every number as it was in the body (shortest round-trip form).
fn sheet(json: &str) -> Body {
    let v: Value = serde_json::from_str(json).unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    for p in v["vertices"].as_array().unwrap() {
        body.add_vertex(vec3(p));
    }
    for e in v["edges"].as_array().unwrap() {
        let c = &e["curve"];
        let curve = if let Some(l) = c.get("line") {
            Curve3::Line(Line::new(vec3(&l["o"]), vec3(&l["d"])).unwrap())
        } else {
            let n = &c["nurbs"];
            Curve3::Nurbs(
                NurbsCurve::new(
                    index(&n["degree"]),
                    floats(&n["knots"]),
                    points(&n["ctrl"]),
                    floats(&n["weights"]),
                )
                .unwrap(),
            )
        };
        let range = (
            e["range"][0].as_f64().unwrap(),
            e["range"][1].as_f64().unwrap(),
        );
        body.add_edge(
            curve,
            range,
            VertexId(id(&e["start"])),
            VertexId(id(&e["end"])),
        );
    }
    let mut loops = Vec::new();
    for l in v["loops"].as_array().unwrap() {
        let coedges = l
            .as_array()
            .unwrap()
            .iter()
            .map(|c| Coedge {
                edge: EdgeId(id(&c[0])),
                reversed: c[1].as_bool().unwrap(),
            })
            .collect();
        loops.push(body.add_loop(coedges));
    }
    let f = &v["faces"][0];
    let n = &f["surface"]["nurbs"];
    let surface = Surface::Nurbs(
        NurbsSurface::new(
            index(&n["degrees"][0]),
            index(&n["degrees"][1]),
            floats(&n["knots_u"]),
            floats(&n["knots_v"]),
            index(&n["counts"][0]),
            index(&n["counts"][1]),
            points(&n["ctrl"]),
            floats(&n["weights"]),
        )
        .unwrap(),
    );
    let face = body.add_face(surface, f["reversed"].as_bool().unwrap(), loops);
    body.add_shell(vec![face]);
    body
}

/// Each face is measured: its area is the slow sums' to 1e-11 (measured:
/// 9e-15 at the most), and that of its mesh to what the mesh is off by.
#[test]
fn faces_whose_edges_run_off_their_surface_are_measured() {
    for (k, json) in FACES.iter().enumerate() {
        let body = sheet(json);
        let area = mass_properties(&body)
            .unwrap_or_else(|e| panic!("face {k}: {e}"))
            .area;
        let slow = SLOW[k];
        assert!(
            (area - slow).abs() <= 1e-11 * slow,
            "face {k}: {area} against the slow sums' {slow}"
        );
        let chord = 1e-4;
        let params = TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        };
        let mesh = tessellate_body(&body, &params).unwrap();
        // A chord `c` short of an arc of radius r cuts off a share of
        // about c / r of the area; the blade's side curves with a few
        // millimetres at the least.
        let off = (mesh.area() - area).abs() / area;
        assert!(off < 1e-4, "face {k}: mesh {} against {area}", mesh.area());
    }
}

/// The measure does not depend on where the face is: turned and moved a
/// thousand units away, each face has the same area to 1e-12. A sum that
/// only settles by chance does not hold that (two of the three were
/// refused as they stood, one only where it stood in its body).
#[test]
fn faces_whose_edges_run_off_their_surface_measure_alike_anywhere() {
    let turn = Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1e3, -2e3, 5e2)));
    for (k, json) in FACES.iter().enumerate() {
        let body = sheet(json);
        let here = mass_properties(&body).unwrap().area;
        let moved = body.transformed(&turn).unwrap();
        let there = mass_properties(&moved)
            .unwrap_or_else(|e| panic!("face {k} moved: {e}"))
            .area;
        assert!(
            (here - there).abs() <= 1e-12 * here,
            "face {k}: {here} against {there}"
        );
    }
}
