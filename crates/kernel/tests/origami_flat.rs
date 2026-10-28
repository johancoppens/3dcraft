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
        // Folded up (the lower half onto the upper), then the left onto
        // the right: from the top, upper left, lower left, lower right,
        // upper right.
        let at = |x: f64, y: f64| arr.facet_at([x * s, y * s]).unwrap();
        let (ul, ll, lr, ur) = (at(0.5, 1.5), at(0.5, 0.5), at(1.5, 0.5), at(1.5, 1.5));
        assert_eq!(ur, fixed);
        assert_eq!(
            [
                layers.layer[ul],
                layers.layer[ll],
                layers.layer[lr],
                layers.layer[ur]
            ],
            [3, 2, 1, 0]
        );
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
        // The fixed triangle at the bottom, the other one on top, each
        // side flap in two layers between (the right one on the right
        // half of the base, the left one on the left: they do not overlap).
        let at = |x: f64, y: f64| arr.facet_at([x * s, y * s]).unwrap();
        let (top, bottom) = (fixed, at(0.0, -0.5));
        let (ru, rl, lu, ll) = (at(0.8, 0.3), at(0.8, -0.3), at(-0.8, 0.3), at(-0.8, -0.3));
        assert_eq!(layers.layer[top], 0);
        assert_eq!([layers.layer[ru], layers.layer[lu]], [1, 1]);
        assert_eq!([layers.layer[rl], layers.layer[ll]], [2, 2]);
        assert_eq!(layers.layer[bottom], 3);
        assert!(!layers.above.contains(&[ru, lu]) && !layers.above.contains(&[lu, ru]));
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

/// The square twist: a central square whose edges are mountains and, at
/// each corner, a pleat of two creases out to the sheet's boundary, at 45
/// and 135 degrees from the square's edges (the valley beside the
/// smallest sector, as big-little-big wants).
fn square_twist(scale: f64) -> Arrangement {
    let l = 5.0 * scale;
    let corners =
        [[1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]].map(|[x, y]| [x * scale, y * scale]);
    let mut creases = Vec::new();
    for i in 0..4 {
        let (p, q) = (corners[i], corners[(i + 1) % 4]);
        creases.push(crease(10 + i as u64, p, q, M));
    }
    // Per corner: the direction of the edge to the corner before it, and
    // the rays at 45 (a valley) and 135 degrees (a mountain) past it.
    for (i, &p) in corners.iter().enumerate() {
        let base = (270.0 + 90.0 * i as f64).to_radians();
        for (k, (turn, kind)) in [(45.0f64, V), (135.0, M)].into_iter().enumerate() {
            let a = base + turn.to_radians();
            let (c, sn) = (a.cos(), a.sin());
            let tx = if c > 0.0 {
                (l - p[0]) / c
            } else if c < 0.0 {
                (-l - p[0]) / c
            } else {
                f64::INFINITY
            };
            let ty = if sn > 0.0 {
                (l - p[1]) / sn
            } else if sn < 0.0 {
                (-l - p[1]) / sn
            } else {
                f64::INFINITY
            };
            let t = tx.min(ty);
            creases.push(crease(
                20 + 2 * i as u64 + k as u64,
                p,
                [p[0] + t * c, p[1] + t * sn],
                kind,
            ));
        }
    }
    arrange(
        &polygon(&[[-l, -l], [l, -l], [l, l], [-l, l]]),
        &creases,
        tol(scale),
    )
    .unwrap()
}

#[test]
fn a_square_twist_folds_flat() {
    for scale in SCALES {
        let arr = square_twist(scale);
        let fixed = arr.facet_at([0.0, 0.0]).unwrap();
        let (folded, layers) = fold_all(&arr, fixed, scale).unwrap();
        // The four pleats tucked under the square overlap in a pinwheel,
        // each with the next in a place of its own: a cyclic order of
        // pairs, which is physical (no three share a place).
        assert!(layers.cyclic);
        let l = 5.0 * scale;
        assert!((area_in_plane(&arr, &folded, scale) - 4.0 * l * l).abs() <= 1e-12 * l * l);
        // The central square stays, face up, over everything tucked under
        // it.
        assert!(!layers.flipped[fixed]);
        for [upper, lower] in &layers.above {
            assert_ne!(*lower, fixed, "{upper} over the square");
        }
    }
}

