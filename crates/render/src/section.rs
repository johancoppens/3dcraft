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

/// Where over a step of the spacing the lines in between begin to fade:
/// the spacing is the largest power of two within the span the hatch
/// allows (`span / spacing` from 1 to 2), and from a ratio of 1.5 the
/// lines at odd multiples of the spacing lose their ink, down to none as
/// the spacing nears doubling. Zooming out, every second line fades away
/// before the spacing doubles; zooming in, the new lines fade in after it
/// halves. No line ever moves, and the density never doubles at once.
pub const HATCH_FADE_FROM: f64 = 1.5;

/// The ink of the lines at odd multiples of the spacing ([`HATCH_FADE_FROM`]),
/// 0 to 1, for the ratio of the span to the spacing (1 to 2).
pub fn hatch_odd_ink(ratio: f64) -> f64 {
    let ink = (2.0 - ratio) / (2.0 - HATCH_FADE_FROM);
    if ink.is_nan() {
        1.0
    } else {
        ink.clamp(0.0, 1.0)
    }
}

/// How much of the lines' own pattern is drawn where the lines stand
/// `across` pixels apart on screen, measured across them: all of it from
/// 3 pixels, none at 2 and below, where the picture could not show them
/// without moiré and the hatching is its mean tone instead.
pub fn hatch_fade(across: f64) -> f64 {
    let k = across - 2.0;
    if k.is_nan() { 0.0 } else { k.clamp(0.0, 1.0) }
}

/// How many classes of bodies the hatching of a cut tells apart: each
/// body's faces carry its class ([`HatchClasses`]), the renderers count
/// what is cut away per class (a channel each on the GPU), and the cap of
/// a body is hatched in its class's direction ([`hatch_direction`]), so
/// that parts that touch read apart (docs/design/rendering.md › *A
/// direction per part*).
pub const HATCH_CLASSES: usize = 4;

/// The bits of a mask of classes, one per class.
pub const HATCH_MASK: u8 = (1u8 << HATCH_CLASSES) - 1;

/// The direction across the lines of hatch class `class` in the plane's
/// frame ([`hatch_frame`]): `(cos, sin)` of its angle from `U` towards
/// `V`: 45°, 135°, 75° and 165° for the classes 0 to 3. Class 0 is the
/// hatching of a part as it always was; 0 and 1 are square to each
/// other, and every pair is at least 30° apart. A class beyond them
/// wraps. Square roots only, exact to the last bit on every platform.
pub fn hatch_direction(class: usize) -> (f64, f64) {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    // cos 75° and sin 75°: (√6 − √2) / 4 and (√6 + √2) / 4.
    let (s6, s2) = (6.0f64.sqrt(), std::f64::consts::SQRT_2);
    let (lo, hi) = ((s6 - s2) / 4.0, (s6 + s2) / 4.0);
    match class % HATCH_CLASSES {
        0 => (h, h),
        1 => (-h, h),
        2 => (lo, hi),
        _ => (-hi, lo),
    }
}

/// The hatch class of every face of a mesh ([`HATCH_CLASSES`]): from a
/// face id on, up to the next such id, the class of a body's faces; a
/// face before the first is of class 0. The engine gives each body of a
/// scene one (a body's faces are a run of ids); a mesh without any is all
/// of class 0 and is drawn as it always was.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HatchClasses {
    /// Ascending first face ids, each with its class.
    runs: Vec<(u32, u8)>,
}

