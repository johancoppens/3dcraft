//! A section plane: a view that cuts the picture open (see
//! [`crate::render`], *Section*). Both renderers take the plane, its
//! tolerance and the rule for what is cut away from here, so a cut-open
//! model is the same model in both.

use crate::{Camera, Projection};
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
    /// The colour of the cap, drawn as it is (not shaded): its paper when
    /// it is hatched.
    pub color: [u8; 3],
    /// Lines of ink on the cap, fixed to the model ([`Hatch`]); `None`:
    /// the cap is one flat colour.
    pub hatch: Option<Hatch>,
}

/// How the cap of a section is hatched: lines of `ink` at 45° on the
/// paper ([`Section::color`]), drawn as a function of the point of the
/// plane in model space, so they move with the part and not with the
/// screen (docs/design/rendering.md › *The cut's hatching*).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hatch {
    /// The colour of the lines.
    pub ink: [u8; 3],
    /// The lines stand at most this many points apart at the camera's
    /// target, and more than half as many: their spacing is the largest
    /// power of two of the model unit within it ([`hatch_spacing`]).
    pub spacing_points: f64,
    /// How wide a line is, in points.
    pub width_points: f64,
    /// The height of the picture in points (the window's view); `None`
    /// takes a pixel of the picture for a point (`view.render`). A picture
    /// of another size of the same view keeps its lines on the same places
    /// of the model.
    pub height_points: Option<f64>,
}

/// The plane's frame for its unit normal `n`, in which the hatching is
/// drawn: `U` the world axis most square to `n` (the least `|n·axis|`; of
/// equals the first of X, Y, Z), made square to it, and `V = n × U`.
/// `None` for a normal that is not a unit vector's direction.
pub fn hatch_frame(n: Vec3) -> Option<(Vec3, Vec3)> {
    let mut best = Vec3::X;
    let mut least = f64::INFINITY;
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        let along = n.dot(axis).abs();
        if along < least {
            least = along;
            best = axis;
        }
    }
    let u = (best - n * best.dot(n)).normalized()?;
    let v = n.cross(u).normalized()?;
    (u.is_finite() && v.is_finite()).then_some((u, v))
}

/// The spacing of the lines for `span` model units (the most the spacing
/// may be): the largest power of two not greater than it, `2^k` for any
/// integer `k`. Exact: the number's exponent with its fraction cleared.
/// `None` for a span that is not a positive normal number.
pub fn hatch_spacing(span: f64) -> Option<f64> {
    (span.is_finite() && span >= f64::MIN_POSITIVE)
        .then(|| f64::from_bits(span.to_bits() & 0xfff0_0000_0000_0000))
}

/// The distance of a point from the nearest line, in model units: the
/// lines lie at `w = k · spacing` for every integer `k`, `w` the point's
/// coordinate across them, `(u + v) / √2`.
pub fn hatch_distance(w: f64, spacing: f64) -> f64 {
    (w - spacing * (w / spacing).round()).abs()
}

/// How much of a pixel a line covers, 0 to 1: `d` the distance of the
/// pixel's point from the line and `half_width` half the line's width,
/// both in pixels. Worked out, not sampled: `0.5 + half_width - d`,
/// clamped, a ramp one pixel wide across the line's edge. A number that
/// is not one covers nothing.
pub fn hatch_coverage(d: f64, half_width: f64) -> f64 {
    let c = 0.5 + half_width - d;
    if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) }
}

/// [`Hatch`] resolved for one picture: the plane's frame, the spacing for
/// the camera, and the size of a pixel. Both renderers make it here, so
/// they draw the same lines (`craft3d_gpu` hands its numbers to the cap's
/// shaders).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hatching {
    /// The plane's frame ([`hatch_frame`]).
    pub u: Vec3,
    pub v: Vec3,
    /// How far apart the lines are, in model units: a power of two.
    pub spacing: f64,
    /// Half a line's width, in pixels of the picture.
    pub half_width: f64,
    /// The model units one pixel covers: at distance 1 along the view in
    /// perspective (times a point's own distance), anywhere in an
    /// orthographic view.
    pub pixel: f64,
    pub perspective: bool,
    pub paper: [u8; 3],
    pub ink: [u8; 3],
}

