//! The review of check/runaway (2026-11-18), on the functions alone: the
//! section outline's cut at the eye plane (`Projector::segment_cut`), its
//! cut to the viewport (`section_handle::dashed_sides`) and its dasher
//! (`section_handle::outline_dashes`), by cases built on the screen and by
//! seeded random ones; the dashes against the ones main drew (epaint's
//! dashing of `outline_paths`, as main had it) at ordinary distances; and
//! the dashes held to the line while the view pans.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::SectionPlane;
use craft3d_math::{Bbox3, Portable, Vec3};
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::shell::section_handle::{
    DashedSide, MAX_OUTLINE_DASHES, dashed_sides, outline_dashes, plane_frame,
};
use craft3d_ui_egui::{CameraState, ProjectionKind, Tokens};
use egui::{Pos2, Rect, Shape, Stroke, pos2, vec2};
use std::time::{Duration, Instant};

/// The branch's margin round the viewport (`DASH_MARGIN`, private).
const MARGIN: f32 = 8.0;

/// A seeded generator (xorshift64*): the same cases every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    /// 10^(lo..hi), with a random sign when `signed`.
    fn magnitude(&mut self, lo: f64, hi: f64, signed: bool) -> f64 {
        let m = 10.0_f64.ppowf(self.range(lo, hi));
        if signed && self.unit() < 0.5 { -m } else { m }
    }
    fn pick<T: Copy>(&mut self, from: &[T]) -> T {
        from[(self.next() % from.len() as u64) as usize]
    }
}

fn machine_bounds() -> Bbox3 {
    Bbox3::from_points([
        Vec3::new(-260.0, -210.0, -18.0),
        Vec3::new(260.0, 210.0, 470.0),
    ])
}

fn camera(
    target: [f64; 3],
    distance: f64,
    yaw: f64,
    pitch: f64,
    projection: ProjectionKind,
) -> CameraState {
    CameraState {
        target,
        distance,
        yaw,
        pitch,
        projection,
        fov_y: 30.0,
        auto_fit: false,
    }
}

/// The point of space that `projector` shows at (`x`, `y`) of its image,
/// `depth` in front of the eye (behind it when negative).
fn at_screen(projector: &Projector, x: f64, y: f64, depth: f64) -> Vec3 {
    let (start, direction) = projector.ray(x, y);
    start + direction * depth
}

/// A point of the eye plane, off to the side toward where (`x`, `y`) is
/// on the image, `reach` times as far as that pixel's ray is from the
/// view's middle at a depth of one (for an orthographic camera, the point
/// of the eye plane under the pixel).
fn in_eye_plane(projector: &Projector, x: f64, y: f64, reach: f64) -> Vec3 {
    let (start, direction) = projector.ray(x, y);
    if projector.perspective() {
        let (_, forward) = projector.ray(500.0, 350.0);
        start + (direction - forward) * reach
    } else {
        start
    }
}

fn bits(p: &craft3d_ui_egui::camera::Projected) -> [u64; 3] {
    [p.x.to_bits(), p.y.to_bits(), p.depth.to_bits()]
}

/// How far `p` lies outside `room` (0 inside).
fn outside_by(room: Rect, p: Pos2) -> f32 {
    let dx = (room.left() - p.x).max(p.x - room.right()).max(0.0);
    let dy = (room.top() - p.y).max(p.y - room.bottom()).max(0.0);
    dx.max(dy)
}

/// The invariants of the branch's sides and dashes for one outline: every
/// end finite and within the viewport grown by the margin (and `slack`),
/// every phase finite and within a period, at most the cap of dashes, each
/// within the grown viewport too. The largest excursion past the margin.
fn check_outline(
    projector: &Projector,
    corners: &[Vec3],
    viewport: Rect,
    dash: [f32; 2],
    slack: f32,
    what: &str,
) -> (Vec<DashedSide>, Vec<[Pos2; 2]>, f32) {
    let sides = dashed_sides(projector, corners, viewport, dash);
    let period = dash[0] + dash[1];
    let room = viewport.expand(MARGIN);
    let mut worst = 0.0_f32;
    assert!(
        sides.len() <= corners.len().max(1),
        "{what}: {}",
        sides.len()
    );
    for side in &sides {
        for end in side.ends {
            assert!(end.x.is_finite() && end.y.is_finite(), "{what}: {side:?}");
            let by = outside_by(room, end);
            worst = worst.max(by);
            assert!(by <= slack, "{what}: {end:?} is {by} past the margin");
        }
        assert!(
            side.phase.is_finite() && side.phase >= 0.0 && side.phase <= period,
            "{what}: phase {}",
            side.phase
        );
    }
    let dashes = outline_dashes(&sides, dash);
    assert!(dashes.len() <= MAX_OUTLINE_DASHES, "{what}");
    for [a, b] in &dashes {
        for p in [a, b] {
            assert!(p.x.is_finite() && p.y.is_finite(), "{what}");
            assert!(outside_by(room, *p) <= slack, "{what}: dash {a:?} {b:?}");
        }
    }
    (sides, dashes, worst)
}

