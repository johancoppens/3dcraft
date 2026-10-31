//! Shells against closed forms (docs/testing.md, oracle 1): every result
//! passes the checker and has the volume, area and counts that follow
//! from the thickness, at three scales.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::{SCALES, assert_close, boxed, mass, rod, solve, tolerance};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp;
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::shell::{Shelled, Skin, shell};
use craft3d_math::{Transform, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

/// The face on a surface of `kind` nearest `point` (for scale 1).
fn face_at(body: &Body, k: f64, kind: &str, point: [f64; 3]) -> FaceId {
    let target = Vec3::new(point[0] * k, point[1] * k, point[2] * k);
    let (index, _) = body
        .faces
        .iter()
        .enumerate()
        .filter(|(_, face)| face.surface.kind() == kind)
        .map(|(i, face)| {
            let (u, v) = face.surface.project(target).unwrap();
            (i, face.surface.eval(u, v).distance(target))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    FaceId(index as u32)
}

fn edge_at(body: &Body, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = Vec3::new(middle[0] * k, middle[1] * k, middle[2] * k);
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(target),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(index as u32)
}

#[track_caller]
fn hollowed(body: &Body, thickness: f64, open: &[FaceId], k: f64) -> Shelled {
    let tol = tolerance(k);
    let made = match shell(body, thickness * k, open, &tol) {
        Ok(made) => made,
        Err(e) => panic!("shell at scale {k}: {e}"),
    };
    assert_eq!(check(&made.body, &tol), [], "at scale {k}");
    assert_eq!(made.faces.len(), made.body.faces.len());
    assert_eq!(made.edges.len(), made.body.edges.len());
    assert_eq!(made.vertices.len(), made.body.vertices.len());
    made
}

#[track_caller]
fn assert_mass(made: &Shelled, volume: f64, area: f64, k: f64) {
    let m = mass(&made.body);
    assert_close(m.volume, volume * k * k * k, "volume");
    assert_close(m.area, area * k * k, "area");
}

fn count(made: &Shelled, which: Skin) -> usize {
    made.faces.iter().filter(|(skin, _)| *skin == which).count()
}

fn refused(body: &Body, thickness: f64, open: &[FaceId], k: f64) -> KernelError {
    match shell(body, thickness * k, open, &tolerance(k)) {
        Ok(_) => panic!("a shell of {thickness} at scale {k} was made"),
        Err(e) => e,
    }
}

#[test]
fn an_open_box_and_a_closed_hollow_block() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 8.0, 6.0]);
        let top = face_at(&block, k, "plane", [5.0, 4.0, 6.0]);
        let made = hollowed(&block, 1.0, &[top], k);
        // The hollow is 8 x 6 x 5; the rim is what is left of the top.
        assert_mass(&made, 480.0 - 240.0, 376.0 - 48.0 + 48.0 + 140.0, k);
        assert_eq!(made.body.shells.len(), 1);
        assert_eq!(count(&made, Skin::Outer), 6);
        assert_eq!(count(&made, Skin::Inner), 5);
        assert!(!made.faces.contains(&(Skin::Inner, top)));

        let made = hollowed(&block, 1.0, &[], k);
        // A cavity of 8 x 6 x 4 inside, a shell of its own.
        assert_mass(&made, 480.0 - 192.0, 376.0 + 2.0 * (48.0 + 32.0 + 24.0), k);
        assert_eq!(made.body.shells.len(), 2);
        assert_eq!(count(&made, Skin::Inner), 6);
    }
}

