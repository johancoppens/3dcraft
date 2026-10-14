//! Lofts: the solid through a row of profiles.
//!
//! The oracles are closed forms where the loft is a solid that has one (a
//! box, the frustum of a cone and of a pyramid, a twisted prism whose
//! cross-section is a polygon with an area quadratic in the height, a
//! polygonal ring), and for the others an integral of the cross-section
//! taken from the returned surfaces themselves (by `Surface::eval`, with
//! Green's theorem around the section and Simpson's rule along the loft),
//! which `mass_properties` and the volume of the mesh have to agree with.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{arc, circle, line, polygon, rectangle};
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::Role;
use craft3d_kernel::loft::{Lofted, Options, Section, loft};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, check, genus, mass_properties};
use std::f64::consts::{PI, TAU};

type P = [f64; 2];

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
const SMOOTH: Options = Options {
    ruled: false,
    closed: false,
};
const RULED: Options = Options {
    ruled: true,
    closed: false,
};

fn tol() -> Tolerance {
    Tolerance::default()
}

/// The plane parallel to the world's xy plane at height `z`.
fn level(z: f64) -> Frame {
    Frame::WORLD.at(Vec3::new(0.0, 0.0, z)).unwrap()
}

fn section(curves: &[SketchCurve], plane: Frame) -> Section {
    let profiles = detect_profiles(curves, &tol()).unwrap();
    assert_eq!(profiles.len(), 1);
    Section {
        plane,
        outline: profiles[0].outer.clone(),
        start: None,
    }
}

fn starting(mut section: Section, start: u64) -> Section {
    section.start = Some(start);
    section
}

fn square(half: f64) -> Vec<SketchCurve> {
    rectangle(1, 1, (-half, half), (-half, half))
}

fn turned(points: &[P], angle: f64) -> Vec<P> {
    let (s, c) = angle.sin_cos();
    points
        .iter()
        .map(|p| [p[0] * c - p[1] * s, p[0] * s + p[1] * c])
        .collect()
}

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

/// A leaf: two cubic splines between the same two points, bulging to
/// either side; `k` scales it, `fat` is how far it bulges.
fn leaf(k: f64, fat: f64) -> Vec<SketchCurve> {
    vec![
        spline(
            1,
            3,
            &[
                [-2.0 * k, 0.0],
                [-k, -fat * k],
                [0.5 * k, -1.2 * fat * k],
                [1.5 * k, -0.6 * fat * k],
                [2.0 * k, 0.0],
            ],
        ),
        spline(
            2,
            3,
            &[
                [2.0 * k, 0.0],
                [1.0 * k, fat * k],
                [-k, 0.8 * fat * k],
                [-2.0 * k, 0.0],
            ],
        ),
    ]
}

fn kinds(lofted: &Lofted, role: Role) -> Vec<&'static str> {
    lofted
        .body
        .faces
        .iter()
        .zip(&lofted.faces)
        .filter(|(_, origin)| origin.role == role)
        .map(|(face, _)| face.surface.kind())
        .collect()
}

/// The volume of the mesh at a chordal tolerance of `fine` times the
/// body's size; the mesh has to be closed.
fn mesh_volume(body: &Body, fine: f64) -> f64 {
    let size = body.bbox().unwrap().diagonal();
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: size * fine,
            ..TessParams::default()
        },
    )
    .unwrap();
    let welded = mesh.weld().unwrap();
    assert!(welded.is_closed(), "the mesh is not watertight at {fine}");
    welded.volume()
}

/// Everything a good loft has to be: valid, measurable, not passing
/// through itself, watertight as a mesh, with the mesh's volume tending
/// to the measured one. Returns (volume, area).
fn sound(lofted: &Lofted) -> (f64, f64) {
    let body = &lofted.body;
    assert_eq!(check(body, &tol()), vec![]);
    assert_eq!(body.faces.len(), lofted.faces.len());
    let mass = mass_properties(body).unwrap();
    assert!(mass.volume > 0.0, "inside out: {}", mass.volume);
    let found = body_self_intersections(body, &SelfCheckParams::new(tol().linear)).unwrap();
    assert!(found.is_clean(), "{found}");
    for fine in [1e-2, 1e-3, 1e-4] {
        let now = (mesh_volume(body, fine) - mass.volume).abs() / mass.volume;
        assert!(now < 30.0 * fine, "mesh volume off by {now} at {fine}");
    }
    (mass.volume, mass.area)
}