impl Hatching {
    /// The hatching of a cap of `paper` with `hatch` on the plane of unit
    /// normal `normal`, in a picture `height` pixels high seen with
    /// `camera`. The spacing is the largest power of two of the model unit
    /// within `hatch.spacing_points` points at the camera's target: the
    /// model units a point covers there are the view's height there over
    /// the picture's height in points. Orbit and pan keep it; zoom changes
    /// it in steps of two, and every line at one spacing is a line at half
    /// of it. `None` when there is nothing to hatch: a camera that looks
    /// nowhere, sizes that are not numbers.
    pub fn new(
        normal: Vec3,
        paper: [u8; 3],
        hatch: &Hatch,
        camera: &Camera,
        height: u32,
    ) -> Option<Hatching> {
        let (u, v) = hatch_frame(normal)?;
        let axes = camera.axes().ok()?;
        let pixels = f64::from(height.max(1));
        let points = hatch
            .height_points
            .filter(|p| p.is_finite() && *p > 0.0)
            .unwrap_or(pixels);
        let perspective = matches!(axes.projection, Projection::Perspective { .. });
        let pixel = 2.0 * axes.half / pixels;
        // How far the target lies along the view: the scale of the
        // picture there.
        let at_target = if perspective {
            (camera.target - axes.eye).dot(axes.forward)
        } else {
            1.0
        };
        let per_point = pixel * at_target * pixels / points;
        let spacing = hatch_spacing(hatch.spacing_points * per_point)?;
        let half_width = 0.5 * hatch.width_points * pixels / points;
        (pixel.is_finite() && pixel > 0.0 && half_width.is_finite()).then_some(Hatching {
            u,
            v,
            spacing,
            half_width,
            pixel,
            perspective,
            paper,
            ink: hatch.ink,
        })
    }

    /// How much ink covers the pixel that shows the point `p` of the plane
    /// at `distance` along the view, 0 to 1 ([`hatch_coverage`]).
    pub fn coverage(&self, p: Vec3, distance: f64) -> f64 {
        let w = (p.dot(self.u) + p.dot(self.v)) * std::f64::consts::FRAC_1_SQRT_2;
        let d = hatch_distance(w, self.spacing);
        let q = if self.perspective {
            self.pixel * distance
        } else {
            self.pixel
        };
        if !(q.is_finite() && q > 0.0) {
            return 0.0;
        }
        hatch_coverage(d / q, self.half_width)
    }

    /// The cap's colour at `p`: the paper mixed with the ink by
    /// [`Self::coverage`], rounded per channel. Unshaded.
    pub fn color(&self, p: Vec3, distance: f64) -> [u8; 3] {
        let c = self.coverage(p, distance);
        let mut out = self.paper;
        for (channel, ink) in out.iter_mut().zip(self.ink) {
            let paper = f64::from(*channel);
            // Between two bytes: within 0..=255.
            *channel = (paper + (f64::from(ink) - paper) * c)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
        out
    }
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
        self.outline_among(mesh, 0..mesh.triangles.len())
    }

    /// [`Cut::outline`], the same segments in the same order, looked for
    /// only among the triangles `slabs` says the plane can cross, when the
    /// slabs were made for `mesh` along this plane's normal (either way);
    /// among all of them otherwise. A plane dragged along its normal finds
    /// its outline among a few triangles, not the whole mesh.
    pub fn outline_in(&self, mesh: &Mesh, slabs: &Slabs) -> Vec<[Vec3; 2]> {
        match slabs.crossing(self, mesh) {
            Some(triangles) => self.outline_among(mesh, triangles.into_iter().map(|t| t as usize)),
            None => self.outline(mesh),
        }
    }