#[test]
fn a_cup_and_a_tube() {
    for k in SCALES {
        let body = rod(k, [0.0, 0.0, 0.0], 3.0, 5.0);
        let lid = face_at(&body, k, "plane", [0.0, 0.0, 5.0]);
        let floor = face_at(&body, k, "plane", [0.0, 0.0, 0.0]);
        let made = hollowed(&body, 0.5, &[lid], k);
        let (r, i) = (3.0, 2.5);
        assert_mass(
            &made,
            PI * r * r * 5.0 - PI * i * i * 4.5,
            2.0 * PI * r * 5.0
                + PI * r * r
                + PI * (r * r - i * i)
                + 2.0 * PI * i * 4.5
                + PI * i * i,
            k,
        );
        let made = hollowed(&body, 0.5, &[lid, floor], k);
        assert_mass(
            &made,
            PI * (r * r - i * i) * 5.0,
            2.0 * PI * (r + i) * 5.0 + 2.0 * PI * (r * r - i * i),
            k,
        );
        assert_eq!(made.body.faces.len(), 4);
        // Half the radius is all the wall there is.
        match refused(&body, 3.0, &[lid], k) {
            KernelError::FaceOpTooFar { largest, .. } => {
                assert!(largest > 2.99 * k && largest < 3.0 * k, "{largest}");
            }
            other => panic!("{other}"),
        }
    }
}

#[test]
fn an_l_shaped_prism_open_at_one_end() {
    for k in SCALES {
        let a = boxed(k, [0.0, 0.0, 0.0], [10.0, 4.0, 6.0]);
        let b = boxed(k, [0.0, 0.0, 0.0], [4.0, 10.0, 6.0]);
        let ell = solve(&a, &b, BooleanOp::Union, k).body;
        assert_eq!(ell.faces.len(), 8);
        let top = face_at(&ell, k, "plane", [2.0, 2.0, 6.0]);
        let made = hollowed(&ell, 1.0, &[top], k);
        // The section of the L is 64; that of the hollow 8 x 2 + 2 x 6.
        let (outer, inner) = (64.0, 28.0);
        let outline = 2.0 * (10.0 + 10.0);
        let inner_outline = 2.0 * (8.0 + 8.0);
        assert_mass(
            &made,
            outer * 6.0 - inner * 5.0,
            outer + outline * 6.0 + (outer - inner) + inner + inner_outline * 5.0,
            k,
        );
        // Half the narrow width: the hollow would have no width.
        match refused(&ell, 2.5, &[top], k) {
            KernelError::FaceOpTooFar { largest, .. } => {
                assert!(largest > 1.99 * k && largest < 2.0 * k, "{largest}");
            }
            other => panic!("{other}"),
        }
    }
}

/// A block of 10 x 8 x 6 whose four upright edges are rounded with
/// radius 2.
fn rounded(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 8.0, 6.0]);
    let edges: Vec<FilletEdge> = [
        [0.0, 0.0, 3.0],
        [10.0, 0.0, 3.0],
        [0.0, 8.0, 3.0],
        [10.0, 8.0, 3.0],
    ]
    .into_iter()
    .map(|middle| FilletEdge {
        edge: edge_at(&block, k, middle),
        radius: 2.0 * k,
    })
    .collect();
    fillet(&block, &edges, &tolerance(k)).unwrap().body
}

/// The area of a w x h rectangle with corners rounded by `r`.
fn rounded_area(w: f64, h: f64, r: f64) -> f64 {
    w * h - (4.0 - PI) * r * r
}

fn rounded_outline(w: f64, h: f64, r: f64) -> f64 {
    2.0 * (w + h) - 8.0 * r + 2.0 * PI * r
}