impl HatchClasses {
    /// None: every face is of class 0.
    pub fn none() -> &'static HatchClasses {
        &NO_HATCH_CLASSES
    }

    /// The faces from `first` on are of `class` (modulo
    /// [`HATCH_CLASSES`]), up to the next run. False, and nothing changes,
    /// once [`crate::MAX_FACE_COLORS`] runs are held.
    pub fn set(&mut self, first: u32, class: usize) -> bool {
        let class = (class % HATCH_CLASSES) as u8;
        match self.runs.binary_search_by_key(&first, |&(f, _)| f) {
            Ok(at) => {
                if let Some(run) = self.runs.get_mut(at) {
                    run.1 = class;
                }
                true
            }
            Err(at) => {
                if self.runs.len() >= crate::MAX_FACE_COLORS {
                    return false;
                }
                self.runs.insert(at, (first, class));
                true
            }
        }
    }

    /// The class of `face`.
    pub fn class(&self, face: u32) -> usize {
        let at = self.runs.partition_point(|&(f, _)| f <= face);
        at.checked_sub(1)
            .and_then(|k| self.runs.get(k))
            .map_or(0, |&(_, c)| usize::from(c))
    }

    /// The bit of `face`'s class in a mask of classes.
    pub fn bit(&self, face: u32) -> u8 {
        1 << self.class(face)
    }

    /// Whether every face is of class 0.
    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|&(_, c)| c == 0)
    }

    /// The runs: each first face id with its class, ascending.
    pub fn runs(&self) -> impl Iterator<Item = (u32, usize)> + '_ {
        self.runs.iter().map(|&(f, c)| (f, usize::from(c)))
    }
}

/// [`HatchClasses::none`].
static NO_HATCH_CLASSES: HatchClasses = HatchClasses { runs: Vec::new() };