#[test]
fn segment_cut_is_segment_bit_for_bit_and_says_which_end_it_cut() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let hostile = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1e300,
        -1e300,
        f64::MAX,
        f64::MIN_POSITIVE,
        0.0,
    ];
    let (mut both, mut neither, mut cut_a, mut cut_b, mut still) = (0, 0, 0, 0, 0);
    for k in 0..400 {
        let projection = if k % 3 == 0 {
            ProjectionKind::Orthographic
        } else {
            ProjectionKind::Perspective
        };
        let cam = camera(
            [
                rng.range(-500.0, 500.0),
                rng.range(-500.0, 500.0),
                rng.range(-500.0, 500.0),
            ],
            rng.magnitude(-9.0, 6.0, false),
            rng.range(-180.0, 180.0),
            rng.range(-89.0, 89.0),
            projection,
        );
        let Some(projector) = Projector::new(&cam.to_render(), 1000.0, 700.0) else {
            continue;
        };
        let eye = cam.eye();
        let forward = (cam.target() - eye).normalized().unwrap();
        for _ in 0..200 {
            let point = |rng: &mut Rng| -> Vec3 {
                match rng.next() % 6 {
                    // Round the eye, at any scale, in front or behind.
                    0 | 1 => {
                        let s = rng.magnitude(-9.0, 7.0, false);
                        eye + Vec3::new(
                            rng.range(-1.0, 1.0),
                            rng.range(-1.0, 1.0),
                            rng.range(-1.0, 1.0),
                        ) * s
                    }
                    // In the eye plane, off to the side.
                    2 => {
                        let x = rng.magnitude(-3.0, 9.0, true);
                        let y = rng.magnitude(-3.0, 9.0, true);
                        in_eye_plane(&projector, x, y, rng.magnitude(-6.0, 3.0, false))
                    }
                    // Just behind or in front of the eye plane.
                    3 => {
                        let x = rng.range(-1e4, 1e4);
                        let y = rng.range(-1e4, 1e4);
                        at_screen(&projector, x, y, rng.magnitude(-12.0, -6.0, true))
                    }
                    // Far ahead.
                    4 => eye + forward * rng.magnitude(0.0, 12.0, false),
                    // Hostile numbers.
                    _ => Vec3::new(
                        rng.pick(&hostile),
                        rng.range(-10.0, 10.0),
                        rng.pick(&hostile),
                    ),
                }
            };
            let a = point(&mut rng);
            let b = if rng.next().is_multiple_of(10) {
                a
            } else {
                point(&mut rng)
            };
            let plain = projector.segment(a, b);
            let with = projector.segment_cut(a, b);
            match (plain, with) {
                (None, None) => {}
                (Some((p, q)), Some((p2, q2, cut))) => {
                    assert_eq!(bits(&p), bits(&p2), "{a:?} {b:?}");
                    assert_eq!(bits(&q), bits(&q2), "{a:?} {b:?}");
                    for c in [p.x, p.y, p.depth, q.x, q.y, q.depth] {
                        assert!(c.is_finite(), "{a:?} {b:?}");
                    }
                    if !projector.perspective() {
                        assert_eq!(cut, [false, false]);
                    }
                    // An end not cut is the end itself, projected.
                    for (end, seen, was_cut) in [(a, &p, cut[0]), (b, &q, cut[1])] {
                        if was_cut {
                            // Within rounding of the eye plane the cut may
                            // not move the end forward: counted.
                            if projector.depth(end) >= seen.depth {
                                still += 1;
                                if still <= 3 {
                                    eprintln!(
                                        "cut, not moved forward: {end:?} at depth {:e}, cut to {:e} (the other end at {:e})",
                                        projector.depth(end),
                                        seen.depth,
                                        projector.depth(if end == a { b } else { a })
                                    );
                                }
                            }
                        } else {
                            let own = projector.project(end).unwrap();
                            assert_eq!(bits(&own), bits(seen), "{end:?}");
                        }
                    }
                    match cut {
                        [false, false] => neither += 1,
                        [true, false] => cut_a += 1,
                        [false, true] => cut_b += 1,
                        [true, true] => both += 1,
                    }
                }
                (plain, with) => panic!("{a:?} {b:?}: {plain:?} against {with:?}"),
            }
        }
    }
    eprintln!("segment_cut: {neither} whole, {cut_a} cut at a, {cut_b} cut at b, {both} at both");
    assert_eq!(both, 0, "both ends cut: the segment is behind the eye");
    assert!(cut_a > 100 && cut_b > 100 && neither > 1000);
}

