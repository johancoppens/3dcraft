//! The twisted blade of the showcase (`docs/showcase/blade.3dcraft`): a
//! loft through four leaf sections that turn by half a right angle, on a
//! round foot. Its top view shows one side of the blade nearly black,
//! which was reported as possibly a face meshed inside out.
//!
//! It is not: the mesh is closed, encloses the body's volume, every
//! triangle runs the way its vertices' normals point, and those normals
//! are the surfaces' own, outward. The side is dark because it faces away
//! from the renderer's one light, which is fixed to the camera, above and
//! to the left of the viewer: seen from above, that side leans back and
//! gets the ambient term only (`shading_of_the_top_view` prints the
//! numbers).

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Surface;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_topo::{Body, check, mass_properties};

type P = [f64; 2];

/// The upper and the lower spline of each section, and its height: the
/// numbers of `docs/showcase/blade.steps.json`.
const SECTIONS: [([P; 4], [P; 2], f64); 4] = [
    (
        [[-15.0, 0.0], [-13.5, 4.8], [-1.5, 6.0], [15.0, 0.0]],
        [[3.0, -0.6], [-12.6, -2.1]],
        0.0,
    ),
    (
        [
            [-12.613844, -3.144985],
            [-12.358855, 1.205944],
            [-2.519378, 4.731039],
            [12.613844, 3.144985],
        ],
        [[2.648568, 0.124443], [-10.155331, -4.407725]],
        25.0,
    ),
    (
        [
            [-8.660254, -5.0],
            [-9.394229, -1.728719],
            [-2.866025, 2.964102],
            [8.660254, 5.0],
        ],
        [[1.932051, 0.65359], [-6.574613, -5.412436]],
        55.0,
    ),
    (
        [
            [-4.014784, -4.458869],
            [-5.040143, -2.728251],
            [-2.185026, 1.160027],
            [4.014784, 4.458869],
        ],
        [[0.981311, 0.731182], [-2.748177, -4.30752]],
        90.0,
    ),
];

fn spline(id: u64, control: Vec<P>) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control,
        }),
    }
}

fn blade() -> Body {
    let tol = Tolerance::default();
    let sections: Vec<Section> = SECTIONS
        .iter()
        .map(|(upper, lower, z)| {
            let curves = [
                spline(1, upper.to_vec()),
                spline(2, vec![upper[3], lower[0], lower[1], upper[0]]),
            ];
            let profiles = detect_profiles(&curves, &tol).unwrap();
            Section {
                plane: Frame::WORLD.at(Vec3::new(0.0, 0.0, *z)).unwrap(),
                outline: profiles[0].outer.clone(),
                start: None,
            }
        })
        .collect();
    let lofted = loft(&sections, &Options::default(), &tol).unwrap().body;
    let foot = primitives::cylinder(
        &Frame::WORLD.at(Vec3::new(0.0, 2.0, -6.0)).unwrap(),
        20.0,
        8.0,
    )
    .unwrap();
    boolean(&foot, &lofted, BooleanOp::Union, &tol)
        .unwrap()
        .body
}

fn mesh_of(body: &Body, tolerance: f64) -> Mesh {
    let params = TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    };
    tessellate_body(body, &params).unwrap()
}

