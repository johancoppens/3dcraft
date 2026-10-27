//! Origami folded flat (docs/design/origami.md › *Folded flat*): the
//! facets placed by reflections across the creases, and the order of the
//! layers found under the published rules, or refused with the reason; at
//! three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::origami::{
    Angle, Arrangement, BoundarySegment, Crease, CreaseKind, FoldError, Folded, Layers, arrange,
    fold_flat, folded_sheet, folds_flat, layer_order,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use std::f64::consts::PI;

use CreaseKind::{Mountain as M, Valley as V};

const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];
const ANGULAR: f64 = 1e-9;

fn tol(scale: f64) -> f64 {
    1e-9 * scale
}

fn polygon(points: &[[f64; 2]]) -> Vec<BoundarySegment> {
    let n = points.len();
    (0..n)
        .map(|i| BoundarySegment {
            id: i as u64 + 1,
            start: points[i],
            end: points[(i + 1) % n],
            reversed: false,
        })
        .collect()
}

fn crease(id: u64, start: [f64; 2], end: [f64; 2], kind: CreaseKind) -> Crease {
    Crease {
        id,
        start,
        end,
        kind,
    }
}

/// Every crease folded by 180 degrees its kind's way.
fn flat(arr: &Arrangement) -> Vec<Angle> {
    arr.hinges
        .iter()
        .map(|h| Angle::Given(arr.creases[h.crease].kind.sign() * PI))
        .collect()
}

fn fold_all(arr: &Arrangement, fixed: usize, scale: f64) -> Result<(Folded, Layers), FoldError> {
    let angles = flat(arr);
    assert!(folds_flat(arr, &angles, ANGULAR).unwrap());
    let folded = fold_flat(arr, &angles, fixed, tol(scale))?;
    let layers = layer_order(arr, &folded, tol(scale))?;
    Ok((folded, layers))
}

/// The area of the body built from the fold, and that every vertex lies
/// in the sheet's plane exactly.
fn area_in_plane(arr: &Arrangement, folded: &Folded, scale: f64) -> f64 {
    let frame = Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap();
    let tolerance = Tolerance::new(tol(scale), ANGULAR).unwrap();
    let made = folded_sheet(arr, folded, &frame, &tolerance).unwrap();
    assert!(craft3d_topo::check(&made.body, &tolerance).is_empty());
    for v in &made.body.vertices {
        assert_eq!(v.point.z, 0.0, "{:?}", v.point);
    }
    craft3d_topo::mass_properties(&made.body).unwrap().area
}

fn above(layers: &Layers, a: usize, b: usize) -> bool {
    layers.above.contains(&[a, b])
}

#[test]
fn a_card_folded_flat_lies_on_itself_in_the_order_of_its_kind() {
    for scale in SCALES {
        let s = 2.0 * scale;
        for kind in [V, M] {
            let arr = arrange(
                &polygon(&[[0.0, 0.0], [s, 0.0], [s, s], [0.0, s]]),
                &[crease(10, [s / 2.0, 0.0], [s / 2.0, s], kind)],
                tol(scale),
            )
            .unwrap();
            let fixed = arr.facet_at([s / 4.0, s / 2.0]).unwrap();
            let other = 1 - fixed;
            let (folded, layers) = fold_all(&arr, fixed, scale).unwrap();
            // The other half lands on the fixed one, face down.
            let corner = folded.transforms[other].point(Vec3::new(s, s, 0.0));
            assert!(corner.distance(Vec3::new(0.0, s, 0.0)) <= 1e-12 * s);
            assert!(layers.flipped[other] && !layers.flipped[fixed]);
            // A valley brings the fronts together: on top, seen from the
            // front; a mountain below.
            assert_eq!(above(&layers, other, fixed), kind == V);
            assert_eq!(layers.above.len(), 1);
            assert_eq!(layers.layer[fixed], usize::from(kind == M));
            assert!((area_in_plane(&arr, &folded, scale) - s * s).abs() <= 1e-12 * s * s);
            assert_eq!(folded.angles[0], kind.sign() * PI);
        }
    }
}