/// [`Hatch`] resolved for one picture: the plane's frame, the spacing for
/// the camera, the size of a pixel and the camera's axes. Both renderers
/// make it here, so they draw the same lines (`craft3d_gpu` hands its
/// numbers to the cap's shaders).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hatching {
    /// The plane's frame ([`hatch_frame`]) and its unit normal.
    pub u: Vec3,
    pub v: Vec3,
    pub normal: Vec3,
    /// How far apart the lines are, in model units: a power of two.
    pub spacing: f64,
    /// The ink of the lines at odd multiples of the spacing
    /// ([`hatch_odd_ink`]); the others have all of theirs.
    pub odd_ink: f64,
    /// Half a line's width, in pixels of the picture.
    pub half_width: f64,
    /// The model units one pixel covers: at distance 1 along the view in
    /// perspective (times a point's own distance), anywhere in an
    /// orthographic view.
    pub pixel: f64,
    pub perspective: bool,
    /// The camera: its eye and axes, for the footprint of a pixel on the
    /// plane ([`Self::footprint`]).
    pub eye: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
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
    /// it in steps of two, every line at one spacing is a line at half of
    /// it, and the lines in between fade over the step
    /// ([`HATCH_FADE_FROM`]). `None` when there is nothing to hatch: a
    /// camera that looks nowhere, sizes that are not numbers.
    pub fn new(
        normal: Vec3,
        paper: [u8; 3],
        hatch: &Hatch,
        camera: &Camera,
        height: u32,
    ) -> Option<Hatching> {
        let normal = normal.normalized()?;
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
        let span = hatch.spacing_points * per_point;
        let spacing = hatch_spacing(span)?;
        let half_width = 0.5 * hatch.width_points * pixels / points;
        (pixel.is_finite() && pixel > 0.0 && half_width.is_finite()).then_some(Hatching {
            u,
            v,
            normal,
            spacing,
            odd_ink: hatch_odd_ink(span / spacing),
            half_width,
            pixel,
            perspective,
            eye: axes.eye,
            right: axes.right,
            up: axes.up,
            forward: axes.forward,
            paper,
            ink: hatch.ink,
        })
    }

    /// The model units one pixel covers at `distance` along the view.
    fn pixel_at(&self, distance: f64) -> f64 {
        if self.perspective {
            self.pixel * distance
        } else {
            self.pixel
        }
    }

    /// How far across the lines (along `w`) the point of the plane under a
    /// pixel moves when the pixel moves by one, in model units, at the
    /// point `p` at `distance` along the view: the length of the gradient
    /// of `w` on screen. Worked out from the plane and the camera, as both
    /// renderers have them: a step `δ` across the screen (the camera's
    /// right or up, a pixel long) moves the ray, and its point on the plane
    /// by `δ − r (n·δ) / (n·r)`, `r` the ray's direction (`p` less the eye
    /// in perspective, the view's direction in an orthographic view).
    /// `None` where the plane is seen edge on. The lines of class 0.
    pub fn footprint(&self, p: Vec3, distance: f64) -> Option<f64> {
        self.footprint_of(p, distance, 0)
    }

    /// The unit direction across the lines of `class` ([`hatch_direction`])
    /// in model space: `w` grows along it. Class 0's is `(U + V) / √2`.
    pub fn across_of(&self, class: usize) -> Vec3 {
        if class == 0 {
            return (self.u + self.v) * std::f64::consts::FRAC_1_SQRT_2;
        }
        let (c, s) = hatch_direction(class);
        self.u * c + self.v * s
    }

    /// The coordinate of `p` across the lines of `class`: they lie at the
    /// whole multiples of the spacing of it. Class 0's is `(u + v) / √2`,
    /// worked out as it always was.
    pub fn w_of(&self, p: Vec3, class: usize) -> f64 {
        if class == 0 {
            return (p.dot(self.u) + p.dot(self.v)) * std::f64::consts::FRAC_1_SQRT_2;
        }
        p.dot(self.across_of(class))
    }

    /// [`Self::footprint`] across the lines of `class`.
    pub fn footprint_of(&self, p: Vec3, distance: f64, class: usize) -> Option<f64> {
        let t = self.across_of(class);
        let r = if self.perspective {
            p - self.eye
        } else {
            self.forward
        };
        let n = self.normal;
        let nr = n.dot(r);
        let edge_on = 1e-12 * r.length();
        if nr.is_nan() || edge_on.is_nan() || nr.abs() <= edge_on {
            return None;
        }
        let across = |step: Vec3| t.dot(step) - t.dot(r) * n.dot(step) / nr;
        let (x, y) = (across(self.right), across(self.up));
        let f = self.pixel_at(distance) * (x * x + y * y).sqrt();
        (f.is_finite() && f > 0.0).then_some(f)
    }

    /// How many pixels apart the lines stand on screen across them, at the
    /// point `p` at `distance` along the view; 0 where the plane is seen
    /// edge on.
    pub fn across(&self, p: Vec3, distance: f64) -> f64 {
        self.footprint(p, distance)
            .map_or(0.0, |f| self.spacing / f)
    }

    /// The hatching's mean tone at `distance` along the view: the lines'
    /// share of the plane, each line's ink over the spacing (a line one
    /// pixel of the point's scale wide over its ramps), 0 to 1.
    pub fn mean(&self, distance: f64) -> f64 {
        let q = self.pixel_at(distance);
        let a = 0.5 + self.half_width;
        // The integral of the ramp across a line, in pixels.
        let line = if a >= 1.0 {
            2.0 * self.half_width
        } else {
            a * a
        };
        let mean = q * line * (1.0 + self.odd_ink) / (2.0 * self.spacing);
        if mean.is_nan() {
            0.0
        } else {
            mean.clamp(0.0, 1.0)
        }
    }

    /// How much ink covers the pixel that shows the point `p` of the plane
    /// at `distance` along the view, 0 to 1. The lines' own pattern: a
    /// ramp one pixel wide at the point's scale ([`hatch_coverage`]) about
    /// each line, the lines at even multiples of the spacing at full ink
    /// and the others at [`Self::odd_ink`]; blended into the mean tone
    /// ([`Self::mean`]) where the lines stand under 3 pixels apart on
    /// screen ([`hatch_fade`], from the footprint across them,
    /// [`Self::footprint`]), all of it under 2. The lines of class 0.
    pub fn coverage(&self, p: Vec3, distance: f64) -> f64 {
        self.coverage_of(p, distance, 0)
    }

    /// The ink where the classes `mask` are cut (bit `k` for class `k`,
    /// [`HatchClasses`]): one class, its own lines ([`Self::coverage_of`]);
    /// two or more, bodies of different classes overlap there, a fault,
    /// and the cap is cross-hatched: the more ink of the lines of classes
    /// 0 and 1 (45° and 135°). No class: none.
    pub fn coverage_mask(&self, p: Vec3, distance: f64, mask: u8) -> f64 {
        let mask = mask & HATCH_MASK;
        match mask.count_ones() {
            0 => 0.0,
            1 => self.coverage_of(p, distance, mask.trailing_zeros() as usize),
            _ => self
                .coverage_of(p, distance, 0)
                .max(self.coverage_of(p, distance, 1)),
        }
    }

    /// [`Self::coverage`] with the lines of `class` ([`hatch_direction`]).
    pub fn coverage_of(&self, p: Vec3, distance: f64, class: usize) -> f64 {
        let q = self.pixel_at(distance);
        if !(q.is_finite() && q > 0.0) {
            return 0.0;
        }
        let mean = self.mean(distance);
        let Some(f) = self.footprint_of(p, distance, class) else {
            return mean;
        };
        let w = self.w_of(p, class);
        let two = 2.0 * self.spacing;
        let even = hatch_coverage(hatch_distance(w, two) / q, self.half_width);
        let odd = hatch_coverage(hatch_distance(w - self.spacing, two) / q, self.half_width);
        let line = even.max(self.odd_ink * odd);
        let across = self.spacing / f;
        let fade = hatch_fade(2.0 * across)
            + self.odd_ink * (hatch_fade(across) - hatch_fade(2.0 * across));
        let c = mean + fade * (line - mean);
        if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) }
    }

    /// The cap's colour at `p`: the paper mixed with the ink by
    /// [`Self::coverage`], rounded per channel. Unshaded. The coverage is
    /// taken to a whole number of 255ths first: an odd denominator, so the
    /// mix of two whole bytes is never a half and both renderers round it
    /// alike (the mean tone fell on halves, 221.5, which the GPU's
    /// conversion and the CPU's rounding took to different bytes). The
    /// lines of class 0.
    pub fn color(&self, p: Vec3, distance: f64) -> [u8; 3] {
        self.color_mask(p, distance, 1)
    }

    /// [`Self::color`] where the classes `mask` are cut
    /// ([`Self::coverage_mask`]).
    pub fn color_mask(&self, p: Vec3, distance: f64, mask: u8) -> [u8; 3] {
        let c = (self.coverage_mask(p, distance, mask) * 255.0).round() / 255.0;
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

    /// Seen nearly edge on, the lines stand closer than 2 pixels apart on
    /// screen across them: the hatching is its mean tone there, the same at
    /// every point (no moiré), the lines' share of the plane; between 2 and
    /// 3 pixels it is between the two.
    #[test]
    fn seen_nearly_edge_on_the_hatching_is_its_mean_tone() {
        let grazing = |height: f64| Camera {
            eye: Vec3::new(0.0, -100.0, height),
            target: Vec3::ZERO,
            up: Vec3::Z,
            projection: Projection::Orthographic { half_height: 50.0 },
        };
        let h = Hatching::new(Vec3::Z, PAPER, &hatch(None), &grazing(2.0), 400).unwrap();
        assert_eq!(h.spacing, 2.0);
        assert_eq!(h.odd_ink, 1.0);
        let p = |w: f64| {
            let r = std::f64::consts::SQRT_2;
            Vec3::new(w / r, w / r, 0.0)
        };
        assert!(h.across(p(1.0), 100.0) < 2.0, "{}", h.across(p(1.0), 100.0));
        // A quarter of a unit per pixel, a line one pixel wide every 2
        // units: an eighth.
        assert!((h.mean(100.0) - 0.125).abs() < 1e-12);
        for k in 0..200 {
            let w = f64::from(k) * 0.0137;
            assert_eq!(h.coverage(p(w), 100.0), h.mean(100.0), "{w}");
        }
        // Face on, the lines themselves.
        let face_on = Hatching::new(Vec3::Z, PAPER, &hatch(None), &from_above(50.0), 400).unwrap();
        assert!(face_on.across(p(1.0), 100.0) > 7.9);
        assert!((face_on.coverage(p(0.0), 100.0) - 1.0).abs() < 1e-12);
        assert_eq!(face_on.coverage(p(1.0), 100.0), 0.0);
        // In between: from the lines to the tone, continuously, as the
        // camera comes down to the plane.
        let mut last: Option<(f64, f64)> = None;
        for step in 0..=400 {
            let height = 100.0 * (1.0 - f64::from(step) / 400.0) + 1.0;
            let h = Hatching::new(Vec3::Z, PAPER, &hatch(None), &grazing(height), 400).unwrap();
            let across = h.across(p(0.0), 100.0);
            let c = h.coverage(p(0.0), 100.0);
            if across >= 3.0 {
                assert!((c - 1.0).abs() < 1e-12, "{height}: {c}");
            }
            if across <= 2.0 {
                assert!((c - h.mean(100.0)).abs() < 1e-12, "{height}: {c}");
            }
            if let Some((a, previous)) = last {
                // A small step of the camera, a small change of the ink.
                assert!(
                    (c - previous).abs() <= (a - across).abs() + 1e-9,
                    "{height}: {previous} {c}"
                );
            }
            last = Some((across, c));
        }
    }

    /// Zooming across a step of the spacing: the lines in between fade out
    /// before the spacing doubles and in after it halves, so the ink at a
    /// fixed point of the plane changes by little per percent of zoom, and
    /// the lines at even multiples keep all their ink.
    #[test]
    fn across_a_step_of_the_spacing_the_ink_changes_little() {
        let r = std::f64::consts::SQRT_2;
        // Points on lines at 1, 2, 3 and 4 units, between them, and on
        // ramps.
        let points: Vec<Vec3> = [1.0, 2.0, 3.0, 4.0, 0.5, 1.5, 2.25, 3.1, 3.9]
            .iter()
            .map(|&w| Vec3::new(w / r, w / r, 0.0))
            .collect();
        let mut previous: Option<(f64, Vec<f64>, f64)> = None;
        let mut steps = 0;
        let mut half_height = 30.0;
        while half_height < 130.0 {
            let h =
                Hatching::new(Vec3::Z, PAPER, &hatch(None), &from_above(half_height), 400).unwrap();
            let ink: Vec<f64> = points.iter().map(|&p| h.coverage(p, 100.0)).collect();
            // A line at an even multiple of the spacing: all of its ink.
            assert!((h.coverage(Vec3::ZERO, 100.0) - 1.0).abs() < 1e-12);
            if let Some((spacing, before, height)) = &previous {
                if *spacing != h.spacing {
                    steps += 1;
                }
                let percent = (half_height / height - 1.0) * 100.0;
                for (a, b) in before.iter().zip(&ink) {
                    assert!(
                        (a - b).abs() <= 0.06 * percent,
                        "{height} -> {half_height}: {a} -> {b}"
                    );
                }
            }
            previous = Some((h.spacing, ink, half_height));
            half_height *= 1.0025;
        }
        // From 30 to 130: the spacings 1, 2 and 4.
        assert!(steps >= 2, "{steps}");
    }

    /// The hatching of class 0 as it was before there were classes
    /// (2026-11-18): the coverage across `(U + V) / √2`, worked out step
    /// by step as `Hatching::coverage` and `Hatching::color` did.
    fn color_before_classes(h: &Hatching, p: Vec3, distance: f64) -> [u8; 3] {
        let coverage = || {
            let q = h.pixel_at(distance);
            if !(q.is_finite() && q > 0.0) {
                return 0.0;
            }
            let mean = h.mean(distance);
            let t = (h.u + h.v) * std::f64::consts::FRAC_1_SQRT_2;
            let r = if h.perspective { p - h.eye } else { h.forward };
            let n = h.normal;
            let nr = n.dot(r);
            let edge_on = 1e-12 * r.length();
            if nr.is_nan() || edge_on.is_nan() || nr.abs() <= edge_on {
                return mean;
            }
            let across = |step: Vec3| t.dot(step) - t.dot(r) * n.dot(step) / nr;
            let (x, y) = (across(h.right), across(h.up));
            let f = q * (x * x + y * y).sqrt();
            if !(f.is_finite() && f > 0.0) {
                return mean;
            }
            let w = (p.dot(h.u) + p.dot(h.v)) * std::f64::consts::FRAC_1_SQRT_2;
            let two = 2.0 * h.spacing;
            let even = hatch_coverage(hatch_distance(w, two) / q, h.half_width);
            let odd = hatch_coverage(hatch_distance(w - h.spacing, two) / q, h.half_width);
            let line = even.max(h.odd_ink * odd);
            let across = h.spacing / f;
            let fade = hatch_fade(2.0 * across)
                + h.odd_ink * (hatch_fade(across) - hatch_fade(2.0 * across));
            let c = mean + fade * (line - mean);
            if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) }
        };
        let c = (coverage() * 255.0).round() / 255.0;
        let mut out = h.paper;
        for (channel, ink) in out.iter_mut().zip(h.ink) {
            let paper = f64::from(*channel);
            *channel = (paper + (f64::from(ink) - paper) * c)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
        out
    }

    /// A body of class 0 (every body of a document with one) is hatched
    /// byte for byte as before the classes: orthographic and in
    /// perspective, face on, oblique and grazing (the fade), at two zooms.
    #[test]
    fn class_zero_is_hatched_byte_for_byte_as_before_the_classes() {
        let mut compared = 0;
        for (normal, camera) in [
            (Vec3::Z, from_above(50.0)),
            (Vec3::Z, from_above(37.3)),
            (
                Vec3::new(0.3, -0.5, 0.8),
                Camera {
                    eye: Vec3::new(30.0, -80.0, 60.0),
                    target: Vec3::new(1.0, 2.0, 0.5),
                    up: Vec3::Z,
                    projection: Projection::Perspective { fov_y_deg: 40.0 },
                },
            ),
            (
                Vec3::Z,
                Camera {
                    eye: Vec3::new(0.0, -100.0, 3.0),
                    target: Vec3::ZERO,
                    up: Vec3::Z,
                    projection: Projection::Orthographic { half_height: 50.0 },
                },
            ),
        ] {
            let h = Hatching::new(normal, PAPER, &hatch(None), &camera, 400).unwrap();
            for k in 0..400 {
                let t = f64::from(k);
                let p = Vec3::new(t.psin() * 13.7, (t * 0.37).pcos() * 9.1, t * 0.013);
                let distance = 50.0 + t * 0.25;
                let before = color_before_classes(&h, p, distance);
                assert_eq!(h.color(p, distance), before, "{p:?}");
                assert_eq!(h.color_mask(p, distance, 1), before, "{p:?}");
                compared += 1;
            }
        }
        assert_eq!(compared, 1600);
    }

    #[test]
    fn the_classes_directions_are_unit_and_at_least_30_degrees_apart() {
        let h = std::f64::consts::FRAC_1_SQRT_2;
        assert_eq!(hatch_direction(0), (h, h));
        assert_eq!(hatch_direction(1), (-h, h));
        let angle = |k: usize| {
            let (c, s) = hatch_direction(k);
            assert!((c * c + s * s - 1.0).abs() < 1e-15, "{k}");
            s.patan2(c).to_degrees()
        };
        let mut least = f64::INFINITY;
        for a in 0..HATCH_CLASSES {
            for b in a + 1..HATCH_CLASSES {
                // Lines, so modulo 180°.
                let d = (angle(a) - angle(b)).abs() % 180.0;
                least = least.min(d.min(180.0 - d));
            }
        }
        assert!((least - 30.0).abs() < 1e-9, "{least}");
        assert!(((angle(1) - angle(0)).abs() - 90.0).abs() < 1e-9);
        // A class beyond wraps.
        assert_eq!(hatch_direction(HATCH_CLASSES + 2), hatch_direction(2));
        // In model space: unit, in the plane, class 0 as it always was.
        let normal = Vec3::new(0.3, -0.5, 0.8);
        let hh = Hatching::new(normal, PAPER, &hatch(None), &from_above(50.0), 400).unwrap();
        for k in 0..HATCH_CLASSES {
            let t = hh.across_of(k);
            assert!((t.length() - 1.0).abs() < 1e-12, "{k}");
            assert!(t.dot(hh.normal).abs() < 1e-12, "{k}");
        }
        assert_eq!(hh.across_of(0), (hh.u + hh.v) * h);
    }

    /// Each class's lines lie where its coordinate is a whole number of
    /// spacings, face on: ink there, paper half a spacing off; where
    /// bodies of two classes overlap the cap is cross-hatched: the more
    /// ink of classes 0 and 1.
    #[test]
    fn each_class_has_its_lines_and_an_overlap_is_cross_hatched() {
        let h = Hatching::new(Vec3::Z, PAPER, &hatch(None), &from_above(50.0), 400).unwrap();
        assert_eq!(h.spacing, 2.0);
        for k in 0..HATCH_CLASSES {
            let t = h.across_of(k);
            let bit = 1 << k;
            for n in [-4.0, -2.0, 0.0, 2.0, 6.0] {
                let on = t * (n * h.spacing);
                assert!((h.w_of(on, k) - n * h.spacing).abs() < 1e-12);
                assert!(
                    (h.coverage_mask(on, 100.0, bit) - 1.0).abs() < 1e-12,
                    "{k} {n}"
                );
                assert_eq!(h.color_mask(on, 100.0, bit), INK, "{k} {n}");
                let off = t * ((n + 0.5) * h.spacing);
                assert_eq!(h.coverage_mask(off, 100.0, bit), 0.0, "{k} {n}");
                assert_eq!(h.color_mask(off, 100.0, bit), PAPER, "{k} {n}");
            }
        }
        // A point on a line of class 0 that is on none of class 1: the
        // classes' lines differ.
        let p = h.across_of(1) * 1.0;
        assert!((h.coverage_of(p, 100.0, 0) - 1.0).abs() < 1e-12);
        assert_eq!(h.coverage_of(p, 100.0, 1), 0.0);
        // Overlaps: the more of classes 0 and 1, whichever classes overlap.
        for k in 0..300 {
            let t = f64::from(k) * 0.0371;
            let p = Vec3::new(t.pcos() * t, t.psin() * t, 0.0);
            let cross = h.coverage_of(p, 100.0, 0).max(h.coverage_of(p, 100.0, 1));
            for mask in [0b11, 0b101, 0b1100, 0b1111] {
                assert_eq!(h.coverage_mask(p, 100.0, mask), cross, "{mask:b}");
            }
            assert_eq!(h.coverage_mask(p, 100.0, 0), 0.0);
            // Bits beyond the classes are not classes.
            assert_eq!(
                h.coverage_mask(p, 100.0, 0b1_0100),
                h.coverage_of(p, 100.0, 2)
            );
        }
    }

    #[test]
    fn hatch_classes_hold_runs_of_faces_in_any_order() {
        let mut c = HatchClasses::default();
        assert_eq!(c.class(0), 0);
        assert_eq!(c.class(u32::MAX), 0);
        assert!(c.is_empty());
        assert!(c.set(100, 1));
        assert!(c.set(10, 2));
        assert!(c.set(50, 7)); // wraps: 3
        assert!(c.set(0, 0));
        assert!(!c.is_empty());
        for (face, class) in [(0, 0), (9, 0), (10, 2), (49, 2), (50, 3), (99, 3), (100, 1)] {
            assert_eq!(c.class(face), class, "{face}");
            assert_eq!(c.bit(face), 1 << class, "{face}");
        }
        assert_eq!(c.class(u32::MAX), 1);
        // The same first face again: its class changes, nothing else.
        assert!(c.set(50, 0));
        assert_eq!(
            c.runs().collect::<Vec<_>>(),
            vec![(0, 0), (10, 2), (50, 0), (100, 1)]
        );
        assert_eq!(HatchClasses::none().class(12), 0);
        assert!(HatchClasses::none().is_empty());
        // Many bodies in order: each set is a push, every lookup right.
        let mut many = HatchClasses::default();
        for body in 0..10_000_u32 {
            assert!(many.set(body * 12, body as usize));
        }
        for body in 0..10_000_u32 {
            assert_eq!(many.class(body * 12 + 11), body as usize % HATCH_CLASSES);
        }
    }
}
