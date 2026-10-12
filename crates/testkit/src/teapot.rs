//! The Utah teapot: 32 bicubic Bézier patches.
//!
//! Dataset by Martin Newell (University of Utah, 1975), in its widely
//! published compact form: 10 patches over 127 control points, completed by
//! mirroring (rim, body, lid and bottom four times; handle and spout twice).
//! Z is up, the spout points along +X. See ATTRIBUTION.md.

use craft3d_geom::{BezierPatch, GeomError};
use craft3d_math::Vec3;

/// Which mirror copies complete a base patch.
#[derive(Clone, Copy)]
enum Symmetry {
    /// Mirrored in the XZ and YZ planes: four copies around the Z axis.
    Quarter,
    /// Mirrored in the XZ plane: two copies.
    Half,
}

use Symmetry::{Half, Quarter};

const PATCHES: [(Symmetry, [usize; 16]); 10] = [
    // rim
    (
        Quarter,
        [102, 103, 104, 105, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    ),
    // body
    (
        Quarter,
        [
            12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
        ],
    ),
    (
        Quarter,
        [
            24, 25, 26, 27, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
        ],
    ),
    // lid
    (
        Quarter,
        [
            96, 96, 96, 96, 97, 98, 99, 100, 101, 101, 101, 101, 0, 1, 2, 3,
        ],
    ),
    (
        Quarter,
        [
            0, 1, 2, 3, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117,
        ],
    ),
    // bottom
    (
        Quarter,
        [
            118, 118, 118, 118, 124, 122, 119, 121, 123, 126, 125, 120, 40, 39, 38, 37,
        ],
    ),
    // handle
    (
        Half,
        [
            41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56,
        ],
    ),
    (
        Half,
        [
            53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 28, 65, 66, 67,
        ],
    ),
    // spout
    (
        Half,
        [
            68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83,
        ],
    ),
    (
        Half,
        [
            80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95,
        ],
    ),
];

#[rustfmt::skip]
const POINTS: [[f64; 3]; 127] = [
    [0.2, 0.0, 2.7], [0.2, -0.112, 2.7], [0.112, -0.2, 2.7], [0.0, -0.2, 2.7],
    [1.3375, 0.0, 2.53125], [1.3375, -0.749, 2.53125], [0.749, -1.3375, 2.53125], [0.0, -1.3375, 2.53125],
    [1.4375, 0.0, 2.53125], [1.4375, -0.805, 2.53125], [0.805, -1.4375, 2.53125], [0.0, -1.4375, 2.53125],
    [1.5, 0.0, 2.4], [1.5, -0.84, 2.4], [0.84, -1.5, 2.4], [0.0, -1.5, 2.4],
    [1.75, 0.0, 1.875], [1.75, -0.98, 1.875], [0.98, -1.75, 1.875], [0.0, -1.75, 1.875],
    [2.0, 0.0, 1.35], [2.0, -1.12, 1.35], [1.12, -2.0, 1.35], [0.0, -2.0, 1.35],
    [2.0, 0.0, 0.9], [2.0, -1.12, 0.9], [1.12, -2.0, 0.9], [0.0, -2.0, 0.9],
    [-2.0, 0.0, 0.9],
    [2.0, 0.0, 0.45], [2.0, -1.12, 0.45], [1.12, -2.0, 0.45], [0.0, -2.0, 0.45],
    [1.5, 0.0, 0.225], [1.5, -0.84, 0.225], [0.84, -1.5, 0.225], [0.0, -1.5, 0.225],
    [1.5, 0.0, 0.15], [1.5, -0.84, 0.15], [0.84, -1.5, 0.15], [0.0, -1.5, 0.15],
    [-1.6, 0.0, 2.025], [-1.6, -0.3, 2.025], [-1.5, -0.3, 2.25], [-1.5, 0.0, 2.25],
    [-2.3, 0.0, 2.025], [-2.3, -0.3, 2.025], [-2.5, -0.3, 2.25], [-2.5, 0.0, 2.25],
    [-2.7, 0.0, 2.025], [-2.7, -0.3, 2.025], [-3.0, -0.3, 2.25], [-3.0, 0.0, 2.25],
    [-2.7, 0.0, 1.8], [-2.7, -0.3, 1.8], [-3.0, -0.3, 1.8], [-3.0, 0.0, 1.8],
    [-2.7, 0.0, 1.575], [-2.7, -0.3, 1.575], [-3.0, -0.3, 1.35], [-3.0, 0.0, 1.35],
    [-2.5, 0.0, 1.125], [-2.5, -0.3, 1.125], [-2.65, -0.3, 0.9375], [-2.65, 0.0, 0.9375],
    [-2.0, -0.3, 0.9], [-1.9, -0.3, 0.6], [-1.9, 0.0, 0.6],
    [1.7, 0.0, 1.425], [1.7, -0.66, 1.425], [1.7, -0.66, 0.6], [1.7, 0.0, 0.6],
    [2.6, 0.0, 1.425], [2.6, -0.66, 1.425], [3.1, -0.66, 0.825], [3.1, 0.0, 0.825],
    [2.3, 0.0, 2.1], [2.3, -0.25, 2.1], [2.4, -0.25, 2.025], [2.4, 0.0, 2.025],
    [2.7, 0.0, 2.4], [2.7, -0.25, 2.4], [3.3, -0.25, 2.4], [3.3, 0.0, 2.4],
    [2.8, 0.0, 2.475], [2.8, -0.25, 2.475], [3.525, -0.25, 2.49375], [3.525, 0.0, 2.49375],
    [2.9, 0.0, 2.475], [2.9, -0.15, 2.475], [3.45, -0.15, 2.5125], [3.45, 0.0, 2.5125],
    [2.8, 0.0, 2.4], [2.8, -0.15, 2.4], [3.2, -0.15, 2.4], [3.2, 0.0, 2.4],
    [0.0, 0.0, 3.15], [0.8, 0.0, 3.15], [0.8, -0.45, 3.15], [0.45, -0.8, 3.15], [0.0, -0.8, 3.15],
    [0.0, 0.0, 2.85],
    [1.4, 0.0, 2.4], [1.4, -0.784, 2.4], [0.784, -1.4, 2.4], [0.0, -1.4, 2.4],
    [0.4, 0.0, 2.55], [0.4, -0.224, 2.55], [0.224, -0.4, 2.55], [0.0, -0.4, 2.55],
    [1.3, 0.0, 2.55], [1.3, -0.728, 2.55], [0.728, -1.3, 2.55], [0.0, -1.3, 2.55],
    [1.3, 0.0, 2.4], [1.3, -0.728, 2.4], [0.728, -1.3, 2.4], [0.0, -1.3, 2.4],
    [0.0, 0.0, 0.0], [1.425, -0.798, 0.0], [1.5, 0.0, 0.075], [1.425, 0.0, 0.0],
    [0.798, -1.425, 0.0], [0.0, -1.5, 0.075], [0.0, -1.425, 0.0], [1.5, -0.84, 0.075],
    [0.84, -1.5, 0.075],
];

/// Number of patches [`teapot`] returns.
pub const PATCH_COUNT: usize = 32;

fn mirror_y(p: &BezierPatch) -> Result<BezierPatch, GeomError> {
    // A mirror flips the normal; reversing u flips it back.
    Ok(p.map_points(|q| Vec3::new(q.x, -q.y, q.z))?.reversed_u())
}

fn mirror_x(p: &BezierPatch) -> Result<BezierPatch, GeomError> {
    Ok(p.map_points(|q| Vec3::new(-q.x, q.y, q.z))?.reversed_u())
}

/// The teapot's 32 patches, all with outward normals. Order: rim (4),
/// body (8), lid (8), bottom (4), handle (4), spout (4).
pub fn teapot() -> Result<Vec<BezierPatch>, GeomError> {
    let mut patches = Vec::with_capacity(PATCH_COUNT);
    for (symmetry, indices) in PATCHES {
        let ctrl: Vec<Vec3> = indices
            .iter()
            .filter_map(|&k| POINTS.get(k))
            .map(|&[x, y, z]| Vec3::new(x, y, z))
            .collect();
        // The published rows run down the profile and the columns around the
        // axis, which makes ∂u × ∂v point inward; transposing fixes that.
        let base = BezierPatch::new(3, 3, ctrl)?.transposed();
        let mirrored = mirror_y(&base)?;
        if let Quarter = symmetry {
            patches.push(mirror_x(&base)?);
            patches.push(mirror_x(&mirrored)?);
        }
        patches.push(base);
        patches.push(mirrored);
    }
    Ok(patches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Bbox3;

    #[test]
    fn has_32_bicubic_patches_inside_the_known_bounds() {
        let patches = teapot().unwrap();
        assert_eq!(patches.len(), PATCH_COUNT);
        let mut bbox = Bbox3::EMPTY;
        for p in &patches {
            assert_eq!((p.degree_u(), p.degree_v()), (3, 3));
            for &c in p.control_points() {
                bbox.add(c);
            }
        }
        // Handle reaches x = -3, spout x = 3.525, body radius 2, lid knob z = 3.15.
        let (lo, hi) = bbox.corners().unwrap();
        assert_eq!(lo, Vec3::new(-3.0, -2.0, 0.0));
        assert_eq!(hi, Vec3::new(3.525, 2.0, 3.15));
    }

    #[test]
    fn landmark_points_are_where_the_teapot_has_them() {
        let patches = teapot().unwrap();
        let on_surface = |target: Vec3| {
            patches.iter().any(|p| {
                [(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (1.0, 1.0)]
                    .iter()
                    .any(|&(u, v)| p.eval(u, v).distance(target) < 1e-12)
            })
        };
        assert!(on_surface(Vec3::new(0.0, 0.0, 3.15)), "top of the lid knob");
        assert!(on_surface(Vec3::new(0.0, 0.0, 0.0)), "centre of the bottom");
        assert!(
            on_surface(Vec3::new(2.0, 0.0, 0.9)),
            "widest point of the body"
        );
        assert!(
            on_surface(Vec3::new(1.4, 0.0, 2.4)),
            "inner edge of the rim"
        );
        assert!(on_surface(Vec3::new(3.2, 0.0, 2.4)), "tip of the spout");
    }

    #[test]
    fn every_patch_has_an_outward_normal() {
        let patches = teapot().unwrap();
        // Patches whose outward side is unambiguous at their middle.
        for (k, p) in patches.iter().enumerate() {
            for (u, v) in [(0.5, 0.5), (0.25, 0.75), (0.8, 0.3)] {
                let pt = p.eval(u, v);
                let n = p.normal(u, v).unwrap();
                let outward = match k {
                    // Rim and upper lid (the knob) curl over: checked through
                    // their seams below.
                    0..4 | 12..16 => continue,
                    // The spout tip folds into the opening near its end,
                    // where outward means towards the XZ plane.
                    30..32 if v > 0.5 => continue,
                    // body: away from the axis.
                    4..12 => n.dot(Vec3::new(pt.x, pt.y, 0.0)) > 0.0,
                    // lower lid: upward.
                    16..20 => n.z > 0.0,
                    // bottom: downward.
                    20..24 => n.z < 0.0,
                    // handle and spout are tubes split along the XZ plane:
                    // away from that plane.
                    _ => n.y * pt.y > 0.0,
                };
                assert!(
                    outward,
                    "patch {k} at ({u}, {v}): point {pt:?}, normal {n:?}"
                );
            }
        }
        // The rim continues smoothly into the body, and the knob into the
        // lower lid: along the shared edge both sides have the same normal.
        for copy in 0..4 {
            for (a, b) in [(copy, 4 + copy), (12 + copy, 16 + copy)] {
                for i in 1..8 {
                    let u = f64::from(i) / 8.0;
                    let (na, nb) = (
                        patches[a].normal(u, 1.0).unwrap(),
                        patches[b].normal(u, 0.0).unwrap(),
                    );
                    assert!(
                        na.dot(nb) > 0.999,
                        "patches {a}/{b} at u = {u}: {na:?} vs {nb:?}"
                    );
                }
            }
        }
    }

    /// Regression: at the collapsed top edge of the lid the tangent along
    /// the edge is rounding noise, not zero; normalising it gave garbage
    /// normals (dark specks on the knob).
    #[test]
    fn the_lid_apex_normal_points_straight_up() {
        let patches = teapot().unwrap();
        for p in &patches[12..16] {
            for i in 0..=16 {
                let u = f64::from(i) / 16.0;
                assert!(p.eval(u, 0.0).distance(Vec3::new(0.0, 0.0, 3.15)) < 1e-12);
                let n = p.normal(u, 0.0).unwrap();
                assert!(n.distance(Vec3::Z) < 1e-5, "u = {u}: normal {n:?}");
            }
        }
    }

    #[test]
    fn is_symmetric_about_the_xz_plane() {
        let patches = teapot().unwrap();
        let samples = |p: &BezierPatch| -> Vec<Vec3> {
            (0..=4)
                .flat_map(|i| (0..=4).map(move |j| (f64::from(i) / 4.0, f64::from(j) / 4.0)))
                .map(|(u, v)| p.eval(u, v))
                .collect()
        };
        for p in &patches {
            let mirrored: Vec<Vec3> = samples(p)
                .iter()
                .map(|q| Vec3::new(q.x, -q.y, q.z))
                .collect();
            let has_twin = patches.iter().any(|other| {
                let pts = samples(other);
                mirrored
                    .iter()
                    .all(|m| pts.iter().any(|q| q.distance(*m) < 1e-12))
            });
            assert!(has_twin);
        }
    }

    #[test]
    fn neighbouring_patches_meet_without_gaps() {
        // Rim meets body, upper body meets lower body, lower lid meets upper lid.
        let patches = teapot().unwrap();
        let edge = |p: &BezierPatch, v: f64| -> Vec<Vec3> {
            (0..=8).map(|i| p.eval(f64::from(i) / 8.0, v)).collect()
        };
        let meets = |a: usize, b: usize| {
            let (pa, pb) = (&patches[a], &patches[b]);
            let (ea, eb) = (edge(pa, 1.0), edge(pb, 0.0));
            ea.iter().zip(&eb).all(|(x, y)| x.distance(*y) < 1e-12)
        };
        for copy in 0..4 {
            assert!(meets(copy, 4 + copy), "rim/body, copy {copy}");
            assert!(meets(4 + copy, 8 + copy), "body/body, copy {copy}");
            assert!(meets(12 + copy, 16 + copy), "lid/lid, copy {copy}");
        }
    }
}
