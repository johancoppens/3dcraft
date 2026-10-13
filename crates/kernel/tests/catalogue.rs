//! Oracles 1 and 2 for profile detection and extrusion: a catalogue of
//! profiles whose areas, centroids and perimeters are closed forms (worked
//! out at each case in `common/mod.rs`), extruded and compared with
//! `volume = area × height`, `surface = 2 × area + perimeter × height` and
//! hand-counted faces, edges and vertices; at three scales, on two planes,
//! with offsets on either side of the plane.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{Case, Rng, catalogue, frames, lines_reversed};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::profile::{Curve2, Shape, detect_profiles};
use craft3d_math::Tolerance;
use craft3d_topo::{check, genus, mass_properties};

/// Closed forms against sums of a few dozen rounded terms: the profile
/// integrals are closed forms per segment, and the body's are 16-point
/// Gauss–Legendre sums that are exact for these integrands up to rounding.
/// What is left is rounding, and cancellation between faces of a body
/// whose plane is a few sizes away from the origin. Measured worst case over
/// the catalogue: between 1e-15 and 3e-15.
const RELATIVE: f64 = 1e-12;

fn close(actual: f64, expected: f64, scale: f64, what: &str, name: &str) {
    assert!(
        (actual - expected).abs() <= RELATIVE * scale,
        "{name}: {what} is {actual:e}, expected {expected:e} (off by {:e} of {scale:e})",
        (actual - expected).abs() / scale
    );
}

fn check_case(case: &Case, k: f64) {
    let tol = Tolerance::default();
    let name = case.name;
    let profiles =
        detect_profiles(&case.curves, &tol).unwrap_or_else(|e| panic!("{name} at scale {k}: {e}"));
    assert_eq!(profiles.len(), case.parts.len(), "{name}: profile count");
    let mut total = 0.0;
    let mut moment = [0.0, 0.0];
    for (profile, part) in profiles.iter().zip(&case.parts) {
        // The size of the profile, for the centroid's bound.
        let size = part.area.sqrt().max(case.perimeter / 4.0);
        close(profile.area(), part.area, part.area, "area", name);
        let c = profile.centroid().unwrap();
        close(c[0], part.centroid[0], size, "centroid x", name);
        close(c[1], part.centroid[1], size, "centroid y", name);
        assert_eq!(profile.holes.len(), part.holes, "{name}: holes");
        assert!(profile.outer.signed_area() > 0.0, "{name}: outer is ccw");
        for hole in &profile.holes {
            assert!(hole.signed_area() < 0.0, "{name}: holes are clockwise");
        }
        // Loops start at their smallest id and are closed chains.
        for path in std::iter::once(&profile.outer).chain(&profile.holes) {
            let first = path.segments[0].source;
            assert!(path.segments.iter().all(|s| s.source >= first), "{name}");
            let n = path.segments.len();
            for i in 0..n {
                assert_eq!(
                    path.segments[i].end,
                    path.segments[(i + 1) % n].start,
                    "{name}: segments share their end points exactly"
                );
            }
        }
        total += part.area;
        moment[0] += part.area * part.centroid[0];
        moment[1] += part.area * part.centroid[1];
    }
    // Profiles are ordered by the smallest id of their outer loop.
    let firsts: Vec<u64> = profiles
        .iter()
        .map(|p| p.outer.segments[0].source)
        .collect();
    assert!(firsts.windows(2).all(|w| w[0] < w[1]), "{name}: {firsts:?}");

    let holes: usize = case.parts.iter().map(|p| p.holes).sum();
    for frame in frames(k) {
        for (from, to) in [(0.0, 7.0 * k), (-2.5 * k, 1.5 * k), (-9.0 * k, -4.0 * k)] {
            let result = extrude(&profiles, &frame, from, to, &tol)
                .unwrap_or_else(|e| panic!("{name} at scale {k}: {e}"));
            let body = &result.body;
            assert_eq!(check(body, &tol), vec![], "{name}");
            assert_eq!(
                (body.faces.len(), body.edges.len(), body.vertices.len()),
                case.counts,
                "{name}: faces, edges, vertices"
            );
            assert_eq!(body.shells.len(), case.parts.len(), "{name}: shells");
            // Every hole is a handle through the solid.
            assert_eq!(genus(body), Some(holes as u32), "{name}: genus");

            let height = to - from;
            let mass = mass_properties(body).unwrap_or_else(|e| panic!("{name}: {e}"));
            let volume = total * height;
            let area = 2.0 * total + case.perimeter * height;
            close(mass.volume, volume, volume, "volume", name);
            close(mass.area, area, area, "surface area", name);
            let expected = frame.point(moment[0] / total, moment[1] / total, (from + to) / 2.0);
            let centroid = mass.centroid.unwrap();
            // Relative to how far the body reaches from the world origin.
            let reach = expected.length() + case.perimeter + height;
            assert!(
                centroid.distance(expected) <= RELATIVE * reach,
                "{name}: centroid {centroid:?}, expected {expected:?}"
            );
        }
    }
}

#[test]
fn catalogue_at_unit_scale() {
    let cases = catalogue(1.0);
    assert!(cases.len() >= 13);
    for case in &cases {
        check_case(case, 1.0);
    }
}

