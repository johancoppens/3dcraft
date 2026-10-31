//! A section plane: a view that cuts the picture open (see
//! [`crate::render`], *Section*). Both renderers take the plane, its
//! tolerance and the rule for what is cut away from here, so a cut-open
//! model is the same model in both.

use craft3d_math::{Bbox3, Vec3};
use craft3d_tess::Mesh;

/// Id-buffer value of a pixel that shows the cap of a section: the flat
/// face where the plane passes through the inside of a body. No face of a
/// mesh has it.
pub const SECTION_FACE: u32 = u32::MAX - 1;

/// A triangle that is parallel to the plane (see [`SECTION_PARALLEL`])
/// lies in it where it is nearer to it than this fraction of the mesh's
/// size (the diagonal of the box around it). 2⁻¹⁴ is far above what a
/// position loses on its way to the GPU (2⁻²⁴ of the size per rounding)
/// and far below a pixel of any picture, so both renderers find the same
/// faces in the plane: such a face is kept or cut away as a whole, never
/// in speckles.
pub const SECTION_SNAP: f64 = 1.0 / 16384.0;

/// A triangle is parallel to the plane when the dot product of its unit
/// normal with the plane's is at least this, either way: within a third of
/// a degree. In single precision the product of two parallel unit vectors
/// is within 2⁻²² of 1.
pub const SECTION_PARALLEL: f64 = 1.0 - 1.0 / 65536.0;

/// The cap is drawn within the box around the mesh, grown by this fraction
/// of its diagonal. Inside a closed body the plane is always within that
/// box; the limit is there for meshes that are not closed, where the rule
/// that finds the cap would otherwise put one arbitrarily far away.
pub const SECTION_MARGIN: f64 = 1e-3;

/// A plane that cuts the picture: what lies on the side its normal points
/// to is not drawn. It is a view, not a change of the model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    /// A point of the plane, in model coordinates.
    pub origin: Vec3,
    /// Points into the half that is cut away; any length but zero.
    pub normal: Vec3,
    /// Close the cut with a flat face where the plane passes through the
    /// inside of a body. That is found by counting the surfaces a ray
    /// crosses, which is right for closed bodies only: a mesh of open
    /// sheets is cut without a cap.
    pub cap: bool,
    /// The colour of the cap, drawn as it is (not shaded).
    pub color: [u8; 3],
}

/// A [`Section`] made ready for one mesh: the unit normal, the tolerance
/// and the box the cap stays in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cut {
    pub origin: Vec3,
    /// Unit length, into the half that is cut away.
    pub normal: Vec3,
    /// A triangle parallel to the plane lies in it up to this distance
    /// ([`SECTION_SNAP`] times the mesh's size).
    pub snap: f64,
    pub cap: bool,
    pub color: [u8; 3],
    /// The corners of the box the cap is drawn in; `None` for an empty
    /// mesh, which has no cap.
    pub within: Option<(Vec3, Vec3)>,
}

/// The box around the triangles of `mesh` whose corners are all finite:
/// the mesh's size for [`Cut::new`], the same box the GPU renderer keeps
/// of what it draws.
pub fn section_bounds(mesh: &Mesh) -> Bbox3 {
    let mut bounds = Bbox3::EMPTY;
    for t in 0..mesh.triangles.len() {
        if let Some(corners) = mesh.corners(t)
            && corners.iter().all(|p| p.is_finite())
        {
            for p in corners {
                bounds.add(p);
            }
        }
    }
    bounds
}

impl Cut {
    /// `section` for a mesh within `bounds`. `None` for a plane that is
    /// none: a normal without direction, a number that is not finite. Such
    /// a section cuts nothing.
    pub fn new(section: &Section, bounds: &Bbox3) -> Option<Cut> {
        let normal = section.normal.normalized().filter(|n| n.is_finite())?;
        if !section.origin.is_finite() {
            return None;
        }
        let diagonal = bounds.diagonal();
        let diagonal = if diagonal.is_finite() { diagonal } else { 0.0 };
        let margin = Vec3::new(1.0, 1.0, 1.0) * (diagonal * SECTION_MARGIN);
        let within = bounds
            .corners()
            .map(|(lo, hi)| (lo - margin, hi + margin))
            .filter(|(lo, hi)| lo.is_finite() && hi.is_finite());
        Some(Cut {
            origin: section.origin,
            normal,
            snap: diagonal * SECTION_SNAP,
            cap: section.cap && within.is_some(),
            color: section.color,
            within,
        })
    }

    /// The distance of `p` from the plane, positive in the half that is
    /// cut away.
    pub fn side(&self, p: Vec3) -> f64 {
        (p - self.origin).dot(self.normal)
    }