#[test]
fn sides_built_on_the_screen_are_cut_to_the_viewport_and_stay_finite() {
    let dash = Tokens::default().section_dash;
    for (origin, projection) in [
        (pos2(0.0, 0.0), ProjectionKind::Perspective),
        (pos2(30.0, 40.0), ProjectionKind::Perspective),
        (pos2(30.0, 40.0), ProjectionKind::Orthographic),
    ] {
        let viewport = Rect::from_min_size(origin, vec2(1000.0, 700.0));
        let cam = camera([0.0, 0.0, 0.0], 50.0, -45.0, 30.0, projection);
        let projector = Projector::new(&cam.to_render(), 1000.0, 700.0).unwrap();
        let perspective = projector.perspective();
        let p = |x: f64, y: f64, depth: f64| at_screen(&projector, x, y, depth);
        let far = 1e9;
        let mut cases: Vec<(&str, Vec<Vec3>, Option<usize>)> = vec![
            (
                "wholly in front, inside",
                vec![
                    p(100.0, 100.0, 10.0),
                    p(900.0, 100.0, 10.0),
                    p(900.0, 600.0, 10.0),
                    p(100.0, 600.0, 10.0),
                ],
                Some(4),
            ),
            (
                "round the viewport, far out",
                vec![
                    p(-far, -far, 10.0),
                    p(far, -far, 10.0),
                    p(far, far, 10.0),
                    p(-far, far, 10.0),
                ],
                Some(0),
            ),
            (
                "a side across the middle",
                vec![
                    p(-far, 350.0, 10.0),
                    p(far, 350.0, 10.0),
                    p(far, far, 10.0),
                    p(-far, far, 10.0),
                ],
                Some(1),
            ),
            (
                "a side along the top, half a point past the margin",
                vec![
                    p(-far, -8.5, 10.0),
                    p(far, -8.5, 10.0),
                    p(far, -far, 10.0),
                    p(-far, -far, 10.0),
                ],
                Some(0),
            ),
            (
                "a side along the top, half a point inside the margin",
                vec![
                    p(-far, -7.5, 10.0),
                    p(far, -7.5, 10.0),
                    p(far, -far, 10.0),
                    p(-far, -far, 10.0),
                ],
                Some(1),
            ),
            (
                "a side along the right edge, just outside the margin",
                vec![
                    p(1008.5, -far, 10.0),
                    p(1008.5, far, 10.0),
                    p(far, far, 10.0),
                ],
                Some(0),
            ),
            (
                "a side through two corners of the viewport",
                vec![
                    p(-1e6, -7e5, 10.0),
                    p(1e6 + 1000.0, 7e5 + 700.0, 10.0),
                    p(far, -far, 10.0),
                ],
                Some(1),
            ),
            (
                "a side of no length",
                vec![
                    p(200.0, 200.0, 10.0),
                    p(200.0, 200.0, 10.0),
                    p(700.0, 500.0, 10.0),
                ],
                None,
            ),
            (
                "seen from a billionth of a unit",
                vec![
                    p(-1e14, -1e14, 1e-9),
                    p(1e14, -1e14, 1e-9),
                    p(1e14, 3e2, 1e-9),
                    p(-1e14, 3e2, 1e-9),
                ],
                None,
            ),
            (
                "a corner that is not a number",
                vec![
                    p(100.0, 100.0, 10.0),
                    Vec3::new(f64::NAN, 0.0, 0.0),
                    p(900.0, 600.0, 10.0),
                ],
                Some(1),
            ),
            (
                "a corner at infinity",
                vec![
                    p(100.0, 100.0, 10.0),
                    Vec3::new(f64::INFINITY, 0.0, 0.0),
                    p(900.0, 600.0, 10.0),
                ],
                Some(1),
            ),
            (
                "a corner at 1e300",
                vec![
                    p(100.0, 100.0, 10.0),
                    Vec3::new(1e300, -1e300, 1e300),
                    p(900.0, 600.0, 10.0),
                ],
                None,
            ),
            ("no corners", vec![], Some(0)),
            ("one corner", vec![p(500.0, 300.0, 10.0)], None),
        ];
        if perspective {
            cases.extend([
                (
                    "wholly behind the eye",
                    vec![
                        p(100.0, 100.0, -10.0),
                        p(900.0, 100.0, -10.0),
                        p(500.0, 600.0, -10.0),
                    ],
                    Some(0),
                ),
                (
                    "lying in the eye plane",
                    vec![
                        in_eye_plane(&projector, 100.0, 100.0, 5.0),
                        in_eye_plane(&projector, 900.0, 100.0, 5.0),
                        in_eye_plane(&projector, 500.0, 600.0, 5.0),
                    ],
                    Some(0),
                ),
                (
                    "a side in the eye plane, the third corner in front",
                    vec![
                        in_eye_plane(&projector, 100.0, 100.0, 5.0),
                        in_eye_plane(&projector, 900.0, 100.0, 5.0),
                        p(500.0, 350.0, 10.0),
                    ],
                    None,
                ),
                (
                    "two sides through the eye plane",
                    vec![
                        p(200.0, 300.0, 10.0),
                        p(800.0, 300.0, 10.0),
                        p(500.0, 350.0, -10.0),
                    ],
                    None,
                ),
                (
                    "a side from just in front to far behind",
                    vec![
                        p(500.0, 350.0, 1e-12),
                        p(500.0, 350.0, -1e9),
                        p(100.0, 100.0, 10.0),
                    ],
                    None,
                ),
            ]);
        }
        for (what, corners, expected) in &cases {
            let what = format!("{what} ({projection:?}, viewport at {origin:?})");
            let (sides, dashes, worst) =
                check_outline(&projector, corners, viewport, dash, 0.5, &what);
            if let Some(n) = expected {
                assert_eq!(sides.len(), *n, "{what}: {sides:?}");
            }
            eprintln!(
                "{what}: {} sides, {} dashes, {worst} past the margin",
                sides.len(),
                dashes.len()
            );
        }
        // The same sides through a viewport at the origin: moved by its
        // corner, the same phases.
        let at_zero = Rect::from_min_size(Pos2::ZERO, viewport.size());
        let (_, corners, _) = &cases[2];
        let (moved, here) = (
            dashed_sides(&projector, corners, viewport, dash),
            dashed_sides(&projector, corners, at_zero, dash),
        );
        assert_eq!(moved.len(), here.len());
        for (m, h) in moved.iter().zip(&here) {
            assert_eq!(m.phase, h.phase);
            for (a, b) in m.ends.iter().zip(h.ends) {
                assert!((*a - (b + origin.to_vec2())).length() < 1e-3);
            }
        }
    }
}