#[test]
fn an_accordion_pleat_stacks_in_a_zigzag() {
    for scale in SCALES {
        let s = scale;
        let arr = arrange(
            &polygon(&[[0.0, 0.0], [3.0 * s, 0.0], [3.0 * s, s], [0.0, s]]),
            &[
                crease(10, [s, 0.0], [s, s], M),
                crease(11, [2.0 * s, 0.0], [2.0 * s, s], V),
            ],
            tol(scale),
        )
        .unwrap();
        let a = arr.facet_at([0.5 * s, 0.5 * s]).unwrap();
        let b = arr.facet_at([1.5 * s, 0.5 * s]).unwrap();
        let c = arr.facet_at([2.5 * s, 0.5 * s]).unwrap();
        let (folded, layers) = fold_all(&arr, a, scale).unwrap();
        // The mountain puts the middle under the first; the valley, seen
        // from the front of the middle facet that now faces down, puts the
        // last one under the middle.
        assert!(above(&layers, a, b) && above(&layers, b, c) && above(&layers, a, c));
        assert_eq!(
            [layers.layer[a], layers.layer[b], layers.layer[c]],
            [2, 1, 0]
        );
        assert!(!layers.flipped[c] && layers.flipped[b]);
        let far = folded.transforms[c].point(Vec3::new(3.0 * s, s, 0.0));
        assert!(far.distance(Vec3::new(s, s, 0.0)) <= 1e-12 * s);
        assert!((area_in_plane(&arr, &folded, scale) - 3.0 * s * s).abs() <= 1e-12 * s * s);
        // Both creases the other way: the mirror image.
        let arr = arrange(
            &polygon(&[[0.0, 0.0], [3.0 * s, 0.0], [3.0 * s, s], [0.0, s]]),
            &[
                crease(10, [s, 0.0], [s, s], V),
                crease(11, [2.0 * s, 0.0], [2.0 * s, s], M),
            ],
            tol(scale),
        )
        .unwrap();
        let (_, layers) = fold_all(&arr, a, scale).unwrap();
        assert_eq!(
            [layers.layer[a], layers.layer[b], layers.layer[c]],
            [0, 1, 2]
        );
    }
}

/// A square of side 2s with a horizontal crease and the two halves of a
/// vertical one, crossing in the middle.
fn quartered(s: f64, scale: f64, kinds: [CreaseKind; 3]) -> Arrangement {
    arrange(
        &polygon(&[
            [0.0, 0.0],
            [2.0 * s, 0.0],
            [2.0 * s, 2.0 * s],
            [0.0, 2.0 * s],
        ]),
        &[
            crease(10, [0.0, s], [2.0 * s, s], kinds[0]),
            crease(11, [s, s], [s, 2.0 * s], kinds[1]),
            crease(12, [s, 0.0], [s, s], kinds[2]),
        ],
        tol(scale),
    )
    .unwrap()
}

#[test]
fn a_square_folded_in_half_twice_has_four_layers() {
    for scale in SCALES {
        let s = scale;
        let arr = quartered(s, scale, [V, V, M]);
        assert_eq!(arr.facets.len(), 4);
        let fixed = arr.facet_at([1.5 * s, 1.5 * s]).unwrap();
        let (folded, layers) = fold_all(&arr, fixed, scale).unwrap();
        // Every facet on the fixed quarter, all four in one stack.
        for f in 0..4 {
            let middle = folded.transforms[f].point(Vec3::new(s, s, 0.0));
            assert!(middle.distance(Vec3::new(s, s, 0.0)) <= 1e-12 * s);
        }
        assert_eq!(layers.above.len(), 6);
        let mut seen = layers.layer.clone();
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 2, 3]);
        assert!(!layers.cyclic);
        assert!((area_in_plane(&arr, &folded, scale) - 4.0 * s * s).abs() <= 1e-12 * s * s);
        // Three valleys and no mountain: Maekawa fails, and no order of
        // the layers exists.
        let arr = quartered(s, scale, [V, V, V]);
        match fold_all(&arr, fixed, scale) {
            Err(FoldError::NoLayers { facets }) => assert!(!facets.is_empty()),
            other => panic!("expected no layers, got {other:?}"),
        }
    }
}

