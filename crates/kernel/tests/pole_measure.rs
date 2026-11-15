//! Faces that close in a point (a pole of their surface) whose meridians
//! leave the pole unevenly: the exact measure against closed forms, and
//! the mesh.
//!
//! The bodies are tips over prisms (`craft3d_testkit::tips`): with
//! quarter ellipses across, two thirds of base times length, with
//! parabolas eleven fifteenths, wherever the tip stands. Where it stands
//! near a corner of the section, or over a thin section, most meridians
//! of the faces beside it leave the pole within a hundredth of a radian
//! of each other, and the search for the foot of a point of a rail in
//! both parameters at once settled on another meridian: the measure came
//! out up to 3.4e-5 of the volume short over an L and 1.8e-3 over a
//! blade, inside a bound as wide (docs/design/topology.md › *A rail
//! beside its border*), and the tessellator refused the loop as one that
//! jumps across its surface (docs/design/tessellator.md › *The rails of a
//! squeezed pole*).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::tips::{PARABOLA, Piece, ROUND, ellipse, polygon, polygon_area, section_tip};
use craft3d_topo::{Body, BodyKind, Coedge, Defect, check, mass_bound};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The model's frame, and one turned and thirty sizes away.
fn frames(k: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(30.0 * k, -20.0 * k, 50.0 * k),
            Vec3::new(1.0, 2.0, 2.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap(),
    ]
}