#[test]
fn random_outlines_whatever_the_camera_stay_in_the_viewport() {
    let mut rng = Rng(0x0dd_ba11_cafe_f00d);
    let dash = Tokens::default().section_dash;
    let viewport = Rect::from_min_size(pos2(12.0, 34.0), vec2(1000.0, 700.0));
    let (mut outlines, mut with_sides, mut dashes_most, mut past) = (0, 0, 0, 0);
    let mut worst = 0.0_f32;
    let mut worst_far = 0.0_f32;
    let started = Instant::now();
    for k in 0..3000 {
        let projection = if k % 4 == 0 {
            ProjectionKind::Orthographic
        } else {
            ProjectionKind::Perspective
        };
        let cam = camera(
            [
                rng.range(-1e3, 1e3),
                rng.range(-1e3, 1e3),
                rng.range(-1e3, 1e3),
            ],
            rng.magnitude(-9.0, 6.0, false),
            rng.range(-180.0, 180.0),
            rng.range(-89.0, 89.0),
            projection,
        );
        let Some(projector) = Projector::new(&cam.to_render(), 1000.0, 700.0) else {
            continue;
        };
        // A plane's outline: three to six corners, on the screen at up to
        // 1e12 points, in front of the eye, in its plane and behind it.
        let n = 3 + (rng.next() % 4) as usize;
        let reach = rng.range(0.0, 12.0);
        let corners: Vec<Vec3> = (0..n)
            .map(|_| {
                let x = 500.0 + rng.magnitude(-1.0, reach, true);
                let y = 350.0 + rng.magnitude(-1.0, reach, true);
                match rng.next() % 6 {
                    0 => at_screen(&projector, x, y, -rng.magnitude(-9.0, 4.0, false)),
                    1 => in_eye_plane(&projector, x, y, rng.magnitude(-6.0, 3.0, false)),
                    _ => at_screen(&projector, x, y, rng.magnitude(-9.0, 4.0, false)),
                }
            })
            .collect();
        outlines += 1;
        // How far past the margin the cut ends land is measured, not
        // asserted: past some 1e15 points `f64` cannot place them.
        let (sides, dashes, by) = check_outline(
            &projector,
            &corners,
            viewport,
            dash,
            f32::INFINITY,
            &format!("outline {k}"),
        );
        if by > 1.0 {
            past += 1;
            let seen: Vec<String> = corners
                .iter()
                .map(|c| {
                    projector.project(*c).map_or("behind".to_string(), |p| {
                        format!("({:.3e}, {:.3e}) at depth {:.3e}", p.x, p.y, p.depth)
                    })
                })
                .collect();
            eprintln!(
                "outline {k}: {by} points past the margin; distance {:e}, {projection:?}; corners {seen:?}",
                cam.distance
            );
        }
        if reach <= 9.0 {
            worst = worst.max(by);
        } else {
            worst_far = worst_far.max(by);
        }
        with_sides += usize::from(!sides.is_empty());
        dashes_most = dashes_most.max(dashes.len());
    }
    eprintln!(
        "{outlines} random outlines in {:?}: {with_sides} with sides in view, at most {dashes_most} dashes; \
         {past} with an end more than a point past the margin; at most {worst} points past it \
         (corners within 1e9 points of the view), {worst_far} (beyond)",
        started.elapsed()
    );
    assert!(dashes_most <= MAX_OUTLINE_DASHES, "{dashes_most}");
    assert!(with_sides > 50, "{with_sides}");
}