/// The waterbomb base: both diagonals valleys, the horizontal midline a
/// mountain (degree 6, all sectors 45 or 90 degrees).
fn waterbomb(s: f64, scale: f64, middle: CreaseKind) -> Arrangement {
    arrange(
        &polygon(&[[-s, -s], [s, -s], [s, s], [-s, s]]),
        &[
            crease(10, [-s, 0.0], [s, 0.0], middle),
            crease(11, [-s, -s], [s, s], V),
            crease(12, [s, -s], [-s, s], V),
        ],
        tol(scale),
    )
    .unwrap()
}

#[test]
fn the_waterbomb_base_folds_flat_into_a_triangle_of_six_layers() {
    for scale in SCALES {
        let s = scale;
        let arr = waterbomb(s, scale, M);
        assert_eq!(arr.facets.len(), 6);
        let fixed = arr.facet_at([0.0, 0.5 * s]).unwrap();
        let (folded, layers) = fold_all(&arr, fixed, scale).unwrap();
        // The base is the fixed triangle, y >= |x|: every facet lands in
        // it.
        for (f, facet) in arr.facets.iter().enumerate() {
            for p in arr.polygon(&facet.outer) {
                let q = folded.transforms[f].point(Vec3::new(p[0], p[1], 0.0));
                assert!(q.y >= q.x.abs() - 1e-12 * s, "{q:?}");
                assert!(q.y <= s + 1e-12 * s, "{q:?}");
            }
        }
        assert!(!layers.cyclic);
        assert!(layers.layer.iter().max().copied().unwrap() >= 2);
        assert!((area_in_plane(&arr, &folded, scale) - 4.0 * s * s).abs() <= 1e-12 * s * s);
        // Every crease a valley: Maekawa is off by 4, no order exists.
        match fold_all(&waterbomb(s, scale, V), fixed, scale) {
            Err(FoldError::NoLayers { facets }) => assert!(!facets.is_empty()),
            other => panic!("expected no layers, got {other:?}"),
        }
    }
}

/// A sheet with one vertex in the middle and creases out to the square's
/// boundary in the given directions (degrees).
fn vertex(s: f64, scale: f64, directions: &[f64], kinds: &[CreaseKind]) -> Arrangement {
    let creases: Vec<Crease> = directions
        .iter()
        .zip(kinds)
        .enumerate()
        .map(|(i, (&a, &kind))| {
            let a = a.to_radians();
            let (c, sn) = (a.cos(), a.sin());
            let t = s / c.abs().max(sn.abs());
            crease(10 + i as u64, [0.0, 0.0], [t * c, t * sn], kind)
        })
        .collect();
    arrange(
        &polygon(&[[-s, -s], [s, -s], [s, s], [-s, s]]),
        &creases,
        tol(scale),
    )
    .unwrap()
}