fn close(a: f64, b: f64, relative: f64) -> bool {
    (a - b).abs() <= relative * b.abs()
}

/// The volume of a loft from its side surfaces alone, which must all be
/// B-spline surfaces through all sections (one band): by the divergence
/// theorem, a third of the flux of the position through the sides
/// (Simpson's rule on `Surface::derivatives`) and through the two flat
/// ends, whose areas come from Green's theorem around the ends of the
/// sides.
fn sliced_volume(lofted: &Lofted) -> f64 {
    let sides: Vec<&NurbsSurface> = lofted
        .body
        .faces
        .iter()
        .zip(&lofted.faces)
        .filter(|(_, origin)| origin.role == Role::Side)
        .map(|(face, _)| {
            assert!(!face.reversed);
            match &face.surface {
                Surface::Nurbs(surface) => surface,
                other => panic!("a {} among the sides", other.kind()),
            }
        })
        .collect();
    // Simpson's rule on each knot span: the integrands are smooth within
    // a span only.
    let simpson = |knots: &[f64], f: &dyn Fn(f64) -> Vec3| -> Vec3 {
        let mut sum = Vec3::ZERO;
        for w in knots.windows(2).filter(|w| w[1] > w[0]) {
            let n = 128;
            let h = (w[1] - w[0]) / n as f64;
            for i in 0..=n {
                let weight = if i == 0 || i == n {
                    1.0
                } else if i % 2 == 1 {
                    4.0
                } else {
                    2.0
                };
                sum += f(w[0] + i as f64 * h) * (weight * h / 3.0);
            }
        }
        sum
    };
    let mut flux = 0.0;
    for surface in &sides {
        flux += simpson(surface.knots_v(), &|v| {
            simpson(surface.knots_u(), &|u| {
                let (p, du, dv) = surface.derivatives(u, v);
                Vec3::new(p.dot(du.cross(dv)), 0.0, 0.0)
            })
        })
        .x;
    }
    // An end: its area as a vector, ½ ∮ p × dp, counter-clockwise about
    // the direction of the loft; a point of its plane dotted with that is
    // three times the volume of the cone from the origin to the end.
    for (v, sign) in [(0.0, -1.0), (1.0, 1.0)] {
        let mut area = Vec3::ZERO;
        for surface in &sides {
            area += simpson(surface.knots_u(), &|u| {
                let (p, du, _) = surface.derivatives(u, v);
                p.cross(du) * 0.5
            });
        }
        flux += sign * sides[0].eval(0.0, v).dot(area);
    }
    flux / 3.0
}

