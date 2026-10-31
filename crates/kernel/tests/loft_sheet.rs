//! A loft through open sections is a sheet (the Garabit valley's finding
//! 1, 2026-10-21: a terrain as a B-spline sheet that cuts a block). Held
//! to: the open chain a sketch's curves make (`profile::open_chain`), the
//! sheet's topology (`topo::check`, one boundary loop, no caps), which
//! way it faces (the chain's direction × the loft's), and the block cut by
//! it: for a ruled loft of quadratic Bézier sections the volume under the
//! sheet in closed form (the height is linear along the loft and
//! quadratic across it, so Simpson's rule is exact), and for a smooth
//! loft through three sections `vol(below) + vol(above) = vol(block)` and
//! `area(S ∩ A) + area(S ∖ A) = area(S)`; at 1e-3, 1 and 1e6.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::*;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::Role;
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::profile::{Curve2, Loop2, SketchCurve, Spline2, open_chain};
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_topo::{BodyKind, ShellId, boundary_loops, check, mass_properties};

type P = [f64; 2];

fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

fn spline(id: u64, degree: usize, control: &[P]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree,
            knots: knots(degree, control.len()),
            control: control.to_vec(),
        }),
    }
}

fn line(id: u64, start: P, end: P) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// The plane at `x` whose sketch axes are the world's y and z (a `yz`
/// sketch at an offset).
fn across(x: f64) -> Frame {
    Frame::new(Vec3::new(x, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap()
}

fn chain(curves: &[SketchCurve], k: f64) -> Loop2 {
    open_chain(curves, &tolerance(k)).unwrap()
}

fn open_section(curves: &[SketchCurve], x: f64, k: f64) -> Section {
    Section {
        plane: across(x),
        outline: chain(curves, k),
        start: None,
    }
}

/// The quadratic Bézier `a, b, c` over `y` from `−w` to `w` (control points
/// evenly spaced in y, so y is linear in its parameter).
fn bezier(t: f64, [a, b, c]: [f64; 3]) -> f64 {
    a * (1.0 - t) * (1.0 - t) + 2.0 * b * t * (1.0 - t) + c * t * t
}

fn heights(id: u64, w: f64, z: [f64; 3], k: f64) -> SketchCurve {
    spline(
        id,
        2,
        &[[-w * k, z[0] * k], [0.0, z[1] * k], [w * k, z[2] * k]],
    )
}

#[test]
fn a_sketch_of_curves_joined_end_to_end_is_one_open_chain() {
    let k = 1.0;
    // One spline: from its own start.
    let one = chain(
        &[spline(
            4,
            3,
            &[[0.0, 0.0], [1.0, 2.0], [3.0, 1.0], [4.0, 0.0]],
        )],
        k,
    );
    assert_eq!(one.segments.len(), 1);
    assert_eq!(one.segments[0].start, [0.0, 0.0]);
    assert!(!one.segments[0].reversed);
    // Lines given in any order and direction: from the free end of the end
    // curve with the smaller id, along the chain.
    let three = chain(
        &[
            line(9, [2.0, 0.0], [3.0, 1.0]),
            line(7, [1.0, 0.0], [0.0, 0.0]),
            line(8, [2.0, 0.0], [1.0, 0.0]),
        ],
        k,
    );
    let sources: Vec<u64> = three.segments.iter().map(|s| s.source).collect();
    assert_eq!(sources, [7, 8, 9]);
    assert_eq!(three.segments[0].start, [0.0, 0.0]);
    assert_eq!(three.segments[2].end, [3.0, 1.0]);
    for pair in three.segments.windows(2) {
        assert_eq!(pair[0].end, pair[1].start);
    }
    // A polyline is one chain of its pieces.
    let poly = chain(&[spline(3, 1, &[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]])], k);
    assert_eq!(poly.segments.len(), 2);
}

#[test]
fn what_is_not_one_open_chain_is_refused() {
    let tol = tolerance(1.0);
    let refused = |curves: &[SketchCurve]| open_chain(curves, &tol).unwrap_err();
    // A loop.
    let square = [
        line(1, [0.0, 0.0], [1.0, 0.0]),
        line(2, [1.0, 0.0], [1.0, 1.0]),
        line(3, [1.0, 1.0], [0.0, 1.0]),
        line(4, [0.0, 1.0], [0.0, 0.0]),
    ];
    assert!(refused(&square).to_string().contains("loop"));
    // Two chains.
    let apart = [
        line(1, [0.0, 0.0], [1.0, 0.0]),
        line(2, [0.0, 1.0], [1.0, 1.0]),
    ];
    assert!(refused(&apart).to_string().contains("more than one chain"));
    // A chain and a loop.
    let mut both = square.to_vec();
    both.push(line(5, [3.0, 0.0], [4.0, 0.0]));
    assert!(matches!(refused(&both), KernelError::InvalidProfile { .. }));
    // A branch.
    let branch = [
        line(1, [0.0, 0.0], [1.0, 0.0]),
        line(2, [1.0, 0.0], [2.0, 0.0]),
        line(3, [1.0, 0.0], [1.0, 1.0]),
    ];
    assert!(matches!(
        refused(&branch),
        KernelError::BranchingProfile { .. }
    ));
    // Curves that cross.
    let crossing = [
        line(1, [0.0, 0.0], [2.0, 2.0]),
        line(2, [2.0, 2.0], [2.0, 0.0]),
        line(3, [2.0, 0.0], [0.0, 2.0]),
    ];
    assert!(matches!(
        refused(&crossing),
        KernelError::CrossingCurves { .. }
    ));
    // A circle, a spline that closes, nothing.
    let circle = SketchCurve {
        id: 1,
        curve: Curve2::Circle {
            center: [0.0, 0.0],
            radius: 1.0,
        },
    };
    assert!(refused(&[circle]).to_string().contains("circle"));
    let closing = spline(1, 3, &[[0.0, 0.0], [1.0, 2.0], [2.0, 1.0], [0.0, 0.0]]);
    assert!(refused(&[closing]).to_string().contains("closes"));
    assert!(matches!(refused(&[]), KernelError::NoProfile));
}

#[test]
fn a_loft_of_open_polylines_is_a_sheet_of_flat_faces() {
    for k in SCALES {
        let tol = tolerance(k);
        // Two lines up a step, the same at x = 0 and x = 3: two flat faces,
        // 3 × 2 and 3 × √2.
        let step = |id: u64| {
            vec![
                line(id, [0.0, 0.0], [2.0 * k, 0.0]),
                line(id + 1, [2.0 * k, 0.0], [3.0 * k, k]),
            ]
        };
        let sections = [
            open_section(&step(1), 0.0, k),
            open_section(&step(5), 3.0 * k, k),
        ];
        let made = loft(&sections, &Options::default(), &tol).unwrap();
        let body = &made.body;
        assert_eq!(body.kind, BodyKind::Sheet);
        assert_eq!(check(body, &tol), []);
        assert_eq!(body.faces.len(), 2);
        assert!(made.faces.iter().all(|o| o.role == Role::Side));
        assert_eq!(made.faces[0].sources, [1, 5]);
        assert_eq!(boundary_loops(body, ShellId(0)), Some(1));
        let area = mass_properties(body).unwrap().area;
        let expected = 3.0 * k * k * (2.0 + 2.0_f64.sqrt());
        assert_close(area, expected, &format!("area at {k}"));
    }
}

/// The block `[x0, x1] × [y0, y1] × [zb, zt]` times `k`.
const BLOCK: ([f64; 3], [f64; 3]) = ([5.0, -30.0, -5.0], [35.0, 40.0, 40.0]);
const HALF_WIDTH: f64 = 50.0;
const LENGTH: f64 = 40.0;
const FIRST: [f64; 3] = [10.0, 30.0, 12.0];
const SECOND: [f64; 3] = [14.0, 2.0, 10.0];

/// The volume of the block under the ruled sheet through `FIRST` at x = 0
/// and `SECOND` at x = `LENGTH`, at scale 1.
fn volume_under() -> f64 {
    let ([x0, y0, zb], [x1, y1, _]) = BLOCK;
    let s = (x0 + x1) / (2.0 * LENGTH);
    let height = |y: f64| {
        let t = (y + HALF_WIDTH) / (2.0 * HALF_WIDTH);
        (1.0 - s) * bezier(t, FIRST) + s * bezier(t, SECOND) - zb
    };
    let ym = (y0 + y1) / 2.0;
    (x1 - x0) * (y1 - y0) / 6.0 * (height(y0) + 4.0 * height(ym) + height(y1))
}

#[test]
fn a_block_cut_by_a_ruled_terrain_sheet_keeps_the_volume_under_it() {
    for k in SCALES {
        let tol = tolerance(k);
        let sections = [
            open_section(&[heights(1, HALF_WIDTH, FIRST, k)], 0.0, k),
            open_section(&[heights(1, HALF_WIDTH, SECOND, k)], LENGTH * k, k),
        ];
        let options = Options {
            ruled: true,
            closed: false,
        };
        let sheet = loft(&sections, &options, &tol).unwrap().body;
        assert_eq!(sheet.kind, BodyKind::Sheet);
        assert_eq!(check(&sheet, &tol), []);
        // The chain runs along +y, the loft along +x: the sheet faces
        // y × x = −z, so a subtract keeps what is under it.
        let block = boxed(k, BLOCK.0, BLOCK.1);
        let below = solve(&block, &sheet, BooleanOp::Subtract, k);
        let above = solve(&block, &sheet, BooleanOp::Intersect, k);
        let (vb, va) = (mass(&below.body).volume, mass(&above.body).volume);
        let k3 = k * k * k;
        assert_close(vb / k3, volume_under(), &format!("under the sheet at {k}"));
        let ([x0, y0, z0], [x1, y1, z1]) = BLOCK;
        let whole = (x1 - x0) * (y1 - y0) * (z1 - z0);
        assert_close(va / k3, whole - volume_under(), &format!("over it at {k}"));
        assert!(mass(&below.body).centroid.unwrap().z < mass(&above.body).centroid.unwrap().z);
    }
}

#[test]
fn a_block_cut_by_a_smooth_terrain_sheet_is_split_without_loss() {
    for k in SCALES {
        let tol = tolerance(k);
        let sections = [
            open_section(&[heights(1, HALF_WIDTH, FIRST, k)], 0.0, k),
            open_section(&[heights(2, HALF_WIDTH, [8.0, 25.0, 20.0], k)], 20.0 * k, k),
            open_section(&[heights(3, HALF_WIDTH, SECOND, k)], LENGTH * k, k),
        ];
        let sheet = loft(&sections, &Options::default(), &tol).unwrap().body;
        assert_eq!(check(&sheet, &tol), []);
        let block = boxed(k, BLOCK.0, BLOCK.1);
        let below = solve(&block, &sheet, BooleanOp::Subtract, k);
        let above = solve(&block, &sheet, BooleanOp::Intersect, k);
        let whole = mass(&block).volume;
        let sum = mass(&below.body).volume + mass(&above.body).volume;
        assert_close(sum, whole, &format!("both sides at {k}"));
        assert!(mass(&below.body).centroid.unwrap().z < mass(&above.body).centroid.unwrap().z);
        // The sheet trimmed by the block, inside and outside.
        let inside = boolean(&sheet, &block, BooleanOp::Intersect, &tol).unwrap();
        let outside = boolean(&sheet, &block, BooleanOp::Subtract, &tol).unwrap();
        let area = |b: &craft3d_topo::Body| mass_properties(b).unwrap().area;
        assert_close(
            area(&inside.body) + area(&outside.body),
            area(&sheet),
            &format!("the sheet's area at {k}"),
        );
    }
}

#[test]
fn a_later_section_is_walked_the_way_the_one_before_it_is() {
    let k = 1.0;
    let tol = tolerance(k);
    // The second section's spline is drawn from the other side: it is
    // walked backwards, so the sheet does not fold.
    let reversed: Vec<P> = [[-50.0, 14.0], [0.0, 2.0], [50.0, 10.0]]
        .iter()
        .rev()
        .copied()
        .collect();
    let sections = [
        open_section(&[heights(1, HALF_WIDTH, FIRST, k)], 0.0, k),
        open_section(&[spline(1, 2, &reversed)], LENGTH, k),
    ];
    let options = Options {
        ruled: true,
        closed: false,
    };
    let sheet = loft(&sections, &options, &tol).unwrap().body;
    let block = boxed(k, BLOCK.0, BLOCK.1);
    let below = solve(&block, &sheet, BooleanOp::Subtract, k);
    assert_close(mass(&below.body).volume, volume_under(), "drawn backwards");
}

/// Which side a sheet faces is read off the sections' sketches (down in
/// each sketch, against its y axis), never off the order of the
/// sections, the ids of their curves or the way a chain was drawn
/// (2026-10-21, the garabit-findings review): the same block keeps the
/// volume under the sheet every time, at three scales and turned off
/// every axis.
#[test]
fn the_side_a_sheet_faces_follows_the_sketches_not_ids_order_or_drawing() {
    let backwards = |z: [f64; 3], id: u64, k: f64| {
        spline(
            id,
            2,
            &[
                [HALF_WIDTH * k, z[2] * k],
                [0.0, z[1] * k],
                [-HALF_WIDTH * k, z[0] * k],
            ],
        )
    };
    // The first and the second section as curves, for each variant.
    type Variant = (&'static str, bool, [SketchCurve; 1], [SketchCurve; 1]);
    let variants = |k: f64| -> Vec<Variant> {
        vec![
            (
                "as drawn",
                false,
                [heights(1, HALF_WIDTH, FIRST, k)],
                [heights(2, HALF_WIDTH, SECOND, k)],
            ),
            (
                "in reverse order",
                true,
                [heights(1, HALF_WIDTH, FIRST, k)],
                [heights(2, HALF_WIDTH, SECOND, k)],
            ),
            (
                "the first chain drawn backwards",
                false,
                [backwards(FIRST, 1, k)],
                [heights(2, HALF_WIDTH, SECOND, k)],
            ),
            (
                "both drawn backwards, in reverse order",
                true,
                [backwards(FIRST, 1, k)],
                [backwards(SECOND, 2, k)],
            ),
            (
                "ids swapped",
                false,
                [heights(9, HALF_WIDTH, FIRST, k)],
                [heights(3, HALF_WIDTH, SECOND, k)],
            ),
        ]
    };
    let turns = [
        None,
        Transform::rotation(Vec3::new(1.0, -2.0, 0.7), 0.83),
        Transform::rotation(Vec3::new(0.3, 1.0, -0.4), 2.9),
    ];
    for k in SCALES {
        let tol = tolerance(k);
        let block = boxed(k, BLOCK.0, BLOCK.1);
        for turn in &turns {
            let place = |frame: Frame| match turn {
                Some(t) => t.frame(&frame).unwrap(),
                None => frame,
            };
            let block = match turn {
                Some(t) => block.transformed(t).unwrap(),
                None => block.clone(),
            };
            for (what, reverse, first, second) in variants(k) {
                let mut sections = vec![
                    Section {
                        plane: place(across(0.0)),
                        outline: chain(&first, k),
                        start: None,
                    },
                    Section {
                        plane: place(across(LENGTH * k)),
                        outline: chain(&second, k),
                        start: None,
                    },
                ];
                if reverse {
                    sections.reverse();
                }
                let options = Options {
                    ruled: true,
                    closed: false,
                };
                let sheet = loft(&sections, &options, &tol).unwrap().body;
                let below = solve(&block, &sheet, BooleanOp::Subtract, k);
                let k3 = k * k * k;
                let volume = mass(&below.body).volume / k3;
                let what = format!("{what} at {k}, turned {}", turn.is_some());
                if turn.is_some() {
                    // Turned, the faces are exact only to the tolerance:
                    // within it times the block's area (the other side
                    // is 51 549 away).
                    let slack = tol.linear / k * mass(&block).area / (k * k);
                    assert!(
                        (volume - volume_under()).abs() <= slack,
                        "{what}: {volume} is not {}",
                        volume_under()
                    );
                } else {
                    assert_close(volume, volume_under(), &what);
                }
            }
        }
    }
}

/// Sections whose sketches disagree on which way is down are refused, and
/// a `start` on the first section settles it.
#[test]
fn sections_whose_sketches_disagree_on_down_are_refused() {
    let k = 1.0;
    let tol = tolerance(k);
    // The second sketch upside down: its normal −x and its x axis the
    // world's y, so its y axis is the world's −z (heights drawn negated
    // put the same curve in the same place).
    let upside_down = Frame::new(Vec3::new(LENGTH, 0.0, 0.0), -Vec3::X, Vec3::Y).unwrap();
    assert!(upside_down.y().dot(Vec3::Z) < -0.999);
    let flipped: Vec<P> = [[-50.0, -14.0], [0.0, -2.0], [50.0, -10.0]].to_vec();
    let sections = [
        open_section(&[heights(1, HALF_WIDTH, FIRST, k)], 0.0, k),
        Section {
            plane: upside_down,
            outline: chain(&[spline(2, 2, &flipped)], k),
            start: None,
        },
    ];
    let options = Options {
        ruled: true,
        closed: false,
    };
    let refused = loft(&sections, &options, &tol).unwrap_err();
    assert!(refused.to_string().contains("disagree"), "{refused}");
    let mut started = sections.clone();
    started[0].start = Some(1);
    assert!(loft(&started, &options, &tol).is_ok());
}

/// The section at angle `theta` round the world's y axis, `radius` from
/// it: its plane square to the bend (normal along the bend, `x` axis the
/// world's y, so its `y` axis points at the axis and down in the sketch
/// is out from it), its chain a line across the sketch from `−half` to
/// `half`.
fn round_the_bend(theta: f64, radius: f64, half: f64) -> Section {
    let (sin, cos) = theta.psin_cos();
    let plane = Frame::new(
        Vec3::new(radius * cos, 0.0, radius * sin),
        Vec3::new(-sin, 0.0, cos),
        Vec3::Y,
    )
    .unwrap();
    Section {
        plane,
        outline: chain(&[line(1, [-half, 0.0], [half, 0.0])], radius / 20.0),
        start: None,
    }
}

/// Every face of `sheet` faces out from the world's y axis (sampled at
/// its vertices' middle for a plane, on a grid of its domain otherwise).
fn faces_out_from_the_y_axis(sheet: &craft3d_topo::Body, what: &str) {
    use craft3d_geom::Surface;
    for (index, face) in sheet.faces.iter().enumerate() {
        let points: Vec<(f64, f64)> = match &face.surface {
            Surface::Plane(_) => {
                let mut sum = Vec3::ZERO;
                let mut count = 0.0;
                for l in &face.loops {
                    for c in &sheet.loops[l.0 as usize].coedges {
                        let edge = &sheet.edges[c.edge.0 as usize];
                        sum += sheet.vertices[edge.start.0 as usize].point;
                        count += 1.0;
                    }
                }
                vec![face.surface.project(sum * (1.0 / count)).unwrap()]
            }
            other => {
                let ((u0, u1), (v0, v1)) = other.domain();
                (1..5)
                    .flat_map(|i| {
                        (1..5).map(move |j| {
                            (
                                u0 + (u1 - u0) * i as f64 / 5.0,
                                v0 + (v1 - v0) * j as f64 / 5.0,
                            )
                        })
                    })
                    .collect()
            }
        };
        for (u, v) in points {
            let at = face.surface.eval(u, v);
            let normal = face.surface.normal(u, v).unwrap();
            let normal = if face.reversed { -normal } else { normal };
            let out = Vec3::new(at.x, 0.0, at.z).normalized().unwrap();
            assert!(
                normal.dot(out) > 0.9,
                "{what}: face {index} faces {normal:?} at {at:?}"
            );
        }
    }
}

/// Open sections whose sketches turn with the loft, round a bend by a
/// right angle or more (the m9-r1 review, 2026-10-22: refused before as
/// disagreeing, since the first and the last section's downs are that far
/// apart): the frames are followed from section to section, so the sheet
/// is built and faces down in every section's sketch (out from the bend's
/// axis), in either order of the sections, ruled to the closed form of
/// its flat strips and smooth within a hundredth of the cylinder it
/// stands for; at 1e-3, 1 and 1e6.
#[test]
fn sections_whose_frames_turn_round_a_bend_build_facing_down_in_each_sketch() {
    let (radius, half) = (20.0, 15.0);
    for k in SCALES {
        let tol = tolerance(k);
        for (span, count) in [(120.0_f64, 7), (180.0, 13), (270.0, 19)] {
            let span = span.to_radians();
            let step = span / (count - 1) as f64;
            for reverse in [false, true] {
                let mut sections: Vec<Section> = (0..count)
                    .map(|i| round_the_bend(i as f64 * step, radius * k, half * k))
                    .collect();
                if reverse {
                    sections.reverse();
                }
                let what = format!("{} degrees at {k}, reversed {reverse}", span.to_degrees());
                let ruled = Options {
                    ruled: true,
                    closed: false,
                };
                let sheet = loft(&sections, &ruled, &tol)
                    .unwrap_or_else(|e| panic!("{what}: {e}"))
                    .body;
                assert_eq!(sheet.kind, BodyKind::Sheet);
                assert_eq!(check(&sheet, &tol), [], "{what}");
                // Flat strips 2 half wide, each across a chord of the bend.
                let strips = (count - 1) as f64 * 2.0 * half * 2.0 * radius * (step / 2.0).psin();
                assert_close(
                    mass_properties(&sheet).unwrap().area,
                    strips * k * k,
                    &format!("ruled area, {what}"),
                );
                faces_out_from_the_y_axis(&sheet, &format!("ruled, {what}"));
                let smooth = loft(&sections, &Options::default(), &tol)
                    .unwrap_or_else(|e| panic!("smooth, {what}: {e}"))
                    .body;
                assert_eq!(check(&smooth, &tol), [], "smooth, {what}");
                let cylinder = 2.0 * half * radius * span * k * k;
                let area = mass_properties(&smooth).unwrap().area;
                assert!(
                    (area - cylinder).abs() <= 0.01 * cylinder,
                    "smooth, {what}: {area} against {cylinder}"
                );
                faces_out_from_the_y_axis(&smooth, &format!("smooth, {what}"));
            }
        }
    }
}

/// Neighbouring sections whose downs are a right angle or more apart are
/// still refused, naming both, even when the frames turn slowly elsewhere.
#[test]
fn neighbours_turned_the_other_way_in_a_bend_are_refused() {
    let tol = tolerance(1.0);
    let step = 15.0_f64.to_radians();
    let mut sections: Vec<Section> = (0..9)
        .map(|i| round_the_bend(i as f64 * step, 20.0, 15.0))
        .collect();
    // Section 6 drawn upside down: the frame turned about its normal by a
    // half turn and the line drawn the other way puts the same line in
    // the same place, with its y axis away from the axis.
    let sixth = &sections[5];
    let normal = sixth.plane.z();
    sections[5] = Section {
        plane: Frame::new(sixth.plane.origin(), normal, -Vec3::Y).unwrap(),
        outline: chain(&[line(1, [15.0, 0.0], [-15.0, 0.0])], 1.0),
        start: None,
    };
    let refused = loft(&sections, &Options::default(), &tol).unwrap_err();
    let text = refused.to_string();
    assert!(text.contains("sections 5 and 6 disagree"), "{text}");
}

#[test]
fn open_and_closed_sections_are_not_mixed() {
    let k = 1.0;
    let tol = tolerance(k);
    let square = [
        line(1, [0.0, 0.0], [1.0, 0.0]),
        line(2, [1.0, 0.0], [1.0, 1.0]),
        line(3, [1.0, 1.0], [0.0, 1.0]),
        line(4, [0.0, 1.0], [0.0, 0.0]),
    ];
    let closed = Section {
        plane: across(5.0),
        outline: craft3d_kernel::profile::detect_profiles(&square, &tol).unwrap()[0]
            .outer
            .clone(),
        start: None,
    };
    let open = open_section(&[heights(1, HALF_WIDTH, FIRST, k)], 0.0, k);
    let mixed = loft(&[open.clone(), closed], &Options::default(), &tol).unwrap_err();
    assert!(mixed.to_string().contains("all closed loops"), "{mixed}");
    let ring = Options {
        ruled: false,
        closed: true,
    };
    let three = [
        open.clone(),
        open_section(&[heights(1, HALF_WIDTH, SECOND, k)], 10.0, k),
        open_section(&[heights(1, HALF_WIDTH, FIRST, k)], 20.0, k),
    ];
    assert!(loft(&three, &ring, &tol).is_err());
    // `start` names a curve at an end of the chain.
    let middle = Section {
        start: Some(8),
        ..open_section(
            &[
                line(7, [0.0, 0.0], [1.0, 0.0]),
                line(8, [1.0, 0.0], [2.0, 1.0]),
                line(9, [2.0, 1.0], [3.0, 1.0]),
            ],
            10.0,
            k,
        )
    };
    let named = loft(&[open, middle], &Options::default(), &tol).unwrap_err();
    assert!(named.to_string().contains("not at an end"), "{named}");
}

#[test]
fn a_loft_through_the_most_sections_is_made_and_one_more_is_refused() {
    use craft3d_kernel::loft::MAX_SECTIONS;
    let k = 1.0;
    let tol = tolerance(k);
    // A wave of `count` sections, each one spline across.
    let wave = |count: usize| -> Vec<Section> {
        (0..count)
            .map(|i| {
                let z = (i as f64 * 0.3).psin();
                open_section(
                    &[heights(1, HALF_WIDTH, [z, 2.0 + z, 1.0 - z], k)],
                    i as f64,
                    k,
                )
            })
            .collect()
    };
    assert_eq!(MAX_SECTIONS, 256);
    let made = loft(&wave(MAX_SECTIONS), &Options::default(), &tol).unwrap();
    assert_eq!(check(&made.body, &tol), []);
    assert_eq!(made.faces[0].sources.len(), MAX_SECTIONS);
    let refused = loft(&wave(MAX_SECTIONS + 1), &Options::default(), &tol).unwrap_err();
    assert!(refused.to_string().contains("at most 256"), "{refused}");
}
