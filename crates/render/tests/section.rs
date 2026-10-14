//! The section plane of the CPU renderer (`craft3d_render::render`,
//! *Section*), against closed forms: the cap of a cut-open solid has the
//! area of its cross-section, a pick gives what the picture shows, and a
//! plane that is none changes nothing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_render::{
    Camera, Cut, Frame as Picture, NO_FACE, Projection, RenderParams, SECTION_FACE, Section,
    cut_faces, pick_cut, render, section_bounds,
};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use std::f64::consts::PI;
use std::path::PathBuf;

const CAP: [u8; 3] = [200, 120, 60];

fn evidence(name: &str, frame: &Picture) {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/render/section");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), frame.to_png().unwrap()).unwrap();
}

fn at(x: f64, y: f64, z: f64) -> Frame {
    Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
}

/// The mesh of a body at a chordal tolerance.
macro_rules! mesh_of {
    ($body:expr, $tolerance:expr $(,)?) => {
        tessellate_body(
            $body,
            &TessParams {
                chordal_tolerance: $tolerance,
                ..TessParams::default()
            },
        )
        .unwrap()
    };
}

fn section(origin: Vec3, normal: Vec3) -> Section {
    Section {
        origin,
        normal,
        cap: true,
        color: CAP,
    }
}

/// An orthographic camera that looks at the plane of `section` head-on
/// from the half that is cut away, with `side` model units from the top of
/// the picture to its bottom.
fn head_on(mesh: &Mesh, section: &Section, side: f64) -> Camera {
    let bbox = mesh.bbox();
    let normal = section.normal.normalized().unwrap();
    let center = bbox.center().unwrap();
    let up = if normal.cross(Vec3::Z).length() > 1e-6 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    Camera {
        eye: center + normal * (bbox.diagonal() * 3.0),
        target: center,
        up,
        projection: Projection::Orthographic {
            half_height: side * 0.5,
        },
    }
}

fn params(side: u32, section: Section) -> RenderParams {
    RenderParams {
        width: side,
        height: side,
        ssaa: 1,
        section: Some(section),
        ..RenderParams::default()
    }
}

/// The area of the cap in a square picture `side` model units wide.
fn cap_area(frame: &Picture, side: f64) -> f64 {
    let pixel = side / f64::from(frame.width);
    frame.ids.iter().filter(|&&id| id == SECTION_FACE).count() as f64 * pixel * pixel
}

/// Checks the cap of `mesh` cut by `section` against the area and the
/// outline's length of its cross-section. A pixel on the outline is in or
/// out by where its centre lies: the count is off by at most the pixels
/// the outline passes through, which is its length in pixels times √2,
/// and by the chord error of a tessellated curve (`chord` per unit of
/// outline).
fn check_cap(name: &str, mesh: &Mesh, section: Section, area: f64, outline: f64, chord: f64) {
    let side = mesh.bbox().diagonal() * 1.1;
    let pixels = 400;
    let camera = head_on(mesh, &section, side);
    let frame = render(mesh, &camera, &params(pixels, section)).unwrap();
    evidence(&format!("{name}.png"), &frame);
    let measured = cap_area(&frame, side);
    let pixel = side / f64::from(pixels);
    let allowed = outline * (pixel * 2.0_f64.sqrt() + chord);
    println!(
        "{name}: cap {measured:.6}, closed form {area:.6}, off by {:.3} % (allowed {:.3} %)",
        (measured - area) / area * 100.0,
        allowed / area * 100.0
    );
    assert!(
        (measured - area).abs() <= allowed,
        "{name}: the cap is {measured}, the cross-section {area}, allowed {allowed}"
    );
    // The outline the renderer's own cut gives has the cross-section's
    // length.
    let cut = Cut::new(&section, &section_bounds(mesh)).unwrap();
    let length: f64 = cut.outline(mesh).iter().map(|[a, b]| a.distance(*b)).sum();
    assert!(
        (length - outline).abs() <= outline * 1e-3 + 1e-9,
        "{name}: the outline is {length} long, the cross-section's {outline}"
    );
    // Seen from the half that is kept, the cap is behind the body: the
    // picture is the whole body's, with no cap in it.
    let behind = Camera {
        eye: camera.target * 2.0 - camera.eye,
        ..camera
    };
    let frame = render(mesh, &behind, &params(pixels, section)).unwrap();
    assert_eq!(
        cap_area(&frame, side),
        0.0,
        "{name}: a cap seen from behind"
    );
    assert!(frame.coverage() > 0.01, "{name}: nothing left");
}