/// A section's name, its corners, and how far from its first corner the
/// tip stands (along both axes).
type Shape = (&'static str, &'static [[f64; 2]], &'static [f64]);
/// The point of a section a share of the way out from its middle, in a
/// direction.
type Inside = Box<dyn Fn(f64, f64) -> [f64; 2]>;

const TRIANGLE: [[f64; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
const SQUARE: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
const L: [[f64; 2]; 6] = [
    [0.0, 0.0],
    [6.0, 0.0],
    [6.0, 2.0],
    [2.0, 2.0],
    [2.0, 6.0],
    [0.0, 6.0],
];

fn blade(thickness: f64) -> [[f64; 2]; 4] {
    let h = 0.5 * thickness;
    [[-0.5, -h], [0.5, -h], [0.5, h], [-0.5, h]]
}

/// The volume of `body` by the exact measure as a share of `want` off
/// it, and its bound as a share of `want`: the measure is right within
/// its own bound (and a rounding of the closed form), and the bound is
/// no wider than `within`.
#[track_caller]
fn measured(body: &Body, k: f64, want: f64, within: f64, what: &str) -> (f64, f64) {
    let tolerance = Tolerance::new(1e-7 * k, 1e-9).unwrap();
    assert_eq!(check(body, &tolerance), [], "{what}");
    let (mass, bound) = mass_bound(body).unwrap_or_else(|e| panic!("{what}: {e}"));
    let off = (mass.volume - want).abs();
    assert!(
        off <= bound.volume + 1e-13 * want,
        "{what}: the volume is {:e} of itself off, outside its bound of {:e}",
        off / want,
        bound.volume / want
    );
    assert!(
        bound.volume <= within * want,
        "{what}: a bound of {:e} of the volume (the volume is {:e} off)",
        bound.volume / want,
        off / want
    );
    (off / want, bound.volume / want)
}

#[test]
fn a_tip_near_a_corner_measures_as_its_closed_form() {
    let shapes: [Shape; 3] = [
        ("triangle", &TRIANGLE, &[0.01, 0.003]),
        ("square", &SQUARE, &[0.01, 0.003]),
        ("L", &L, &[0.03, 0.01, 0.003]),
    ];
    let (mut worst, mut widest) = (0.0_f64, 0.0_f64);
    for k in SCALES {
        for frame in frames(k) {
            for (name, corners, offs) in shapes {
                let area = polygon_area(corners);
                for &off in offs {
                    for (fullness, share) in [PARABOLA, ROUND] {
                        let what = format!(
                            "{name}, tip {off} from its corner, fullness {fullness:.3}, at {k}"
                        );
                        let (_, body) =
                            section_tip(&frame, k, &polygon(corners), [off, off], 0.5, fullness)
                                .unwrap_or_else(|e| panic!("{what}: {e}"));
                        let want = share * area * 0.5 * k * k * k;
                        let (off, bound) = measured(&body, k, want, 1e-10, &what);
                        worst = worst.max(off);
                        widest = widest.max(bound);
                    }
                }
            }
        }
    }
    println!("tips near a corner: within {worst:e} of the closed forms, bounds up to {widest:e}");
    assert!(worst <= 1e-12, "{worst:e}");
}

#[test]
fn a_tip_over_a_thin_blade_measures_as_its_closed_form() {
    let (fullness, share) = ROUND;
    let (mut worst, mut widest) = (0.0_f64, 0.0_f64);
    let mut cases = Vec::new();
    for frame in frames(1.0) {
        for thickness in [1e-3, 1e-4, 1e-5] {
            cases.push((1.0, frame, thickness));
        }
    }
    cases.push((1e-3, Frame::WORLD, 1e-4));
    cases.push((1e6, Frame::WORLD, 1e-4));
    for (k, frame, thickness) in cases {
        for length in [1e-3, 0.3] {
            let what = format!("a blade {thickness:e} thick, a tip {length} long, at {k}");
            let (_, body) = section_tip(
                &frame,
                k,
                &polygon(&blade(thickness)),
                [0.0, 0.0],
                length,
                fullness,
            )
            .unwrap_or_else(|e| panic!("{what}: {e}"));
            let want = share * thickness * length * k * k * k;
            // A face a hundred-thousandth as wide as it is long is known
            // to that many digits less than its coordinates are.
            let (off, bound) = measured(&body, k, want, 1e-4, &what);
            worst = worst.max(off);
            widest = widest.max(bound);
        }
    }
    println!("tips over blades: within {worst:e} of the closed forms, bounds up to {widest:e}");
    assert!(worst <= 1e-8, "{worst:e}");
}

/// A tip's name, its section's corners, the foot of its tip, its length
/// and its fullness with the share of base times length it fills.
type Continued = (&'static str, Vec<[f64; 2]>, [f64; 2], f64, (f64, f64));

/// The tip over `corners` with every face's surface continued past its
/// rails by the shares `by` of its range in `u` (the rails are then lines
/// of constant `u` inside their surfaces, as another modeller's trimmed
/// faces are), and the body's closed-form volume.
fn on_continued_surfaces(
    frame: &Frame,
    k: f64,
    corners: &[[f64; 2]],
    (foot, length, (fullness, share)): ([f64; 2], f64, (f64, f64)),
    by: (f64, f64),
) -> (Body, f64) {
    let (tip, mut body) = section_tip(frame, k, &polygon(corners), foot, length, fullness).unwrap();
    for (face, net) in body.faces.iter_mut().zip(&tip.faces) {
        let ((u0, u1), v) = net.domain();
        let grown = net
            .extended((u0 - by.0 * (u1 - u0), u1 + by.1 * (u1 - u0)), v, 1e-9 * k)
            .unwrap();
        face.surface = Surface::Nurbs(grown);
    }
    (body, share * polygon_area(corners) * length * k * k * k)
}

#[test]
fn a_tip_on_surfaces_that_run_on_past_its_rails_measures_as_its_closed_form() {
    // The rails are no borders: until 2026-11-15 the feet of their points
    // next to the pole were the search's, on other meridians, and a blade
    // 1e-3 thick with a tip 1e-3 long came out 2.4e-4 to 4.9e-4 of its
    // volume off, inside its bound (docs/design/topology.md › *A rail
    // beside its border*). Now such a rail is found to run along a line
    // of constant parameter, and its points are looked for along it.
    let shapes: [Continued; 3] = [
        (
            "the L, tip 0.01 from the corner",
            L.to_vec(),
            [0.01, 0.01],
            0.5,
            PARABOLA,
        ),
        (
            "the triangle, tip 0.003 from the corner",
            TRIANGLE.to_vec(),
            [0.003, 0.003],
            0.5,
            PARABOLA,
        ),
        (
            "a blade 1e-3 thick",
            blade(1e-3).to_vec(),
            [0.0, 0.0],
            1e-3,
            ROUND,
        ),
    ];
    let mut cases: Vec<(f64, Frame)> = SCALES.iter().map(|&k| (k, Frame::WORLD)).collect();
    cases.push((1.0, frames(1.0)[1]));
    let (mut worst, mut widest) = (0.0_f64, 0.0_f64);
    for (k, frame) in cases {
        for (name, corners, foot, length, fullness) in &shapes {
            for by in [(0.2, 0.2), (1e-3, 1e-3), (0.0, 0.3)] {
                let what = format!("{name}, its surfaces continued by {by:?}, at {k}");
                let (body, want) =
                    on_continued_surfaces(&frame, k, corners, (*foot, *length, *fullness), by);
                let (mass, bound) = mass_bound(&body).unwrap_or_else(|e| panic!("{what}: {e}"));
                let off = (mass.volume - want).abs();
                assert!(
                    off <= bound.volume + 1e-13 * want,
                    "{what}: the volume is {:e} of itself off, outside its bound of {:e}",
                    off / want,
                    bound.volume / want
                );
                worst = worst.max(off / want);
                widest = widest.max(bound.volume / want);
            }
        }
    }
    println!(
        "tips on continued surfaces: within {worst:e} of the closed forms, bounds up to {widest:e}"
    );
    assert!(worst <= 1e-9, "{worst:e}");
}

#[test]
fn the_check_is_known_to_find_a_blade_on_continued_surfaces_off_its_faces() {
    // KNOWN, owed (docs/design/topology.md › *A rail beside its border*,
    // found by the review of 2026-11-14, on the commit before as well):
    // `topo::check` finds the edges of a blade 1e-3 thick whose faces
    // run on past its rails 3.4e-5 to 2.4e-4 of the size off their faces
    // (`EdgeOffSurface`), and so calls 36 valid bodies of the review
    // invalid (these, and faces cut along a meridian). The body measures
    // as its closed form (above), so it is the check's own search for the
    // edges' feet that settles on another meridian, the measure's old
    // fault. When this fails because the check passes, the pin goes.
    let (body, _) = on_continued_surfaces(
        &Frame::WORLD,
        1.0,
        &blade(1e-3),
        ([0.0, 0.0], 1e-3, ROUND),
        (0.2, 0.2),
    );
    let defects = check(&body, &Tolerance::new(1e-7, 1e-9).unwrap());
    println!("the check of a blade on continued surfaces: {defects:?}");
    assert!(
        defects
            .iter()
            .any(|defect| matches!(defect, Defect::EdgeOffSurface { .. })),
        "the check passes now: take the pin off ({defects:?})"
    );
}

/// Meshed at a chordal tolerance of `chord`: watertight, with the
/// volume within the chord bound (area times tolerance) of `volume`, and
/// at the tip the normal of the tip's plane (the faces close tangent to
/// it). `Err` with the tessellator's reason where it refuses.
#[track_caller]
fn meshed(
    body: &Body,
    (volume, area): (f64, f64),
    chord: f64,
    (tip, normal, size): (Vec3, Vec3, f64),
    what: &str,
) -> Result<(), String> {
    let params = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).map_err(|e| e.to_string())?;
    let mut at_tip = 0;
    for (position, n) in mesh.positions.iter().zip(&mesh.normals) {
        if position.distance(tip) <= 1e-9 * size && n.length() > 0.0 {
            at_tip += 1;
            assert!(
                n.normalized().unwrap().dot(normal) >= 0.99,
                "{what}: a normal at the tip {:?} against {normal:?}",
                n
            );
        }
    }
    assert!(at_tip > 0, "{what}: no normal at the tip");
    let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert!(welded.is_closed(), "{what}: the mesh is not watertight");
    assert!(
        (welded.volume() - volume).abs() <= area * chord,
        "{what}: mesh volume {} against {volume}",
        welded.volume()
    );
    Ok(())
}

#[test]
fn faces_refused_as_the_search_placed_them_are_meshed_along_their_borders() {
    // Blunt tips (fullness 0.95) a hundredth and a thousandth of the L's
    // size from a corner or an edge of its kernel: at the finer tolerance
    // each was refused, "its boundary is not closed in parameter space:
    // between two neighbouring points of edge … it jumps across the
    // surface" (two) and "the boundary crosses itself in parameter space"
    // (one). The body is meshed as the search places its loops first, as
    // ever; only where a face is refused that way is it tried with the
    // loop placed along the borders (which costs the refusal first: the
    // other scales and the turned frame for one of the three only).
    let mut cases = vec![
        (1.0, Frame::WORLD, [1.99, 1.99]),
        (1.0, Frame::WORLD, [0.01, 0.01]),
        (1.0, Frame::WORLD, [1.999, 1.0]),
    ];
    cases.push((1.0, frames(1.0)[1], [1.99, 1.99]));
    for k in [1e-3, 1e6] {
        cases.extend(frames(k).map(|frame| (k, frame, [1.99, 1.99])));
    }
    for (k, frame, foot) in cases {
        let what = format!("the L with a blunt tip over {foot:?} at {k}");
        let (tip, body) = section_tip(&frame, k, &polygon(&L), foot, 0.5, 0.95)
            .unwrap_or_else(|e| panic!("{what}: {e}"));
        let (mass, _) = mass_bound(&body).unwrap_or_else(|e| panic!("{what}: {e}"));
        for chord in [2e-2, 2e-3] {
            meshed(
                &body,
                (mass.volume, mass.area),
                chord * 12.0 * k,
                (tip.tip, frame.z(), 12.0 * k),
                &what,
            )
            .unwrap_or_else(|e| panic!("{what}, at {chord}: {e}"));
        }
    }
}

#[test]
fn the_normal_at_the_tip_of_a_very_pointed_end_is_known_to_be_off() {
    // KNOWN, owed (docs/design/tessellator.md › *The rails of a squeezed
    // pole*, found by the review of 2026-11-14 on the commit before as
    // well): at a fullness of 0.001 the mesh is watertight and within its
    // chord bound, and the normals it gives at the tip are not the tip
    // plane's (0.858 against it on a blade 1e-3 thick with a tip 0.3
    // long; down to 0.06 on thinner blades). The cause is not looked
    // into (the face turns into the tip's plane within a thousandth of
    // its length there, and the normal at a pole is read a little way
    // off it). When this fails because the normal is right, the pin
    // goes.
    // And at a fullness of 0.01 on a blade 1e-4 thick (0.860).
    let frame = Frame::WORLD;
    for (thickness, fullness) in [(1e-3, 0.001), (1e-4, 0.01)] {
        let (tip, body) = section_tip(
            &frame,
            1.0,
            &polygon(&blade(thickness)),
            [0.0, 0.0],
            0.3,
            fullness,
        )
        .unwrap();
        let params = TessParams {
            chordal_tolerance: 0.02 * 0.5,
            ..TessParams::default()
        };
        let mesh = tessellate_body(&body, &params).unwrap();
        let least = mesh
            .positions
            .iter()
            .zip(&mesh.normals)
            .filter(|(position, n)| position.distance(tip.tip) <= 1e-9 && n.length() > 0.0)
            .map(|(_, n)| n.normalized().unwrap().dot(frame.z()))
            .fold(1.0_f64, f64::min);
        println!(
            "the least normal at the tip of a blade {thickness:e} thick at fullness {fullness} against its plane: {least:.4}"
        );
        assert!(mesh.weld().unwrap().is_closed());
        assert!(least < 0.99, "right now, take the pin off: {least}");
    }
}

/// A dome over the ellipse with the half axes `a` and `b`, closed across
/// a seam (on `+x`, or `−x` with `flip`) in a tip `length` over `foot`:
/// one rational face of degree 2 × 2 whose border `v = 1` is the tip, the
/// seam an edge used twice, and a flat base.
fn seamed_dome(frame: &Frame, (a, b): (f64, f64), flip: bool, foot: [f64; 2], length: f64) -> Body {
    let s = if flip { -1.0 } else { 1.0 };
    let ring = [
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (-1.0, 1.0),
        (-1.0, 0.0),
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (1.0, 0.0),
    ];
    let top = frame.point(foot[0], foot[1], length);
    let (mut net, mut weights, mut rim, mut rim_weights) = (vec![], vec![], vec![], vec![]);
    for (i, &(x, y)) in ring.iter().enumerate() {
        let w = if i % 2 == 1 {
            std::f64::consts::FRAC_1_SQRT_2
        } else {
            1.0
        };
        let low = frame.point(s * a * x, s * b * y, 0.0);
        net.extend([low, frame.point(s * a * x, s * b * y, length), top]);
        weights.extend([w; 3]);
        rim.push(low);
        rim_weights.push(w);
    }
    let knots = vec![
        0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
    ];
    let ends = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let surface = NurbsSurface::new(
        2,
        2,
        knots.clone(),
        ends.clone(),
        9,
        3,
        net.clone(),
        weights,
    )
    .unwrap();
    let rim = NurbsCurve::new(2, knots, rim, rim_weights).unwrap();
    let seam = NurbsCurve::new(2, ends, net[..3].to_vec(), vec![1.0; 3]).unwrap();
    let (forward, backward) = (
        |edge| Coedge {
            edge,
            reversed: false,
        },
        |edge| Coedge {
            edge,
            reversed: true,
        },
    );
    let mut body = Body::new(BodyKind::Solid);
    let (start, apex) = (body.add_vertex(net[0]), body.add_vertex(top));
    let rim = body.add_edge(Curve3::Nurbs(rim), (0.0, 1.0), start, start);
    let seam = body.add_edge(Curve3::Nurbs(seam), (0.0, 1.0), start, apex);
    let outline = body.add_loop(vec![forward(rim), forward(seam), backward(seam)]);
    let dome = body.add_face(Surface::Nurbs(surface), false, vec![outline]);
    let down = frame.reversed_at(frame.origin()).unwrap();
    let outline = body.add_loop(vec![backward(rim)]);
    let base = body.add_face(Surface::Plane(Plane::new(down)), false, vec![outline]);
    body.add_shell(vec![dome, base]);
    body
}

#[test]
fn a_dome_closed_across_a_seam_is_known_to_mesh_as_no_manifold() {
    // KNOWN, owed (docs/design/tessellator.md › *The rails of a squeezed
    // pole*; found by the review of 2026-11-14, its shape by the review
    // of 2026-11-15; on the commit before as well, so on 0.1.x): a dome a
    // hundred times as long as wide, closed across a seam in a tip near
    // one end, is meshed without an error, and the mesh is no manifold.
    // Not a gap: every directed edge has its reverse. At 0.002 three
    // vertices within 1e-14 of each other by the tip have edges used
    // twice each way; at 0.02 one edge 0.3 to 0.45 from the tip is shared
    // by four triangles. A user reaches it today: such a dome in a STEP
    // file is read by `file.import`, measures right, and `file.export` to
    // STL writes the mesh without a word. The body is valid and measures
    // as its closed form, eleven fifteenths of its base times its length.
    // When this fails because the mesh is a closed manifold or refused,
    // the pin goes.
    let frame = Frame::new(
        Vec3::new(3.0, -2.0, 5.0),
        Vec3::new(2.0, -3.0, 6.0),
        Vec3::new(1.0, 0.3, 0.0),
    )
    .unwrap();
    for (flip, chord) in [(false, 0.002), (true, 0.02)] {
        let what = format!(
            "the dome 1 x 0.01, its seam on {}x, at {chord}",
            if flip { "-" } else { "+" }
        );
        let body = seamed_dome(&frame, (1.0, 0.01), flip, [0.9, 0.0], 0.5);
        let tolerance = Tolerance::new(1e-7, 1e-9).unwrap();
        assert_eq!(check(&body, &tolerance), [], "{what}");
        let want = 11.0 / 15.0 * PI * 0.01 * 0.5;
        let (mass, bound) = mass_bound(&body).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(
            (mass.volume - want).abs() <= bound.volume + 1e-13 * want,
            "{what}"
        );
        let params = TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        };
        let welded = tessellate_body(&body, &params)
            .unwrap_or_else(|e| panic!("{what}: refused now, take the pin off: {e}"))
            .weld()
            .unwrap();
        // Each directed edge, and how many triangles use it.
        let mut uses = std::collections::BTreeMap::new();
        for &[a, b, c] in &welded.triangles {
            for edge in [(a, b), (b, c), (c, a)] {
                *uses.entry(edge).or_insert(0) += 1;
            }
        }
        let twins = uses.keys().all(|&(a, b)| uses.contains_key(&(b, a)));
        let shared = uses.values().filter(|&&n| n > 1).count();
        println!(
            "{what}: {} triangles, {shared} directed edges used more than once, every edge with its twin: {twins}",
            welded.triangles.len()
        );
        assert!(twins, "{what}: a gap now");
        assert!(
            shared > 0 && !welded.is_closed(),
            "{what}: a closed manifold now, take the pin off"
        );
    }
}

#[test]
fn a_face_the_search_places_and_meshes_is_meshed_as_before() {
    // The L with its tip 0.03 from the corner, 0.5 long, at 0.002 of its
    // size: its rails' points near the pole are found on other meridians
    // by the search, and the face meshes with them (the triangles at the
    // pole happen to stand right). Placed along the borders it is one of
    // the faces the tessellator cannot mesh yet (docs/design/tessellator.md
    // › *The rails of a squeezed pole*): so a face that meshes as the
    // search places it is left as the search places it. The triangle
    // counts are those of the commit before the borders were looked at.
    for ((fullness, _), triangles) in [(PARABOLA, 3074), (ROUND, 0)] {
        let what = format!("the L with its tip 0.03 from the corner, fullness {fullness:.3}");
        let (tip, body) = section_tip(
            &Frame::WORLD,
            1.0,
            &polygon(&L),
            [0.03, 0.03],
            0.5,
            fullness,
        )
        .unwrap_or_else(|e| panic!("{what}: {e}"));
        let (mass, _) = mass_bound(&body).unwrap_or_else(|e| panic!("{what}: {e}"));
        let chord = 0.002 * 6.0;
        meshed(
            &body,
            (mass.volume, mass.area),
            chord,
            (tip.tip, Frame::WORLD.z(), 6.0),
            &what,
        )
        .unwrap_or_else(|e| panic!("{what}: {e}"));
        let params = TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        };
        let made = tessellate_body(&body, &params).unwrap().triangles.len();
        println!("{what}: {made} triangles");
        if triangles > 0 {
            assert_eq!(made, triangles, "{what}");
        }
    }
}

/// A seeded stream of numbers in `[0, 1)` (a linear congruential
/// generator: the same on every machine).
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[test]
fn a_seeded_sweep_of_tips_is_measured_within_its_bounds_and_meshed() {
    // Convex polygons and ovals, whose kernel is all of them: the tip
    // anywhere inside, as near the rim as a thousandth of the way in;
    // any length and either closed-form fullness. Every measure is right
    // within its own bound, and nearly all of them to 1e-9; every mesh
    // that is made is watertight and within its chord bound. What the
    // tessellator still refuses is counted (docs/design/tessellator.md ›
    // *The rails of a squeezed pole* says which, and why).
    let mut seeded = Seeded(20261114);
    let (mut bodies, mut exact, mut meshes, mut refused) = (0, 0, 0, 0);
    let mut reasons = std::collections::BTreeMap::new();
    for case in 0..40 {
        let k = SCALES[case % 3];
        let frame = frames(k)[case % 2];
        let (section, area, inside): (Vec<Piece>, f64, Inside) = if case % 4 == 3 {
            let (a, b) = (1.0 + 4.0 * seeded.next(), 0.2 + 1.5 * seeded.next());
            (
                ellipse(a, b),
                PI * a * b,
                Box::new(move |r, t| [a * r * t.pcos(), b * r * t.psin()]),
            )
        } else {
            // Corners on an ellipse at rising angles: convex.
            let n = 3 + case % 5;
            let (a, b) = (1.0 + 3.0 * seeded.next(), 0.5 + 2.0 * seeded.next());
            let corners: Vec<[f64; 2]> = (0..n)
                .map(|i| {
                    let t = 2.0 * PI * (i as f64 + 0.6 * seeded.next()) / n as f64;
                    [a * t.pcos(), b * t.psin()]
                })
                .collect();
            let towards = corners.clone();
            (
                polygon(&corners),
                polygon_area(&corners),
                // From the middle of the corners towards a point of
                // the rim: inside, the polygon being convex.
                Box::new(move |r, t| {
                    let n = towards.len();
                    let middle = towards.iter().fold([0.0, 0.0], |m, c| {
                        [m[0] + c[0] / n as f64, m[1] + c[1] / n as f64]
                    });
                    let along = t / (2.0 * PI) * n as f64;
                    let (i, share) = (along.floor() as usize % n, along.fract());
                    let (p, q) = (towards[i], towards[(i + 1) % n]);
                    let rim = [p[0] + share * (q[0] - p[0]), p[1] + share * (q[1] - p[1])];
                    [
                        middle[0] + r * (rim[0] - middle[0]),
                        middle[1] + r * (rim[1] - middle[1]),
                    ]
                }),
            )
        };
        // Half of them near the rim: 0.9 to 0.999 of the way out.
        let r = if case % 2 == 0 {
            0.9 * seeded.next()
        } else {
            1.0 - 0.1 * 10.0_f64.ppowf(-2.0 * seeded.next())
        };
        let foot = inside(r, 2.0 * PI * seeded.next());
        let length = 0.2 + 3.0 * seeded.next();
        let (fullness, share) = if seeded.next() < 0.5 { ROUND } else { PARABOLA };
        let what = format!(
            "case {case}: {} pieces, tip over {foot:?} ({r:.4} of the way out), {length:.3} long, fullness {fullness:.3}, at {k}",
            section.len()
        );
        let (tip, body) = match section_tip(&frame, k, &section, foot, length, fullness) {
            Ok(made) => made,
            // (A tip the module does not prove is not this test's.)
            Err(_) => continue,
        };
        bodies += 1;
        let want = share * area * length * k * k * k;
        let tolerance = Tolerance::new(1e-7 * k, 1e-9).unwrap();
        assert_eq!(check(&body, &tolerance), [], "{what}");
        let (mass, bound) = mass_bound(&body).unwrap_or_else(|e| panic!("{what}: {e}"));
        let off = (mass.volume - want).abs();
        assert!(
            off <= bound.volume + 1e-13 * want,
            "{what}: the volume is {:e} of itself off, outside its bound of {:e}",
            off / want,
            bound.volume / want
        );
        if off <= 1e-9 * want {
            exact += 1;
        }
        let size = section
            .iter()
            .flat_map(|(_, points, _)| points.iter())
            .fold(length, |m, p| m.max(p[0].abs()).max(p[1].abs()));
        for chord in [2e-2, 2e-3] {
            match meshed(
                &body,
                (want, mass.area),
                chord * size * k,
                (tip.tip, frame.z(), size * k),
                &what,
            ) {
                Ok(()) => meshes += 1,
                Err(reason) => {
                    refused += 1;
                    let kind: String = reason
                        .split(": ")
                        .nth(1)
                        .unwrap_or(&reason)
                        .chars()
                        .take(48)
                        .collect();
                    *reasons.entry(kind).or_insert(0) += 1;
                }
            }
        }
    }
    println!(
        "{bodies} tips: {exact} measured within 1e-9 of the closed form, all within their bounds; {meshes} meshes watertight and within their chord bounds, {refused} refused: {reasons:?}"
    );
    assert!(bodies >= 30, "{bodies}");
    assert_eq!(exact, bodies);
    // No loop is refused as one that jumps or crosses in parameter space.
    assert!(
        reasons
            .keys()
            .all(|reason| !reason.contains("parameter space")),
        "{reasons:?}"
    );
}

/// A cubic profile `[distance from the axis, along the axis]` that
/// starts on the axis, closed along the top and down the axis, turned
/// about the axis by `sweep`: a body the revolve command builds, with
/// one face on a spline surface whose border on the axis is a pole and
/// two flat ends along its meridians.
fn turned_spline(frame: &Frame, k: f64, control: [[f64; 2]; 4], sweep: f64) -> Body {
    use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    let tolerance = Tolerance::new(1e-7 * 3.0 * k, 1e-9).unwrap();
    let control: Vec<[f64; 2]> = control.iter().map(|p| [p[0] * k, p[1] * k]).collect();
    let (first, last) = (control[0], control[3]);
    let top = [0.0, last[1]];
    let line = |id, start: [f64; 2], end: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    };
    let curves = vec![
        SketchCurve {
            id: 3,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control,
            }),
        },
        line(5, last, top),
        line(7, top, first),
    ];
    let profiles = detect_profiles(&curves, &tolerance).unwrap();
    craft3d_kernel::revolve::revolve(
        &profiles,
        frame,
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        sweep,
        &tolerance,
    )
    .unwrap()
    .body
}