    /// The segments of [`Cut::outline`] of the triangles `triangles`, in
    /// their order.
    fn outline_among(&self, mesh: &Mesh, triangles: impl Iterator<Item = usize>) -> Vec<[Vec3; 2]> {
        let mut out = Vec::new();
        for t in triangles {
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

/// Most slabs a mesh is sorted into, and how many triangles a slab holds
/// on average at least ([`Slabs::new`]).
const MAX_SLABS: usize = 1 << 16;
const PER_SLAB: usize = 32;
/// A triangle that reaches across more slabs than this is kept on a list
/// of its own, looked at for every plane: one long side of a cylinder's
/// wall would otherwise be in every slab.
const MAX_SPAN: usize = 4;

/// The triangles of a mesh sorted by where they lie along a direction:
/// which of them a plane square to it can cross at a given place. Made
/// once per mesh and direction (a pass over the triangles), it lets a
/// plane moved along its normal find its outline ([`Cut::outline_in`])
/// among the triangles near it.
#[derive(Clone, Debug, PartialEq)]
pub struct Slabs {
    base: Vec3,
    along: Vec3,
    /// How many triangles the mesh had.
    triangles: usize,
    /// Where the first slab starts along `along` from `base`, and how
    /// thick each is.
    start: f64,
    thickness: f64,
    /// The triangles of slab `k` are `items[starts[k]..starts[k + 1]]`,
    /// ascending.
    starts: Vec<u32>,
    items: Vec<u32>,
    /// The triangles that reach across more than [`MAX_SPAN`] slabs,
    /// ascending.
    wide: Vec<u32>,
    /// The least and greatest place of a corner of any triangle.
    reach: [f64; 2],
}

impl Slabs {
    /// The triangles of `mesh` sorted along the unit direction `along`,
    /// places measured from `base`. `None` for a direction or a point that
    /// is not one, or a mesh of more triangles than an index can name.
    pub fn new(mesh: &Mesh, base: Vec3, along: Vec3) -> Option<Slabs> {
        let along = along.normalized().filter(|a| a.is_finite())?;
        if !base.is_finite() || u32::try_from(mesh.triangles.len()).is_err() {
            return None;
        }
        // Where each triangle lies along `along`: NaN for one that has no
        // place (a corner that is missing or not finite), which no plane
        // crosses (`Cut::outline` leaves it out).
        let spans: Vec<[f64; 2]> = (0..mesh.triangles.len())
            .map(|t| match mesh.corners(t) {
                Some(corners) if corners.iter().all(|p| p.is_finite()) => {
                    let [a, b, c] = corners.map(|p| (p - base).dot(along));
                    [a.min(b).min(c), a.max(b).max(c)]
                }
                _ => [f64::NAN; 2],
            })
            .collect();
        let (lo, hi) = spans
            .iter()
            .filter(|[a, _]| a.is_finite())
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), [a, b]| {
                (lo.min(*a), hi.max(*b))
            });
        // Slabs about as thick as the triangles are long along `along`
        // (nine in ten no longer), so that most lie in one or two; never
        // more slabs than the triangles fill.
        let mut lengths: Vec<f64> = spans
            .iter()
            .filter(|[a, _]| a.is_finite())
            .map(|[a, b]| b - a)
            .collect();
        let long = if lengths.is_empty() {
            0.0
        } else {
            let k = (lengths.len() * 9 / 10).min(lengths.len() - 1);
            *lengths.select_nth_unstable_by(k, f64::total_cmp).1
        };
        let range = hi - lo;
        let most = (mesh.triangles.len() / PER_SLAB).clamp(1, MAX_SLABS);
        let wanted = if long > 0.0 {
            range / long
        } else {
            most as f64
        };
        let count = if wanted.is_finite() {
            (wanted.ceil().max(1.0) as usize).min(most)
        } else {
            1
        };
        let thickness = range / count as f64;
        let usable = lo.is_finite() && hi.is_finite() && thickness > 0.0 && thickness.is_finite();
        let count = if usable { count } else { 1 };
        let slab = |s: f64| -> usize {
            if usable {
                (((s - lo) / thickness).floor().max(0.0) as usize).min(count - 1)
            } else {
                0
            }
        };
        let mut sizes = vec![0_u32; count + 1];
        let mut wide = Vec::new();
        for (t, &[a, b]) in spans.iter().enumerate() {
            if !a.is_finite() {
                continue;
            }
            let (first, last) = (slab(a), slab(b));
            if last - first > MAX_SPAN {
                wide.push(t as u32);
                continue;
            }
            for k in first..=last {
                if let Some(size) = sizes.get_mut(k + 1) {
                    *size += 1;
                }
            }
        }
        // Running totals: where each slab's triangles start.
        for k in 1..sizes.len() {
            let before = sizes.get(k - 1).copied().unwrap_or(0);
            if let Some(size) = sizes.get_mut(k) {
                *size = size.saturating_add(before);
            }
        }
        let starts = sizes;
        let mut items = vec![0_u32; starts.last().copied().unwrap_or(0) as usize];
        let mut filled = starts.clone();
        for (t, &[a, b]) in spans.iter().enumerate() {
            if !a.is_finite() {
                continue;
            }
            let (first, last) = (slab(a), slab(b));
            if last - first > MAX_SPAN {
                continue;
            }
            for k in first..=last {
                if let Some(at) = filled.get_mut(k)
                    && let Some(item) = items.get_mut(*at as usize)
                {
                    *item = t as u32;
                    *at += 1;
                }
            }
        }
        Some(Slabs {
            base,
            along,
            triangles: mesh.triangles.len(),
            start: lo,
            thickness: if usable { thickness } else { 0.0 },
            starts,
            items,
            wide,
            reach: [lo, hi],
        })
    }

    /// The triangles of `mesh` that `cut` can cross, ascending, each
    /// once: those that reach within its tolerance of it, and a few more.
    /// `None` when these slabs were not made for `mesh`, or not along the
    /// cut's normal exactly (either way round): a direction off by the
    /// least turns the cut's distances across the mesh by its sine times
    /// the mesh's reach from the cut's point, which the slabs do not
    /// allow for, so such a cut is outlined over the whole mesh.
    fn crossing(&self, cut: &Cut, mesh: &Mesh) -> Option<Vec<u32>> {
        let along = cut.normal == self.along || cut.normal == Vec3::ZERO - self.along;
        if mesh.triangles.len() != self.triangles || !along {
            return None;
        }
        let at = (cut.origin - self.base).dot(self.along);
        let [lo, hi] = self.reach;
        // The tolerance of the cut, and rounding at the mesh's size: a
        // superset of the triangles that give a segment is enough.
        let slack = cut.snap + 1e-9 * (at.abs() + (hi - lo).abs() + 1.0);
        if !(at.is_finite() && slack.is_finite()) {
            return None;
        }
        if at + slack < lo || at - slack > hi {
            return Some(Vec::new());
        }
        let last = self.starts.len().saturating_sub(2);
        let slab = |s: f64| -> usize {
            if self.thickness > 0.0 {
                (((s - self.start) / self.thickness).floor().max(0.0) as usize).min(last)
            } else {
                0
            }
        };
        let mut out: Vec<u32> = self.wide.clone();
        for k in slab(at - slack)..=slab(at + slack) {
            let (Some(&from), Some(&to)) = (self.starts.get(k), self.starts.get(k + 1)) else {
                continue;
            };
            if let Some(items) = self.items.get(from as usize..to as usize) {
                out.extend_from_slice(items);
            }
        }
        out.sort_unstable();
        out.dedup();
        Some(out)
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
                hatch: None,
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
            hatch: None,
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
                    hatch: None,
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

    const PAPER: [u8; 3] = [232, 228, 220];
    const INK: [u8; 3] = [88, 84, 78];

    fn hatch(height_points: Option<f64>) -> Hatch {
        Hatch {
            ink: INK,
            spacing_points: 9.0,
            width_points: 1.0,
            height_points,
        }
    }

    /// Looking down at the plane z = 0 from 100 above, `half_height`
    /// units above and below the target.
    fn from_above(half_height: f64) -> Camera {
        Camera {
            eye: Vec3::new(0.0, 0.0, 100.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            projection: Projection::Orthographic { half_height },
        }
    }

    #[test]
    fn a_point_on_a_line_is_ink_and_half_a_spacing_off_is_paper() {
        // 100 units over 400 pixels: a quarter of a unit per pixel, 2.25
        // units in 9 points: the lines stand 2 apart.
        let h = Hatching::new(Vec3::Z, PAPER, &hatch(None), &from_above(50.0), 400).unwrap();
        assert_eq!(h.spacing, 2.0);
        assert_eq!(h.half_width, 0.5);
        // On the plane z = 0: U is X (X and Y are both square to the
        // normal; X comes first), V is Z × X = Y.
        assert_eq!((h.u, h.v), (Vec3::X, Vec3::Y));
        // w = (x + y) / √2: the point (√2, √2) lies on the line w = 2.
        let r = std::f64::consts::SQRT_2;
        for k in [-3.0, -1.0, 0.0, 1.0, 5.0] {
            let on = Vec3::new(k * r, k * r, 0.0);
            assert_eq!(h.color(on, 100.0), INK, "{k}");
            assert!((h.coverage(on, 100.0) - 1.0).abs() < 1e-9);
            // Half a spacing off, w = 2k + 1: paper.
            let off = Vec3::new((k + 0.5) * r, (k + 0.5) * r, 0.0);
            assert_eq!(h.color(off, 100.0), PAPER, "{k}");
            assert_eq!(h.coverage(off, 100.0), 0.0);
        }
        // Along a line (u up, v down by as much) nothing changes.
        let along = Vec3::new(r + 7.25, r - 7.25, 0.0);
        assert_eq!(h.color(along, 100.0), INK);
        // A line one point wide: a ramp one pixel wide on either side of
        // its edge, half a pixel from its middle. A quarter of a pixel off
        // is three quarters ink, one pixel off a quarter.
        let w_off = |pixels: f64| {
            let w = 2.0 + pixels * 0.25;
            Vec3::new(w / r, w / r, 0.0)
        };
        assert!((h.coverage(w_off(0.25), 100.0) - 0.75).abs() < 1e-9);
        assert!((h.coverage(w_off(1.0), 100.0) - 0.0).abs() < 1e-9);
        assert!((h.coverage(w_off(-0.5), 100.0) - 0.5).abs() < 1e-9);
        // The paper and the ink mixed, rounded per channel.
        assert_eq!(h.color(w_off(0.5), 100.0), [160, 156, 149]);
    }

    #[test]
    fn the_spacing_is_the_largest_power_of_two_within_nine_points() {
        for (span, spacing) in [
            (1.0, 1.0),
            (1.5, 1.0),
            (0.75, 0.5),
            (2.25, 2.0),
            (1e6, 524_288.0),
            (3e-7, 1.0 / 4_194_304.0),
        ] {
            assert_eq!(hatch_spacing(span), Some(spacing), "{span}");
        }
        for span in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-310] {
            assert_eq!(hatch_spacing(span), None, "{span}");
        }
        // Three zooms of the same view: a quarter, a half and a whole unit
        // per point at the target give 2, 4 and 8; a view a thousand times
        // smaller 2⁻⁹.
        for (half_height, spacing) in [(50.0, 2.0), (100.0, 4.0), (200.0, 8.0), (0.05, 1.0 / 512.0)]
        {
            let h =
                Hatching::new(Vec3::Z, PAPER, &hatch(None), &from_above(half_height), 400).unwrap();
            assert_eq!(h.spacing, spacing, "{half_height}");
        }
        // In perspective, by the target's distance: 90° at 100 from the
        // target over 400 pixels is half a unit per pixel there.
        let camera = Camera {
            projection: Projection::Perspective { fov_y_deg: 90.0 },
            ..from_above(1.0)
        };
        let h = Hatching::new(Vec3::Z, PAPER, &hatch(None), &camera, 400).unwrap();
        assert_eq!(h.spacing, 4.0);
        assert!(h.perspective);
        // A point twice as far covers twice as much per pixel: the same
        // offset from a line is half as many pixels.
        let r = std::f64::consts::SQRT_2;
        let near = Vec3::new(2.0 * r + 0.1, 2.0 * r, 0.0);
        let w = (near.x + near.y) * std::f64::consts::FRAC_1_SQRT_2;
        let d = hatch_distance(w, 4.0);
        assert!((h.coverage(near, 100.0) - hatch_coverage(d / 0.5, 0.5)).abs() < 1e-9);
        assert!((h.coverage(near, 200.0) - hatch_coverage(d / 1.0, 0.5)).abs() < 1e-9);
        // The picture's height in points, when it is not its pixels: 400
        // pixels as 200 points is twice the units per point and a line
        // two pixels wide.
        let h = Hatching::new(Vec3::Z, PAPER, &hatch(Some(200.0)), &from_above(50.0), 400).unwrap();
        assert_eq!(h.spacing, 4.0);
        assert_eq!(h.half_width, 1.0);
        // The same view in a picture of twice the pixels and the same
        // points: the same lines.
        let big =
            Hatching::new(Vec3::Z, PAPER, &hatch(Some(200.0)), &from_above(50.0), 800).unwrap();
        assert_eq!(big.spacing, 4.0);
        assert_eq!(big.half_width, 2.0);
    }

    #[test]
    fn every_line_at_twice_the_spacing_is_a_line_at_the_spacing() {
        let spacing = 0.125;
        for k in -40..=40 {
            let w = f64::from(k) * 2.0 * spacing;
            assert_eq!(hatch_distance(w, 2.0 * spacing), 0.0, "{k}");
            assert_eq!(hatch_distance(w, spacing), 0.0, "{k}");
            // Between two lines at twice the spacing lies one at the
            // spacing, none at twice it.
            let between = w + spacing;
            assert_eq!(hatch_distance(between, spacing), 0.0, "{k}");
            assert_eq!(hatch_distance(between, 2.0 * spacing), spacing, "{k}");
        }
    }

    #[test]
    fn the_planes_frame_is_square_and_starts_from_the_squarest_axis() {
        assert_eq!(hatch_frame(Vec3::Z), Some((Vec3::X, Vec3::Y)));
        // Square to X: Y and Z tie, Y comes first.
        assert_eq!(hatch_frame(Vec3::X), Some((Vec3::Y, Vec3::Z)));
        assert_eq!(hatch_frame(-Vec3::Z), Some((Vec3::X, -Vec3::Y)));
        for n in [
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(0.3, -1.0, 0.2),
            Vec3::new(-0.9, 0.1, 0.4),
        ] {
            let n = n.normalized().unwrap();
            let (u, v) = hatch_frame(n).unwrap();
            assert!(u.dot(n).abs() < 1e-12 && v.dot(n).abs() < 1e-12 && u.dot(v).abs() < 1e-12);
            assert!((u.length() - 1.0).abs() < 1e-12 && (v.length() - 1.0).abs() < 1e-12);
            assert!((u.cross(v) - n).length() < 1e-12, "right-handed");
        }
        // No direction, no frame; nor a hatching.
        assert_eq!(hatch_frame(Vec3::ZERO), None);
        assert_eq!(
            Hatching::new(Vec3::ZERO, PAPER, &hatch(None), &from_above(50.0), 400),
            None
        );
        // Sizes that are not numbers cover nothing.
        assert_eq!(hatch_coverage(f64::NAN, 0.5), 0.0);
        assert_eq!(hatch_coverage(0.0, f64::NAN), 0.0);
    }
}