#[test]
fn the_cap_of_a_block_is_a_rectangle() {
    let mesh = mesh_of!(
        &primitives::block(&at(-2.0, -1.5, -1.0), 4.0, 3.0, 2.0).unwrap(),
        0.01,
    );
    // Across each axis, at three places, and both ways.
    for (axis, name, area, outline) in [
        (Vec3::X, "x", 3.0 * 2.0, 10.0),
        (Vec3::Y, "y", 4.0 * 2.0, 12.0),
        (Vec3::Z, "z", 4.0 * 3.0, 14.0),
    ] {
        for offset in [-0.7, 0.013, 0.61] {
            for way in [1.0, -1.0] {
                check_cap(
                    &format!("block-{name}-{offset}-{way}"),
                    &mesh,
                    section(axis * offset, axis * way),
                    area,
                    outline,
                    0.0,
                );
            }
        }
    }
    // An oblique plane through the centre, parallel to y: a rectangle 3
    // deep and as long as the plane's line across the 4 × 2 side. With the
    // normal (1, 0, 1)/√2 that line runs from (-1, -1) to (1, 1) in x, z.
    check_cap(
        "block-oblique",
        &mesh,
        section(Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0)),
        3.0 * 8.0_f64.sqrt(),
        2.0 * (3.0 + 8.0_f64.sqrt()),
        0.0,
    );
}

#[test]
fn the_cap_of_a_cylinder_cut_across_is_a_disc_and_of_a_ball_a_smaller_disc() {
    let tolerance = 1e-4;
    let (r, h) = (1.5, 4.0);
    let cylinder = mesh_of!(
        &primitives::cylinder(&at(0.0, 0.0, -2.0), r, h).unwrap(),
        tolerance
    );
    for offset in [-1.3, 0.2, 1.7] {
        check_cap(
            &format!("cylinder-across-{offset}"),
            &cylinder,
            section(Vec3::Z * offset, Vec3::Z),
            PI * r * r,
            2.0 * PI * r,
            tolerance,
        );
    }
    // Along its axis, off centre: a rectangle as high as the cylinder and
    // as wide as the chord.
    let d: f64 = 0.6;
    let chord = 2.0 * (r * r - d * d).sqrt();
    check_cap(
        "cylinder-along",
        &cylinder,
        section(Vec3::X * d, Vec3::X),
        chord * h,
        2.0 * (chord + h),
        // The chord's ends move with the tessellation of the circle: by
        // the chord error over the sine of the angle the plane meets it.
        tolerance * 4.0,
    );
    let ball_radius = 2.0;
    let ball = mesh_of!(
        &primitives::sphere(&at(0.3, -0.2, 0.1), ball_radius).unwrap(),
        tolerance
    );
    for d in [-1.6_f64, 0.0, 0.9, 1.9] {
        let small = (ball_radius * ball_radius - d * d).sqrt();
        for normal in [Vec3::Z, Vec3::new(1.0, 2.0, -0.5)] {
            let unit = normal.normalized().unwrap();
            check_cap(
                &format!(
                    "ball-{d}-{}",
                    if normal == Vec3::Z { "z" } else { "oblique" }
                ),
                &ball,
                section(Vec3::new(0.3, -0.2, 0.1) + unit * d, normal),
                PI * small * small,
                2.0 * PI * small,
                // A chord error of the sphere moves the circle by it over
                // the sine of the latitude.
                tolerance * ball_radius / small,
            );
        }
    }
}

/// A block with a closed cavity in it, made by a boolean.
fn hollow_block() -> Mesh {
    let outer = primitives::block(&at(-3.0, -2.0, -1.5), 6.0, 4.0, 3.0).unwrap();
    let inner = primitives::block(&at(-2.0, -1.0, -1.0), 4.0, 2.0, 2.0).unwrap();
    let hollow = boolean(&outer, &inner, BooleanOp::Subtract, &Tolerance::default()).unwrap();
    mesh_of!(&hollow.body, 0.01)
}