/// The largest distance from points of a curve to the nearest side face.
fn off_the_sides(lofted: &Lofted, points: &[Vec3]) -> f64 {
    points
        .iter()
        .map(|p| {
            lofted
                .body
                .faces
                .iter()
                .zip(&lofted.faces)
                .filter(|(_, origin)| origin.role == Role::Side)
                .filter_map(|(face, _)| {
                    let (u, v) = face.surface.project(*p).ok()?;
                    Some(face.surface.eval(u, v).distance(*p))
                })
                .fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, f64::max)
}

#[test]
fn a_square_to_the_same_square_is_a_box_with_plane_faces() {
    for k in SCALES {
        let made = loft(
            &[
                section(&square(k), level(0.0)),
                section(&square(k), level(3.0 * k)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        assert_eq!(kinds(&made, Role::Side), vec!["plane"; 4]);
        assert_eq!(made.body.faces.len(), 6);
        assert_eq!((made.body.edges.len(), made.body.vertices.len()), (12, 8));
        let (volume, area) = sound(&made);
        assert!(close(volume, 12.0 * k.powi(3), 1e-12), "{volume}");
        assert!(close(area, 32.0 * k * k, 1e-12), "{area}");
        // Named after the curves of both sections, in order.
        let sides: Vec<&Vec<u64>> = made
            .faces
            .iter()
            .filter(|o| o.role == Role::Side)
            .map(|o| &o.sources)
            .collect();
        assert_eq!(sides, [&vec![1, 1], &vec![2, 2], &vec![3, 3], &vec![4, 4]]);
        assert_eq!(made.faces[4].role, Role::StartCap);
        assert_eq!(made.faces[5].role, Role::EndCap);
        assert_eq!(made.faces[5].sources, vec![1, 2, 3, 4]);
    }
}

#[test]
fn a_circle_to_a_coaxial_circle_is_a_frustum_with_a_cone_face() {
    for k in SCALES {
        let (r0, r1, h) = (2.0 * k, 0.75 * k, 3.0 * k);
        let made = loft(
            &[
                section(&[circle(1, [k, -k], r0)], level(0.0)),
                section(&[circle(5, [k, -k], r1)], level(h)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        assert_eq!(kinds(&made, Role::Side), vec!["cone"]);
        assert_eq!(made.body.faces.len(), 3);
        let (volume, area) = sound(&made);
        let exact = PI * h / 3.0 * (r0 * r0 + r0 * r1 + r1 * r1);
        assert!(close(volume, exact, 1e-12), "{volume} for {exact}");
        let slant = (h * h + (r0 - r1).powi(2)).sqrt();
        let exact = PI * (r0 + r1) * slant + PI * (r0 * r0 + r1 * r1);
        assert!(close(area, exact, 1e-12), "{area} for {exact}");

        // The same radius: a cylinder.
        let made = loft(
            &[
                section(&[circle(1, [k, -k], r0)], level(0.0)),
                section(&[circle(5, [k, -k], r0)], level(h)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        assert_eq!(kinds(&made, Role::Side), vec!["cylinder"]);
        let (volume, _) = sound(&made);
        assert!(close(volume, PI * r0 * r0 * h, 1e-12));
    }
}

#[test]
fn a_square_to_a_smaller_square_is_the_frustum_of_a_pyramid() {
    for k in SCALES {
        let (a, b, h) = (2.0 * k, 0.5 * k, 5.0 * k);
        let made = loft(
            &[
                section(&square(a), level(0.0)),
                section(&square(b), level(h)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        assert_eq!(kinds(&made, Role::Side), vec!["plane"; 4]);
        let (volume, area) = sound(&made);
        let (a1, a2) = (4.0 * a * a, 4.0 * b * b);
        let exact = h / 3.0 * (a1 + a2 + (a1 * a2).sqrt());
        assert!(close(volume, exact, 1e-12), "{volume} for {exact}");
        // Four trapezia of slant height √(h² + (a − b)²).
        let slant = (h * h + (a - b).powi(2)).sqrt();
        let exact = a1 + a2 + 4.0 * (a + b) * slant;
        assert!(close(area, exact, 1e-12), "{area} for {exact}");
    }
}

/// The volume between a polygon and another with as many corners, a
/// height `h` above it, joined corner to corner by straight lines: the
/// cross-section at fraction t is the polygon of the corners
/// `(1 − t) p + t q`, whose area is `(1 − t)² A₀ + t (1 − t) M + t² A₁`
/// with `M = ½ Σ (pᵢ × qᵢ₊₁ + qᵢ × pᵢ₊₁)`.
fn twisted_volume(p: &[P], q: &[P], h: f64) -> f64 {
    let cross = |a: P, b: P| a[0] * b[1] - a[1] * b[0];
    let n = p.len();
    let (mut a0, mut a1, mut mixed) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let j = (i + 1) % n;
        a0 += 0.5 * cross(p[i], p[j]);
        a1 += 0.5 * cross(q[i], q[j]);
        mixed += 0.5 * (cross(p[i], q[j]) + cross(q[i], p[j]));
    }
    h * (a0 / 3.0 + mixed / 6.0 + a1 / 3.0)
}

#[test]
fn a_rectangle_to_the_same_rectangle_turned_is_a_twisted_prism() {
    for k in SCALES {
        let corners = [[-2.0 * k, -k], [2.0 * k, -k], [2.0 * k, k], [-2.0 * k, k]];
        let above = turned(&corners, 30f64.to_radians());
        let h = 4.0 * k;
        let made = loft(
            &[
                section(&polygon(1, 1, &corners), level(0.0)),
                section(&polygon(1, 1, &above), level(h)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        // Doubly ruled, not flat.
        assert_eq!(kinds(&made, Role::Side), vec!["nurbs surface"; 4]);
        let (volume, _) = sound(&made);
        let exact = twisted_volume(&corners, &above, h);
        assert!(close(volume, exact, 1e-9), "{volume} for {exact}");
        assert!(close(sliced_volume(&made), exact, 1e-9));
        // Less than the straight prism, which the twist pinches.
        assert!(volume < 8.0 * k * k * h);
    }
}

#[test]
fn a_smooth_loft_through_circles_whose_radii_grow_evenly_is_a_cone() {
    // Radii linear in the height, at even heights: the interpolation of
    // both is linear, so the skinned surface is the cone itself.
    for k in SCALES {
        let sections: Vec<Section> = (0..4)
            .map(|i| {
                let z = f64::from(i) * 2.0 * k;
                section(&[circle(1, [0.0, 0.0], k + 0.25 * z)], level(z))
            })
            .collect();
        let made = loft(&sections, &SMOOTH, &tol()).unwrap();
        assert_eq!(kinds(&made, Role::Side), vec!["nurbs surface"; 4]);
        let (volume, area) = sound(&made);
        let (r0, r1, h) = (k, 2.5 * k, 6.0 * k);
        let exact = PI * h / 3.0 * (r0 * r0 + r0 * r1 + r1 * r1);
        assert!(close(volume, exact, 1e-9), "{volume} for {exact}");
        let slant = (h * h + (r1 - r0).powi(2)).sqrt();
        let exact = PI * (r0 + r1) * slant + PI * (r0 * r0 + r1 * r1);
        assert!(close(area, exact, 1e-9), "{area} for {exact}");
        // Ruled, the same circles give cones: one face per band.
        let ruled = loft(&sections, &RULED, &tol()).unwrap();
        assert_eq!(kinds(&ruled, Role::Side), vec!["cone"; 3]);
        assert!(close(sound(&ruled).0, volume, 1e-9));
    }
}

/// Points on the circle of a section, in space.
fn on_circle(center: P, radius: f64, plane: &Frame) -> Vec<Vec3> {
    (0..64)
        .map(|i| {
            let a = TAU * f64::from(i) / 64.0;
            plane.point(
                center[0] + radius * a.cos(),
                center[1] + radius * a.sin(),
                0.0,
            )
        })
        .collect()
}

#[test]
fn a_smooth_loft_passes_through_every_section_and_its_volume_is_its_slices() {
    for k in SCALES {
        // A vase: circles that widen, narrow and move off the axis.
        let rings = [
            ([0.0, 0.0], 2.0 * k, 0.0),
            ([0.3 * k, 0.0], 3.0 * k, 2.0 * k),
            ([0.0, 0.4 * k], 1.2 * k, 5.0 * k),
            ([0.0, 0.0], 1.6 * k, 6.5 * k),
            ([-0.2 * k, 0.0], 0.8 * k, 9.0 * k),
        ];
        let sections: Vec<Section> = rings
            .iter()
            .map(|&(c, r, z)| section(&[circle(1, c, r)], level(z)))
            .collect();
        let made = loft(&sections, &SMOOTH, &tol()).unwrap();
        assert_eq!(made.body.faces.len(), 6);
        let (volume, _) = sound(&made);
        // The measure settles to about 1e-8 on these faces (1.1e-8 seen).
        let sliced = sliced_volume(&made);
        assert!(close(volume, sliced, 1e-7), "{volume} against {sliced}");
        for &(c, r, z) in &rings {
            let off = off_the_sides(&made, &on_circle(c, r, &level(z)));
            assert!(off < 1e-9 * k, "a section is {off} off the surface");
        }
        // Ruled through the same sections is another solid, with one band
        // of faces per pair of sections, and passes through them as well.
        let ruled = loft(&sections, &RULED, &tol()).unwrap();
        assert_eq!(ruled.body.faces.len(), 4 * 4 + 2);
        let (ruled_volume, _) = sound(&ruled);
        assert!(!close(ruled_volume, volume, 1e-3));
        for &(c, r, z) in &rings {
            assert!(off_the_sides(&ruled, &on_circle(c, r, &level(z))) < 1e-9 * k);
        }
    }
}

#[test]
fn a_smooth_loft_through_squares_and_rounded_shapes() {
    for k in SCALES {
        // Three squares of different sizes: the sides are curved.
        let made = loft(
            &[
                section(&square(2.0 * k), level(0.0)),
                section(&square(k), level(3.0 * k)),
                section(&square(2.5 * k), level(5.0 * k)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        assert_eq!(kinds(&made, Role::Side), vec!["nurbs surface"; 4]);
        let (volume, _) = sound(&made);
        assert!(close(volume, sliced_volume(&made), 1e-8));
        // Three squares of one size in a row: a box, in planes.
        let made = loft(
            &[
                section(&square(k), level(0.0)),
                section(&square(k), level(k)),
                section(&square(k), level(5.0 * k)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        assert_eq!(kinds(&made, Role::Side), vec!["plane"; 4]);
        assert!(close(sound(&made).0, 20.0 * k.powi(3), 1e-12));
        // A slot (two lines, two half circles) to a larger one, turned:
        // arcs and lines in one loop.
        let slot = |id: u64, half: f64, r: f64| {
            vec![
                line(id, [-half, -r], [half, -r]),
                arc(id + 1, [half, 0.0], [half, -r], [half, r]),
                line(id + 2, [half, r], [-half, r]),
                arc(id + 3, [-half, 0.0], [-half, r], [-half, -r]),
            ]
        };
        let made = loft(
            &[
                section(&slot(1, 2.0 * k, k), level(0.0)),
                section(&slot(1, 3.0 * k, 1.5 * k), level(4.0 * k)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        assert_eq!(
            kinds(&made, Role::Side),
            ["plane", "nurbs surface", "plane", "nurbs surface"]
        );
        sound(&made);
    }
}

#[test]
fn a_leaf_of_two_splines_to_a_larger_leaf() {
    for k in SCALES {
        let made = loft(
            &[
                section(&leaf(k, 1.0), level(0.0)),
                section(&leaf(1.5 * k, 1.4), level(4.0 * k)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        assert_eq!(kinds(&made, Role::Side), vec!["nurbs surface"; 2]);
        let (volume, _) = sound(&made);
        assert!(close(volume, sliced_volume(&made), 1e-8));
        // Through three leaves, smooth.
        let made = loft(
            &[
                section(&leaf(k, 1.0), level(0.0)),
                section(&leaf(1.5 * k, 1.4), level(4.0 * k)),
                section(&leaf(0.8 * k, 0.7), level(7.0 * k)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        let (volume, _) = sound(&made);
        assert!(close(volume, sliced_volume(&made), 1e-8));
    }
}

/// A plane through the world's z axis, turned by `angle` about it: its x
/// axis points away from the z axis, its y axis is the world's z.
fn spoke(angle: f64) -> Frame {
    let (s, c) = angle.sin_cos();
    Frame::new(Vec3::ZERO, Vec3::new(-s, c, 0.0), Vec3::new(c, s, 0.0)).unwrap()
}

#[test]
fn sections_in_planes_that_are_not_parallel_make_a_bend() {
    for k in SCALES {
        let profile = rectangle(1, 1, (3.0 * k, 5.0 * k), (-k, k));
        let angles = [0.0, 0.5, 1.1, 1.5];
        let sections: Vec<Section> = angles
            .iter()
            .map(|a| section(&profile, spoke(*a)))
            .collect();
        // Ruled: each band is the wedge between two planes through the
        // axis; a point at distance ρ sweeps a chord, so the band's volume
        // is sin(α) · (area · mean ρ).
        let ruled = loft(&sections, &RULED, &tol()).unwrap();
        assert_eq!(kinds(&ruled, Role::Side), vec!["plane"; 12]);
        let (volume, _) = sound(&ruled);
        let moment = 4.0 * k * k * 4.0 * k;
        let exact: f64 = angles
            .windows(2)
            .map(|w| (w[1] - w[0]).sin() * moment)
            .sum();
        assert!(close(volume, exact, 1e-11), "{volume} for {exact}");
        // Smooth: close to the piece of a ring, which is 1.5 · area · ρ.
        let smooth = loft(&sections, &SMOOTH, &tol()).unwrap();
        let (volume, _) = sound(&smooth);
        assert!(close(volume, 1.5 * moment, 2e-2), "{volume}");
        assert!(volume > exact);
    }
}

#[test]
fn a_closed_loft_is_a_ring() {
    for k in SCALES {
        let profile = rectangle(1, 1, (3.0 * k, 5.0 * k), (-k, k));
        let count = 6;
        let sections: Vec<Section> = (0..count)
            .map(|i| section(&profile, spoke(TAU * f64::from(i) / f64::from(count))))
            .collect();
        let closed = |ruled| Options {
            ruled,
            closed: true,
        };
        let ruled = loft(&sections, &closed(true), &tol()).unwrap();
        assert_eq!(ruled.body.faces.len(), 24);
        assert!(ruled.faces.iter().all(|o| o.role == Role::Side));
        assert_eq!(genus(&ruled.body), Some(1));
        let (volume, _) = sound(&ruled);
        let moment = 4.0 * k * k * 4.0 * k;
        let exact = 6.0 * (TAU / 6.0).sin() * moment;
        assert!(close(volume, exact, 1e-11), "{volume} for {exact}");
        // Smooth: nearly the ring of revolution (Pappus), and the sections
        // lie on it.
        let smooth = loft(&sections, &closed(false), &tol()).unwrap();
        assert_eq!(genus(&smooth.body), Some(1));
        let (volume, _) = sound(&smooth);
        assert!(close(volume, TAU * moment, 1e-2), "{volume}");
        // A ring of circles.
        let sections: Vec<Section> = (0..5)
            .map(|i| {
                section(
                    &[circle(1, [4.0 * k, 0.0], k)],
                    spoke(TAU * f64::from(i) / 5.0),
                )
            })
            .collect();
        let ring = loft(&sections, &closed(false), &tol()).unwrap();
        let (volume, _) = sound(&ring);
        assert!(close(volume, TAU * 4.0 * k * PI * k * k, 3e-2), "{volume}");
        for i in 0..5 {
            let plane = spoke(TAU * f64::from(i) / 5.0);
            let off = off_the_sides(&ring, &on_circle([4.0 * k, 0.0], k, &plane));
            assert!(off < 1e-9 * k, "{off}");
        }
    }
}

#[test]
fn sections_with_different_numbers_of_curves_are_cut_to_match() {
    for k in SCALES {
        // A square to a circle: the circle in four quarters, from the
        // point nearest to the square's first corner.
        let made = loft(
            &[
                section(&square(2.0 * k), level(0.0)),
                section(&[circle(1, [0.0, 0.0], 1.5 * k)], level(4.0 * k)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        assert_eq!(made.body.faces.len(), 6);
        let (volume, _) = sound(&made);
        assert!(close(volume, sliced_volume(&made), 1e-8));
        // By symmetry each quarter starts on a diagonal of the square.
        let start = made.body.vertices[4].point;
        assert!((start.x - start.y).abs() < 1e-12 * k && start.x < 0.0);
        // A triangle to a hexagon: every side of the triangle in two.
        let triangle: Vec<P> = (0..3)
            .map(|i| {
                let a = TAU * f64::from(i) / 3.0;
                [2.0 * k * a.cos(), 2.0 * k * a.sin()]
            })
            .collect();
        let hexagon: Vec<P> = (0..6)
            .map(|i| {
                let a = TAU * f64::from(i) / 6.0;
                [2.0 * k * a.cos(), 2.0 * k * a.sin()]
            })
            .collect();
        let made = loft(
            &[
                section(&polygon(1, 1, &triangle), level(0.0)),
                section(&polygon(1, 1, &hexagon), level(3.0 * k)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        assert_eq!(made.body.faces.len(), 8);
        // The halves of a side share the name of the side.
        assert_eq!(made.faces[0].sources, vec![1, 1]);
        assert_eq!(made.faces[1].sources, vec![1, 2]);
        let (volume, _) = sound(&made);
        let halves: Vec<P> = (0..6)
            .map(|i| {
                let (a, b) = (triangle[i / 2], triangle[(i / 2 + 1) % 3]);
                if i % 2 == 0 {
                    a
                } else {
                    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
                }
            })
            .collect();
        let exact = twisted_volume(&halves, &hexagon, 3.0 * k);
        assert!(close(volume, exact, 1e-9), "{volume} for {exact}");
        // A leaf (two splines) to a circle: the circle and the splines cut
        // where the other has its corners.
        let made = loft(
            &[
                section(&leaf(k, 1.0), level(0.0)),
                section(&[circle(1, [0.0, 0.0], 1.5 * k)], level(4.0 * k)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap();
        let (volume, _) = sound(&made);
        assert!(close(volume, sliced_volume(&made), 1e-8));
    }
}

#[test]
fn the_start_of_a_section_shifts_the_matching() {
    let k = 1.0;
    let sections = |start: Option<u64>| {
        let mut top = section(&square(k), level(3.0 * k));
        top.start = start;
        [section(&square(k), level(0.0)), top]
    };
    // Matched side to side: the box.
    let straight = loft(&sections(None), &SMOOTH, &tol()).unwrap();
    assert!(close(sound(&straight).0, 12.0, 1e-12));
    // One side on: a quarter turn of twist, still a solid, of less volume.
    let twisted = loft(&sections(Some(2)), &SMOOTH, &tol()).unwrap();
    assert_eq!(twisted.faces[0].sources, vec![1, 2]);
    let (volume, _) = sound(&twisted);
    let corners = [[-k, -k], [k, -k], [k, k], [-k, k]];
    let exact = twisted_volume(&corners, &turned(&corners, PI / 2.0), 3.0 * k);
    assert!(close(volume, exact, 1e-9), "{volume} for {exact}");
    // Two sides on: half a turn. Every side passes through the axis and
    // folds there.
    let error = loft(&sections(Some(3)), &SMOOTH, &tol()).unwrap_err();
    assert!(error.to_string().contains("folds onto itself"), "{error}");
    assert!(matches!(error, KernelError::Loft { ref sections, .. } if sections == &[0, 1]));
    // What folds nowhere and still passes through itself is for the
    // self-intersection check: a bend that goes on for more than a full
    // turn and runs into its own beginning.
    let profile = rectangle(1, 1, (3.0, 5.0), (-1.0, 1.0));
    let spiral: Vec<Section> = [0.0, 2.9, 5.8, 8.7]
        .iter()
        .map(|a| section(&profile, spoke(*a)))
        .collect();
    let crossed = loft(&spiral, &RULED, &tol()).unwrap();
    assert_eq!(check(&crossed.body, &tol()), vec![]);
    let found =
        body_self_intersections(&crossed.body, &SelfCheckParams::new(tol().linear)).unwrap();
    assert!(!found.is_clean());
    // A start the section does not have.
    let error = loft(&sections(Some(9)), &SMOOTH, &tol()).unwrap_err();
    assert!(
        error.to_string().contains("section 2 has no curve 9"),
        "{error}"
    );
}

#[test]
fn a_loft_is_the_same_whichever_way_the_planes_face_and_the_loops_run() {
    let k = 1.0;
    let facing_down = Frame::WORLD
        .reversed_at(Vec3::new(0.0, 0.0, 3.0 * k))
        .unwrap();
    // The upper sketch lies on a plane that faces down: its loop runs the
    // other way round in space, and y is mirrored.
    let made = loft(
        &[
            section(&square(2.0 * k), level(0.0)),
            section(&square(k), facing_down),
        ],
        &SMOOTH,
        &tol(),
    )
    .unwrap();
    // Without a start the upper square starts where the lower one does:
    // the frustum, although its own first curve lies across.
    let (volume, _) = sound(&made);
    let exact = 3.0 / 3.0 * (16.0 + 4.0 + 8.0);
    assert!(close(volume, exact, 1e-12), "{volume}");
    assert_eq!(made.faces[0].sources, vec![1, 3]);
    // Of the four starts it could be given, that is the one.
    let fixed = [1, 2, 3, 4]
        .into_iter()
        .filter_map(|start| {
            loft(
                &[
                    section(&square(2.0 * k), level(0.0)),
                    starting(section(&square(k), facing_down), start),
                ],
                &SMOOTH,
                &tol(),
            )
            .ok()
        })
        .filter(|made| close(mass_properties(&made.body).unwrap().volume, exact, 1e-12))
        .count();
    assert_eq!(fixed, 1);
    // From the top down is the same solid.
    let up = loft(
        &[
            section(&square(2.0 * k), level(0.0)),
            section(&square(k), level(3.0 * k)),
        ],
        &SMOOTH,
        &tol(),
    )
    .unwrap();
    let down = loft(
        &[
            section(&square(k), level(3.0 * k)),
            section(&square(2.0 * k), level(0.0)),
        ],
        &SMOOTH,
        &tol(),
    )
    .unwrap();
    assert!(close(sound(&down).0, sound(&up).0, 1e-12));
    assert_eq!(down.faces[4].role, Role::StartCap);
    // And the same input gives the same body, to the bit.
    assert_eq!(
        up,
        loft(
            &[
                section(&square(2.0 * k), level(0.0)),
                section(&square(k), level(3.0 * k)),
            ],
            &SMOOTH,
            &tol(),
        )
        .unwrap()
    );
}

#[test]
fn what_cannot_be_lofted_is_an_error_that_says_why() {
    let k = 1.0;
    let base = section(&square(k), level(0.0));
    let refused = |sections: &[Section], options: &Options| -> String {
        match loft(sections, options, &tol()) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("a loft that should have been refused"),
        }
    };
    assert!(refused(std::slice::from_ref(&base), &SMOOTH).contains("at least 2 sections"));
    assert!(refused(&[], &SMOOTH).contains("at least 2 sections"));
    let closed = Options {
        ruled: false,
        closed: true,
    };
    assert!(
        refused(&[base.clone(), section(&square(k), level(1.0))], &closed)
            .contains("at least 3 sections")
    );
    // Twice the same section.
    assert!(refused(&[base.clone(), base.clone()], &SMOOTH).contains("lies along the loft"));
    // A section whose plane cuts through the one before it.
    let tilted = Frame::new(
        Vec3::new(0.0, 0.0, 0.5),
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, -1.0),
    )
    .unwrap();
    let message = refused(&[base.clone(), section(&square(k), tilted)], &SMOOTH);
    assert!(message.contains("sections 1 and 2"), "{message}");
    assert!(
        message.contains("planes cross within the profiles"),
        "{message}"
    );
    // A section beside the first, in the same plane.
    let beside = Frame::WORLD.at(Vec3::new(5.0, 0.0, 0.0)).unwrap();
    assert!(
        refused(&[base.clone(), section(&square(k), beside)], &SMOOTH)
            .contains("lies along the loft")
    );
    // A tolerance that is none.
    assert!(matches!(
        loft(
            &[base.clone(), section(&square(k), level(1.0))],
            &SMOOTH,
            &Tolerance {
                linear: f64::NAN,
                angular: 1e-9
            }
        ),
        Err(KernelError::InvalidSize { .. })
    ));
    // Coordinates that are not finite.
    let far = Frame::WORLD.at(Vec3::new(0.0, 0.0, f64::MAX)).unwrap();
    let mut huge = section(&square(k), far);
    huge.plane = far;
    assert!(loft(&[base, huge], &SMOOTH, &tol()).is_err());
}

/// Reproducer for the boolean kernel. A loft with B-spline sides at the
/// scale 1e6 and the default tolerance of 1e-6 is refused as an operand:
/// "invalid geometry: the box has no size, or the tolerance is finer than
/// rounding allows at its size". The same bodies at the scales 1e-3 and 1
/// combine and satisfy the set identities (`crates/engine/tests/loft.rs`).
#[test]
#[ignore = "the boolean refuses a freeform loft at 1e6 with a tolerance of 1e-6"]
fn a_loft_at_1e6_in_a_boolean() {
    use craft3d_kernel::boolean::{BooleanOp, boolean};
    let k = 1e6;
    let made = loft(
        &[
            section(&leaf(k, 1.0), level(0.0)),
            section(&leaf(1.5 * k, 1.4), level(4.0 * k)),
        ],
        &SMOOTH,
        &tol(),
    )
    .unwrap();
    let corner = Frame::WORLD.at(Vec3::new(-0.5 * k, -3.0 * k, k)).unwrap();
    let block = craft3d_kernel::primitives::block(&corner, 4.0 * k, 6.0 * k, 2.0 * k).unwrap();
    for op in [BooleanOp::Union, BooleanOp::Intersect, BooleanOp::Subtract] {
        let result = boolean(&made.body, &block, op, &tol());
        assert!(result.is_ok(), "{op:?}: {}", result.unwrap_err());
    }
}
