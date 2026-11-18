//! The review of the hatch's direction per body (`check/hatch-body`), by
//! `review-ux-render`: the GPU against the CPU renderer on parts that truly
//! touch (no gap), opaque and see-through, with the classes and without;
//! the count per channel on nested bodies, a body in another's cavity, a
//! ray through hundreds of cut-away surfaces and an open sheet; the plane
//! the builder's test leaves out; the fade at each class's angle; and what
//! the classes cost a frame. Findings are printed as `REVIEW` lines.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{Parity, compare_with, offscreens, orthographic, params, perspective};
use craft3d_gpu::{
    Camera, FaceColors, Frame, Mesh, Offscreen, Projection, RenderParams, SECTION_FACE, Section,
};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame as Placement, Portable, Tolerance, Vec3};
use craft3d_render::{Hatch, Hatching};
use craft3d_tess::{TessParams, tessellate_body};

const PAPER: [u8; 3] = [232, 228, 220];
const HATCH: Hatch = Hatch {
    ink: [88, 84, 78],
    spacing_points: 9.0,
    width_points: 1.0,
    height_points: None,
};

fn at(x: f64, y: f64, z: f64) -> Placement {
    Placement::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
}

fn tess() -> TessParams {
    TessParams {
        chordal_tolerance: 0.01,
        ..TessParams::default()
    }
}

/// A block from (x, y, z), `dx` × `dy` × `dz`, meshed.
fn block(x: f64, y: f64, z: f64, dx: f64, dy: f64, dz: f64) -> Mesh {
    let body = primitives::block(&at(x, y, z), dx, dy, dz).unwrap();
    tessellate_body(&body, &tess()).unwrap()
}

/// Parts as one mesh, a run of face ids each, and the first face of each.
fn combine(parts: &[Mesh]) -> (Mesh, Vec<u32>) {
    let mut mesh = Mesh::default();
    let mut firsts = Vec::new();
    let mut faces = 0_u32;
    for part in parts {
        let base = mesh.positions.len() as u32;
        firsts.push(faces);
        mesh.positions.extend(&part.positions);
        mesh.normals.extend(&part.normals);
        mesh.triangles
            .extend(part.triangles.iter().map(|t| t.map(|i| i + base)));
        mesh.face_ids
            .extend(part.face_ids.iter().map(|f| f + faces));
        faces += part.face_ids.iter().copied().max().map_or(0, |m| m + 1);
    }
    (mesh, firsts)
}

/// Each part of the class of its place.
fn classed(firsts: &[u32]) -> FaceColors {
    let mut colors = FaceColors::new();
    for (k, &first) in firsts.iter().enumerate() {
        assert!(colors.set_hatch_class(first, k));
    }
    colors
}

fn section(origin: Vec3, normal: Vec3) -> Section {
    Section {
        origin,
        normal,
        cap: true,
        color: PAPER,
        hatch: Some(HATCH),
    }
}

/// Cap pixels outside the band (both renderers' ids the cap's) by their
/// largest channel difference: 0, 1, 2, 3 or more; and the pixels of the
/// whole picture that differ by more than 8 and by more than 32 levels.
fn differences(cpu: &Frame, gpu: &Frame, band: &[bool]) -> ([usize; 4], usize, usize) {
    let mut caps = [0; 4];
    let (mut over8, mut over32) = (0, 0);
    for (i, &in_band) in band.iter().enumerate().take(cpu.ids.len()) {
        let off = (0..3)
            .map(|c| cpu.rgb[i * 3 + c].abs_diff(gpu.rgb[i * 3 + c]))
            .max()
            .unwrap();
        over8 += usize::from(off > 8);
        over32 += usize::from(off > 32);
        if in_band || cpu.ids[i] != SECTION_FACE || gpu.ids[i] != SECTION_FACE {
            continue;
        }
        caps[usize::from(off).min(3)] += 1;
    }
    (caps, over8, over32)
}

fn summary(parity: &Parity) -> String {
    format!(
        "outside the band {:?} by level, {} over the tolerance, {:.0} allowed a level, \
         {} ids; band {} of {}: {} ids, {} colours",
        parity.color_histogram,
        parity.color_outside,
        parity.color_differ_allowed,
        parity.id_outside,
        parity.band,
        parity.pixels,
        parity.id_band,
        parity.color_band
    )
}

