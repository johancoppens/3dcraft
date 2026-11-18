//! The review of the hatch's direction per body (`check/hatch-body`), by
//! `review-ux-render`, where the engine's scenes and both renderers meet:
//! the published toggle clamp and hydraulic cylinder cut through, CPU
//! against GPU on every adapter, with their classes and with every part of
//! class 0; the classes of an assembly's parts when one is deleted, undone
//! and added; and the engine's pick against the CPU picture at 1000 places
//! of a row of touching and overlapping parts. Findings are printed as
//! `REVIEW` lines.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Scene, Session, Workspace};
use craft3d_gpu::Offscreen;
use craft3d_math::Vec3;
use craft3d_render::{
    Camera, FaceColors, Frame, Hatch, NO_FACE, Projection, RenderParams, SECTION_FACE, Section,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

const PAPER: [u8; 3] = [232, 228, 220];
const HATCH: Hatch = Hatch {
    ink: [88, 84, 78],
    spacing_points: 9.0,
    width_points: 1.0,
    height_points: None,
};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn published(file: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/examples/published")
        .join(file)
        .to_string_lossy()
        .into_owned()
}

fn open(file: &str) -> (Session, Arc<Scene>) {
    let mut session = Session::new(Workspace::unrestricted());
    run(
        &mut session,
        "file.open",
        json!({ "path": published(file) }),
    );
    let scene = session.scene(None).unwrap().unwrap();
    (session, scene)
}

/// The hatch classes of the scene's bodies, in order, by name.
fn classes(scene: &Scene) -> Vec<(String, usize)> {
    let classes = scene.face_colors.hatch_classes();
    scene
        .bodies
        .iter()
        .map(|b| (b.name.clone(), classes.class(b.first_face_id)))
        .collect()
}

/// The scene's colours with every body of class 0: the cut as it was
/// drawn before the classes.
fn without_classes(colors: &FaceColors) -> FaceColors {
    let mut out = colors.clone();
    let runs: Vec<(u32, usize)> = out.hatch_classes().runs().collect();
    for (first, _) in runs {
        out.set_hatch_class(first, 0);
    }
    out
}

/// Pixels whose id is another in the two frames; and of the pixels whose
/// 3 × 3 neighbourhood has one id in both frames (away from every edge,
/// silhouette and border of a cap), those whose colour differs by more
/// than 2, 8 and 32 levels; and the cap's pixels of each.
fn differ(cpu: &Frame, gpu: &Frame) -> String {
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let (mut ids, mut inner, mut over2, mut over8, mut over32) = (0, 0, 0, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if cpu.ids[i] != gpu.ids[i] {
                ids += 1;
                continue;
            }
            let uniform = (y.saturating_sub(1)..=(y + 1).min(h - 1)).all(|ny| {
                (x.saturating_sub(1)..=(x + 1).min(w - 1)).all(|nx| {
                    let n = ny * w + nx;
                    cpu.ids[n] == cpu.ids[i] && gpu.ids[n] == cpu.ids[i]
                })
            });
            if !uniform {
                continue;
            }
            inner += 1;
            let off = (0..3)
                .map(|c| cpu.rgb[i * 3 + c].abs_diff(gpu.rgb[i * 3 + c]))
                .max()
                .unwrap();
            over2 += usize::from(off > 2);
            over8 += usize::from(off > 8);
            over32 += usize::from(off > 32);
        }
    }
    let caps = |f: &Frame| f.ids.iter().filter(|&&id| id == SECTION_FACE).count();
    format!(
        "cap {} / {} pixels; ids differ at {ids}; of {inner} inner pixels {over2} over 2 \
         levels, {over8} over 8, {over32} over 32",
        caps(cpu),
        caps(gpu)
    )
}