#[test]
fn a_stack_too_high_is_undecided_at_once_without_the_memory() {
    // A strip pleated 1200 times: every facet on every other one, 1200
    // facets in one place, some 2.9e8 triples to hold: refused before any
    // is made.
    let n = 1200usize;
    let s = 1.0;
    let creases: Vec<Crease> = (1..n)
        .map(|i| {
            let x = i as f64 * s;
            crease(
                10 + i as u64,
                [x, 0.0],
                [x, s],
                if i % 2 == 0 { V } else { M },
            )
        })
        .collect();
    let len = n as f64 * s;
    let arr = arrange(
        &polygon(&[[0.0, 0.0], [len, 0.0], [len, s], [0.0, s]]),
        &creases,
        tol(1.0),
    )
    .unwrap();
    let fixed = arr.facet_at([0.5, 0.5]).unwrap();
    let started = std::time::Instant::now();
    let folded = fold_flat(&arr, &flat(&arr), fixed, tol(1.0)).unwrap();
    match layer_order(&arr, &folded, tol(1.0)) {
        Err(FoldError::LayersUndecided { count, .. }) => assert!(count >= n, "{count}"),
        other => panic!("expected undecided, got {other:?}"),
    }
    let took = started.elapsed();
    println!("LAYERS-CAP refused a stack of {n} in {took:?}");
    assert!(took.as_secs() < 60, "{took:?}");
}

#[test]
fn how_fast_the_layer_search_works() {
    // A pleat of 120 layers, decided: its work against its time gives the
    // time the work cap (MAX_LAYER_WORK) stands for.
    let n = 120usize;
    let creases: Vec<Crease> = (1..n)
        .map(|i| {
            let x = i as f64;
            crease(
                10 + i as u64,
                [x, 0.0],
                [x, 1.0],
                if i % 2 == 0 { V } else { M },
            )
        })
        .collect();
    let len = n as f64;
    let arr = arrange(
        &polygon(&[[0.0, 0.0], [len, 0.0], [len, 1.0], [0.0, 1.0]]),
        &creases,
        tol(1.0),
    )
    .unwrap();
    let fixed = arr.facet_at([0.5, 0.5]).unwrap();
    let started = std::time::Instant::now();
    let (_, layers) = fold_all(&arr, fixed, 1.0).unwrap();
    let took = started.elapsed().as_secs_f64();
    let max = layers.layer.iter().max().copied().unwrap();
    assert_eq!(max, n - 1);
    let per_step = took / layers.work.max(1) as f64;
    println!(
        "LAYERS-TIMING {n} layers: {} steps in {took:.3} s, {:.1} ns a step, the cap of {} steps about {:.1} s",
        layers.work,
        per_step * 1e9,
        craft3d_kernel::origami::MAX_LAYER_WORK,
        per_step * craft3d_kernel::origami::MAX_LAYER_WORK as f64
    );
}