/// The scene with the classes and without, on `offscreen`: the parity of
/// each, printed; the hatch's own failure (with classes, where without
/// passes) returned.
fn compare_both(
    offscreen: &mut Offscreen,
    label: &str,
    mesh: &Mesh,
    colors: &FaceColors,
    camera: &Camera,
    params: &RenderParams,
) -> Option<String> {
    let adapter = offscreen.adapter().clone();
    let mut failures = Vec::new();
    for (which, colors) in [("no classes", &FaceColors::new()), ("classes", colors)] {
        let cpu = craft3d_render::render_colored(mesh, colors, camera, params).unwrap();
        let gpu = offscreen
            .render_colored(mesh, colors, camera, params)
            .unwrap();
        let (parity, band) = compare_with(mesh, camera, params, &cpu, &gpu);
        let (caps, over8, over32) = differences(&cpu, &gpu, &band);
        let cap_pixels = cpu.ids.iter().filter(|&&id| id == SECTION_FACE).count();
        let failure = parity.failure(&format!("{label}, {which} on {adapter}"));
        eprintln!(
            "REVIEW {label}, {which}, on {}: {cap_pixels} cap pixels, by level {caps:?}; {over8} \
             pixels over 8 levels, {over32} over 32; {}; {}",
            adapter.name,
            summary(&parity),
            failure.as_deref().map_or("passes", |_| "FAILS")
        );
        failures.push(failure);
    }
    match (failures.first(), failures.get(1)) {
        (Some(None), Some(Some(failure))) => Some(failure.clone()),
        _ => None,
    }
}