#[test]
fn catalogue_at_a_thousandth() {
    for case in &catalogue(1e-3) {
        check_case(case, 1e-3);
    }
}

#[test]
fn catalogue_at_a_million() {
    for case in &catalogue(1e6) {
        check_case(case, 1e6);
    }
}

/// The same curves in another order, with every line drawn the other way,
/// give the same profiles (a line's `reversed` flag flips, since it says how
/// the line as drawn is run through) and bit-identical bodies and origins.
#[test]
fn drawing_direction_and_input_order_do_not_matter() {
    let tol = Tolerance::default();
    let mut rng = Rng(0x03DC_4AF7);
    for k in [1e-3, 1.0, 1e6] {
        for case in catalogue(k) {
            let name = case.name;
            let base = detect_profiles(&case.curves, &tol).unwrap();
            let frame = frames(k)[1];
            let (from, to) = (-2.5 * k, 1.5 * k);
            let body = extrude(&base, &frame, from, to, &tol).unwrap();
            for round in 0..4 {
                let mut curves = if round % 2 == 0 {
                    lines_reversed(&case.curves)
                } else {
                    case.curves.clone()
                };
                rng.shuffle(&mut curves);
                let drawn_backwards = round % 2 == 0;
                let other = detect_profiles(&curves, &tol).unwrap();
                assert_eq!(other.len(), base.len(), "{name}");
                for (a, b) in base.iter().zip(&other) {
                    let loops = |p: &craft3d_kernel::profile::Profile| {
                        std::iter::once(p.outer.clone())
                            .chain(p.holes.clone())
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(a.holes.len(), b.holes.len(), "{name}");
                    for (la, lb) in loops(a).iter().zip(&loops(b)) {
                        assert_eq!(la.segments.len(), lb.segments.len(), "{name}");
                        for (sa, sb) in la.segments.iter().zip(&lb.segments) {
                            let flips = drawn_backwards && sa.shape == Shape::Line;
                            assert_eq!(sa.reversed != flips, sb.reversed, "{name}");
                            let mut same = sb.clone();
                            same.reversed = sa.reversed;
                            // Debug prints every bit of an f64, and tells
                            // 0.0 from −0.0.
                            assert_eq!(format!("{sa:?}"), format!("{same:?}"), "{name}");
                        }
                    }
                }
                let again = extrude(&other, &frame, from, to, &tol).unwrap();
                assert_eq!(body, again, "{name}");
                assert_eq!(format!("{body:?}"), format!("{again:?}"), "{name}");
            }
        }
    }
}

/// `reversed` says how the curve as drawn is run through: the segment's
/// start is the drawn end exactly when it is set (lines), and arcs and
/// circles run clockwise exactly when it is set.
#[test]
fn the_reversed_flag_tells_how_each_curve_is_traversed() {
    let tol = Tolerance::default();
    for case in catalogue(1.0) {
        for curves in [case.curves.clone(), lines_reversed(&case.curves)] {
            let profiles = detect_profiles(&curves, &tol).unwrap();
            for profile in &profiles {
                let outer = std::iter::once((&profile.outer, false));
                for (path, hole) in outer.chain(profile.holes.iter().map(|h| (h, true))) {
                    for s in &path.segments {
                        let drawn = curves.iter().find(|c| c.id == s.source).unwrap();
                        match drawn.curve {
                            Curve2::Line { start, end } => {
                                let (from, to) = if s.reversed {
                                    (end, start)
                                } else {
                                    (start, end)
                                };
                                assert_eq!((s.start, s.end), (from, to), "{}", case.name);
                            }
                            Curve2::Spline(ref spline) => {
                                let (start, end) =
                                    (spline.control[0], spline.control[spline.control.len() - 1]);
                                let (from, to) = if s.reversed {
                                    (end, start)
                                } else {
                                    (start, end)
                                };
                                assert_eq!((s.start, s.end), (from, to), "{}", case.name);
                            }
                            Curve2::Arc { start, end, .. } => {
                                let (from, to) = if s.reversed {
                                    (end, start)
                                } else {
                                    (start, end)
                                };
                                assert_eq!((s.start, s.end), (from, to), "{}", case.name);
                            }
                            Curve2::Circle { center, radius } => {
                                // A circle alone is a whole loop: ccw as an
                                // outer boundary, clockwise as a hole.
                                assert_eq!(s.reversed, hole, "{}", case.name);
                                assert_eq!(s.start, [center[0] + radius, center[1]]);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A body of several shells (one per profile) passes the checker and has
/// the mass properties of the parts together: the two rectangles 2 × 1 and
/// 3 × 2 of height 4 have volume (2 + 6)·4 = 32.
#[test]
fn several_profiles_make_one_body_of_several_shells() {
    let tol = Tolerance::default();
    let case = catalogue(1.0)
        .into_iter()
        .find(|c| c.name == "two rectangles")
        .unwrap();
    let profiles = detect_profiles(&case.curves, &tol).unwrap();
    let result = extrude(&profiles, &frames(1.0)[0], 0.0, 4.0, &tol).unwrap();
    assert_eq!(result.body.shells.len(), 2);
    assert_eq!(check(&result.body, &tol), vec![]);
    let mass = mass_properties(&result.body).unwrap();
    assert!((mass.volume - 32.0).abs() < 1e-12, "{}", mass.volume);
}