#[test]
fn the_dasher_ends_whatever_the_pattern_and_the_side() {
    let values = [
        0.0_f32,
        -0.0,
        -1.0,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1e-45,
        f32::MIN_POSITIVE,
        1e-30,
        1e-6,
        4.0,
        5.0,
        1e30,
        f32::MAX,
    ];
    let phases = [
        0.0_f32,
        3.0,
        -7.5,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1e30,
        -1e30,
    ];
    let ends = [
        [pos2(0.0, 0.0), pos2(1e9, 0.0)],
        [pos2(-1e9, -1e9), pos2(1e9, 1e9)],
        [pos2(0.0, 0.0), pos2(10.0, 0.0)],
        [pos2(3.0, 3.0), pos2(3.0, 3.0)],
        [pos2(f32::NAN, 0.0), pos2(1.0, 1.0)],
        [pos2(f32::NEG_INFINITY, 0.0), pos2(f32::INFINITY, 0.0)],
        [pos2(f32::MAX, f32::MAX), pos2(-f32::MAX, -f32::MAX)],
    ];
    let started = Instant::now();
    let mut slowest = Duration::ZERO;
    let (mut none, mut whole, mut capped) = (0, 0, 0);
    for &d in &values {
        for &g in &values {
            for &phase in &phases {
                let sides: Vec<DashedSide> = ends
                    .iter()
                    .map(|&ends| DashedSide { ends, phase })
                    .collect();
                let one = Instant::now();
                let dashes = outline_dashes(&sides, [d, g]);
                let took = one.elapsed();
                slowest = slowest.max(took);
                assert!(dashes.len() <= MAX_OUTLINE_DASHES, "{d} {g} {phase}");
                assert!(
                    took < Duration::from_millis(500),
                    "{d} {g} {phase}: {took:?}"
                );
                for [a, b] in &dashes {
                    assert!(
                        a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite(),
                        "{d} {g} {phase}: {a:?} {b:?}"
                    );
                }
                match dashes.len() {
                    0 => none += 1,
                    MAX_OUTLINE_DASHES => capped += 1,
                    _ => whole += 1,
                }
                // dashed_sides takes the same pattern without harm.
                let projector = Projector::new(
                    &camera([0.0; 3], 10.0, 0.0, 0.0, ProjectionKind::Perspective).to_render(),
                    100.0,
                    100.0,
                )
                .unwrap();
                let corners = [
                    at_screen(&projector, -1e8, 50.0, 1.0),
                    at_screen(&projector, 1e8, 50.0, 1.0),
                    at_screen(&projector, 1e8, 1e8, 1.0),
                ];
                let sides = dashed_sides(
                    &projector,
                    &corners,
                    Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0)),
                    [d, g],
                );
                assert!(sides.len() <= 3);
                assert!(outline_dashes(&sides, [d, g]).len() <= MAX_OUTLINE_DASHES);
            }
        }
    }
    eprintln!(
        "{} patterns × {} phases × {} sides in {:?} (slowest {slowest:?}): {none} drew nothing, \
         {capped} reached the cap, {whole} in between",
        values.len() * values.len(),
        phases.len(),
        ends.len(),
        started.elapsed()
    );
    // What a pattern that is no pattern draws (theme tokens are data).
    let long = DashedSide {
        ends: [pos2(0.0, 0.0), pos2(100.0, 0.0)],
        phase: 0.0,
    };
    for pattern in [
        [5.0, -4.0],
        [5.0, f32::INFINITY],
        [f32::INFINITY, 4.0],
        [-5.0, 4.0],
        [0.0, 0.0],
    ] {
        eprintln!(
            "pattern {pattern:?}: {} dashes on a side of 100 points",
            outline_dashes(&[long], pattern).len()
        );
    }
}

