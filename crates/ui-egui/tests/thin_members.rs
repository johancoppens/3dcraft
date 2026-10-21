//! The depth test of the lines over the picture, on the direct path
//! against the CPU oracle, at several zooms: edges a few millimetres
//! behind a thin plate are hidden however far out the view is zoomed, an
//! edge in the hollow where two faces meet stays in sight, the outline of
//! the plate stays in sight, and the two paths decide alike. The
//! measurement that found the cause (docs/design/rendering.md, *Edges
//! behind thin members on the direct path*) is printed with it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Vec3;
use craft3d_render::NO_FACE;
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::backend::{Backend, RendererChoice};
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::state::{CameraState, ProjectionKind};
use craft3d_ui_egui::viewport::{self, Composed, LineKind, Quality};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn blocks(list: &[([f64; 3], [f64; 3])]) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (size, position) in list {
        run(
            &mut s,
            "feature.block",
            json!({"size": size, "position": position}),
        );
    }
    s
}

/// A leg of a lattice pier, as the developer saw it: a 9 mm web (x from
/// 0 to 9, its front face x = 9 towards the camera), flanges behind it and
/// stiffeners touching its back face, whose long edges at x = 0 lie 9 mm
/// behind the front face (15 to 19 mm along the view, which meets the web
/// at a slant). z is up.
fn leg() -> Session {
    blocks(&[
        ([9.0, 400.0, 4000.0], [0.0, 0.0, 0.0]),
        ([150.0, 9.0, 4000.0], [-150.0, 0.0, 0.0]),
        ([150.0, 9.0, 4000.0], [-150.0, 391.0, 0.0]),
        ([60.0, 9.0, 4000.0], [-60.0, 100.0, 0.0]),
        ([60.0, 9.0, 4000.0], [-60.0, 195.0, 0.0]),
        ([60.0, 9.0, 4000.0], [-60.0, 300.0, 0.0]),
    ])
}

/// The stiffeners' long edges at x = 0: hidden behind the web.
fn leg_hidden_edges() -> Vec<[Vec3; 2]> {
    let mut out = Vec::new();
    for y in [100.0, 195.0, 300.0] {
        for yy in [y, y + 9.0] {
            out.push([Vec3::new(0.0, yy, 0.0), Vec3::new(0.0, yy, 4000.0)]);
        }
    }
    out
}

/// The long edges of the web's front face: its outline, in sight.
fn leg_outline_edges() -> Vec<[Vec3; 2]> {
    vec![
        [Vec3::new(9.0, 0.0, 0.0), Vec3::new(9.0, 0.0, 4000.0)],
        [Vec3::new(9.0, 400.0, 0.0), Vec3::new(9.0, 400.0, 4000.0)],
    ]
}

/// Two walls meeting in an inside corner (an L in plan): the face x = 50
/// of one meets the face y = 50 of the other in a hollow along (50, 50, z),
/// the furthest line of the picture around it.
fn corner() -> Session {
    blocks(&[
        ([400.0, 50.0, 400.0], [0.0, 0.0, 0.0]),
        ([50.0, 300.0, 400.0], [0.0, 50.0, 0.0]),
    ])
}

fn corner_hollow_edge() -> Vec<[Vec3; 2]> {
    vec![[Vec3::new(50.0, 50.0, 0.0), Vec3::new(50.0, 50.0, 400.0)]]
}