#[test]
fn a_spline_turned_onto_its_axis_is_measured_to_a_bound_that_says_so() {
    // Bodies a command builds, measured right before and after: what
    // moved for them on 2026-11-14 is the feet of their meridians beside
    // the pole, and with them the bound (docs/design/topology.md › *A
    // rail beside its border*). The volume is half the sweep times the
    // integral of the squared distance from the axis along it, a
    // polynomial.
    let shapes = [
        ("a dome", [[0.0, 0.0], [1.0, 0.0], [2.0, 1.0], [2.0, 3.0]]),
        ("a point", [[0.0, 0.0], [0.5, 1.0], [2.0, 1.5], [2.0, 3.0]]),
        (
            "a needle",
            [[0.0, 0.0], [0.02, 1.0], [1.0, 2.0], [2.0, 3.0]],
        ),
    ];
    let rule = craft3d_math::quadrature::Rule::new(16);
    let (mut worst, mut widest, mut beside) = (0.0_f64, 0.0_f64, 0.0_f64);
    for k in SCALES {
        for frame in frames(k) {
            for (name, control) in shapes {
                for sweep in [1.0, PI] {
                    let what = format!("{name} turned by {sweep:.3} at {k}");
                    let body = turned_spline(&frame, k, control, sweep);
                    // The profile's coordinate `c` and its rate at `t`.
                    let bezier = |t: f64, c: usize| {
                        let s = 1.0 - t;
                        let [a, b, d, e] = control.map(|p| p[c]);
                        (
                            s * s * s * a
                                + 3.0 * s * s * t * b
                                + 3.0 * s * t * t * d
                                + t * t * t * e,
                            3.0 * (s * s * (b - a) + 2.0 * s * t * (d - b) + t * t * (e - d)),
                        )
                    };
                    let integral: f64 = rule
                        .points(0.0, 1.0)
                        .map(|(t, w)| {
                            let (r, _) = bezier(t, 0);
                            let (_, dz) = bezier(t, 1);
                            w * r * r * dz
                        })
                        .sum();
                    let want = 0.5 * sweep * integral * k * k * k;
                    let (off, bound) = measured(&body, k, want, 1e-9, &what);
                    worst = worst.max(off);
                    widest = widest.max(bound);
                    // The meridians are borders of the spline face: every
                    // point of them is found there to rounding, where the
                    // search in both parameters may end beside it.
                    for face in &body.faces {
                        if craft3d_geom::pole::poles(&face.surface).is_empty() {
                            continue;
                        }
                        let mut feet = craft3d_geom::pole::BorderFeet::new(&face.surface).unwrap();
                        let on = feet.rounding();
                        for edge in &body.edges {
                            let (t0, t1) = edge.range;
                            let middle = edge.curve.eval(0.5 * (t0 + t1));
                            if !feet.nearest(middle).is_some_and(|foot| foot.off <= on) {
                                continue;
                            }
                            for i in 1..64 {
                                let p = edge.curve.eval(t0 + (t1 - t0) * f64::from(i) / 64.0);
                                let foot = feet.nearest(p).unwrap();
                                assert!(
                                    foot.off <= on + 16.0 * f64::EPSILON * p.length(),
                                    "{what}: {:e}",
                                    foot.off
                                );
                                let (u, v) = face.surface.project(p).unwrap();
                                let searched = face.surface.eval(u, v).distance(p);
                                beside = beside.max(searched / (3.0 * k));
                            }
                        }
                    }
                }
            }
        }
    }
    println!(
        "splines turned onto their axis: within {worst:e} of the closed form, bounds up to {widest:e}; the search in both parameters ends up to {beside:e} of the size beside a point of a meridian"
    );
    assert!(worst <= 1e-12, "{worst:e}");
}