/// The published toggle clamp (an assembly) and hydraulic cylinder (a rod
/// in its gland, a piston in its barrel: parts that truly touch), cut
/// across x, y and z through the middle, opaque and see-through, by the
/// CPU and on every GPU adapter, with the scene's classes and with every
/// part of class 0: the pixels that differ, printed.
#[test]
fn the_published_parts_cut_through_on_both_renderers() {
    let adapters = craft3d_gpu::adapters();
    assert!(!adapters.is_empty(), "no adapter");
    for file in ["toggle-clamp.3dcraftasm", "hydraulic-cylinder.3dcraft"] {
        let (_session, scene) = open(file);
        let mesh = &scene.mesh;
        let (lo, hi) = mesh.bbox().corners().unwrap();
        let center = (lo + hi) * 0.5;
        let diagonal = (hi - lo).length();
        let with = (*scene.face_colors).clone();
        let without = without_classes(&with);
        eprintln!(
            "REVIEW examples: {file}: {} bodies, classes {:?}",
            scene.bodies.len(),
            classes(&scene).iter().map(|c| c.1).collect::<Vec<_>>()
        );
        for adapter in &adapters {
            let mut offscreen = Offscreen::on_adapter(adapter).unwrap();
            let name = offscreen.adapter().name.clone();
            for (axis, normal, toward) in [
                ("x", Vec3::X, Vec3::new(1.0, -0.45, 0.4)),
                ("y", -Vec3::Y, Vec3::new(0.35, -1.0, 0.4)),
                ("z", Vec3::Z, Vec3::new(0.3, -0.4, 1.0)),
            ] {
                for opacity in [1.0, 0.5] {
                    let params = RenderParams {
                        width: 400,
                        height: 300,
                        ssaa: 2,
                        opacity,
                        section: Some(Section {
                            origin: center,
                            normal,
                            cap: true,
                            color: PAPER,
                            hatch: Some(HATCH),
                        }),
                        ..RenderParams::default()
                    };
                    let camera = Camera {
                        eye: center + toward.normalized().unwrap() * (diagonal * 2.0),
                        target: center,
                        up: Vec3::Z,
                        projection: Projection::Orthographic {
                            half_height: diagonal * 0.4,
                        },
                    };
                    for (which, colors) in [("classes", &with), ("all class 0", &without)] {
                        let cpu =
                            craft3d_render::render_colored(mesh, colors, &camera, &params).unwrap();
                        let gpu = offscreen
                            .render_colored(mesh, colors, &camera, &params)
                            .unwrap();
                        eprintln!(
                            "REVIEW examples: {file}, across {axis}, opacity {opacity}, {which}, \
                             on {name}: {}",
                            differ(&cpu, &gpu)
                        );
                    }
                }
            }
        }
    }
}

/// The toggle clamp's parts and their classes; one part deleted (the
/// parts after it change direction: by their place), undone (every class
/// as before), a block added (the next class); the same scene built twice
/// gives the same classes. Nothing panics.
#[test]
fn an_assemblys_classes_when_a_part_is_deleted_undone_and_added() {
    let (mut session, scene) = open("toggle-clamp.3dcraftasm");
    let before = classes(&scene);
    assert!(before.len() >= 3, "{before:?}");
    eprintln!("REVIEW assembly: before {before:?}");
    // An instance by its name: the first of these the assembly lets go.
    let mut deleted = None;
    for name in ["Bar", "Link", "Workpiece", "Upper nut", "Lower nut", "Base"] {
        match session.execute("instance.delete", &json!({ "name": name })) {
            Ok(_) => {
                deleted = Some(name.to_string());
                break;
            }
            Err(e) => eprintln!("REVIEW assembly: instance.delete {name}: {e}"),
        }
    }
    let after_delete = classes(&session.scene(None).unwrap().unwrap());
    eprintln!("REVIEW assembly: {deleted:?} deleted: {after_delete:?}");
    if deleted.is_some() {
        run(&mut session, "edit.undo", json!({}));
        let undone = classes(&session.scene(None).unwrap().unwrap());
        eprintln!("REVIEW assembly: undone: {undone:?}");
        assert_eq!(undone, before, "undone: every class as before");
    }
    let added = session.execute(
        "instance.add",
        &json!({"source": published("hydraulic-cylinder.3dcraft"), "name": "Added", "position": [1000, 0, 0]}),
    );
    let with_added = classes(&session.scene(None).unwrap().unwrap());
    eprintln!(
        "REVIEW assembly: a block added ({}): {with_added:?}",
        added.map_or_else(|e| format!("refused: {e}"), |_| "taken".to_string())
    );
    let (_again, rebuilt) = open("toggle-clamp.3dcraftasm");
    assert_eq!(
        classes(&rebuilt),
        before,
        "the same classes when built again"
    );
}