#[test]
fn the_cap_of_a_hollow_block_is_a_frame_and_the_cavity_shows() {
    let mesh = hollow_block();
    for (axis, name, area, outline) in [
        (Vec3::Z, "z", 6.0 * 4.0 - 4.0 * 2.0, 20.0 + 12.0),
        (Vec3::X, "x", 4.0 * 3.0 - 2.0 * 2.0, 14.0 + 8.0),
        (Vec3::Y, "y", 6.0 * 3.0 - 4.0 * 2.0, 18.0 + 12.0),
    ] {
        check_cap(
            &format!("hollow-{name}"),
            &mesh,
            section(axis * 0.25, axis),
            area,
            outline,
            0.0,
        );
    }
    // Through the wall only, above the cavity: the whole rectangle.
    check_cap(
        "hollow-wall",
        &mesh,
        section(Vec3::Z * 1.25, Vec3::Z),
        24.0,
        20.0,
        0.0,
    );
    // Looking into the cavity: at the centre of the picture its floor,
    // which is a face of the body, at the depth of the floor.
    let cut = section(Vec3::Z * 0.25, Vec3::Z);
    let side = mesh.bbox().diagonal() * 1.1;
    let camera = head_on(&mesh, &cut, side);
    let p = params(400, cut);
    let frame = render(&mesh, &camera, &p).unwrap();
    let centre = 200 * 400 + 200;
    assert!(frame.ids[centre] != SECTION_FACE && frame.ids[centre] != NO_FACE);
    let hit = pick_cut(&mesh, &camera, 400, 400, 200.5, 200.5, Some(&cut))
        .unwrap()
        .unwrap();
    assert_eq!(hit.face, frame.ids[centre]);
    assert!((hit.point.z + 1.0).abs() < 1e-9, "the floor of the cavity");
    // Without the section the same pixel is the top of the block.
    let whole = render(&mesh, &camera, &RenderParams { section: None, ..p }).unwrap();
    assert_ne!(whole.ids[centre], frame.ids[centre]);
    let top = craft3d_render::pick(&mesh, &camera, 400, 400, 200.5, 200.5)
        .unwrap()
        .unwrap();
    assert!((top.point.z - 1.5).abs() < 1e-9);
}

#[test]
fn a_pick_on_a_cut_open_model_is_what_the_picture_shows() {
    let mesh = hollow_block();
    let cut = section(Vec3::new(0.3, 0.0, 0.2), Vec3::new(0.4, -0.3, 1.0));
    let (width, height) = (160, 120);
    for perspective in [true, false] {
        let toward = Vec3::new(1.0, -1.3, 1.4);
        let camera = if perspective {
            Camera::framing(&mesh.bbox(), toward, Vec3::Z, 30.0, 4.0 / 3.0).unwrap()
        } else {
            Camera::framing_orthographic(&mesh.bbox(), toward, Vec3::Z, 4.0 / 3.0).unwrap()
        };
        let p = RenderParams {
            width,
            height,
            ssaa: 1,
            section: Some(cut),
            ..RenderParams::default()
        };
        let frame = render(&mesh, &camera, &p).unwrap();
        evidence(&format!("pick-{perspective}.png"), &frame);
        let ready = Cut::new(&cut, &section_bounds(&mesh)).unwrap();
        let (mut caps, mut faces) = (0, 0);
        for j in 0..height {
            for i in 0..width {
                let (x, y) = (f64::from(i) + 0.5, f64::from(j) + 0.5);
                let hit = pick_cut(&mesh, &camera, width, height, x, y, Some(&cut)).unwrap();
                let k = (j * width + i) as usize;
                assert_eq!(
                    hit.map_or(NO_FACE, |hit| hit.face),
                    frame.ids[k],
                    "{i}, {j}"
                );
                let Some(hit) = hit else {
                    continue;
                };
                // The point is of the plane, or of a face in the half that
                // is kept, and its distance is the frame's.
                let side = ready.side(hit.point);
                let left_out = cut_faces(&mesh, &camera, width, height, x, y, Some(&cut)).unwrap();
                if hit.face == SECTION_FACE {
                    caps += 1;
                    assert!(side.abs() < 1e-9, "the cap is in the plane: {side}");
                    assert_eq!(hit.triangle, usize::MAX);
                    assert_eq!(left_out.len() % 2, 1, "inside the body: {left_out:?}");
                    assert!((hit.depth - frame.depth[k]).abs() < 1e-9);
                } else {
                    faces += 1;
                    assert!(side <= 1e-9, "a face that is cut away was picked: {side}");
                }
            }
        }
        assert!(caps > 300 && faces > 300, "{caps} cap, {faces} faces");
    }
}