/// How main drew the outline (`section_handle::outline_paths` of
/// fd13129d, word for word): one closed path when the sides join, one
/// line between the farthest points seen edge on, else side by side.
fn main_outline_paths(outline: &[[Pos2; 2]]) -> Vec<Vec<Pos2>> {
    let joined = !outline.is_empty()
        && outline.iter().enumerate().all(|(k, [_, end])| {
            outline
                .get((k + 1) % outline.len())
                .is_some_and(|[next, _]| next.distance(*end) < 0.01)
        });
    if !joined {
        return outline.iter().map(|side| side.to_vec()).collect();
    }
    let points: Vec<Pos2> = outline.iter().map(|[a, _]| *a).collect();
    let n = points.len();
    let (mut area, mut perimeter) = (0.0_f32, 0.0_f32);
    for k in 0..n {
        let (Some(a), Some(b)) = (points.get(k), points.get((k + 1) % n)) else {
            continue;
        };
        area += a.x * b.y - b.x * a.y;
        perimeter += a.distance(*b);
    }
    if perimeter > 0.0 && area.abs() / perimeter < 1.5 {
        let mut far = (0.0_f32, None);
        for a in &points {
            for b in &points {
                let d = a.distance(*b);
                if d > far.0 {
                    far = (d, Some([*a, *b]));
                }
            }
        }
        return far.1.map(|[a, b]| vec![vec![a, b]]).unwrap_or_default();
    }
    let mut closed = points;
    if let Some(&first) = closed.first() {
        closed.push(first);
    }
    vec![closed]
}

/// Main's dashes of `outline`: epaint's, of main's paths.
fn main_dashes(outline: &[[Pos2; 2]], [dash, gap]: [f32; 2]) -> Vec<[Pos2; 2]> {
    let mut out = Vec::new();
    for path in main_outline_paths(outline) {
        for shape in Shape::dashed_line(&path, Stroke::new(1.0, egui::Color32::WHITE), dash, gap) {
            if let Shape::LineSegment { points, .. } = shape {
                out.push(points);
            }
        }
    }
    out
}