#[test]
fn a_rounded_box_has_inner_rounds_of_the_radius_less_the_wall() {
    for k in SCALES {
        let body = rounded(k);
        let top = face_at(&body, k, "plane", [5.0, 4.0, 6.0]);
        // From the radius of the rounds on (2026-10-21), the inner rounds
        // are gone: the hollow has sharp upright edges, the rounds
        // collapsed into them (`shell::vanishing`).
        for t in [0.5, 1.0, 1.5, 2.0, 2.5] {
            let made = hollowed(&body, t, &[top], k);
            let r = (2.0 - t).max(0.0);
            let (outer, inner) = (
                rounded_area(10.0, 8.0, 2.0),
                rounded_area(10.0 - 2.0 * t, 8.0 - 2.0 * t, r),
            );
            let h = 6.0 - t;
            assert_mass(
                &made,
                outer * 6.0 - inner * h,
                2.0 * outer - inner
                    + rounded_outline(10.0, 8.0, 2.0) * 6.0
                    + inner
                    + rounded_outline(10.0 - 2.0 * t, 8.0 - 2.0 * t, r) * h,
                k,
            );
            // The inner rounds are cylinders of radius 2 - t, while there
            // are any.
            let radii: Vec<f64> = made
                .faces
                .iter()
                .enumerate()
                .filter(|(_, (skin, _))| *skin == Skin::Inner)
                .filter_map(|(i, _)| match &made.body.faces[i].surface {
                    craft3d_geom::Surface::Cylinder(c) => Some(c.radius()),
                    _ => None,
                })
                .collect();
            assert_eq!(radii.len(), if t < 2.0 { 4 } else { 0 }, "at {t}");
            for r in radii {
                assert_close(r, (2.0 - t) * k, "inner radius");
            }
            // The hollow: a floor, four walls and the rounds while there
            // are any; the skin: the bottom, four walls, four rounds and
            // the rim.
            assert_eq!(count(&made, Skin::Inner), if t < 2.0 { 9 } else { 5 });
            assert_eq!(count(&made, Skin::Outer), 10);
        }
        // Where the walls meet, nothing is left: refused with what fits.
        match refused(&body, 4.0, &[top], k) {
            KernelError::FaceOpTooFar { largest, faces, .. } => {
                assert!(largest > 3.99 * k && largest < 4.0 * k, "{largest}");
                assert!(!faces.is_empty());
            }
            other => panic!("{other}"),
        }
    }
}

/// The volume of a block `w × d × h` whose every edge is rounded with
/// radius `r`.
fn rounded_block_volume(w: f64, d: f64, h: f64, r: f64) -> f64 {
    let (a, b, c) = (w - 2.0 * r, d - 2.0 * r, h - 2.0 * r);
    a * b * c
        + 2.0 * r * (a * b + a * c + b * c)
        + PI * r * r * (a + b + c)
        + 4.0 / 3.0 * PI * r.ppowi(3)
}

/// A block of 10 x 8 x 6 whose twelve edges are rounded with radius 2,
/// the balls in its corners.
fn rounded_all_round(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 8.0, 6.0]);
    let mut middles = Vec::new();
    for (x, y) in [(0.0, 0.0), (10.0, 0.0), (0.0, 8.0), (10.0, 8.0)] {
        middles.push([x, y, 3.0]);
    }
    for z in [0.0, 6.0] {
        middles.extend([[5.0, 0.0, z], [5.0, 8.0, z], [0.0, 4.0, z], [10.0, 4.0, z]]);
    }
    let edges: Vec<FilletEdge> = middles
        .into_iter()
        .map(|middle| FilletEdge {
            edge: edge_at(&block, k, middle),
            radius: 2.0 * k,
        })
        .collect();
    let made = fillet(&block, &edges, &tolerance(k)).unwrap().body;
    assert_close(
        mass(&made).volume,
        rounded_block_volume(10.0, 8.0, 6.0, 2.0) * k * k * k,
        "the rounded block",
    );
    made
}