/// A stack folded on (docs/design/origami.md › *A stack folded on*): a
/// square folded in half by a valley at 180 degrees, then the doubled
/// layers folded by a crossing crease. The crossing crease is two lines
/// meeting at the middle, a valley on the half that stays face up and a
/// mountain on the half that lies face down (Maekawa at the middle: one
/// mountain, three valleys). Given the valley, the mountain is solved to
/// the same angle; the two layers of the folded part lie on each other
/// at (−s, s cos ρ, s sin ρ) whichever corner, the area is kept, and the
/// layers are the flat state's within each stack. A crossing crease on
/// one layer only cannot fold the stack; a crossing crease of the wrong
/// kind turns the wrong way.
#[test]
fn a_stack_folded_in_half_is_folded_on_by_a_crossing_crease() {
    use craft3d_kernel::origami::{fold_stacked, folds_stacked};
    for scale in SCALES {
        let s = scale;
        let t = 1e-9 * scale;
        let boundary = polygon(&[[-s, -s], [s, -s], [s, s], [-s, s]]);
        let half = crease(10, [0.0, -s], [0.0, s], CreaseKind::Valley);
        let left = crease(11, [-s, 0.0], [0.0, 0.0], CreaseKind::Valley);
        let right = crease(12, [0.0, 0.0], [s, 0.0], CreaseKind::Mountain);
        let arr = arrange(&boundary, &[half.clone(), left.clone(), right.clone()], t).unwrap();
        assert_eq!(arr.facets.len(), 4);
        let at = |x: f64, y: f64| arr.facet_at([x * s, y * s]).unwrap();
        let (ul, ur, ll, lr) = (at(-0.5, 0.5), at(0.5, 0.5), at(-0.5, -0.5), at(0.5, -0.5));
        for degrees in [30.0f64, 90.0, 150.0] {
            let rho = degrees.to_radians();
            let angles: Vec<Angle> = arr
                .hinges
                .iter()
                .map(|h| match arr.creases[h.crease].id {
                    10 => Angle::Given(PI),
                    11 => Angle::Given(rho),
                    _ => Angle::Free { sign: -1.0 },
                })
                .collect();
            assert!(folds_stacked(&angles, ANGULAR));
            let stacked = fold_stacked(&arr, &angles, ll, t, ANGULAR).unwrap();
            let f = &stacked.folded;
            // The mountain solved to the same angle.
            for (h, a) in arr.hinges.iter().zip(&f.angles) {
                let want = match arr.creases[h.crease].id {
                    10 => PI,
                    11 => rho,
                    _ => -rho,
                };
                assert!((a - want).abs() <= 1e-9, "{degrees}: {a} vs {want}");
            }
            // Two stacks, the folded one's two corners on each other.
            assert_eq!(stacked.stack[ul], stacked.stack[ur]);
            assert_eq!(stacked.stack[ll], stacked.stack[lr]);
            assert_ne!(stacked.stack[ul], stacked.stack[ll]);
            let want = Vec3::new(-s, s * rho.cos(), s * rho.sin());
            for (facet, corner) in [(ul, [-s, s]), (ur, [s, s])] {
                let got = f.transforms[facet].point(Vec3::new(corner[0], corner[1], 0.0));
                assert!(
                    got.distance(want) <= 1e-9 * s,
                    "{degrees}: {got:?} vs {want:?}"
                );
            }
            // The fixed stack lies flat, its right half face down on it.
            let got = f.transforms[lr].point(Vec3::new(s, -s, 0.0));
            assert!(got.distance(Vec3::new(-s, -s, 0.0)) <= 1e-9 * s, "{got:?}");
            // Layers within each stack: the right halves above.
            assert_eq!(stacked.layers.layer[lr], stacked.layers.layer[ur]);
            assert!(stacked.layers.layer[lr] > stacked.layers.layer[ll]);
            assert_eq!(stacked.layers.above, {
                let mut v = vec![[lr, ll], [ur, ul]];
                v.sort_unstable();
                v
            });
            // No meeting other than along the crease both are folded about.
            assert!(
                craft3d_kernel::origami::first_meeting_stacked(
                    &arr,
                    f,
                    t,
                    ANGULAR,
                    &stacked.stack,
                    &stacked.bends
                )
                .unwrap()
                .is_none()
            );
            let made = folded_sheet(
                &arr,
                f,
                &Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap(),
                &Tolerance::new(t, ANGULAR).unwrap(),
            )
            .unwrap();
            let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
            assert!((area - 4.0 * s * s).abs() <= 1e-10 * s * s, "{area}");
        }
        // The crossing crease on one layer only: one stack, the crease
        // inside it cannot turn.
        let one = arrange(&boundary, &[half.clone(), left.clone()], t).unwrap();
        let angles: Vec<Angle> = one
            .hinges
            .iter()
            .map(|h| match one.creases[h.crease].id {
                10 => Angle::Given(PI),
                _ => Angle::Given(PI / 2.0),
            })
            .collect();
        let e = fold_stacked(&one, &angles, 0, t, ANGULAR);
        assert!(
            matches!(e, Err(FoldError::NotClosing { id: 11, .. })),
            "{e:?}"
        );
        // Both halves of the crossing crease valleys: the solved one would
        // turn against its kind.
        let wrong = arrange(
            &boundary,
            &[
                half.clone(),
                left.clone(),
                crease(12, [0.0, 0.0], [s, 0.0], CreaseKind::Valley),
            ],
            t,
        )
        .unwrap();
        let angles: Vec<Angle> = wrong
            .hinges
            .iter()
            .map(|h| match wrong.creases[h.crease].id {
                10 => Angle::Given(PI),
                11 => Angle::Given(PI / 2.0),
                _ => Angle::Free { sign: 1.0 },
            })
            .collect();
        let e = fold_stacked(&wrong, &angles, 0, t, ANGULAR);
        assert!(e.is_err(), "{e:?}");
    }
}