#[test]
fn a_face_in_the_plane_goes_with_its_body() {
    let mesh = mesh_of!(
        &primitives::block(&at(-2.0, -1.5, -1.0), 4.0, 3.0, 2.0).unwrap(),
        0.01,
    );
    let camera =
        Camera::framing(&mesh.bbox(), Vec3::new(1.0, -1.3, 0.9), Vec3::Z, 30.0, 1.0).unwrap();
    let whole = RenderParams {
        width: 200,
        height: 200,
        ssaa: 2,
        ..RenderParams::default()
    };
    let plain = render(&mesh, &camera, &whole).unwrap();
    // The plane through the top face, what is above it cut away: the
    // whole block, bit for bit, and no cap.
    let top = section(Vec3::Z, Vec3::Z);
    let kept = render(
        &mesh,
        &camera,
        &RenderParams {
            section: Some(top),
            ..whole
        },
    )
    .unwrap();
    assert_eq!(kept, plain);
    // The same plane, what is below it cut away: nothing at all.
    let below = section(Vec3::Z, -Vec3::Z);
    let gone = render(
        &mesh,
        &camera,
        &RenderParams {
            section: Some(below),
            ..whole
        },
    )
    .unwrap();
    assert_eq!(gone.coverage(), 0.0);
    assert!(gone.rgb.chunks(3).all(|px| px == whole.background));
    // A plane beyond the model: everything, or nothing.
    for (origin, normal, everything) in [
        (Vec3::Z * 50.0, Vec3::Z, true),
        (Vec3::Z * 50.0, -Vec3::Z, false),
        (Vec3::X * -1e9, Vec3::X, false),
        (Vec3::X * -1e9, -Vec3::X, true),
    ] {
        let cut = section(origin, normal);
        let frame = render(
            &mesh,
            &camera,
            &RenderParams {
                section: Some(cut),
                ..whole
            },
        )
        .unwrap();
        if everything {
            assert_eq!(frame, plain);
        } else {
            assert_eq!(frame.coverage(), 0.0);
        }
    }
}

#[test]
fn a_plane_that_is_none_and_no_plane_are_the_whole_picture() {
    let mesh = hollow_block();
    let camera =
        Camera::framing(&mesh.bbox(), Vec3::new(1.0, -1.3, 0.9), Vec3::Z, 30.0, 1.5).unwrap();
    for opacity in [1.0, 0.4] {
        let whole = RenderParams {
            width: 150,
            height: 100,
            ssaa: 2,
            opacity,
            ..RenderParams::default()
        };
        let plain = render(&mesh, &camera, &whole).unwrap();
        for (origin, normal) in [
            (Vec3::ZERO, Vec3::ZERO),
            (Vec3::ZERO, Vec3::new(f64::NAN, 0.0, 0.0)),
            (Vec3::new(f64::INFINITY, 0.0, 0.0), Vec3::Z),
            (Vec3::new(0.0, f64::NAN, 0.0), Vec3::Z),
            (Vec3::ZERO, Vec3::new(f64::INFINITY, 1.0, 0.0)),
        ] {
            let p = RenderParams {
                section: Some(section(origin, normal)),
                ..whole
            };
            assert_eq!(render(&mesh, &camera, &p).unwrap(), plain);
            let hit = pick_cut(&mesh, &camera, 150, 100, 75.5, 50.5, p.section.as_ref()).unwrap();
            let plain_hit = craft3d_render::pick(&mesh, &camera, 150, 100, 75.5, 50.5).unwrap();
            assert_eq!(hit, plain_hit);
        }
    }
    // An empty mesh with a section is the background.
    let p = params(32, section(Vec3::ZERO, Vec3::Z));
    let empty = render(&Mesh::default(), &camera, &p).unwrap();
    assert_eq!(empty.coverage(), 0.0);
    // Huge and tiny numbers that are still numbers: a picture, no panic.
    for (origin, normal) in [
        (Vec3::new(1e300, 0.0, 0.0), Vec3::X),
        (Vec3::ZERO, Vec3::new(1e-300, 0.0, 0.0)),
        (Vec3::ZERO, Vec3::new(1e300, 1e300, 0.0)),
        (Vec3::new(-1e300, 1e300, 0.0), Vec3::new(1.0, 1.0, 1e-200)),
    ] {
        let p = RenderParams {
            width: 60,
            height: 40,
            section: Some(section(origin, normal)),
            ..RenderParams::default()
        };
        render(&mesh, &camera, &p).unwrap();
    }
}