/// The classic enclosure (the developer's R1 blocker, 2026-10-21): every
/// edge rounded, then shelled as thick as the rounds or more. The hollow
/// is the block shrunk by the wall, its edges sharp: the rounds and the
/// balls in the corners vanish together. Below the radius they are
/// rounds and balls of the radius less the wall. At three scales, and
/// turned off every axis.
#[test]
fn a_block_rounded_all_round_shells_as_thick_as_its_rounds_and_more() {
    let turn = Transform::rotation(Vec3::new(0.4, -1.0, 0.7), 1.1).unwrap();
    for k in SCALES {
        let tol = tolerance(k);
        let upright = rounded_all_round(k);
        let turned = upright.transformed(&turn).unwrap();
        for (body, what) in [(&upright, "upright"), (&turned, "turned")] {
            for t in [1.0, 2.0, 2.5] {
                let made = match shell(body, t * k, &[], &tol) {
                    Ok(made) => made,
                    Err(e) => panic!("{what} at {t}, scale {k}: {e}"),
                };
                assert_eq!(check(&made.body, &tol), [], "{what} at {t}, scale {k}");
                assert_eq!(made.body.shells.len(), 2);
                let r = (2.0 - t).max(0.0);
                let expected = rounded_block_volume(10.0, 8.0, 6.0, 2.0)
                    - rounded_block_volume(10.0 - 2.0 * t, 8.0 - 2.0 * t, 6.0 - 2.0 * t, r);
                let m = mass(&made.body);
                // Exact faces, exactly placed upright; turned, within the
                // tolerance over the area.
                let slack = if what == "upright" {
                    1e-9 * expected * k * k * k
                } else {
                    tol.linear * m.area
                };
                let off = m.volume - expected * k * k * k;
                assert!(
                    off.abs() <= slack,
                    "{what} at {t}, scale {k}: off by {off:e}"
                );
                // The hollow: six faces once the rounds are gone.
                let inner = count(&made, Skin::Inner);
                assert_eq!(inner, if t < 2.0 { 26 } else { 6 }, "{what} at {t}");
            }
        }
    }
}

/// The block rounded all round, open at its top (2026-10-22; refused
/// before: the top rounds run along the open face). The top's rounds keep
/// their walls (offset to `r − t`, or collapsed from `t = r` on), and the
/// hollow is opened through the top by the prism on its face behind the
/// top: the lip is the round's wall, `t` thick all along, ending square
/// to the top where the top's rim was. Its volume in closed form: the
/// solid less the closed hollow (a block `2t` smaller rounded `r − t`)
/// less the slab of the hollow's ceiling, `(w − 2m)(d − 2m)` with
/// `m = max(r, t)`, `t` high. Upright exact to 1e-9, turned off every
/// axis to the tolerance over the area, at 1e-3, 1 and 1e6.
#[test]
fn a_block_rounded_all_round_and_open_at_its_top_keeps_a_lip_along_its_rounds() {
    let turn = Transform::rotation(Vec3::new(0.4, -1.0, 0.7), 1.1).unwrap();
    let (w, d, h, r): (f64, f64, f64, f64) = (10.0, 8.0, 6.0, 2.0);
    for k in SCALES {
        let tol = tolerance(k);
        let upright = rounded_all_round(k);
        let top = face_at(&upright, k, "plane", [5.0, 4.0, 6.0]);
        let turned = upright.transformed(&turn).unwrap();
        for (body, what) in [(&upright, "upright"), (&turned, "turned")] {
            for t in [0.5, 1.0, 2.0, 2.5] {
                let made = match shell(body, t * k, &[top], &tol) {
                    Ok(made) => made,
                    Err(e) => panic!("{what} at {t}, scale {k}: {e}"),
                };
                assert_eq!(check(&made.body, &tol), [], "{what} at {t}, scale {k}");
                assert_eq!(made.body.shells.len(), 1, "{what} at {t}: one skin, open");
                let m = r.max(t);
                let expected = rounded_block_volume(w, d, h, r)
                    - rounded_block_volume(w - 2.0 * t, d - 2.0 * t, h - 2.0 * t, (r - t).max(0.0))
                    - (w - 2.0 * m) * (d - 2.0 * m) * t;
                let mp = mass(&made.body);
                let slack = if what == "upright" {
                    1e-9 * expected * k * k * k
                } else {
                    tol.linear * mp.area
                };
                let off = mp.volume - expected * k * k * k;
                assert!(
                    off.abs() <= slack,
                    "{what} at {t}, scale {k}: off by {off:e}"
                );
                // The top is gone up to the thickness of the rounds; past
                // it a ring of it, `t − r` wide, is the lip's top. The
                // lip's ends, square to the top, are the hollow behind it:
                // four planes while the rounds keep a wall, none once the
                // walls run straight up (they are the hollow's walls
                // there).
                assert_eq!(
                    made.faces.contains(&(Skin::Outer, top)),
                    t > r,
                    "{what} at {t}"
                );
                let ends = made
                    .faces
                    .iter()
                    .enumerate()
                    .filter(|(_, source)| **source == (Skin::Inner, top))
                    .map(|(i, _)| &made.body.faces[i])
                    .collect::<Vec<_>>();
                assert_eq!(ends.len(), if t < r { 4 } else { 0 }, "{what} at {t}");
                assert!(ends.iter().all(|f| f.surface.kind() == "plane"));
            }
        }
    }
}