/// The double-exponential (tanh–sinh) rule on [0, 1] with the step
/// `2^-level`: per node how far it is from the nearer end (so that a node
/// next to an end keeps its digits), whether that is the upper end, and
/// its weight. A rule of its own, apart from the kernel's Gauss–Legendre
/// panels (the review of 2026-11-15).
fn tanh_sinh(level: u32) -> Vec<(f64, bool, f64)> {
    let h = 0.5_f64.ppowi(level as i32);
    let n = (3.3 / h).ceil() as i64;
    let mut out = Vec::new();
    for k in -n..=n {
        let t = k as f64 * h;
        let e = t.pexp();
        let (sinh, cosh) = (0.5 * (e - 1.0 / e), 0.5 * (e + 1.0 / e));
        let s = 0.5 * PI * sinh;
        let small = 1.0 / (1.0 + (2.0 * s.abs()).pexp());
        let weight = h * PI * cosh * small * (1.0 - small);
        if weight >= 1e-300 && small > 0.0 {
            out.push((small, t > 0.0, weight));
        }
    }
    out
}

/// `∬ |Sᵤ × Sᵥ| du dv` over the whole of a NURBS surface, span by span, by
/// [`tanh_sinh`] in both parameters.
fn area_by_tanh_sinh(surface: &Surface, level: u32) -> f64 {
    let Surface::Nurbs(s) = surface else {
        return f64::NAN;
    };
    let spans = |knots: &[f64]| -> Vec<(f64, f64)> {
        knots
            .windows(2)
            .filter(|w| w[1] > w[0])
            .map(|w| (w[0], w[1]))
            .collect()
    };
    let rule = tanh_sinh(level);
    let place = |(a, b): (f64, f64), &(small, high, _): &(f64, bool, f64)| {
        if high {
            b - (b - a) * small
        } else {
            a + (b - a) * small
        }
    };
    let mut sum = 0.0;
    for su in spans(s.knots_u()) {
        for sv in spans(s.knots_v()) {
            for nu in &rule {
                let u = place(su, nu);
                let mut inner = 0.0;
                for nv in &rule {
                    let (_, a, b) = surface.derivatives(u, place(sv, nv));
                    inner += a.cross(b).length() * nv.2 * (sv.1 - sv.0);
                }
                sum += inner * nu.2 * (su.1 - su.0);
            }
        }
    }
    sum
}

