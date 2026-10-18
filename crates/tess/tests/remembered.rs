//! Meshes of faces remembered (`remembered.rs`): a face meshed before, from
//! the same surface, edges and points, is given the mesh made for it then.
//! Held here: whatever was kept, every mesh and every check of a body
//! against itself is the one made with nothing kept, bit for bit; a face
//! two bodies share is meshed once.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::{Frame, Vec3};
use craft3d_tess::{
    SelfCheckParams, TessParams, body_self_intersections, forget_kept_meshes, kept_meshes,
    tessellate_body,
};
use craft3d_testkit::solids;
use craft3d_topo::{Body, merge};

fn bits(mesh: &craft3d_tess::Mesh) -> String {
    let words = |points: &[Vec3]| -> Vec<[u64; 3]> {
        points
            .iter()
            .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
            .collect()
    };
    format!(
        "{:?} {:?} {:?} {:?}",
        words(&mesh.positions),
        words(&mesh.normals),
        mesh.triangles,
        mesh.face_ids
    )
}

#[test]
fn remembered_faces_mesh_as_faces_made_afresh() {
    for scale in [1e-3, 1.0, 1e6] {
        let frame = Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap();
        let far = Frame::new(
            Vec3::new(9.0, 1.0, -2.0) * scale,
            Vec3::new(0.0, 0.2, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap();
        let block = solids::block(&frame, 4.0 * scale, 2.0 * scale, 1.0 * scale)
            .unwrap()
            .body;
        let cone = solids::spline_cone(&frame, 1.5 * scale, 2.0 * scale)
            .unwrap()
            .body;
        let others = [
            solids::sphere(&far, 1.5 * scale).unwrap().body,
            solids::torus(&far, 2.0 * scale, 0.5 * scale).unwrap().body,
            solids::spline_cone(&far, 1.0 * scale, 3.0 * scale)
                .unwrap()
                .body,
        ];
        let bodies: Vec<Body> = others
            .iter()
            .flat_map(|other| {
                [
                    merge(&[&block, other]).unwrap().0,
                    merge(&[&cone, other]).unwrap().0,
                ]
            })
            .collect();
        let params = TessParams {
            chordal_tolerance: 2e-3 * scale,
            ..TessParams::default()
        };
        let checks = SelfCheckParams::new(1e-6 * scale);
        // Made with nothing kept, one body after another.
        let fresh: Vec<(String, String)> = bodies
            .iter()
            .map(|body| {
                forget_kept_meshes();
                let mesh = bits(&tessellate_body(body, &params).unwrap());
                forget_kept_meshes();
                let check = format!("{:?}", body_self_intersections(body, &checks).unwrap());
                (mesh, check)
            })
            .collect();
        // Made with whatever the bodies before left kept (the block's and
        // the cone's faces, and the others'), twice over.
        forget_kept_meshes();
        for _ in 0..2 {
            for (body, (mesh, check)) in bodies.iter().zip(&fresh) {
                assert_eq!(&bits(&tessellate_body(body, &params).unwrap()), mesh);
                assert_eq!(
                    &format!("{:?}", body_self_intersections(body, &checks).unwrap()),
                    check
                );
            }
        }
        assert!(kept_meshes().0 > 0);
    }
}