/// The part of the segment inside `room`, longer than a thousandth of a
/// point: what of a dash shows.
fn shown_part([a, b]: [Pos2; 2], room: Rect) -> Option<[Pos2; 2]> {
    let (ax, ay, bx, by) = (
        f64::from(a.x),
        f64::from(a.y),
        f64::from(b.x),
        f64::from(b.y),
    );
    let (dx, dy) = (bx - ax, by - ay);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (delta, start, lo, hi) in [
        (dx, ax, f64::from(room.left()), f64::from(room.right())),
        (dy, ay, f64::from(room.top()), f64::from(room.bottom())),
    ] {
        if delta == 0.0 {
            if start < lo || start > hi {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = ((lo - start) / delta, (hi - start) / delta);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    let p = |t: f64| pos2((ax + dx * t) as f32, (ay + dy * t) as f32);
    let (p0, p1) = (p(t0), p(t1));
    (p0.distance(p1) > 1e-3).then_some([p0, p1])
}

/// Every piece of `a` has a piece of `b` with both ends within `tolerance`:
/// how many have none, and the largest distance of those that have.
fn unmatched(a: &[[Pos2; 2]], b: &[[Pos2; 2]], tolerance: f32) -> (usize, f32) {
    let mut missing = 0;
    let mut most = 0.0_f32;
    for [p, q] in a {
        let best = b
            .iter()
            .map(|[r, s]| p.distance(*r).max(q.distance(*s)))
            .fold(f32::INFINITY, f32::min);
        if best <= tolerance {
            most = most.max(best);
        } else {
            missing += 1;
        }
    }
    (missing, most)
}

#[test]
fn at_ordinary_distances_the_dashes_that_show_are_mains() {
    let tokens = Tokens::default();
    let viewport = Rect::from_min_size(pos2(30.0, 40.0), vec2(1000.0, 700.0));
    let bounds = machine_bounds();
    let mut fitted = CameraState {
        auto_fit: false,
        ..CameraState::default()
    };
    fitted.fit(&bounds, 1000.0 / 700.0);
    let (mut views, mut compared, mut most) = (0, 0, 0.0_f32);
    let (mut edge_on, mut edge_on_differ) = (0, 0);
    for (plane_name, normal) in [
        ("zx", Vec3::new(0.0, 1.0, 0.0)),
        ("xy", Vec3::new(0.0, 0.0, 1.0)),
        ("yz", Vec3::new(1.0, 0.0, 0.0)),
    ] {
        for offset in [-150.0, 0.0, 120.0] {
            let base = Vec3::new(0.0, 0.0, 226.0);
            let plane = SectionPlane {
                origin: base + normal * offset,
                normal,
                base,
                along: normal,
            };
            for projection in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
                for k in 0..12 {
                    for (pitch, scale) in [(0.0, 1.0), (12.0, 0.8), (35.0, 1.0), (65.0, 1.6)] {
                        let cam = CameraState {
                            yaw: -180.0 + 30.0 * f64::from(k),
                            pitch,
                            distance: fitted.distance * scale,
                            projection,
                            ..fitted
                        };
                        let Some(frame) =
                            plane_frame(&cam, viewport, &[], &plane, &bounds, &tokens)
                        else {
                            continue;
                        };
                        views += 1;
                        let seen_on_edge = main_outline_paths(&frame.outline)
                            .first()
                            .is_some_and(|path| path.len() == 2);
                        edge_on += usize::from(seen_on_edge);
                        let shown = |dashes: Vec<[Pos2; 2]>| -> Vec<[Pos2; 2]> {
                            dashes
                                .into_iter()
                                .filter_map(|d| shown_part(d, viewport))
                                .collect()
                        };
                        let theirs = shown(main_dashes(&frame.outline, tokens.section_dash));
                        let ours = shown(outline_dashes(&frame.dashed, tokens.section_dash));
                        let what = format!(
                            "{plane_name} at {offset}, yaw {}, pitch {pitch}, {scale} × fitted, {projection:?}",
                            cam.yaw
                        );
                        let (missing, far) = unmatched(&theirs, &ours, 0.05);
                        let (extra, far2) = unmatched(&ours, &theirs, 0.05);
                        if seen_on_edge && (missing > 0 || extra > 0) {
                            // Seen edge on, which farthest pair (and so
                            // which end the pattern starts at) may differ
                            // in the last bits: counted, not asserted.
                            edge_on_differ += 1;
                            eprintln!(
                                "{what}, edge on: {missing} of main's {} dashes and {extra} of ours {} differ",
                                theirs.len(),
                                ours.len()
                            );
                            continue;
                        }
                        assert!(
                            missing == 0 && extra == 0,
                            "{what}: {missing} of main's {} dashes not drawn, {extra} of ours {} not main's",
                            theirs.len(),
                            ours.len()
                        );
                        compared += theirs.len();
                        most = most.max(far).max(far2);
                    }
                }
            }
        }
    }
    eprintln!(
        "{views} ordinary views ({edge_on} edge on, {edge_on_differ} of them dashed otherwise): \
         {compared} dashes that show compared with main's, at most {most} points apart"
    );
    assert!(views > 500 && compared > 10_000);
}

#[test]
fn the_dashes_stay_on_the_line_when_the_view_pans() {
    let tokens = Tokens::default();
    let dash = tokens.section_dash;
    let viewport = Rect::from_min_size(pos2(30.0, 40.0), vec2(1000.0, 700.0));
    let bounds = machine_bounds();
    let normal = Vec3::new(0.0, 1.0, 0.0);
    let plane = SectionPlane {
        origin: Vec3::new(0.0, 0.0, 226.0),
        normal,
        base: Vec3::new(0.0, 0.0, 226.0),
        along: normal,
    };
    // The outline's corner at about (-275.2, 0, -32.2) (the box's cut grown
    // by 4 % of its larger side along its diagonals) and the sides from
    // it, close by (they run far past the view), panned along the bottom
    // side (x), along the left side (z) and across both; orthographic at
    // any angle and perspective face on, where a pan moves the whole
    // picture alike.
    let bottom = 226.0 - 244.0 * (1.0 + 20.8 / 260.0_f64.phypot(244.0));
    let cases = [
        (
            "orthographic, turned, zoomed in",
            camera(
                [-270.0, 0.0, -30.0],
                40.0,
                -70.0,
                20.0,
                ProjectionKind::Orthographic,
            ),
        ),
        (
            "orthographic, face on, the bottom side from a hundredth of a unit",
            camera(
                [0.0, 0.0, bottom],
                1e-2,
                -90.0,
                0.0,
                ProjectionKind::Orthographic,
            ),
        ),
        (
            "perspective, face on from 20 mm",
            camera(
                [-277.0, 0.0, -35.0],
                20.0,
                -90.0,
                0.0,
                ProjectionKind::Perspective,
            ),
        ),
    ];
    let mut crawled = 0;
    for (what, before) in cases {
        for (way, shift) in [
            ("along x", Vec3::new(1.0, 0.0, 0.0)),
            ("along z", Vec3::new(0.0, 0.0, 1.0)),
            ("across", Vec3::new(0.6, 0.0, 0.8)),
        ] {
            let step = before.distance * 0.037;
            let mut after = before;
            let t = before.target() + shift * step;
            after.target = [t.x, t.y, t.z];
            let frames = [before, after]
                .map(|cam| plane_frame(&cam, viewport, &[], &plane, &bounds, &tokens).unwrap());
            let projectors =
                [before, after].map(|cam| Projector::new(&cam.to_render(), 1000.0, 700.0).unwrap());
            // How far the picture moved: any point of the plane will do.
            let anchor = Vec3::new(-200.0, 0.0, 30.0);
            let (p, q) = (
                projectors[0].project(anchor).unwrap(),
                projectors[1].project(anchor).unwrap(),
            );
            let moved = vec2((q.x - p.x) as f32, (q.y - p.y) as f32);
            let dashes = frames
                .clone()
                .map(|frame| outline_dashes(&frame.dashed, dash));
            let inner = viewport.shrink(20.0);
            // Well inside the view before the pan as after it: a dash the
            // first view cut at its margin is no dash to find again.
            let shown: Vec<[Pos2; 2]> = dashes[0]
                .iter()
                .filter(|[a, b]| inner.contains(*a) && inner.contains(*b))
                .map(|[a, b]| [*a + moved, *b + moved])
                .filter(|[a, b]| inner.contains(*a) && inner.contains(*b))
                .collect();
            // Pieces shorter than a twentieth of a point (a dash that ends
            // on a corner, by rounding) are no dash a person sees.
            let shown: Vec<[Pos2; 2]> = shown
                .into_iter()
                .filter(|[a, b]| a.distance(*b) >= 0.05)
                .collect();
            let (missing, most) = unmatched(&shown, &dashes[1], 0.05);
            for [a, b] in &shown {
                let best = dashes[1]
                    .iter()
                    .map(|[r, t]| a.distance(*r).max(b.distance(*t)))
                    .fold(f32::INFINITY, f32::min);
                if best > 0.05 {
                    eprintln!(
                        "{what} {way}: the dash {a:?}-{b:?} ({} long) is {best} from any after the pan",
                        a.distance(*b)
                    );
                }
            }
            eprintln!(
                "{what}, panned {way} by {:.3} points: {} dashes well inside, {missing} moved off \
                 the line's pattern, the rest within {most} points",
                moved.length(),
                shown.len()
            );
            assert!(shown.len() > 20, "{what} {way}: {}", shown.len());
            crawled += missing;
        }
    }
    assert_eq!(crawled, 0, "dashes moved along their line under a pan");
}