/// Five blocks of 10 in a row (B0 to B3 touching, B4 sunk 4 into B3), cut
/// across y at 5, seen from the front: at 1000 places of the picture (an
/// LCG's), the engine's pick says what the CPU picture shows there (a
/// face's id, the cap, nothing); on the cap, the body is named except
/// where B3 and B4 overlap (cross-hatched: no one body).
#[test]
fn the_pick_agrees_with_the_picture_at_1000_places() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    for k in 0..5 {
        let x = if k == 4 { 36.0 } else { f64::from(k) * 10.0 };
        run(
            &mut session,
            "feature.block",
            json!({"size": [10, 10, 10], "position": [x, 0, 0], "name": format!("B{k}")}),
        );
    }
    let scene = session.scene(None).unwrap().unwrap();
    eprintln!("REVIEW pick: classes {:?}", classes(&scene));
    let section = Section {
        origin: Vec3::new(0.0, 5.0, 0.0),
        normal: -Vec3::Y,
        cap: true,
        color: PAPER,
        hatch: Some(HATCH),
    };
    let (w, h) = (460_u32, 200_u32);
    for (name, camera) in [
        (
            "front",
            Camera {
                eye: Vec3::new(23.0, -60.0, 5.0),
                target: Vec3::new(23.0, 5.0, 5.0),
                up: Vec3::Z,
                projection: Projection::Orthographic { half_height: 10.0 },
            },
        ),
        (
            "oblique",
            Camera {
                eye: Vec3::new(40.0, -50.0, 30.0),
                target: Vec3::new(23.0, 5.0, 5.0),
                up: Vec3::Z,
                projection: Projection::Perspective { fov_y_deg: 40.0 },
            },
        ),
    ] {
        let params = RenderParams {
            width: w,
            height: h,
            ssaa: 1,
            section: Some(section),
            ..RenderParams::default()
        };
        let frame =
            craft3d_render::render_colored(&scene.mesh, &scene.face_colors, &camera, &params)
                .unwrap();
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let (mut caps, mut named, mut unnamed, mut faces, mut empty, mut wrong) =
            (0, 0, 0, 0, 0, Vec::new());
        for _ in 0..1000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let x = ((state >> 33) % u64::from(w)) as u32;
            let y = ((state >> 13) % u64::from(h)) as u32;
            let i = (y * w + x) as usize;
            let pointed = scene
                .point_at_cut(
                    &camera,
                    w,
                    h,
                    f64::from(x) + 0.5,
                    f64::from(y) + 0.5,
                    0.0,
                    Some(&section),
                )
                .unwrap();
            let id = frame.ids[i];
            let agrees = if id == SECTION_FACE {
                caps += 1;
                let cut = pointed.section.as_ref();
                match cut.and_then(|c| c.body.clone()) {
                    Some(_) => named += 1,
                    None => {
                        unnamed += 1;
                        // Only where B3 and B4 overlap: x from 36 to 40.
                        let p = cut.map(|c| c.point).unwrap_or(Vec3::ZERO);
                        if !(35.9..=40.1).contains(&p.x) {
                            wrong.push(format!("({x}, {y}): no body named at {p:?}"));
                        }
                    }
                }
                pointed.face.is_none() && cut.is_some()
            } else if id == NO_FACE {
                empty += 1;
                pointed.face.is_none() && pointed.section.is_none()
            } else {
                faces += 1;
                pointed.face.as_ref().map(|f| f.face_id) == Some(id)
            };
            if !agrees {
                wrong.push(format!(
                    "({x}, {y}): picture {id}, pick {:?} / {:?}",
                    pointed.face.as_ref().map(|f| f.face_id),
                    pointed.section.as_ref().map(|c| c.body.clone())
                ));
            }
        }
        eprintln!(
            "REVIEW pick: {name}: 1000 places: {caps} on the cap ({named} with a body named, \
             {unnamed} without), {faces} on a face, {empty} on nothing; {} disagree: {:?}",
            wrong.len(),
            wrong.iter().take(8).collect::<Vec<_>>()
        );
        assert!(caps > 50 && unnamed > 0, "{caps} {unnamed}");
        assert!(wrong.is_empty(), "{name}: {wrong:#?}");
    }
}