    /// Whether a point of a surface is cut away: `side` is its distance
    /// from the plane as [`Cut::side`] gives it (interpolated across a
    /// triangle from its corners), `facing` the dot product of the
    /// triangle's own unit normal with the plane's.
    ///
    /// Beyond the plane it is. A triangle that lies in the plane (parallel
    /// to it by [`SECTION_PARALLEL`], and the point within [`Cut::snap`]
    /// of it) goes with its body: it is cut away when it faces away from
    /// the half that is cut, since its body then lies in that half
    /// (normals point out of a body). A number that is not one is kept.
    pub fn clips(&self, side: f64, facing: f64) -> bool {
        if side.abs() <= self.snap && facing.abs() >= SECTION_PARALLEL {
            facing < 0.0
        } else {
            side > 0.0
        }
    }

    /// True when `p` is in the box the cap is drawn in.
    pub fn holds_cap(&self, p: Vec3) -> bool {
        self.cap
            && self.within.is_some_and(|(lo, hi)| {
                p.x >= lo.x
                    && p.y >= lo.y
                    && p.z >= lo.z
                    && p.x <= hi.x
                    && p.y <= hi.y
                    && p.z <= hi.z
            })
    }

    /// The part of the segment from `a` to `b` that is not cut away, for
    /// the lines drawn over a picture (a line has no facing: on the plane
    /// it is kept). `None` when nothing is left.
    pub fn clip_segment(&self, a: Vec3, b: Vec3) -> Option<[Vec3; 2]> {
        let (sa, sb) = (self.side(a), self.side(b));
        match (sa > 0.0, sb > 0.0) {
            (false, false) => Some([a, b]),
            (true, true) => None,
            (cut_a, _) => {
                // One end on each side: `sa - sb` is not zero.
                let crossing = a + (b - a) * (sa / (sa - sb));
                if !crossing.is_finite() {
                    return None;
                }
                Some(if cut_a { [crossing, b] } else { [a, crossing] })
            }
        }
    }

