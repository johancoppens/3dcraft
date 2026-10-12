//! Oracle 4 (docs/testing.md): mesh area converges to the area computed
//! from the exact B-rep. The two share no code beyond surface evaluation:
//! one sums triangles, the other integrates around the face's boundary.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{BezierPatch, Curve3, NurbsCurve, Surface};
use craft3d_math::{Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_patches};
use craft3d_topo::{Body, BodyKind, Coedge, check, mass_properties};

fn bumpy() -> BezierPatch {
    let mut net = [[Vec3::ZERO; 4]; 4];
    for (i, row) in net.iter_mut().enumerate() {
        for (j, p) in row.iter_mut().enumerate() {
            let (x, y) = (i as f64, j as f64 * 1.5);
            *p = Vec3::new(x + 0.2 * y, y, ((i * 7 + j * 3) % 5) as f64 * 0.5 - 0.3 * x);
        }
    }
    BezierPatch::bicubic(net).unwrap()
}

/// A sheet body with `patch` as its one face, bounded by its four edges.
fn sheet(patch: &BezierPatch) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let corner = |i, j| patch.control_point(i, j).unwrap();
    let v: Vec<_> = [(0, 0), (3, 0), (3, 3), (0, 3)]
        .iter()
        .map(|&(i, j)| body.add_vertex(corner(i, j)))
        .collect();
    let cubic = |points: Vec<Vec3>| {
        Curve3::Nurbs(
            NurbsCurve::polynomial(3, vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], points)
                .unwrap(),
        )
    };
    let along_u = |j: usize| cubic((0..4).map(|i| corner(i, j)).collect());
    let along_v = |i: usize| cubic((0..4).map(|j| corner(i, j)).collect());
    let bottom = body.add_edge(along_u(0), (0.0, 1.0), v[0], v[1]);
    let right = body.add_edge(along_v(3), (0.0, 1.0), v[1], v[2]);
    let top = body.add_edge(along_u(3), (0.0, 1.0), v[3], v[2]);
    let left = body.add_edge(along_v(0), (0.0, 1.0), v[0], v[3]);
    // Counter-clockwise in (u, v): the face is not reversed.
    let use_ = |edge, reversed| Coedge { edge, reversed };
    let loop_ = body.add_loop(vec![
        use_(bottom, false),
        use_(right, false),
        use_(top, true),
        use_(left, true),
    ]);
    let face = body.add_face(Surface::Bezier(patch.clone()), false, vec![loop_]);
    body.add_shell(vec![face]);
    body
}

#[test]
fn mesh_area_converges_to_the_exact_area() {
    let patch = bumpy();
    let body = sheet(&patch);
    assert_eq!(check(&body, &Tolerance::default()), vec![]);
    let exact = mass_properties(&body).unwrap().area;

    let mut previous = f64::INFINITY;
    for tolerance in [1e-2, 1e-3, 1e-4, 1e-5] {
        let params = TessParams {
            chordal_tolerance: tolerance,
            ..TessParams::default()
        };
        let mesh = tessellate_patches(std::slice::from_ref(&patch), &params).unwrap();
        let error = (mesh.area() - exact).abs() / exact;
        println!(
            "tolerance {tolerance:e}: mesh {} exact {exact} relative error {error:e}",
            mesh.area()
        );
        // The error shrinks with the tolerance (from either side: triangles
        // across a twisted cell can also over-estimate).
        assert!(error < previous);
        previous = error;
    }
    assert!(
        previous < 1e-5,
        "relative error {previous:e} at the finest tolerance"
    );
}

#[test]
fn the_reversed_sheet_has_the_same_area() {
    let patch = bumpy();
    let mut body = sheet(&patch);
    body.faces[0].reversed = true;
    body.loops[0].coedges.reverse();
    for coedge in &mut body.loops[0].coedges {
        coedge.reversed = !coedge.reversed;
    }
    let flipped = mass_properties(&body).unwrap().area;
    let plain = mass_properties(&sheet(&patch)).unwrap().area;
    assert!((flipped - plain).abs() < 1e-12 * plain);
}