#[test]
fn without_a_cap_a_cut_open_body_shows_its_inside() {
    let mesh = mesh_of!(&primitives::sphere(&at(0.0, 0.0, 0.0), 2.0).unwrap(), 1e-3);
    let cut = Section {
        cap: false,
        ..section(Vec3::Z * 0.5, Vec3::Z)
    };
    let side = 5.0;
    let camera = head_on(&mesh, &cut, side);
    let frame = render(&mesh, &camera, &params(200, cut)).unwrap();
    evidence("ball-no-cap.png", &frame);
    assert!(!frame.ids.contains(&SECTION_FACE));
    // The inside of the ball, below the plane, at the centre.
    let centre = 100 * 200 + 100;
    assert_ne!(frame.ids[centre], NO_FACE);
    let eye_height = camera.eye.z;
    assert!((eye_height - frame.depth[centre] + 2.0).abs() < 1e-2);
}

#[test]
fn a_see_through_cap_is_blended_over_what_is_behind_it() {
    let mesh = hollow_block();
    let cut = section(Vec3::Z * 0.25, Vec3::Z);
    let side = mesh.bbox().diagonal() * 1.1;
    let camera = head_on(&mesh, &cut, side);
    let opaque = params(200, cut);
    let front = render(&mesh, &camera, &opaque).unwrap();
    let clear = render(
        &mesh,
        &camera,
        &RenderParams {
            opacity: 0.0,
            ..opaque
        },
    )
    .unwrap();
    let half = render(
        &mesh,
        &camera,
        &RenderParams {
            opacity: 0.5,
            ..opaque
        },
    )
    .unwrap();
    evidence("hollow-see-through.png", &half);
    assert_eq!(half.ids, front.ids, "the ids are the front's");
    let mut caps = 0;
    for (k, &id) in front.ids.iter().enumerate() {
        let px = |frame: &Picture| [frame.rgb[k * 3], frame.rgb[k * 3 + 1], frame.rgb[k * 3 + 2]];
        if id == SECTION_FACE {
            caps += 1;
            assert_eq!(px(&front), CAP);
            // Behind the cap: the bottom of the block, seen from inside.
            assert_ne!(px(&clear), CAP);
        }
        for c in 0..3 {
            let mixed = (f64::from(px(&front)[c]) * 0.5 + f64::from(px(&clear)[c]) * 0.5).round();
            assert_eq!(f64::from(px(&half)[c]), mixed);
        }
    }
    assert!(caps > 1000);
    // With the other half kept, the top of the block is in front and the
    // cap is the surface behind it wherever the plane is in the wall: there
    // the clear picture shows the cap's colour, and no id is the cap's.
    let p = params(200, section(Vec3::Z * 0.25, -Vec3::Z));
    let front = render(&mesh, &camera, &p).unwrap();
    assert!(!front.ids.contains(&SECTION_FACE));
    let clear = render(&mesh, &camera, &RenderParams { opacity: 0.0, ..p }).unwrap();
    let seen = clear.rgb.chunks(3).filter(|px| *px == CAP).count();
    // The frame of the wall: 24 − 8 model units of a picture of 200 × 200.
    let expected = 16.0 / (side * side) * 40_000.0;
    assert!(
        (seen as f64 - expected).abs() < expected * 0.05,
        "{seen} pixels show the cap behind the front, {expected} expected"
    );
}