fn evidence(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// One point of an edge, as the two paths decided it.
struct Point {
    cpu_hidden: bool,
    gpu_hidden: bool,
    /// The rule before 2026-10-18 on the CPU (the furthest of the nine
    /// pixels around the point plus the slack is nearer) and on the GPU
    /// (plus the spread of the nine as well), evaluated on today's CPU
    /// frame (whose distance is the true one), for the record.
    old_cpu_hidden: bool,
    old_gpu_hidden: bool,
    /// Every pixel within two of the point shows the same face: the
    /// surface around the point is one plane, where the rule is exact.
    plain: bool,
}

/// Draws `session` with `camera` on both paths and decides every point
/// (one per two pixels) of `edges`. The pictures go to the evidence
/// directory as `<name>-cpu.png` and `<name>-gpu.png`.
fn measure(
    shell: &mut Shell,
    session: &mut Session,
    camera: CameraState,
    edges: &[[Vec3; 2]],
    name: &str,
) -> Vec<Point> {
    let tokens = shell.tokens;
    shell.state.camera = camera;
    let (w, h) = (1000u32, 700u32);
    let mut window = common_window(w as f32, h as f32);
    window.frame(shell, session);
    window.frame(shell, session);
    let [vw, vh] = shell.viewport_size();
    let (_, _, gpu_rgb) = shell.direct_picture().unwrap().unwrap();
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let quality = Quality {
        width: vw,
        height: vh,
        ssaa: 1,
    };
    let params = viewport::render_params(quality, &tokens);
    let frame = craft3d_render::render(&scene.mesh, &camera.to_render(), &params).unwrap();
    let slack = viewport::depth_slack(Backend::Gpu, Some(&scene), &camera);
    let overlay = viewport::overlay_see_through(
        Some(&scene),
        &camera,
        &frame,
        None,
        slack,
        &viewport::Marks::default(),
        false,
    );
    let overlay_lines = overlay.lines.clone();
    let picture = |overlay: viewport::Overlay| {
        viewport::rasterise(
            Composed {
                width: vw,
                height: vh,
                rgb: frame.rgb.clone(),
                overlay,
                coverage: frame.coverage(),
                triangles: scene.triangles(),
                backend: Backend::Cpu,
            },
            &tokens,
            1.0,
        )
        .rgb
    };
    // The picture without the edges (the axes still on it): what the GPU
    // picture is compared with for the edges' ink.
    let mut without_edges = overlay.clone();
    without_edges.lines.retain(|l| l.kind != LineKind::Edge);
    let base_rgb = picture(without_edges);
    let cpu_rgb = picture(overlay);
    for (suffix, rgb) in [("cpu", &cpu_rgb), ("gpu", &gpu_rgb)] {
        let png = craft3d_render::encode_png(vw, vh, rgb).unwrap();
        std::fs::write(evidence(&format!("{name}-{suffix}.png")), png).unwrap();
    }
    let (wu, hu) = (vw as usize, vh as usize);
    let lvl = |a: &[u8], i: usize| -> [u8; 3] { [a[i * 3], a[i * 3 + 1], a[i * 3 + 2]] };
    // Ink at a point of the GPU picture: one of the four pixels whose
    // centres are nearest to it differs from the picture without edges by
    // a quarter of the edge colour's contrast there (the nearest centre
    // is within 0.71 px of the line, where a 1 px line inks at least 0.29
    // of its colour).
    let nearest_four = |x: f64, y: f64| {
        let (cx, cy) = ((x - 0.5).floor() as i64, (y - 0.5).floor() as i64);
        (0..4).map(move |n| {
            let px = (cx + n % 2).clamp(0, wu as i64 - 1) as usize;
            let py = (cy + n / 2).clamp(0, hu as i64 - 1) as usize;
            py * wu + px
        })
    };
    let gpu_inked = |x: f64, y: f64| {
        nearest_four(x, y).any(|i| {
            let (p, q) = (lvl(&gpu_rgb, i), lvl(&base_rgb, i));
            let contrast = (0..3).map(|c| tokens.edge[c].abs_diff(q[c])).max().unwrap();
            let diff = (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap();
            u32::from(diff) * 4 > u32::from(contrast) && diff > 3
        })
    };
    // Where another line (an axis) is drawn over the bodies the edge's
    // ink cannot be told: such a point is not measured.
    let measurable =
        |x: f64, y: f64| nearest_four(x, y).all(|i| lvl(&base_rgb, i) == lvl(&frame.rgb, i));
    // The CPU's decision, from the overlay itself: the piece of an edge
    // line through the point; a piece the overlay left out is hidden.
    let cpu_hidden_at = |x: f64, y: f64| -> bool {
        overlay_lines
            .iter()
            .filter(|l| l.kind == LineKind::Edge)
            .find(|l| {
                let (ax, ay, bx, by) = (l.from[0], l.from[1], l.to[0], l.to[1]);
                let (dx, dy) = (bx - ax, by - ay);
                let len2 = dx * dx + dy * dy;
                if len2 == 0.0 {
                    return false;
                }
                let t = ((x - ax) * dx + (y - ay) * dy) / len2;
                (-1e-6..=1.0 + 1e-6).contains(&t)
                    && (ax + dx * t - x).abs() < 1e-6
                    && (ay + dy * t - y).abs() < 1e-6
            })
            .is_none_or(|l| l.hidden)
    };
    let projector = Projector::new(&camera.to_render(), vw as f64, vh as f64).unwrap();
    let mut points = Vec::new();
    for [a, b] in edges {
        let Some((pa, pb)) = projector.segment(*a, *b) else {
            continue;
        };
        let len = ((pb.x - pa.x).powi(2) + (pb.y - pa.y).powi(2)).sqrt();
        let steps = (len / 2.0).ceil().max(2.0) as usize;
        for k in 1..steps {
            let t = k as f64 / steps as f64;
            let (x, y) = (pa.x + (pb.x - pa.x) * t, pa.y + (pb.y - pa.y) * t);
            let dist = projector.depth_between(&pa, &pb, t);
            // Not at the picture's border, not within three pixels of the
            // edge's ends (where the faces it ends at come into the
            // samples) and not under another line.
            if x < 3.0 || y < 3.0 || x >= vw as f64 - 3.0 || y >= vh as f64 - 3.0 {
                continue;
            }
            if t * len < 3.0 || (1.0 - t) * len < 3.0 || !measurable(x, y) {
                continue;
            }
            let (px, py) = (x.floor() as usize, y.floor() as usize);
            let (mut far, mut near) = (0.0_f64, f64::INFINITY);
            for ny in py - 1..=py + 1 {
                for nx in px - 1..=px + 1 {
                    let d = frame.depth[ny * wu + nx];
                    far = far.max(d);
                    near = near.min(d);
                }
            }
            let own = frame.ids[py * wu + px];
            let plain = own != NO_FACE
                && (py - 2..=py + 2)
                    .all(|ny| (px - 2..=px + 2).all(|nx| frame.ids[ny * wu + nx] == own));
            let (cpu_hidden, gpu_hidden) = (cpu_hidden_at(x, y), !gpu_inked(x, y));
            points.push(Point {
                cpu_hidden,
                gpu_hidden,
                old_cpu_hidden: far + slack < dist,
                old_gpu_hidden: far + slack + (far - near) < dist,
                plain,
            });
        }
    }
    let count = |f: &dyn Fn(&Point) -> bool| points.iter().filter(|p| f(p)).count();
    println!(
        "{name}: {} points ({} on one plane): hidden CPU {} GPU {}; the old rules on these frames CPU {} GPU {}; CPU and GPU differ {}",
        points.len(),
        count(&|p| p.plain),
        count(&|p| p.cpu_hidden),
        count(&|p| p.gpu_hidden),
        count(&|p| p.old_cpu_hidden),
        count(&|p| p.old_gpu_hidden),
        count(&|p| p.cpu_hidden != p.gpu_hidden),
    );
    points
}

/// `camera` with its target moved `along` millimetres further along the
/// view: in orthographic the picture does not change, but the eye plane
/// moves with the target, into the model's box when `along` exceeds the
/// camera's distance, and the surfaces in front of it lie at negative
/// distances (`depth_range` fits near below zero): the depth test must
/// take those as surfaces, not as nothing.
fn moved_along_the_view(camera: CameraState, along: f64) -> CameraState {
    let forward = (camera.target() - camera.eye()).normalized().unwrap();
    let target = camera.target() + forward * along;
    CameraState {
        target: [target.x, target.y, target.z],
        ..camera
    }
}

/// The developer's view of the pier (yaw 24.553°, pitch 46.293°), at
/// `px_mm` model millimetres per pixel at the target.
fn camera(target: [f64; 3], projection: ProjectionKind, px_mm: f64) -> CameraState {
    let fov: f64 = 30.0;
    let half_tan = (fov.to_radians() * 0.5).tan();
    let distance = px_mm * 350.0 / half_tan;
    CameraState {
        target,
        distance,
        yaw: 24.553,
        pitch: 46.293,
        projection,
        fov_y: fov,
        auto_fit: false,
    }
}

#[test]
fn edges_behind_a_thin_plate_are_hidden_at_every_zoom_and_a_hollow_stays_in_sight() {
    let state = headless_render_state().unwrap();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.attach_gpu(&state).unwrap();
    assert!(shell.draws_directly());
    // The edges in a colour that stands out from every shade of the grey
    // faces: in the light theme (2026-10-21) the theme's near-black edge
    // is within a few levels of a face turned away from the light, where
    // its ink could not be told. What is measured is where an edge is
    // hidden, not its colour.
    shell.tokens.edge = [255, 0, 0];
    let mut leg_session = leg();
    let mut corner_session = corner();
    let mut failures = Vec::new();
    for (projection, pname) in [
        (ProjectionKind::Orthographic, "ortho"),
        (ProjectionKind::Perspective, "persp"),
    ] {
        // In perspective the eye stays outside the leg (4 m long).
        let zooms: &[f64] = if projection == ProjectionKind::Perspective {
            &[4.0, 8.0, 16.0, 30.0]
        } else {
            &[1.0, 4.0, 8.0, 16.0, 30.0]
        };
        // In orthographic also with the eye plane inside the leg (the
        // camera at 4 mm/px is 5224 mm from its target).
        let shifted: &[f64] = if projection == ProjectionKind::Orthographic {
            &[0.0, 6500.0]
        } else {
            &[0.0]
        };
        let mut cases: Vec<(f64, f64)> = Vec::new();
        for &px_mm in zooms {
            for &along in shifted {
                if along == 0.0 || px_mm == 4.0 || px_mm == 16.0 {
                    cases.push((px_mm, along));
                }
            }
        }
        for (px_mm, along) in cases {
            let cam = moved_along_the_view(camera([0.0, 200.0, 2000.0], projection, px_mm), along);
            let name = if along == 0.0 {
                format!("leg-{pname}-{px_mm}")
            } else {
                format!("leg-{pname}-{px_mm}-eye-inside")
            };
            let hidden = measure(
                &mut shell,
                &mut leg_session,
                cam,
                &leg_hidden_edges(),
                &name,
            );
            let plain: Vec<&Point> = hidden.iter().filter(|p| p.plain).collect();
            // Every point behind the plate alone is hidden on both paths.
            let shown_cpu = plain.iter().filter(|p| !p.cpu_hidden).count();
            let shown_gpu = plain.iter().filter(|p| !p.gpu_hidden).count();
            if plain.len() < 50 || shown_cpu > 0 || shown_gpu > 0 {
                failures.push(format!(
                    "{name}: of {} points behind the plate the CPU shows {shown_cpu} and the GPU {shown_gpu}",
                    plain.len()
                ));
            }
            // And the two paths decide alike almost everywhere, also at
            // the plate's outline and ends.
            let differ = hidden
                .iter()
                .filter(|p| p.cpu_hidden != p.gpu_hidden)
                .count();
            if differ * 100 > hidden.len() {
                failures.push(format!(
                    "{name}: the paths differ at {differ} of {} points",
                    hidden.len()
                ));
            }
            let outline = measure(
                &mut shell,
                &mut leg_session,
                cam,
                &leg_outline_edges(),
                &format!("{name}-outline"),
            );
            let hidden_cpu = outline.iter().filter(|p| p.cpu_hidden).count();
            let hidden_gpu = outline.iter().filter(|p| p.gpu_hidden).count();
            if outline.len() < 50 || hidden_cpu > 0 || hidden_gpu > 0 {
                failures.push(format!(
                    "{name}: of {} points of the plate's outline the CPU hides {hidden_cpu} and the GPU {hidden_gpu}",
                    outline.len()
                ));
            }
        }
        for px_mm in [0.25, 1.0, 2.0, 4.0] {
            let cam = camera([100.0, 100.0, 200.0], projection, px_mm);
            let name = format!("corner-{pname}-{px_mm}");
            let hollow = measure(
                &mut shell,
                &mut corner_session,
                cam,
                &corner_hollow_edge(),
                &name,
            );
            let hidden_cpu = hollow.iter().filter(|p| p.cpu_hidden).count();
            let hidden_gpu = hollow.iter().filter(|p| p.gpu_hidden).count();
            if hollow.len() < 20 || hidden_cpu > 0 || hidden_gpu > 0 {
                failures.push(format!(
                    "{name}: of {} points of the hollow edge the CPU hides {hidden_cpu} and the GPU {hidden_gpu}",
                    hollow.len()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

struct Win {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

fn common_window(width: f32, height: f32) -> Win {
    Win {
        ctx: egui::Context::default(),
        size: egui::vec2(width, height),
        time: 0.0,
    }
}

impl Win {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            time: Some(self.time),
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }
}
