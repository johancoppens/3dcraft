//! An opaque, whole studio picture keeps its bytes: the checksums of
//! pictures in layer 1's look and the polished one, a transmissive face
//! among them, as main drew them before see-through and cut-open studio
//! pictures were added (fef3410c, Check run 41132978097, 2026-10-30).
//! Those pictures go through the same code; this pins that they still
//! give the same bytes (docs/design/rendering.md, *See-through and
//! cut-open pictures in the studio look*).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use craft3d_math::Portable;
use craft3d_math::Vec3;
use craft3d_render::studio::layers::Optics;
use craft3d_render::studio::{HdrImage, Look, Prepared, env::texel_direction, render_studio};
use craft3d_render::{Camera, FaceColors, Finish, Projection, RenderParams};
use craft3d_tess::Mesh;

fn environment(w: usize, f: impl Fn(Vec3) -> [f64; 3]) -> HdrImage {
    let h = w / 2;
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            pixels.push(f(texel_direction(x, y, w, h)).map(|v| v as f32));
        }
    }
    HdrImage {
        width: w,
        height: h,
        pixels,
    }
}

fn sphere(center: Vec3, r: f64, rings: u32, segments: u32, face: u32) -> Mesh {
    let mut mesh = Mesh::default();
    for i in 0..=rings {
        let t = std::f64::consts::PI * f64::from(i) / f64::from(rings);
        for j in 0..=segments {
            let p = 2.0 * std::f64::consts::PI * f64::from(j) / f64::from(segments);
            let n = Vec3::new(t.psin() * p.pcos(), t.psin() * p.psin(), t.pcos());
            mesh.positions.push(center + n * r);
            mesh.normals.push(n);
        }
    }
    let row = segments + 1;
    for i in 0..rings {
        for j in 0..segments {
            let (a, b, c, d) = (
                i * row + j,
                i * row + j + 1,
                (i + 1) * row + j,
                (i + 1) * row + j + 1,
            );
            mesh.triangles.push([a, c, b]);
            mesh.triangles.push([b, c, d]);
            mesh.face_ids.push(face);
            mesh.face_ids.push(face);
        }
    }
    mesh
}

fn cuboid(lo: Vec3, hi: Vec3, first: u32) -> Mesh {
    let mut mesh = Mesh::default();
    let corner = |i: u32| {
        Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    };
    let faces: [([u32; 4], Vec3); 6] = [
        ([0, 2, 6, 4], Vec3::new(-1.0, 0.0, 0.0)),
        ([1, 5, 7, 3], Vec3::new(1.0, 0.0, 0.0)),
        ([0, 4, 5, 1], Vec3::new(0.0, -1.0, 0.0)),
        ([2, 3, 7, 6], Vec3::new(0.0, 1.0, 0.0)),
        ([0, 1, 3, 2], Vec3::new(0.0, 0.0, -1.0)),
        ([4, 6, 7, 5], Vec3::new(0.0, 0.0, 1.0)),
    ];
    for (k, (quad, n)) in faces.iter().enumerate() {
        let base = mesh.positions.len() as u32;
        for &c in quad {
            mesh.positions.push(corner(c));
            mesh.normals.push(*n);
        }
        mesh.triangles.push([base, base + 1, base + 2]);
        mesh.triangles.push([base, base + 2, base + 3]);
        mesh.face_ids.push(first + k as u32);
        mesh.face_ids.push(first + k as u32);
    }
    mesh
}

/// FNV-1a, 64 bits, over the colours and the face ids.
fn checksum(frame: &craft3d_render::Frame) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for &b in &frame.rgb {
        eat(b);
    }
    for id in &frame.ids {
        for b in id.to_le_bytes() {
            eat(b);
        }
    }
    h
}

/// The checksums main's renderer gave (fef3410c, Check run 41132978097).
const EXPECTED: [(&str, u64); 2] = [
    ("layer 1", 0x4e80_4a82_d40b_87d7),
    ("polished", 0xc86e_844c_fd78_8411),
];

#[test]
fn opaque_whole_studio_pictures_keep_their_bytes() {
    let env = Prepared::new(&environment(256, |d| {
        if d.z > 0.97 && d.x > 0.1 {
            [300.0; 3]
        } else {
            [0.4 + 0.3 * d.z, 0.5 + 0.2 * d.z, 0.6 + 0.1 * d.x]
        }
    }))
    .unwrap();
    let mut mesh = sphere(Vec3::new(0.0, 0.0, 0.6), 0.6, 24, 48, 3);
    let block = cuboid(Vec3::new(0.7, -0.4, 0.0), Vec3::new(1.5, 0.4, 0.8), 10);
    let base = mesh.positions.len() as u32;
    mesh.positions.extend(block.positions);
    mesh.normals.extend(block.normals);
    mesh.triangles
        .extend(block.triangles.iter().map(|t| t.map(|k| k + base)));
    mesh.face_ids.extend(block.face_ids);
    let mut colors = FaceColors::default();
    colors.set_look(
        3,
        Some(([200, 70, 50], Finish::from_factors(Some(0.0), Some(0.3)))),
    );
    colors.set_look(
        12,
        Some(([210, 170, 80], Finish::from_factors(Some(1.0), Some(0.2)))),
    );
    colors.set(11, Some([200, 230, 220]));
    colors.set_optics(
        11,
        Some(Optics {
            transmission: 0.9,
            ior: 1.5,
            ..Optics::default()
        }),
    );
    let camera = Camera {
        eye: Vec3::new(3.0, -4.0, 2.5),
        target: Vec3::new(0.5, 0.0, 0.4),
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 35.0 },
    };
    let params = RenderParams {
        width: 120,
        height: 90,
        ssaa: 1,
        ..RenderParams::default()
    };
    let mut failures = Vec::new();
    for ((name, look), (_, want)) in [("layer 1", Look::default()), ("polished", Look::polished())]
        .into_iter()
        .zip(EXPECTED)
    {
        let frame = render_studio(&mesh, &colors, &camera, &params, &look, &env).unwrap();
        let got = checksum(&frame);
        eprintln!("CHECKSUM {name} {got:#018x}");
        if want != 0 && got != want {
            failures.push(format!("{name}: {got:#018x}, main gave {want:#018x}"));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
