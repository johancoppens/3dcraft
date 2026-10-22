//! The mesh of a body with loose entities (tolerant modelling, slice 2):
//! closed, and every point on its face's surface within the tolerance of
//! the entity it lies on (`tess::off_surfaces`): rounding inside a face,
//! the edge's or vertex's own on the boundary. Without the table the same
//! mesh is off its surfaces, so the check reads the entities' tolerances.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_tess::{TessParams, off_surfaces, tessellate_body};
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::{Body, EdgeId, FaceId, Tolerances, check_capped, record_edge, record_vertex};

fn edges_of(body: &Body, face: FaceId) -> Vec<EdgeId> {
    let mut edges: Vec<EdgeId> = body.faces[face.0 as usize]
        .loops
        .iter()
        .flat_map(|l| body.loops[l.0 as usize].coedges.iter().map(|c| c.edge))
        .collect();
    edges.sort();
    edges.dedup();
    edges
}

/// `body` with the surface of every other face moved by `gap` along a
/// slanted direction (off its edges), every gap measured and recorded.
fn loosened(body: &Body, gap: f64, tol: &Tolerance, cap: f64) -> Body {
    let mut out = body.clone();
    let dir = Vec3::new(0.48, -0.6, 0.64);
    for f in (0..out.faces.len()).step_by(2) {
        if out.faces[f].loops.is_empty() {
            continue;
        }
        let surface = out.faces[f]
            .surface
            .transformed(&Transform::translation(dir * gap))
            .unwrap();
        out.faces[f].surface = surface;
        for e in edges_of(&out, FaceId(f as u32)) {
            record_edge(&mut out, e, tol, cap).unwrap();
        }
    }
    for v in 0..out.vertices.len() {
        record_vertex(&mut out, craft3d_topo::VertexId(v as u32), tol, cap).unwrap();
    }
    out
}

#[test]
fn a_loose_body_is_meshed_on_its_surfaces_within_its_entities_tolerances() {
    let mut checked = 0;
    for scale in [1e-3, 1.0, 1e6] {
        let tol = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
        let cap = 1000.0 * tol.linear;
        let frame = Frame::WORLD;
        let mut bodies: Vec<(String, Body)> = solids::all(&frame, scale)
            .unwrap()
            .into_iter()
            .map(|r| (r.name.to_string(), r.body))
            .collect();
        bodies.push((
            "domed block".into(),
            trimmed::domed_block(&frame, 2.0 * scale, 1.5 * scale, scale, true)
                .unwrap()
                .body,
        ));
        for (name, body) in bodies {
            for k in [30.0, 300.0] {
                let what = format!("{name} at {scale:e}, k = {k}");
                let loose = loosened(&body, k * tol.linear, &tol, cap);
                if !loose.is_tolerant() {
                    continue;
                }
                assert!(check_capped(&loose, &tol, cap).is_empty(), "{what}");
                let params = TessParams {
                    chordal_tolerance: 1e-3 * scale,
                    ..TessParams::default()
                };
                let mesh = tessellate_body(&loose, &params).unwrap();
                assert!(mesh.weld().unwrap().is_closed(), "{what}: not closed");
                let rounding = 1e-9 * scale;
                let off = off_surfaces(&loose, &mesh, tol.linear, rounding);
                assert!(off.is_empty(), "{what}: {:?}", &off[..off.len().min(3)]);
                // The same mesh against the same body without its table:
                // its boundary points are off their surfaces.
                let mut bare = loose.clone();
                bare.tolerances = Tolerances::default();
                let off = off_surfaces(&bare, &mesh, tol.linear, rounding);
                assert!(!off.is_empty(), "{what}: the table is not what holds it");
                checked += 1;
            }
        }
    }
    println!("{checked} loose bodies meshed on their surfaces");
    assert!(checked >= 12, "{checked}");
}

/// An exact body's mesh is on its surfaces to rounding everywhere: the
/// check is the oracle's of before for a body without loose entities.
#[test]
fn an_exact_body_is_meshed_on_its_surfaces_to_rounding() {
    for r in solids::all(&Frame::WORLD, 1.0).unwrap() {
        let params = TessParams {
            chordal_tolerance: 1e-3,
            ..TessParams::default()
        };
        let mesh = tessellate_body(&r.body, &params).unwrap();
        let off = off_surfaces(&r.body, &mesh, 1e-6, 1e-9);
        assert!(off.is_empty(), "{}: {off:?}", r.name);
    }
}
