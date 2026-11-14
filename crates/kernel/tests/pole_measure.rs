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

use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::tips::{PARABOLA, Piece, ROUND, ellipse, polygon, polygon_area, section_tip};
use craft3d_topo::{Body, check, mass_bound};
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
fn rails_found_beside_their_border_are_meshed() {
    // Blunt tips (fullness 0.95) a hundredth and a thousandth of the L's
    // size from a corner or an edge of its kernel: at the finer tolerance
    // each was refused, "its boundary is not closed in parameter space:
    // between two neighbouring points of edge … it jumps across the
    // surface" (two) and "the boundary crosses itself in parameter space"
    // (one).
    for k in SCALES {
        for frame in frames(k) {
            for foot in [[1.99, 1.99], [0.01, 0.01], [1.999, 1.0]] {
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