/// A tip's name, the corners of its section, its foot, its length and
/// its fullness.
type Tip<'a> = (&'a str, &'a [[f64; 2]], [f64; 2], f64, f64);

#[test]
fn the_area_of_a_thin_blade_s_tip_is_within_its_bound() {
    // The review of 2026-11-15, by a tanh–sinh rule face by face: a blade
    // 1e-4 thick with a tip 0.3 long measured 1.87e-12 (quarter ellipses)
    // and 6.90e-12 (parabolas) of its area off within a bound of 1.0e-12,
    // the L with its tip 1e-3 inside its kernel's edge, three long,
    // 1.59e-12 within 1.0e-12. The inner integrals along the faces into
    // the pole have a feature narrower than their panels, and two sums
    // agreed on the plateau before it (docs/design/topology.md › *A rail
    // beside its border*). Before the feet were taken along the borders
    // the bound held by accident (9.57e-8, 6.55e-7, 1.20e-8 off within as
    // much). Now the inner integrals of such a face are shown to converge.
    let blade = blade(1e-4);
    let cases: [Tip; 4] = [
        (
            "the blade 1e-4, quarter ellipses",
            &blade,
            [0.0, 0.0],
            0.3,
            ROUND.0,
        ),
        (
            "the blade 1e-4, parabolas",
            &blade,
            [0.0, 0.0],
            0.3,
            PARABOLA.0,
        ),
        ("the blade 1e-4, fullness 0.9", &blade, [0.0, 0.0], 0.3, 0.9),
        (
            "the L, tip 1e-3 inside its kernel's edge",
            &L,
            [1.999, 1.0],
            3.0,
            ROUND.0,
        ),
    ];
    for (name, corners, foot, length, fullness) in cases {
        for k in SCALES {
            let what = format!("{name} at {k}");
            let (_, body) =
                section_tip(&Frame::WORLD, k, &polygon(corners), foot, length, fullness)
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
            let (mass, bound) = mass_bound(&body).unwrap_or_else(|e| panic!("{what}: {e}"));
            let (mut truth, mut apart) = (0.0, 0.0_f64);
            for face in &body.faces {
                truth += match &face.surface {
                    Surface::Plane(_) => polygon_area(corners) * k * k,
                    surface => {
                        let (five, six) =
                            (area_by_tanh_sinh(surface, 5), area_by_tanh_sinh(surface, 6));
                        apart = apart.max((five - six).abs() / six);
                        six
                    }
                };
            }
            let off = (mass.area - truth).abs();
            println!(
                "{what}: the area {:e} of itself off the rule (its two steps {apart:.1e} apart), bound {:e}",
                off / truth,
                bound.area / truth
            );
            assert!(
                apart <= 1e-14,
                "{what}: the rule has not settled: {apart:e}"
            );
            assert!(
                off <= bound.area + 1e-14 * truth,
                "{what}: {:e} of the area off, outside its bound of {:e}",
                off / truth,
                bound.area / truth
            );
        }
    }
}