#[test]
fn every_triangle_of_the_blade_faces_outward_and_its_normals_are_the_surfaces() {
    let body = blade();
    assert_eq!(check(&body, &Tolerance::default()), vec![]);
    let exact = mass_properties(&body).unwrap();
    // The automatic tolerance of the viewer is about 0.08 here.
    for tolerance in [0.3, 0.08, 0.01] {
        let mesh = mesh_of(&body, tolerance);
        let welded = mesh.weld().unwrap();
        // Closed and every edge used once each way: one orientation
        // throughout; with a positive volume, the outward one.
        assert!(welded.is_closed(), "at {tolerance}");
        assert!(
            (welded.volume() - exact.volume).abs() <= exact.area * tolerance,
            "at {tolerance}: {} for {}",
            welded.volume(),
            exact.volume
        );
        let mut worst = 1.0_f64;
        for (t, triangle) in mesh.triangles.iter().enumerate() {
            let [a, b, c] = mesh.corners(t).unwrap();
            let facing = (b - a).cross(c - a).normalized().unwrap();
            for &v in triangle {
                let normal = mesh.normals[v as usize];
                assert!((normal.length() - 1.0).abs() < 1e-12);
                worst = worst.min(facing.dot(normal));
            }
        }
        // A facet turns from the surface by about the angle whose cosine
        // is 1 − chord / radius: on the leading edge of the blade, whose
        // radius is of the order of 1, half a radian at a chord of 0.3.
        // What matters is the sign; the size is held at the finer chords.
        assert!(
            worst > if tolerance > 0.1 { 0.0 } else { 0.9 },
            "at {tolerance}: a triangle turns {worst} from its normals"
        );
        // The normals are the surfaces' own, the way the face faces.
        let mut seen = vec![false; mesh.positions.len()];
        for (t, triangle) in mesh.triangles.iter().enumerate() {
            let face = &body.faces[mesh.face_ids[t] as usize];
            let outward = if face.reversed { -1.0 } else { 1.0 };
            for &v in triangle {
                if std::mem::replace(&mut seen[v as usize], true) {
                    continue;
                }
                let p = mesh.positions[v as usize];
                let (u, w) = face.surface.project(p).unwrap();
                assert!(face.surface.eval(u, w).distance(p) < 1e-6, "at {tolerance}");
                let Some(normal) = face.surface.normal(u, w) else {
                    continue;
                };
                assert!(
                    (normal * outward).distance(mesh.normals[v as usize]) < 1e-5,
                    "at {tolerance}: vertex {v} of face {}",
                    mesh.face_ids[t]
                );
            }
        }
    }
}

#[test]
#[ignore = "prints how the top view lights the blade's sides; run with --ignored --nocapture"]
fn shading_of_the_top_view() {
    let body = blade();
    let mesh = mesh_of(&body, 0.08);
    // `craft3d_render::ViewAxes::light` for the top view: forward −z, up
    // +y, right +x; the light is up × 0.6 − right × 0.45 − forward.
    let light = Vec3::new(-0.45, 0.6, 1.0).normalized().unwrap();
    for (index, face) in body.faces.iter().enumerate() {
        let (mut area, mut seen, mut unlit, mut diffuse) = (0.0, 0.0, 0.0, 0.0);
        let mut mean = Vec3::ZERO;
        for (t, _) in mesh.triangles.iter().enumerate() {
            if mesh.face_ids[t] as usize != index {
                continue;
            }
            let [a, b, c] = mesh.corners(t).unwrap();
            let facing = (b - a).cross(c - a);
            let size = facing.length() / 2.0;
            let normal = facing.normalized().unwrap();
            area += size;
            mean += normal * size;
            // Seen from above when it faces up.
            if normal.z > 0.0 {
                seen += size;
                diffuse += size * normal.dot(light).max(0.0);
                if normal.dot(light) <= 0.0 {
                    unlit += size;
                }
            }
        }
        let kind = match face.surface {
            Surface::Plane(_) => "plane",
            Surface::Cylinder(_) => "cylinder",
            Surface::Nurbs(_) => "nurbs",
            _ => "other",
        };
        let mean = mean.normalized().unwrap_or(Vec3::ZERO);
        println!(
            "face {index} ({kind}): area {area:.1}, mean normal ({:.2}, {:.2}, {:.2}), faces \
             up {:.0} %, of that without direct light {:.0} %, mean diffuse {:.2}",
            mean.x,
            mean.y,
            mean.z,
            100.0 * seen / area,
            100.0 * unlit / seen.max(1e-300),
            diffuse / seen.max(1e-300)
        );
    }
}