    /// Where the plane crosses the surface of `mesh`: one segment per
    /// triangle that has a part on each side, and per triangle edge that
    /// lies in the plane with the rest of its triangle kept. For a closed
    /// body these are the outline of the cap.
    pub fn outline(&self, mesh: &Mesh) -> Vec<[Vec3; 2]> {
        let mut out = Vec::new();
        for t in 0..mesh.triangles.len() {
            let Some(corners) = mesh.corners(t) else {
                continue;
            };
            // For a line, a corner within the tolerance is on the plane.
            let sides = corners.map(|p| {
                let s = self.side(p);
                if s.abs() <= self.snap { 0.0 } else { s }
            });
            if !sides.iter().all(|s| s.is_finite()) {
                continue;
            }
            let kept = sides.iter().filter(|&&s| s < 0.0).count();
            let cut = sides.iter().filter(|&&s| s > 0.0).count();
            if kept == 0 || (cut == 0 && kept != 1) {
                // Nothing kept, or no edge in the plane.
                continue;
            }
            // The corners in the plane, and where an edge passes through it.
            let mut points: [Option<Vec3>; 2] = [None, None];
            let mut push = |p: Vec3| {
                if let Some(slot) = points.iter_mut().find(|slot| slot.is_none()) {
                    *slot = Some(p);
                }
            };
            for (k, (&p, &s)) in corners.iter().zip(&sides).enumerate() {
                if s == 0.0 {
                    push(p);
                }
                let next = (k + 1) % 3;
                if let (Some(&q), Some(&sq)) = (corners.get(next), sides.get(next))
                    && ((s < 0.0 && sq > 0.0) || (s > 0.0 && sq < 0.0))
                {
                    push(p + (q - p) * (s / (s - sq)));
                }
            }
            if let [Some(a), Some(b)] = points
                && a != b
                && a.is_finite()
                && b.is_finite()
            {
                out.push([a, b]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Portable;

    fn cut(origin: Vec3, normal: Vec3) -> Cut {
        let bounds = Bbox3::from_points([Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0)]);
        Cut::new(
            &Section {
                origin,
                normal,
                cap: true,
                color: [1, 2, 3],
            },
            &bounds,
        )
        .unwrap()
    }

    #[test]
    fn a_plane_that_is_none_cuts_nothing() {
        let bounds = Bbox3::from_points([Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)]);
        let section = |origin, normal| Section {
            origin,
            normal,
            cap: true,
            color: [0; 3],
        };
        for (origin, normal) in [
            (Vec3::ZERO, Vec3::ZERO),
            (Vec3::ZERO, Vec3::new(f64::NAN, 0.0, 1.0)),
            (Vec3::ZERO, Vec3::new(f64::INFINITY, 0.0, 0.0)),
            (Vec3::new(0.0, f64::NAN, 0.0), Vec3::Z),
            (Vec3::new(f64::INFINITY, 0.0, 0.0), Vec3::Z),
        ] {
            assert_eq!(Cut::new(&section(origin, normal), &bounds), None);
        }
        // Any length of normal is the same plane; an empty mesh has no cap.
        let long = Cut::new(&section(Vec3::ZERO, Vec3::Z * 1e9), &bounds).unwrap();
        assert_eq!(long.normal, Vec3::Z);
        assert!(long.cap);
        let empty = Cut::new(&section(Vec3::ZERO, Vec3::Z), &Bbox3::EMPTY).unwrap();
        assert!(!empty.cap);
        assert_eq!(empty.snap, 0.0);
    }

    #[test]
    fn a_face_in_the_plane_goes_with_its_body_and_everything_else_by_its_side() {
        let cut = cut(Vec3::ZERO, Vec3::Z);
        assert_eq!(cut.side(Vec3::new(5.0, 5.0, 0.25)), 0.25);
        assert_eq!(cut.side(Vec3::new(5.0, 5.0, -0.25)), -0.25);
        assert!(cut.clips(0.25, 1.0));
        assert!(cut.clips(0.25, -1.0));
        assert!(!cut.clips(-0.25, -1.0));
        // The box has a diagonal of 2√3: a parallel face within 2⁻¹⁴ of
        // that lies in the plane. The top of a body below it stays, the
        // bottom of a body above it goes, whichever side rounding put it.
        for side in [0.0, 1e-4, -1e-4] {
            assert!(!cut.clips(side, 1.0));
            assert!(cut.clips(side, -1.0));
        }
        assert!(cut.clips(3e-4, 1.0));
        assert!(!cut.clips(-3e-4, -1.0));
        // A face that crosses the plane is cut where it crosses it.
        assert!(cut.clips(1e-6, 0.5));
        assert!(!cut.clips(-1e-6, 0.5));
        assert!(!cut.clips(0.0, 0.5));
        assert!(!cut.clips(f64::NAN, 1.0));
        assert!(!cut.clips(0.0, f64::NAN));
    }

    #[test]
    fn a_segment_keeps_its_part_below_the_plane() {
        let cut = cut(Vec3::new(0.0, 0.0, 0.5), Vec3::Z);
        let (low, high) = (Vec3::new(0.0, 0.0, -0.5), Vec3::new(2.0, 0.0, 1.5));
        assert_eq!(
            cut.clip_segment(low, low + Vec3::X),
            Some([low, low + Vec3::X])
        );
        assert_eq!(cut.clip_segment(high, high + Vec3::X), None);
        let crossing = Vec3::new(1.0, 0.0, 0.5);
        assert_eq!(cut.clip_segment(low, high), Some([low, crossing]));
        assert_eq!(cut.clip_segment(high, low), Some([crossing, low]));
        // A segment in the plane is kept; one that is not a number is kept
        // as it is (whoever draws it leaves it out).
        let on = Vec3::new(0.0, 0.0, 0.5);
        assert_eq!(cut.clip_segment(on, on + Vec3::X), Some([on, on + Vec3::X]));
        let nan = Vec3::new(f64::NAN, 0.0, 0.0);
        assert!(cut.clip_segment(nan, low).is_some());
    }

    #[test]
    fn the_outline_of_a_tetrahedron_is_a_triangle() {
        let mesh = Mesh {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
            normals: vec![Vec3::ZERO; 4],
            triangles: vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]],
            face_ids: vec![0, 1, 2, 3],
        };
        let bounds = section_bounds(&mesh);
        let at = |z: f64| {
            Cut::new(
                &Section {
                    origin: Vec3::new(0.0, 0.0, z),
                    normal: Vec3::Z,
                    cap: true,
                    color: [0; 3],
                },
                &bounds,
            )
            .unwrap()
        };
        // Half-way up: three segments of a triangle with corners at half
        // the base's, each of the three sides cut once.
        let outline = at(0.5).outline(&mesh);
        assert_eq!(outline.len(), 3);
        let length: f64 = outline.iter().map(|[a, b]| a.distance(*b)).sum();
        assert!((length - (0.5 + 0.5 + 0.5_f64.phypot(0.5))).abs() < 1e-12);
        assert!(outline.iter().flatten().all(|p| (p.z - 0.5).abs() < 1e-12));
        // Through the base, everything above it cut away: nothing is kept.
        assert_eq!(at(0.0).outline(&mesh).len(), 0);
        // Through the base, what is above it kept: the three edges of the
        // base, once each, from the sides that stand on them (the base
        // itself lies in the plane).
        let below = |z: f64| Cut {
            normal: -Vec3::Z,
            ..at(z)
        };
        let outline = below(0.0).outline(&mesh);
        assert_eq!(outline.len(), 3);
        let length: f64 = outline.iter().map(|[a, b]| a.distance(*b)).sum();
        assert!((length - (2.0 + 2.0_f64.sqrt())).abs() < 1e-12);
        // At the apex: one point, no segment.
        assert_eq!(below(1.0).outline(&mesh).len(), 0);
        // Above everything and below everything: nothing.
        assert!(at(2.0).outline(&mesh).is_empty());
        assert!(at(-2.0).outline(&mesh).is_empty());
    }
}