#[test]
fn the_volume_of_a_tip_is_the_same_with_or_without_its_inertia() {
    // `mass_properties` and `mass_and_inertia` give one volume: the sums
    // without the second moments are those the first gives, also where a
    // face is measured once more with the search's feet because its
    // second moments did not settle with feet along its borders (the
    // review of 2026-11-15; on the commit before the feet the L with its
    // tip 0.01 from the corner at fullness 0.9 was 8.9e-15 apart).
    for (corners, foot, fullness) in [
        (&L[..], [0.01, 0.01], 0.9),
        (&L[..], [0.01, 0.01], PARABOLA.0),
        (&TRIANGLE[..], [0.003, 0.003], 0.95),
        (&blade(1e-4)[..], [0.0, 0.0], ROUND.0),
    ] {
        let (_, body) =
            section_tip(&Frame::WORLD, 1.0, &polygon(corners), foot, 0.5, fullness).unwrap();
        let alone = craft3d_topo::mass_properties(&body).map(|m| m.volume.to_bits());
        let with = craft3d_topo::mass_and_inertia(&body)
            .0
            .map(|m| m.volume.to_bits());
        assert_eq!(
            alone, with,
            "{corners:?}, tip over {foot:?}, fullness {fullness}"
        );
    }
}

#[test]
fn a_face_refused_as_the_search_places_it_is_not_tried_again_under_a_bound() {
    // Under a bound on its work (an operation's own look at what it made,
    // as `file.import`'s search for crossings) the tessellator meshes a
    // body as the search places its loops and tries nothing more
    // (docs/design/tessellator.md › *The rails of a squeezed pole*): a
    // body it meshes plainly is refused there. Pinned: the L with a blunt
    // tip a hundredth of its size from its corner, at the finer tolerance.
    let (_, body) = section_tip(&Frame::WORLD, 1.0, &polygon(&L), [1.99, 1.99], 0.5, 0.95).unwrap();
    let params = TessParams {
        chordal_tolerance: 2e-3 * 12.0,
        ..TessParams::default()
    };
    assert!(tessellate_body(&body, &params).is_ok());
    let bounded = craft3d_geom::allowance::within(u64::MAX / 4, || tessellate_body(&body, &params));
    println!(
        "under a bound: {:?}",
        bounded.as_ref().map(|mesh| mesh.triangles.len())
    );
    assert!(
        bounded.is_err(),
        "meshed under a bound now: say so in tessellator.md and take the pin off"
    );
}