#[test]
fn a_degree_four_vertex_folds_flat_only_when_big_little_big_holds() {
    // Sectors 30, 60, 150 and 120 degrees: Kawasaki holds (30 + 150 =
    // 60 + 120). The smallest sector lies between the creases at 0 and
    // 30 degrees, so they must differ (Demaine and O'Rourke 2007, §12.2):
    // with Maekawa's three mountains and one valley, the valley is one of
    // those two.
    let directions = [0.0, 30.0, 90.0, 240.0];
    for scale in SCALES {
        let s = scale;
        for (kinds, folds) in [
            ([V, M, M, M], true),
            ([M, V, M, M], true),
            ([M, M, V, M], false),
            ([M, M, M, V], false),
        ] {
            let arr = vertex(s, scale, &directions, &kinds);
            let fixed = arr.facet_at([-0.5 * s, 0.6 * s]).unwrap();
            let got = fold_all(&arr, fixed, scale);
            match (folds, got) {
                (true, Ok((folded, layers))) => {
                    assert!(!layers.cyclic);
                    assert!(
                        (area_in_plane(&arr, &folded, scale) - 4.0 * s * s).abs() <= 1e-12 * s * s
                    );
                }
                (false, Err(FoldError::NoLayers { facets })) => {
                    assert!(!facets.is_empty());
                }
                (folds, other) => panic!("{kinds:?} should fold: {folds}, got {other:?}"),
            }
        }
        // Kawasaki off by 2 degrees: the loop round the vertex does not
        // close when folded flat.
        let arr = vertex(s, scale, &[0.0, 32.0, 90.0, 240.0], &[V, M, M, M]);
        let fixed = arr.facet_at([-0.5 * s, 0.6 * s]).unwrap();
        match fold_flat(&arr, &flat(&arr), fixed, tol(scale)) {
            Err(FoldError::NotFlat { id, gap }) => {
                assert!((10..14).contains(&id));
                assert!(gap > tol(scale));
            }
            other => panic!("expected not flat, got {other:?}"),
        }
    }
}

#[test]
fn what_folds_flat_and_what_is_mixed() {
    let s = 1.0;
    let arr = arrange(
        &polygon(&[[0.0, 0.0], [3.0 * s, 0.0], [3.0 * s, s], [0.0, s]]),
        &[
            crease(10, [s, 0.0], [s, s], M),
            crease(11, [2.0 * s, 0.0], [2.0 * s, s], V),
        ],
        tol(s),
    )
    .unwrap();
    let k = |id: u64| {
        arr.hinges
            .iter()
            .position(|h| arr.creases[h.crease].id == id)
            .unwrap()
    };
    let mut angles = vec![Angle::Free { sign: 1.0 }; 2];
    // Nothing given: not flat (the rigid fold decides).
    assert!(!folds_flat(&arr, &angles, ANGULAR).unwrap());
    // One given 180 degrees, the other solved: flat, the solved one by
    // its kind.
    angles[k(10)] = Angle::Given(-PI);
    angles[k(11)] = Angle::Free { sign: 1.0 };
    assert!(folds_flat(&arr, &angles, ANGULAR).unwrap());
    let fixed = arr.facet_at([0.5, 0.5]).unwrap();
    let folded = fold_flat(&arr, &angles, fixed, tol(s)).unwrap();
    assert_eq!(folded.angles[k(11)], PI);
    // 180 beside 90 degrees: refused, naming the flat crease.
    angles[k(11)] = Angle::Given(PI / 2.0);
    assert_eq!(
        folds_flat(&arr, &angles, ANGULAR),
        Err(FoldError::MixedFlat { id: 10 })
    );
    // Within the angular tolerance of 180 is 180.
    angles[k(11)] = Angle::Given(PI - 0.5 * ANGULAR);
    assert!(folds_flat(&arr, &angles, ANGULAR).unwrap());
    // Bad input is an error, never a panic.
    assert!(fold_flat(&arr, &angles, 99, tol(s)).is_err());
    assert!(fold_flat(&arr, &angles[..1], fixed, tol(s)).is_err());
    assert!(fold_flat(&arr, &angles, fixed, f64::NAN).is_err());
    assert!(fold_flat(&arr, &[Angle::Free { sign: 0.0 }; 2], fixed, tol(s)).is_err());
    let mut wrong = folded.clone();
    wrong.transforms.pop();
    assert!(layer_order(&arr, &wrong, tol(s)).is_err());
    assert!(layer_order(&arr, &folded, 0.0).is_err());
}

#[test]
fn folding_flat_is_deterministic() {
    let s = 1.0;
    let arr = waterbomb(s, 1.0, M);
    let fixed = arr.facet_at([0.0, 0.5]).unwrap();
    let first = fold_all(&arr, fixed, 1.0).unwrap();
    for _ in 0..3 {
        let again = fold_all(&arr, fixed, 1.0).unwrap();
        assert_eq!(format!("{first:?}"), format!("{again:?}"));
    }
}