/// A slot (a stadium `2R` wide with straight sides `L` long, `H` high)
/// left open at one of its flat sides, which its half-cylinders are
/// tangent to (2026-10-22: refused before, since a tangent neighbour
/// cannot follow the face outward): the half-cylinders keep their walls
/// and the hollow is opened through the side by the prism on the hollow's
/// face behind it, a channel. Its volume: the slot less the closed hollow
/// (the slot `t` smaller all round) less the slab `L × (H − 2t) × t`.
#[test]
fn a_slot_open_at_a_side_its_ends_are_tangent_to() {
    use craft3d_kernel::extrude::extrude;
    use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
    use craft3d_math::Frame;
    let (l, radius, height) = (12.0, 3.0, 5.0);
    for k in SCALES {
        let tol = tolerance(k);
        let (half, r) = (l / 2.0 * k, radius * k);
        let curves = vec![
            SketchCurve {
                id: 1,
                curve: Curve2::Line {
                    start: [-half, -r],
                    end: [half, -r],
                },
            },
            SketchCurve {
                id: 2,
                curve: Curve2::Arc {
                    center: [half, 0.0],
                    start: [half, -r],
                    end: [half, r],
                },
            },
            SketchCurve {
                id: 3,
                curve: Curve2::Line {
                    start: [half, r],
                    end: [-half, r],
                },
            },
            SketchCurve {
                id: 4,
                curve: Curve2::Arc {
                    center: [-half, 0.0],
                    start: [-half, r],
                    end: [-half, -r],
                },
            },
        ];
        let profiles = detect_profiles(&curves, &tol).unwrap();
        let slot = extrude(&profiles, &Frame::WORLD, 0.0, height * k, &tol)
            .unwrap()
            .body;
        let side = face_at(&slot, k, "plane", [0.0, radius, height / 2.0]);
        for t in [0.5, 1.0] {
            let made = hollowed(&slot, t, &[side], k);
            assert_eq!(made.body.shells.len(), 1);
            let stadium = |w: f64| l * 2.0 * w + PI * w * w;
            let expected = stadium(radius) * height
                - stadium(radius - t) * (height - 2.0 * t)
                - l * (height - 2.0 * t) * t;
            let off = mass(&made.body).volume - expected * k * k * k;
            assert!(
                off.abs() <= 1e-9 * expected * k * k * k,
                "at {t}, scale {k}: off by {off:e}"
            );
        }
    }
}

/// A block whose top-front edge is rounded, shelled open at the top, the
/// round and the front together (the review of 2026-10-22: built before
/// the lip, then refused because the round counted as a blend along the
/// top although it is open too). Open faces tangent to each other all go
/// outward, as before: the cross-section is the block's less the part of
/// it the hollow takes, `y ≤ 7, z ≥ 1` with the corner rounded, along the
/// 8 between the side walls.
#[test]
fn a_round_left_open_with_the_faces_it_joins_goes_outward_with_them() {
    let corner = 4.0 * (1.0 - PI / 4.0);
    let expected = 10.0 * (48.0 - corner) - 8.0 * (35.0 - corner);
    for k in SCALES {
        let tol = tolerance(k);
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 8.0, 6.0]);
        let edge = edge_at(&block, k, [5.0, 0.0, 6.0]);
        let rounded = fillet(
            &block,
            &[FilletEdge {
                edge,
                radius: 2.0 * k,
            }],
            &tol,
        )
        .unwrap()
        .body;
        let top = face_at(&rounded, k, "plane", [5.0, 5.0, 6.0]);
        let front = face_at(&rounded, k, "plane", [5.0, 0.0, 2.0]);
        let round = face_at(&rounded, k, "cylinder", [5.0, 0.6, 5.4]);
        let made = hollowed(&rounded, 1.0, &[top, round, front], k);
        let off = mass(&made.body).volume - expected * k * k * k;
        assert!(
            off.abs() <= 1e-9 * expected * k * k * k,
            "at scale {k}: off by {off:e}"
        );
        assert!(
            (expected - 198.283).abs() < 1e-3,
            "the volume before the lip"
        );
    }
}

