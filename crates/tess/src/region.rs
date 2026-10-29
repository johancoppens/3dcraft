//! The triangles of a planar region, for callers outside the tessellator
//! that need a mesh of a region of the plane (the finite elements of a
//! beam's section, `craft3d_kernel::below::torsion`): the constrained
//! Delaunay triangulation of `cdt.rs`, with its guarantees (every
//! triangle certainly counter-clockwise with non-zero area, boundary
//! points and segments kept as given, the region by the even–odd rule).

use crate::cdt::{CdtError, triangulate_with_clearance};
use std::fmt;

/// A triangulated region of the plane.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionMesh {
    /// The distinct boundary points, then the interior points kept.
    pub points: Vec<(f64, f64)>,
    /// Counter-clockwise index triples into `points`.
    pub triangles: Vec<[usize; 3]>,
    /// The index in `points` of each boundary point given.
    pub boundary: Vec<usize>,
    /// The index in `points` of each interior point given, `None` for one
    /// that was skipped (outside the region, nearer to a segment than its
    /// clearance, or on top of another point).
    pub interior: Vec<Option<usize>>,
}

/// Why a region could not be triangulated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionError(&'static str);

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for RegionError {}

/// The region bounded by the closed chains `segments` (index pairs into
/// `points`), by the even–odd rule, so a hole needs no bridge: its
/// triangles, with the interior points that lie inside it at least their
/// `clearance` (by place; missing ones are 0) from every segment. The
/// boundary is kept exactly: its points are never moved and its segments
/// never split. Segments that cross, chains that do not close and points
/// rounding cannot tell apart are errors, never a bad mesh.
pub fn triangulate_region(
    points: &[(f64, f64)],
    segments: &[(usize, usize)],
    interior: &[(f64, f64)],
    clearance: &[f64],
) -> Result<RegionMesh, RegionError> {
    let found = triangulate_with_clearance(points, segments, interior, clearance).map_err(|e| {
        RegionError(match e {
            CdtError::NonFinite => "a point is not finite",
            CdtError::Degenerate => {
                "points coincide or are collinear within rounding, or the region has no area"
            }
            CdtError::Crossing => "two boundary segments cross",
            CdtError::Open => "the boundary segments do not form closed chains",
            CdtError::TooMany => "more points than the triangulation takes",
            CdtError::Internal => "the triangulation's bookkeeping is inconsistent",
            CdtError::TooSlow => "the triangulation took more work than its points justify",
        })
    })?;
    Ok(RegionMesh {
        points: found.points,
        triangles: found.triangles,
        boundary: found.map,
        interior: found.kept,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square with a square hole: every triangle counter-clockwise, the
    /// area the square's less the hole's, an interior point in the hole
    /// skipped and one in the material kept.
    #[test]
    fn a_square_with_a_hole_is_tiled() {
        let points = [
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 4.0),
            (0.0, 4.0),
            (1.0, 1.0),
            (3.0, 1.0),
            (3.0, 3.0),
            (1.0, 3.0),
        ];
        let segments = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
        ];
        let mesh = triangulate_region(&points, &segments, &[(2.0, 2.0), (0.5, 2.0)], &[]).unwrap();
        let mut area = 0.0;
        for t in &mesh.triangles {
            let [a, b, c] = t.map(|i| mesh.points[i]);
            let twice = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            assert!(twice > 0.0);
            area += 0.5 * twice;
        }
        assert!((area - 12.0).abs() < 1e-12, "{area}");
        assert_eq!(mesh.interior[0], None);
        assert!(mesh.interior[1].is_some());
        assert_eq!(mesh.boundary.len(), 8);
    }

    #[test]
    fn crossing_segments_are_refused() {
        let points = [(0.0, 0.0), (1.0, 1.0), (1.0, 0.0), (0.0, 1.0)];
        let segments = [(0, 1), (1, 2), (2, 3), (3, 0)];
        assert!(triangulate_region(&points, &segments, &[], &[]).is_err());
        assert!(triangulate_region(&[(f64::NAN, 0.0)], &[], &[], &[]).is_err());
        assert!(triangulate_region(&[], &[], &[], &[]).is_err());
    }
}