/// Four blocks 2 × 3 × 2 in a row that truly touch (no gap), each of the
/// class of its place, cut across y, across z and obliquely, opaque and
/// see-through, in perspective and orthographic, one and two samples a
/// side. Opaque: asserted (where the picture without classes passes);
/// see-through: printed (two faces that touch in one plane are a tie
/// behind the front).
#[test]
fn parts_that_truly_touch_match_the_cpu_renderer() {
    let parts: Vec<Mesh> = (0..4)
        .map(|k| block(f64::from(k) * 2.0, 0.0, 0.0, 2.0, 3.0, 2.0))
        .collect();
    let (mesh, firsts) = combine(&parts);
    let colors = classed(&firsts);
    let mut failures = Vec::new();
    for mut offscreen in offscreens() {
        for (k, (plane, origin, normal, toward)) in [
            (
                "y",
                Vec3::new(4.0, 1.5, 1.0),
                -Vec3::Y,
                Vec3::new(0.3, -1.0, 0.45),
            ),
            (
                "z",
                Vec3::new(4.0, 1.5, 1.2),
                Vec3::Z,
                Vec3::new(0.35, -0.5, 1.0),
            ),
            (
                "oblique",
                Vec3::new(4.0, 1.5, 1.0),
                Vec3::new(0.2, -0.7, 0.6),
                Vec3::new(0.4, -0.8, 0.9),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            for (opacity, persp) in [(1.0, false), (1.0, true), (0.5, false), (0.5, true)] {
                let p = RenderParams {
                    opacity,
                    section: Some(section(origin, normal)),
                    ..params(256, 192, 1 + (k as u32 + u32::from(persp)) % 2)
                };
                let camera = if persp {
                    perspective(&mesh, toward, 35.0, &p)
                } else {
                    orthographic(&mesh, toward, &p)
                };
                let label = format!(
                    "touching, plane {plane}, {}, opacity {opacity}, ssaa {}",
                    if persp { "persp" } else { "ortho" },
                    p.ssaa
                );
                if let Some(failure) =
                    compare_both(&mut offscreen, &label, &mesh, &colors, &camera, &p)
                    && opacity >= 1.0
                {
                    failures.push(failure);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The count kept per channel: a block inside another (an overlap of two
/// classes: cross-hatched), a block in another's closed cavity (two caps,
/// each its own class), 300 thin slabs along the view (a ray through 600
/// cut-away surfaces of one class), an open sheet of another class across
/// a block, and a body cut where its cut-away part is crossed twice
/// (an even and an odd count). Opaque; asserted where the picture without
/// classes passes.
#[test]
fn the_count_per_class_matches_the_cpu_renderer() {
    let tolerance = Tolerance::default();
    let mut scenes: Vec<(&str, Mesh, FaceColors, Section, Vec3)> = Vec::new();
    // Nested: a block in a block.
    let (mesh, firsts) = combine(&[
        block(0.0, 0.0, 0.0, 6.0, 4.0, 2.0),
        block(2.0, 1.0, 0.5, 2.0, 2.0, 1.0),
    ]);
    scenes.push((
        "nested",
        mesh,
        classed(&firsts),
        section(Vec3::new(3.0, 2.0, 1.0), Vec3::Z),
        Vec3::new(0.3, -0.6, 1.0),
    ));
    // A block in a closed cavity, apart from its walls.
    let outer = primitives::block(&at(0.0, 0.0, 0.0), 6.0, 4.0, 3.0).unwrap();
    let inner = primitives::block(&at(1.0, 1.0, 0.5), 4.0, 2.0, 2.0).unwrap();
    let hollow = boolean(&outer, &inner, BooleanOp::Subtract, &tolerance).unwrap();
    let (mesh, firsts) = combine(&[
        tessellate_body(&hollow.body, &tess()).unwrap(),
        block(2.0, 1.5, 1.0, 2.0, 1.0, 1.0),
    ]);
    scenes.push((
        "cavity",
        mesh,
        classed(&firsts),
        section(Vec3::new(3.0, 2.0, 1.5), Vec3::Z),
        Vec3::new(0.3, -0.6, 1.0),
    ));
    // 300 slabs along y, each 0.02 thick with 0.02 between and 20 tall,
    // of one class (the third part), beside a block of class 0 and one of
    // class 1: cut at z = 1, a ray from the front and above crosses up to
    // 600 of their cut-away faces before it meets the plane.
    let mut parts = vec![
        block(-3.0, 0.0, 0.0, 2.0, 12.0, 20.0),
        block(-6.0, 0.0, 0.0, 2.0, 12.0, 20.0),
    ];
    let mut slabs = Mesh::default();
    for k in 0..300 {
        let (one, _) = combine(&[
            slabs.clone(),
            block(0.0, f64::from(k) * 0.04, 0.0, 4.0, 0.02, 20.0),
        ]);
        slabs = one;
    }
    // One part: every slab's faces of the third part's class.
    let mut flat = slabs.clone();
    flat.face_ids.iter_mut().for_each(|f| *f = 0);
    parts.push(flat);
    let (mesh, firsts) = combine(&parts);
    scenes.push((
        "slabs",
        mesh,
        classed(&firsts),
        section(Vec3::new(0.0, 6.0, 1.0), Vec3::Z),
        Vec3::new(0.15, -1.0, 0.35),
    ));
    // An open sheet (two triangles) of class 1 standing across a block of
    // class 0, through the plane.
    let mut sheet = Mesh::default();
    sheet.positions = vec![
        Vec3::new(1.0, 2.0, -0.5),
        Vec3::new(5.0, 2.0, -0.5),
        Vec3::new(5.0, 2.0, 2.5),
        Vec3::new(1.0, 2.0, 2.5),
    ];
    sheet.normals = vec![-Vec3::Y; 4];
    sheet.triangles = vec![[0, 1, 2], [0, 2, 3]];
    sheet.face_ids = vec![0, 0];
    let (mesh, firsts) = combine(&[block(0.0, 0.0, 0.0, 6.0, 4.0, 2.0), sheet]);
    scenes.push((
        "open sheet",
        mesh,
        classed(&firsts),
        section(Vec3::new(3.0, 2.0, 1.0), Vec3::Z),
        Vec3::new(0.3, -0.6, 1.0),
    ));
    // A U whose cut-away arms are crossed twice by the rays: a channel
    // flipped an even number of times in front of an odd one.
    let u = {
        let base = primitives::block(&at(0.0, 0.0, 0.0), 6.0, 4.0, 3.0).unwrap();
        let slot = primitives::block(&at(2.0, -1.0, 1.0), 2.0, 6.0, 3.0).unwrap();
        boolean(&base, &slot, BooleanOp::Subtract, &tolerance)
            .unwrap()
            .body
    };
    let (mesh, firsts) = combine(&[
        block(-3.0, 0.0, 0.0, 2.0, 4.0, 3.0),
        tessellate_body(&u, &tess()).unwrap(),
    ]);
    scenes.push((
        "u even and odd",
        mesh,
        classed(&firsts),
        section(Vec3::new(3.0, 2.0, 0.5), Vec3::Z),
        Vec3::new(0.1, -0.9, 0.5),
    ));
    let mut failures = Vec::new();
    for mut offscreen in offscreens() {
        for (k, (name, mesh, colors, cut, toward)) in scenes.iter().enumerate() {
            let p = RenderParams {
                section: Some(*cut),
                ..params(256, 192, 1 + (k as u32) % 2)
            };
            let camera = orthographic(mesh, *toward, &p);
            if let Some(failure) = compare_both(&mut offscreen, name, mesh, colors, &camera, &p) {
                failures.push(failure);
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The plane the builder's `hatch_classes_of_parts_match_the_cpu_renderer`
/// leaves out: across z at two thirds of the row's height, the part below
/// cut away, which keeps only the top of the block that sinks into the
/// last one; framed as that test frames it, and framed on what is kept.
#[test]
fn the_plane_left_out_of_the_builders_test() {
    let (mesh, firsts) = combine(&[
        block(0.0, 0.0, 0.0, 1.98, 3.0, 2.0),
        block(2.0, 0.0, 0.0, 1.98, 3.0, 2.0),
        block(4.0, 0.0, 0.0, 1.98, 3.0, 2.0),
        block(6.0, 0.0, 0.0, 1.98, 3.0, 2.0),
        block(6.5, 0.5, 1.5, 1.0, 2.0, 1.5),
    ]);
    let colors = classed(&firsts);
    let (lo, hi) = mesh.bbox().corners().unwrap();
    let origin = lo + (hi - lo) * 0.67;
    let cut = section(origin, -Vec3::Z);
    let toward = Vec3::new(-0.6, 0.35, -0.7);
    for mut offscreen in offscreens() {
        for (ssaa, framed_on_kept) in [(2, false), (2, true), (1, true)] {
            let p = RenderParams {
                section: Some(cut),
                ..params(233, 177, ssaa)
            };
            let camera = if framed_on_kept {
                let kept = block(6.5, 0.5, 2.0, 1.0, 2.0, 1.0);
                orthographic(&kept, toward, &p)
            } else {
                orthographic(&mesh, toward, &p)
            };
            let label = format!(
                "the left-out plane z1, {}, ssaa {ssaa}",
                if framed_on_kept {
                    "framed on what is kept"
                } else {
                    "framed on the row"
                }
            );
            let _ = compare_both(&mut offscreen, &label, &mesh, &colors, &camera, &p);
        }
    }
}

fn toward(azimuth: f64, elevation: f64) -> Vec3 {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    Vec3::new(a.pcos() * e.pcos(), a.psin() * e.pcos(), e.psin())
}

const TARGET: Vec3 = Vec3::new(3.0, 2.0, 1.0);
const HEIGHT: u32 = 192;

/// A camera on the cut z = 1 from `elevation` above it, at `azimuth`,
/// zoomed so that the span is `ratio` times a spacing of a quarter unit.
fn camera(perspective: bool, ratio: f64, elevation: f64, azimuth: f64) -> Camera {
    let span = ratio * 0.25;
    let direction = toward(azimuth, elevation);
    if perspective {
        let tan = 17.5_f64.to_radians().ptan();
        let distance = span * f64::from(HEIGHT) / (18.0 * tan);
        Camera {
            eye: TARGET + direction * distance,
            target: TARGET,
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: 35.0 },
        }
    } else {
        Camera {
            eye: TARGET + direction * 14.0,
            target: TARGET,
            up: Vec3::Z,
            projection: Projection::Orthographic {
                half_height: span * f64::from(HEIGHT) / 18.0,
            },
        }
    }
}

/// How many pixels apart class `class`'s lines stand at the target.
fn across(camera: &Camera, class: usize) -> (Hatching, f64) {
    let h = Hatching::new(Vec3::Z, PAPER, &HATCH, camera, HEIGHT).unwrap();
    let distance = (TARGET - camera.eye).dot(h.forward);
    let f = h.footprint_of(TARGET, distance, class).unwrap();
    (h, h.spacing / f)
}

/// The fade into the mean tone and a half-faded step of the spacing for
/// each class's angle: a block 6 × 4 × 2 all of class `k`, the camera
/// from the azimuth of the class's direction (45°, 135°, 75°, 165° from
/// X), face on, with the class's lines 2.5 and 1.6 pixels apart, in
/// perspective and orthographic, one to three samples a side.
#[test]
fn the_fade_of_each_class_matches_the_cpu_renderer() {
    let mesh = block(0.0, 0.0, 0.0, 6.0, 4.0, 2.0);
    let cut = section(Vec3::new(3.0, 2.0, 1.0), Vec3::Z);
    let mut scenes = Vec::new();
    let mut exercised = [[0_usize; 2]; 4];
    for class in 0..4 {
        let (c, s) = craft3d_render::hatch_direction(class);
        let azimuth = s.patan2(c).to_degrees();
        for perspective in [false, true] {
            for wanted in [None, Some(2.5), Some(1.6)] {
                let mut elevation = 70.0;
                if let Some(wanted) = wanted {
                    elevation = 85.0;
                    while elevation > 3.0
                        && across(&camera(perspective, 1.75, elevation, azimuth), class).1 > wanted
                    {
                        elevation -= 0.25;
                    }
                }
                let cam = camera(perspective, 1.75, elevation, azimuth);
                let (h, a) = across(&cam, class);
                if a > 2.0 && a < 3.0 {
                    exercised[class][0] += 1;
                }
                if a < 2.0 {
                    exercised[class][1] += 1;
                }
                scenes.push((class, perspective, elevation, a, h.odd_ink, cam));
            }
        }
    }
    let mut failures = Vec::new();
    for mut offscreen in offscreens() {
        for (k, &(class, persp, elevation, a, odd, cam)) in scenes.iter().enumerate() {
            let mut colors = FaceColors::new();
            colors.set_hatch_class(0, class);
            let p = RenderParams {
                section: Some(cut),
                ..params(256, HEIGHT, 1 + (k as u32) % 3)
            };
            let label = format!(
                "fade, class {class}, {} at {elevation}° (lines {a:.2} px apart, odd ink \
                 {odd:.2}), ssaa {}",
                if persp { "persp" } else { "ortho" },
                p.ssaa
            );
            if let Some(failure) = compare_both(&mut offscreen, &label, &mesh, &colors, &cam, &p) {
                failures.push(failure);
            }
        }
    }
    eprintln!("REVIEW fade: per class, scenes 2 to 3 px apart and under 2: {exercised:?}");
    assert!(
        exercised.iter().all(|e| e[0] > 0 && e[1] > 0),
        "{exercised:?}"
    );
    assert!(failures.is_empty(), "{failures:#?}");
}

/// What the classes cost the GPU: 400 blocks in a 20 × 20 grid, each
/// touching its neighbours, cut through the middle at 1280 × 800, the cut
/// without classes and with 400 parts in four, ten frames each in turn,
/// the medians, per adapter.
#[test]
fn what_the_classes_cost_the_gpu() {
    let parts: Vec<Mesh> = (0..400)
        .map(|k| {
            let (i, j) = (f64::from(k % 20), f64::from(k / 20));
            block(i, 0.0, j, 1.0, 1.0, 1.0)
        })
        .collect();
    let (mesh, firsts) = combine(&parts);
    let colors = classed(&firsts);
    let cut = section(Vec3::new(10.0, 0.5, 10.0), -Vec3::Y);
    let p = RenderParams {
        section: Some(cut),
        ..params(1280, 800, 1)
    };
    let camera = perspective(&mesh, Vec3::new(0.3, -1.0, 0.4), 40.0, &p);
    for mut offscreen in offscreens() {
        let adapter = offscreen.adapter().name.clone();
        let none = FaceColors::new();
        let up = [
            offscreen.renderer().upload_colored(&mesh, &none).unwrap(),
            offscreen.renderer().upload_colored(&mesh, &colors).unwrap(),
        ];
        let mut times = [Vec::new(), Vec::new()];
        for u in &up {
            offscreen.render_uploaded(u, &camera, &p).unwrap();
        }
        for _ in 0..10 {
            for (k, u) in up.iter().enumerate() {
                let started = std::time::Instant::now();
                offscreen.render_uploaded(u, &camera, &p).unwrap();
                times[k].push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        for t in &mut times {
            t.sort_by(f64::total_cmp);
        }
        eprintln!(
            "REVIEW gpu cost: {adapter}, {} triangles, 400 parts: no classes {:.1} ms, four \
             classes {:.1} ms (medians of 10)",
            mesh.triangles.len(),
            times[0][5],
            times[1][5]
        );
    }
}