/// A prism whose cross-section is a trapezoid (its sides slanting in or
/// out, 4 over 10), its top-front edge rounded, shelled open at the top:
/// the opening square to the top would cut the slanted walls thinner over
/// the top `t` (0.63 for 1 when built so), so it is refused, naming the
/// top and the slanted side, at three scales.
#[test]
fn an_open_face_with_a_blend_along_it_and_a_slanted_neighbour_is_refused() {
    use craft3d_kernel::extrude::extrude;
    use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
    use craft3d_math::Frame;
    for k in SCALES {
        let tol = tolerance(k);
        for top_half in [6.0, 14.0] {
            let points = [
                [-10.0 * k, 0.0],
                [10.0 * k, 0.0],
                [top_half * k, 10.0 * k],
                [-top_half * k, 10.0 * k],
            ];
            let curves: Vec<SketchCurve> = (0..4)
                .map(|i| SketchCurve {
                    id: i as u64 + 1,
                    curve: Curve2::Line {
                        start: points[i],
                        end: points[(i + 1) % 4],
                    },
                })
                .collect();
            let profiles = detect_profiles(&curves, &tol).unwrap();
            // Sketch x along x, y along z; extruded along −y, 8 deep.
            let frame = Frame::new(Vec3::new(0.0, 0.0, 0.0), -Vec3::Y, Vec3::X).unwrap();
            let prism = extrude(&profiles, &frame, 0.0, 8.0 * k, &tol).unwrap().body;
            let edge = edge_at(&prism, k, [0.0, 0.0, 10.0]);
            let rounded = fillet(
                &prism,
                &[FilletEdge {
                    edge,
                    radius: 2.0 * k,
                }],
                &tol,
            )
            .unwrap_or_else(|e| panic!("{top_half} at {k}: {e}"))
            .body;
            let top = face_at(&rounded, k, "plane", [0.0, -5.0, 10.0]);
            match shell(&rounded, k, &[top], &tol) {
                Err(KernelError::FaceOp { faces, reason, .. }) => {
                    assert!(reason.contains("neither tangent"), "{reason}");
                    assert!(faces.contains(&top), "{faces:?}");
                }
                other => panic!("{top_half} at {k}: expected a refusal, got {other:?}"),
            }
        }
    }
}

/// A rod whose top rim is rounded (a torus), shelled open at its bottom
/// as thick as the round and more: the hollow is a plain rod.
#[test]
fn a_rod_with_a_rounded_rim_shells_past_its_round() {
    let (radius, height, r) = (5.0, 8.0, 1.5);
    // Pappus's rule over the corner a ball of radius ρ takes off a rim of
    // radius `big`.
    let taken = |big: f64, rho: f64| {
        let corner = rho * rho * (1.0 - PI / 4.0);
        let from_axis = big - rho + rho.ppowi(3) / (6.0 * corner);
        2.0 * PI * from_axis * corner
    };
    let outer = PI * radius * radius * height - taken(radius, r);
    for k in SCALES {
        let tol = tolerance(k);
        let plain = rod(k, [0.0, 0.0, 0.0], radius, height);
        let rim = (0..plain.edges.len())
            .map(|e| EdgeId(e as u32))
            .find(|e| {
                let edge = &plain.edges[e.0 as usize];
                let p = edge.curve.eval(edge.range.0);
                (p.z - height * k).abs() <= 1e-9 * k
                    && matches!(edge.curve, craft3d_geom::Curve3::Circle(_))
            })
            .unwrap();
        let body = fillet(
            &plain,
            &[FilletEdge {
                edge: rim,
                radius: r * k,
            }],
            &tol,
        )
        .unwrap()
        .body;
        assert_close(mass(&body).volume, outer * k * k * k, "the rounded rod");
        let bottom = face_at(&body, k, "plane", [0.0, 0.0, 0.0]);
        for t in [1.0, 1.5, 2.0] {
            let made = hollowed(&body, t, &[bottom], k);
            let mut hollow = PI * (radius - t).ppowi(2) * (height - t);
            if t < r {
                hollow -= taken(radius - t, r - t);
            }
            let m = mass(&made.body);
            let off = m.volume - (outer - hollow) * k * k * k;
            assert!(
                off.abs() <= 1e-9 * outer * k * k * k,
                "at {t}, scale {k}: off by {off:e}"
            );
            let tori = made
                .faces
                .iter()
                .enumerate()
                .filter(|(i, (skin, _))| {
                    *skin == Skin::Inner
                        && matches!(made.body.faces[*i].surface, craft3d_geom::Surface::Torus(_))
                })
                .count();
            assert_eq!(tori, usize::from(t < r), "at {t}");
        }
    }
}

#[test]
fn a_hole_through_the_open_side_and_a_boss_on_the_closed_one() {
    for k in SCALES {
        // A hole of radius 1 through a block of 10 x 10 x 6, open at the
        // top: a tube of wall 1 stands in the hollow round the hole.
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 6.0]);
        let drill = rod(k, [5.0, 5.0, -1.0], 1.0, 8.0);
        let drilled = solve(&block, &drill, BooleanOp::Subtract, k).body;
        let top = face_at(&drilled, k, "plane", [0.5, 0.5, 6.0]);
        let made = hollowed(&drilled, 1.0, &[top], k);
        assert_mass(
            &made,
            (100.0 - PI) * 6.0 - (64.0 - PI * 4.0) * 5.0,
            mass(&drilled).area / (k * k) - (100.0 - PI)
                + (36.0 + 3.0 * PI)
                + (64.0 - 4.0 * PI)
                + 32.0 * 5.0
                + 2.0 * PI * 2.0 * 5.0,
            k,
        );
        // A boss of radius 2 and height 3 on the top of a block of
        // 10 x 10 x 5, open at the bottom: the boss is a cup upside down,
        // and the hollow is 4 high inside the block.
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
        let boss = rod(k, [5.0, 5.0, 5.0], 2.0, 3.0);
        let bossed = solve(&block, &boss, BooleanOp::Union, k).body;
        let bottom = face_at(&bossed, k, "plane", [5.0, 5.0, 0.0]);
        let made = hollowed(&bossed, 1.0, &[bottom], k);
        assert_mass(
            &made,
            500.0 + PI * 4.0 * 3.0 - (64.0 * 4.0 + PI * 3.0),
            mass(&bossed).area / (k * k) - 100.0
                + 36.0
                + (64.0 - PI)
                + 32.0 * 4.0
                + 2.0 * PI * 3.0
                + PI,
            k,
        );
    }
}

#[test]
fn what_is_not_a_shell_is_an_error() {
    let k = 1.0;
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 8.0, 6.0]);
    let tol = tolerance(k);
    for t in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            shell(&block, t, &[], &tol),
            Err(KernelError::InvalidSize { .. })
        ));
    }
    assert!(matches!(
        shell(&block, 1.0, &[FaceId(99)], &tol),
        Err(KernelError::FaceOp { .. })
    ));
    assert!(matches!(
        shell(&block, 1.0, &[FaceId(1), FaceId(1)], &tol),
        Err(KernelError::FaceOp { .. })
    ));
    let all: Vec<FaceId> = (0..6).map(FaceId).collect();
    assert!(matches!(
        shell(&block, 1.0, &all, &tol),
        Err(KernelError::FaceOp { .. })
    ));
    // Half the smallest size: no hollow is left.
    match shell(&block, 3.0, &[], &tol) {
        Err(KernelError::FaceOpTooFar { largest, .. }) => {
            assert!(largest > 2.99 && largest < 3.0, "{largest}");
        }
        other => panic!("{other:?}"),
    }
}
